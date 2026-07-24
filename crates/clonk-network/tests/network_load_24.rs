use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clonk_engine::{
    ClientUpdateControlData, ControlPacket as EngineControlPacket, ControlPlayerInfoEntry,
    ControlPlayerInfoRegistry, LegacyCString, PlayerControlData, CLIENT_PLAYER_INFO_FLAG_INITIAL,
    CLIENT_UPDATE_ACTIVATE,
};
use clonk_network::{
    connect_client, connect_dual_client, decode_control_entry_payload, decode_control_packet,
    encode_control_entry_payload, encode_control_packet, ClientConfig, ClientEvent, ClientHandle,
    ClientPlayerInfosSnapshot, ControlDelivery, ControlPacket, HostConfig, HostEvent,
    LegacyControlFrame, NetworkProtocol, NetworkStatus, ParticipantKind, PlayerInfoListSnapshot,
    Tick, NETWORK_STATE_GO,
};
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{sleep_until, timeout, Instant};

const PLAYER_COUNT: usize = 24;
const HOST_CLIENT_ID: u32 = 0;
const CONTROL_TARGET_FPS: u32 = 38;
const NATIVE_GAME_TICK: Duration = Duration::from_millis(28);
const CONTROL_RATE: u32 = 2;
const WARMUP_SECONDS: u32 = 2;
const DEFAULT_MEASUREMENT_SECONDS: u64 = 60;
const CLEANUP_GRACE: Duration = Duration::from_secs(30);
const EVENT_WAIT: Duration = Duration::from_secs(10);
const MESH_WAIT: Duration = Duration::from_secs(30);
const LOOPBACK_P99_LIMIT_US: i64 = 25_000;
const LOOPBACK_RTT_P99_LIMIT_MS: i64 = 25;
const LOAD_WORKLOAD: &str =
    "same-process Tokio IPv4-loopback real-socket HarpoonRace-shaped control transport";
const LOAD_WORKLOAD_SCOPE: &str =
    "HarpoonRace-shaped lobby/control parameters only; no scenario/resource loading or game simulation";
const LOAD_SEQUENCE: &str =
    "synthetic max_players=24 JoinData -> 24 PlayerInfo joins -> activate all -> GO";
const LOAD_RTT_SCOPE: &str =
    "client-to-host ping samples only; all endpoints run in one Tokio process over IPv4 loopback";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum LoadTopology {
    Tcp,
    Udp,
    Relay,
}

impl LoadTopology {
    const fn is_direct_mesh(self) -> bool {
        !matches!(self, Self::Relay)
    }

    const fn is_direct_tcp_mesh(self) -> bool {
        matches!(self, Self::Tcp)
    }

    const fn preferred_message_protocol(self) -> NetworkProtocol {
        match self {
            Self::Udp => NetworkProtocol::Udp,
            Self::Tcp | Self::Relay => NetworkProtocol::Tcp,
        }
    }
}

fn parse_load_topology(
    topology: Option<&str>,
    legacy_direct_mesh: Option<&str>,
) -> Result<LoadTopology, String> {
    match topology {
        Some("tcp") => Ok(LoadTopology::Tcp),
        Some("udp") => Ok(LoadTopology::Udp),
        Some("relay") => Ok(LoadTopology::Relay),
        Some(value) => Err(format!(
            "LC_NETWORK_LOAD_TOPOLOGY must be tcp, udp, or relay; got {value:?}"
        )),
        None if legacy_direct_mesh == Some("0") => Ok(LoadTopology::Relay),
        None => Ok(LoadTopology::Tcp),
    }
}

fn load_topology_from_environment() -> LoadTopology {
    let topology = std::env::var("LC_NETWORK_LOAD_TOPOLOGY").ok();
    let legacy_direct_mesh = std::env::var("LC_NETWORK_LOAD_DIRECT_MESH").ok();
    parse_load_topology(topology.as_deref(), legacy_direct_mesh.as_deref())
        .unwrap_or_else(|error| panic!("{error}"))
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct MetricSummary {
    samples: usize,
    p50: Option<i64>,
    p95: Option<i64>,
    p99: Option<i64>,
    max: Option<i64>,
}

#[derive(Debug, Serialize)]
struct MetricSeries {
    unit: &'static str,
    summary: MetricSummary,
    raw_samples: Vec<i64>,
}

#[derive(Debug, Serialize)]
struct ClientMetricSeries {
    client_id: u32,
    metrics: MetricSeries,
}

impl MetricSeries {
    fn new(unit: &'static str, raw_samples: Vec<i64>) -> Self {
        Self {
            unit,
            summary: summarize(&raw_samples),
            raw_samples,
        }
    }
}

#[derive(Debug, Serialize)]
struct LoadFingerprint {
    source_commit: Option<String>,
    source_dirty: bool,
    content_revision: Option<String>,
    rustc: Option<String>,
    target_os: &'static str,
    target_arch: &'static str,
    cpu: Option<String>,
    os_version: Option<String>,
    cargo_profile: &'static str,
}

#[derive(Debug, Serialize)]
struct ProcessRuntimeSample {
    elapsed_ms: u64,
    process_client_id: u32,
    route_count: usize,
    tcp_input_rate: u64,
    tcp_output_rate: u64,
    udp_input_rate: u64,
    udp_output_rate: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct PreferredMessageRoute {
    process_client_id: u32,
    peer_client_id: u32,
    protocol: String,
}

#[derive(Debug, Serialize)]
struct NetworkLoadReport {
    schema_version: u32,
    workload: &'static str,
    workload_scope: &'static str,
    sequence: &'static str,
    round_trip_scope: &'static str,
    authoritative_duration: bool,
    topology: LoadTopology,
    preferred_message_protocol: &'static str,
    /// Retained for consumers of schema v3. New consumers should use
    /// `topology`, because direct reliable UDP is also a full mesh.
    direct_tcp_mesh: bool,
    player_profiles_joined: usize,
    host_player_profiles: usize,
    active_control_participants: usize,
    control_target_fps: u32,
    native_game_tick_ms: u64,
    native_control_interval_ms: u64,
    control_rate: u32,
    warmup_ticks: u32,
    requested_measurement_ms: u64,
    measurement_wall_elapsed_ms: u64,
    minimum_native_control_ticks: usize,
    measured_ticks: usize,
    expected_ready_deliveries: usize,
    observed_ready_deliveries: usize,
    mesh_establishment_us: Option<i64>,
    final_route_peers: Vec<(u32, Vec<u32>)>,
    final_preferred_message_routes: Vec<PreferredMessageRoute>,
    join_duration: MetricSeries,
    client_to_host_round_trip: MetricSeries,
    client_to_host_round_trip_by_client: Vec<ClientMetricSeries>,
    control_completion_wait: MetricSeries,
    participant_ready: MetricSeries,
    cadence_lateness: MetricSeries,
    native_control_wait: MetricSeries,
    runtime_samples: Vec<ProcessRuntimeSample>,
    fingerprint: LoadFingerprint,
    result: &'static str,
    assertions: Vec<LoadAssertion>,
}

#[derive(Debug, Serialize)]
struct LoadAssertion {
    name: String,
    passed: bool,
    detail: String,
}

#[derive(Debug)]
enum ProbeEvent {
    Status(NetworkStatus),
    StatusAck(NetworkStatus),
    Ready {
        packet: ControlPacket,
        observed_at: Instant,
    },
    Failure(String),
}

struct ClientProbe {
    client_id: u32,
    handle: ClientHandle,
    events: mpsc::UnboundedReceiver<ProbeEvent>,
    rtt_samples_ms: Arc<Mutex<Vec<i64>>>,
    player_infos: Arc<Mutex<ControlPlayerInfoRegistry>>,
    collector: tokio::task::JoinHandle<()>,
}

impl ClientProbe {
    fn new(mut handle: ClientHandle, initial_player_infos: ControlPlayerInfoRegistry) -> Self {
        let client_id = handle.client_id();
        let mut source = handle.take_event_receiver();
        let (event_tx, events) = mpsc::unbounded_channel();
        let rtt_samples_ms = Arc::new(Mutex::new(Vec::new()));
        let collector_rtt = Arc::clone(&rtt_samples_ms);
        let player_infos = Arc::new(Mutex::new(initial_player_infos));
        let collector_player_infos = Arc::clone(&player_infos);
        let collector = tokio::spawn(async move {
            while let Some(event) = source.recv().await {
                let forwarded = match event {
                    ClientEvent::PingMeasured { round_trip_ms } => {
                        if round_trip_ms >= 0 {
                            collector_rtt
                                .lock()
                                .expect("RTT sample lock poisoned")
                                .push(i64::from(round_trip_ms));
                        }
                        None
                    }
                    ClientEvent::Direct { data, .. } => {
                        if let Ok(EngineControlPacket::PlayerInfo(info)) =
                            decode_control_entry_payload(&data)
                        {
                            apply_observed_player_info(
                                &mut collector_player_infos
                                    .lock()
                                    .expect("player-info roster lock poisoned"),
                                info,
                            );
                        }
                        None
                    }
                    ClientEvent::Status(status) => Some(ProbeEvent::Status(status)),
                    ClientEvent::StatusAck(status) => Some(ProbeEvent::StatusAck(status)),
                    ClientEvent::Ready { packet } => Some(ProbeEvent::Ready {
                        packet,
                        observed_at: Instant::now(),
                    }),
                    ClientEvent::Disconnected { reason } => Some(ProbeEvent::Failure(format!(
                        "client {client_id} disconnected: {reason:?}"
                    ))),
                    ClientEvent::UnhandledPacket { packet_type } => {
                        Some(ProbeEvent::Failure(format!(
                            "client {client_id} received unhandled packet type {packet_type:#04x}"
                        )))
                    }
                    ClientEvent::ResourceLoadFailed { resource_id } => Some(ProbeEvent::Failure(
                        format!("client {client_id} failed to load resource {resource_id}"),
                    )),
                    ClientEvent::ResourceDeriveUnsupported { core } => Some(ProbeEvent::Failure(
                        format!("client {client_id} could not derive resource {}", core.id),
                    )),
                    _ => None,
                };
                if forwarded.is_some_and(|event| event_tx.send(event).is_err()) {
                    return;
                }
            }
        });
        Self {
            client_id,
            handle,
            events,
            rtt_samples_ms,
            player_infos,
            collector,
        }
    }

    async fn wait_for_status(&mut self, expected: NetworkStatus) {
        loop {
            match timeout(EVENT_WAIT, self.events.recv()).await {
                Ok(Some(ProbeEvent::Status(status))) if status == expected => return,
                Ok(Some(ProbeEvent::Failure(error))) => panic!("{error}"),
                Ok(Some(_)) => continue,
                Ok(None) => panic!("client {} probe stream ended", self.client_id),
                Err(_) => panic!(
                    "timed out waiting for client {} status {expected:?}",
                    self.client_id
                ),
            }
        }
    }

    async fn wait_for_ready(&mut self, expected_tick: Tick) -> (ControlPacket, Instant) {
        loop {
            match timeout(EVENT_WAIT, self.events.recv()).await {
                Ok(Some(ProbeEvent::Ready {
                    packet,
                    observed_at,
                })) if packet.tick() == expected_tick => return (packet, observed_at),
                Ok(Some(ProbeEvent::Ready { packet, .. })) => panic!(
                    "client {} produced tick {} while waiting for {expected_tick}",
                    self.client_id,
                    packet.tick()
                ),
                Ok(Some(ProbeEvent::Failure(error))) => panic!("{error}"),
                Ok(Some(_)) => continue,
                Ok(None) => panic!("client {} probe stream ended", self.client_id),
                Err(_) => panic!(
                    "timed out waiting for client {} ready tick {expected_tick}",
                    self.client_id
                ),
            }
        }
    }

    async fn wait_for_status_ack(&mut self, expected: NetworkStatus) {
        loop {
            match timeout(EVENT_WAIT, self.events.recv()).await {
                Ok(Some(ProbeEvent::StatusAck(status))) if status == expected => return,
                Ok(Some(ProbeEvent::Failure(error))) => panic!("{error}"),
                Ok(Some(_)) => continue,
                Ok(None) => panic!("client {} probe stream ended", self.client_id),
                Err(_) => panic!(
                    "timed out waiting for client {} status acknowledgement {expected:?}",
                    self.client_id
                ),
            }
        }
    }

    fn clear_rtt_samples(&self) {
        self.rtt_samples_ms
            .lock()
            .expect("RTT sample lock poisoned")
            .clear();
    }

    fn rtt_samples(&self) -> Vec<i64> {
        self.rtt_samples_ms
            .lock()
            .expect("RTT sample lock poisoned")
            .clone()
    }

    fn has_exact_synthetic_roster(&self) -> bool {
        has_exact_synthetic_roster(
            &self
                .player_infos
                .lock()
                .expect("player-info roster lock poisoned"),
        )
    }

    async fn shutdown(self) -> Result<(), String> {
        self.handle
            .shutdown()
            .await
            .map_err(|error| format!("client {} shutdown failed: {error}", self.client_id))?;
        self.collector.await.map_err(|error| {
            format!(
                "client {} event collector failed to join: {error}",
                self.client_id
            )
        })?;
        Ok(())
    }
}

#[derive(Debug)]
struct TickMeasurement {
    completion_us: i64,
    ready_us: Vec<i64>,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "explicit 24-player real-socket load benchmark; takes at least 62 seconds"]
async fn harpoonrace_shaped_24_player_control_transport_sustains_lockstep() {
    let measurement_seconds = std::env::var("LC_NETWORK_LOAD_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds != 0)
        .unwrap_or(DEFAULT_MEASUREMENT_SECONDS);
    let topology = load_topology_from_environment();
    let setup_budget = if topology.is_direct_mesh() {
        MESH_WAIT + Duration::from_secs(60)
    } else {
        Duration::from_secs(60)
    };
    let total_budget = setup_budget
        + Duration::from_secs(u64::from(WARMUP_SECONDS))
        + Duration::from_secs(measurement_seconds)
        + CLEANUP_GRACE;

    timeout(
        total_budget,
        run_harpoonrace_shaped_24_player_load(measurement_seconds, topology),
    )
    .await
    .unwrap_or_else(|_| {
        panic!(
            "24-player load harness exceeded its bounded {total_budget:?} setup/measurement budget"
        )
    });
}

async fn run_harpoonrace_shaped_24_player_load(measurement_seconds: u64, topology: LoadTopology) {
    // This transport-only workload directly configures synthetic JoinData with
    // HarpoonRace-shaped MaxPlayer/ControlRate values. It does not execute
    // `/set`, load scenario resources, or simulate the game. Every activated
    // participant contributes a synthetic control, and native PackCompleteCtrl
    // waits for all clients before packing in client-ID order
    // (oracle src/C4GameControlNetwork.cpp:156-179,741-783).
    eprintln!("LC_NETWORK_LOAD_24 phase=host_setup");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HarpoonRace-shaped transport-test host");
    let host_address = listener.local_addr().expect("host listener address");
    let mut host_config = HostConfig::default();
    host_config.max_players = PLAYER_COUNT;
    configure_host_transport(&mut host_config, topology);
    let host_name = legacy_string("LoadHost");
    host_config.local_core.name = host_name.clone();
    host_config.local_core.nick = host_name;
    {
        let join_snapshot = host_config
            .initial_join_snapshot
            .as_mut()
            .expect("load test uses the synthetic socket JoinData");
        join_snapshot.parameters.max_players = PLAYER_COUNT as i32;
        join_snapshot.parameters.control_rate = CONTROL_RATE as i32;
        join_snapshot.parameters.title = legacy_string("HarpoonRace");
        join_snapshot.parameters.scenario.filename = legacy_string("HarpoonRace.c4s");
        join_snapshot.parameters.clients.clients[0] = host_config.local_core.clone();
    }
    let mut published_join_snapshot = host_config
        .initial_join_snapshot
        .clone()
        .expect("load test retains its synthetic JoinData snapshot");

    let mut host = clonk_network::start_host(listener, host_config)
        .await
        .expect("start HarpoonRace-shaped transport-test host");
    let mut host_events = host.take_event_receiver();
    let mut player_infos = ControlPlayerInfoRegistry::default();
    let mut probes = Vec::with_capacity(PLAYER_COUNT);
    let mut join_duration_us = Vec::with_capacity(PLAYER_COUNT);

    eprintln!("LC_NETWORK_LOAD_24 phase=lobby_join");
    for player_index in 1..=PLAYER_COUNT {
        let player_name = format!("LoadPlayer{player_index:02}");
        let client_config = configure_client_transport(
            ClientConfig::new(&player_name, ParticipantKind::Player),
            topology,
        );
        let join_started = Instant::now();
        let mut client = timeout(EVENT_WAIT, async {
            match topology {
                LoadTopology::Udp => {
                    let udp_address = host
                        .udp_local_addr()
                        .expect("UDP load topology starts a reliable-UDP host listener");
                    connect_dual_client(host_address, udp_address, client_config).await
                }
                LoadTopology::Tcp | LoadTopology::Relay => {
                    connect_client(host_address, client_config).await
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("timed out connecting {player_name}"))
        .unwrap_or_else(|error| panic!("connect {player_name}: {error}"));
        join_duration_us.push(duration_us(join_started.elapsed()));
        let client_id = client.client_id();
        let join_data = client
            .take_join_data()
            .expect("connected client retains its initial JoinData");
        let initial_status = join_data.status;
        assert_eq!(
            client_id, player_index as u32,
            "fresh host must allocate contiguous C++ client IDs"
        );
        let mut probe = ClientProbe::new(
            client,
            player_info_registry_from_snapshot(&join_data.parameters.player_infos),
        );
        wait_for_host_join(&mut host_events, client_id).await;
        probe
            .handle
            .submit_status_ack(initial_status)
            .await
            .expect("acknowledge initial JoinData lobby status");
        wait_for_host_status_ack(&mut host_events, client_id, initial_status).await;
        probe.wait_for_status_ack(initial_status).await;

        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: client_id as i32,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![ControlPlayerInfoEntry {
                name: legacy_string(&player_name),
                filename: legacy_string(&format!("{player_name}.c4p")),
                league_progress_data_is_null: false,
                ..Default::default()
            }],
        };
        probe
            .handle
            .submit_player_info_update(request)
            .await
            .unwrap_or_else(|error| panic!("submit {player_name} player info: {error}"));
        let request = wait_for_player_info(&mut host_events, client_id).await;
        assert_eq!(
            request.players.len(),
            1,
            "every transport participant must carry one real player profile"
        );
        let admitted = player_infos
            .admit_request(request, PLAYER_COUNT)
            .unwrap_or_else(|| panic!("host rejected {player_name} inside MaxPlayer=24"));
        assert_eq!(admitted.players.len(), 1);
        host.submit_packet(
            ControlDelivery::Direct,
            encode_control_entry_payload(&EngineControlPacket::PlayerInfo(admitted.clone()))
                .expect("encode authoritative PlayerInfo"),
        )
        .await
        .expect("broadcast authoritative PlayerInfo");
        player_infos.apply(admitted);
        publish_player_info_snapshot(&host, &mut published_join_snapshot, &player_infos).await;
        timeout(
            EVENT_WAIT,
            activate_client(&host, &mut host_events, client_id),
        )
        .await
        .unwrap_or_else(|_| panic!("activation phase timed out for client {client_id}"));
        probes.push(probe);
        eprintln!("LC_NETWORK_LOAD_24 phase=lobby_join joined={player_index}/{PLAYER_COUNT}");
    }

    assert_eq!(player_infos.nonremoved_player_count(), PLAYER_COUNT);
    let host_logical_peers = host
        .runtime_connections()
        .await
        .expect("inspect host routes")
        .into_iter()
        .map(|route| route.client_id)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        host_logical_peers,
        (1..=PLAYER_COUNT as u32).collect::<BTreeSet<_>>(),
        "host must retain at least one selected transport route per joined player"
    );
    wait_for_exact_synthetic_rosters(&probes).await;

    let mesh_started = Instant::now();
    let mesh_establishment_us = if topology.is_direct_mesh() {
        eprintln!("LC_NETWORK_LOAD_24 phase=mesh topology={topology:?}");
        wait_for_full_mesh(&host, &probes, topology).await;
        Some(duration_us(mesh_started.elapsed()))
    } else {
        None
    };

    eprintln!("LC_NETWORK_LOAD_24 phase=go");
    let running = NetworkStatus {
        state: NETWORK_STATE_GO,
        control_mode: 0,
        target_tick: 0,
    };
    host.begin_go(running, false)
        .await
        .expect("close joins and begin GO");
    for probe in &mut probes {
        probe.wait_for_status(running).await;
        probe
            .handle
            .submit_status_ack(running)
            .await
            .expect("acknowledge GO");
    }
    host.status_reached(running, running.target_tick)
        .await
        .expect("host reaches GO");
    wait_for_status_commit(&mut host_events, running).await;
    for probe in &mut probes {
        probe.wait_for_status_ack(running).await;
    }

    let control_interval = native_control_interval();
    let warmup_ticks = u32::try_from(minimum_native_control_ticks(Duration::from_secs(
        u64::from(WARMUP_SECONDS),
    )))
    .expect("warmup control tick count fits u32");
    eprintln!("LC_NETWORK_LOAD_24 phase=warmup ticks={warmup_ticks}");
    let mut scheduled = Instant::now();
    for tick in 0..warmup_ticks {
        sleep_until(scheduled).await;
        let _ = timeout(
            EVENT_WAIT,
            drive_tick(&host, &mut host_events, &mut probes, tick, scheduled),
        )
        .await
        .unwrap_or_else(|_| panic!("warmup control tick {tick} exceeded {EVENT_WAIT:?}"));
        scheduled += control_interval;
    }
    let _ = host
        .runtime_client_states(warmup_ticks, true)
        .await
        .expect("reset host wait metrics after warmup");
    for probe in &probes {
        let _ = probe
            .handle
            .runtime_client_states(warmup_ticks, true)
            .await
            .expect("reset client wait metrics after warmup");
        probe.clear_rtt_samples();
    }

    let measurement_started = Instant::now();
    eprintln!("LC_NETWORK_LOAD_24 phase=measurement seconds={measurement_seconds}");
    let measurement_deadline = measurement_started + Duration::from_secs(measurement_seconds);
    scheduled = measurement_started;
    let mut tick = warmup_ticks;
    let mut control_completion_us = Vec::new();
    let mut participant_ready_us = Vec::new();
    let mut cadence_lateness_us = Vec::new();
    let mut native_wait_ms = Vec::new();
    let mut runtime_samples = Vec::new();
    let mut next_runtime_sample = measurement_started + Duration::from_secs(1);

    while scheduled < measurement_deadline {
        sleep_until(scheduled).await;
        cadence_lateness_us.push(duration_us(
            Instant::now().saturating_duration_since(scheduled),
        ));
        let measurement = timeout(
            EVENT_WAIT,
            drive_tick(&host, &mut host_events, &mut probes, tick, scheduled),
        )
        .await
        .unwrap_or_else(|_| panic!("measured control tick {tick} exceeded {EVENT_WAIT:?}"));
        control_completion_us.push(measurement.completion_us);
        participant_ready_us.extend(measurement.ready_us);
        tick += 1;
        scheduled += control_interval;

        if Instant::now() >= next_runtime_sample {
            timeout(
                EVENT_WAIT,
                sample_runtime(
                    &host,
                    &probes,
                    tick,
                    measurement_started,
                    &mut native_wait_ms,
                    &mut runtime_samples,
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("runtime sample at tick {tick} exceeded {EVENT_WAIT:?}"));
            next_runtime_sample += Duration::from_secs(1);
        }
    }

    // Capture the measured wall interval before final telemetry or report I/O.
    // The last control is scheduled one native interval before the deadline.
    let measurement_wall_elapsed = measurement_started.elapsed();
    timeout(
        EVENT_WAIT,
        sample_runtime(
            &host,
            &probes,
            tick,
            measurement_started,
            &mut native_wait_ms,
            &mut runtime_samples,
        ),
    )
    .await
    .unwrap_or_else(|_| panic!("final runtime sample at tick {tick} exceeded {EVENT_WAIT:?}"));
    let client_to_host_round_trip_by_client = probes
        .iter()
        .map(|probe| ClientMetricSeries {
            client_id: probe.client_id,
            metrics: MetricSeries::new("milliseconds", probe.rtt_samples()),
        })
        .collect::<Vec<_>>();
    let round_trip_ms = client_to_host_round_trip_by_client
        .iter()
        .flat_map(|series| series.metrics.raw_samples.iter().copied())
        .collect::<Vec<_>>();
    let final_preferred_message_routes = preferred_message_routes(&host, &probes).await;
    let final_route_peers = route_peers(&final_preferred_message_routes);
    let measured_ticks = control_completion_us.len();
    let requested_measurement = Duration::from_secs(measurement_seconds);
    let minimum_native_control_ticks = minimum_native_control_ticks(requested_measurement);
    let expected_ready_deliveries = measured_ticks * (PLAYER_COUNT + 1);
    let mut report = NetworkLoadReport {
        schema_version: 4,
        workload: LOAD_WORKLOAD,
        workload_scope: LOAD_WORKLOAD_SCOPE,
        sequence: LOAD_SEQUENCE,
        round_trip_scope: LOAD_RTT_SCOPE,
        authoritative_duration: measurement_seconds >= DEFAULT_MEASUREMENT_SECONDS,
        topology,
        preferred_message_protocol: protocol_name(topology.preferred_message_protocol()),
        direct_tcp_mesh: topology.is_direct_tcp_mesh(),
        player_profiles_joined: player_infos.nonremoved_player_count(),
        host_player_profiles: 0,
        active_control_participants: PLAYER_COUNT + 1,
        control_target_fps: CONTROL_TARGET_FPS,
        native_game_tick_ms: NATIVE_GAME_TICK.as_millis() as u64,
        native_control_interval_ms: control_interval.as_millis() as u64,
        control_rate: CONTROL_RATE,
        warmup_ticks,
        requested_measurement_ms: requested_measurement.as_millis() as u64,
        measurement_wall_elapsed_ms: measurement_wall_elapsed.as_millis() as u64,
        minimum_native_control_ticks,
        measured_ticks,
        expected_ready_deliveries,
        observed_ready_deliveries: participant_ready_us.len(),
        mesh_establishment_us,
        final_route_peers,
        final_preferred_message_routes,
        join_duration: MetricSeries::new("microseconds", join_duration_us),
        client_to_host_round_trip: MetricSeries::new("milliseconds", round_trip_ms),
        client_to_host_round_trip_by_client,
        control_completion_wait: MetricSeries::new("microseconds", control_completion_us),
        participant_ready: MetricSeries::new("microseconds", participant_ready_us),
        cadence_lateness: MetricSeries::new("microseconds", cadence_lateness_us),
        native_control_wait: MetricSeries::new("milliseconds", native_wait_ms),
        runtime_samples,
        fingerprint: fingerprint(),
        result: "pending",
        assertions: Vec::new(),
    };
    report.assertions = evaluate_load_assertions(
        &report,
        requested_measurement,
        measurement_wall_elapsed,
        topology,
    );
    eprintln!("LC_NETWORK_LOAD_24 phase=cleanup");
    drop(host_events);
    let cleanup = timeout(CLEANUP_GRACE, shutdown_session(host, probes)).await;
    let (cleanup_passed, cleanup_detail) = match cleanup {
        Ok(Ok(())) => (true, "all host/client tasks joined".to_string()),
        Ok(Err(error)) => (false, error),
        Err(_) => (
            false,
            format!("network load cleanup exceeded {CLEANUP_GRACE:?}"),
        ),
    };
    report.assertions.push(LoadAssertion {
        name: "clean-shutdown".to_string(),
        passed: cleanup_passed,
        detail: cleanup_detail,
    });
    eprintln!("LC_NETWORK_LOAD_24 phase=complete");
    report.result = load_assertion_result(&report.assertions);
    let report_path = write_report(&report);
    println!(
        "LC_NETWORK_LOAD_24 report={} result={} players={} ticks={} wall_elapsed_ms={} join_p99_ms={:.3} client_host_rtt_p99_ms={} control_wait_p99_ms={:.3}",
        report_path.display(),
        report.result,
        report.player_profiles_joined,
        report.measured_ticks,
        report.measurement_wall_elapsed_ms,
        micros_to_millis(report.join_duration.summary.p99),
        report.client_to_host_round_trip.summary.p99.unwrap_or(-1),
        micros_to_millis(report.control_completion_wait.summary.p99),
    );
    let failed = report
        .assertions
        .iter()
        .filter(|assertion| !assertion.passed)
        .map(|assertion| format!("{}: {}", assertion.name, assertion.detail))
        .collect::<Vec<_>>();
    assert!(
        failed.is_empty(),
        "network load acceptance failed; report={}: {}",
        report_path.display(),
        failed.join("; ")
    );
}

async fn drive_tick(
    host: &clonk_network::HostHandle,
    host_events: &mut mpsc::Receiver<HostEvent>,
    probes: &mut [ClientProbe],
    tick: Tick,
    reached_at: Instant,
) -> TickMeasurement {
    host.control_tick_reached(
        tick,
        CONTROL_RATE as i32,
        CONTROL_TARGET_FPS as i32,
        reached_at,
    )
    .await
    .expect("stamp host control cadence");
    for probe in probes.iter() {
        probe
            .handle
            .control_tick_reached(tick, reached_at)
            .await
            .expect("stamp client control cadence");
    }

    let submitted_at = Instant::now();
    host.submit_local_control(control_contribution(HOST_CLIENT_ID, tick))
        .await
        .expect("submit host control contribution");
    for probe in probes.iter() {
        probe
            .handle
            .submit_control(control_contribution(probe.client_id, tick))
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "submit client {} control tick {tick}: {error}",
                    probe.client_id
                )
            });
    }

    let (host_ready, host_ready_at) = wait_for_host_ready(host_events, tick).await;
    let mut ready = vec![(host_ready, host_ready_at)];
    for probe in probes.iter_mut() {
        ready.push(probe.wait_for_ready(tick).await);
    }
    let expected = &ready[0].0;
    for (participant_index, (packet, _)) in ready.iter().enumerate().skip(1) {
        assert_eq!(
            packet, expected,
            "participant {participant_index} desynchronized at control tick {tick}"
        );
    }
    validate_aggregate(expected, tick);

    let consumed_at = ready
        .iter()
        .map(|(_, observed_at)| *observed_at)
        .max()
        .expect("host and clients all produced Ready");
    let active_ids = (0..=PLAYER_COUNT as u32).collect::<Vec<_>>();
    host.control_tick_consumed(tick, consumed_at, active_ids.clone(), false)
        .await
        .expect("record host control completion");
    for probe in probes.iter() {
        probe
            .handle
            .control_tick_consumed(tick, consumed_at, active_ids.clone(), false)
            .await
            .expect("record client control completion");
    }

    TickMeasurement {
        completion_us: duration_us(consumed_at.saturating_duration_since(submitted_at)),
        ready_us: ready
            .into_iter()
            .map(|(_, observed_at)| {
                duration_us(observed_at.saturating_duration_since(submitted_at))
            })
            .collect(),
    }
}

fn control_contribution(client_id: u32, tick: Tick) -> ControlPacket {
    let controls = (client_id != HOST_CLIENT_ID)
        .then(|| {
            EngineControlPacket::PlayerControl(PlayerControlData {
                player: client_id as i32,
                command: (tick % 16) as i32,
                data: tick as i32,
                by_client: client_id as i32,
            })
        })
        .into_iter()
        .collect();
    encode_control_packet(&LegacyControlFrame {
        client_id,
        tick,
        timestamp_ms: 0,
        controls,
    })
    .expect("encode load-test control contribution")
}

fn validate_aggregate(packet: &ControlPacket, tick: Tick) {
    let decoded = decode_control_packet(packet).expect("decode complete load-test control");
    assert_eq!(decoded.tick, tick);
    assert_eq!(decoded.controls.len(), PLAYER_COUNT);
    for (expected_client, control) in (1..=PLAYER_COUNT as i32).zip(decoded.controls) {
        let EngineControlPacket::PlayerControl(control) = control else {
            panic!("tick {tick} contained a non-player control");
        };
        assert_eq!(control.player, expected_client);
        assert_eq!(control.by_client, expected_client);
        assert_eq!(control.data, tick as i32);
    }
}

async fn wait_for_host_join(events: &mut mpsc::Receiver<HostEvent>, expected_client: u32) {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::ClientJoined {
                client_id, kind, ..
            })) if client_id == expected_client => {
                assert_eq!(kind, ParticipantKind::Player);
                return;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before client {expected_client} joined: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left while client {expected_client} was joining")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before client {expected_client} joined"),
            Err(_) => panic!("timed out waiting for client {expected_client} to join"),
        }
    }
}

async fn wait_for_player_info(
    events: &mut mpsc::Receiver<HostEvent>,
    expected_client: u32,
) -> clonk_network::PlayerInfoUpdateRequest {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::PlayerInfoUpdate { client_id, request }))
                if client_id == expected_client =>
            {
                assert_eq!(request.client_id, expected_client as i32);
                return request;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before client {expected_client} PlayerInfo: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left before client {expected_client} PlayerInfo")
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                panic!("host event stream ended before client {expected_client} PlayerInfo")
            }
            Err(_) => panic!("timed out waiting for client {expected_client} PlayerInfo"),
        }
    }
}

async fn wait_for_host_status_ack(
    events: &mut mpsc::Receiver<HostEvent>,
    expected_client: u32,
    expected_status: NetworkStatus,
) {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::StatusAck { client_id, status }))
                if client_id == expected_client && status == expected_status =>
            {
                return;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!(
                    "transport error before client {expected_client} acknowledged {expected_status:?}: {error}"
                )
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left before initial lobby acknowledgement")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!(
                "host event stream ended before client {expected_client} acknowledged lobby status"
            ),
            Err(_) => panic!(
                "timed out waiting for client {expected_client} lobby status acknowledgement"
            ),
        }
    }
}

async fn activate_client(
    host: &clonk_network::HostHandle,
    events: &mut mpsc::Receiver<HostEvent>,
    client_id: u32,
) {
    let update = ClientUpdateControlData {
        update_type: CLIENT_UPDATE_ACTIVATE,
        client_id: client_id as i32,
        data: 1,
        by_client: HOST_CLIENT_ID as i32,
    };
    host.submit_packet(
        ControlDelivery::Sync,
        encode_control_entry_payload(&EngineControlPacket::ClientUpdate(update.clone()))
            .expect("encode client activation"),
    )
    .await
    .expect("submit client activation");
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::SyncScheduled { controls, .. }))
                if controls == vec![EngineControlPacket::ClientUpdate(update.clone())] =>
            {
                return;
            }
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error while activating client {client_id}: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left during activation")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended while activating client {client_id}"),
            Err(_) => panic!("timed out activating client {client_id}"),
        }
    }
}

async fn wait_for_status_commit(events: &mut mpsc::Receiver<HostEvent>, expected: NetworkStatus) {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::StatusCommitted(status))) if status == expected => return,
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before GO committed: {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left before GO committed")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before GO committed"),
            Err(_) => panic!("timed out waiting for GO commit"),
        }
    }
}

async fn wait_for_host_ready(
    events: &mut mpsc::Receiver<HostEvent>,
    expected_tick: Tick,
) -> (ControlPacket, Instant) {
    loop {
        match timeout(EVENT_WAIT, events.recv()).await {
            Ok(Some(HostEvent::Ready { packet })) if packet.tick() == expected_tick => {
                return (packet, Instant::now());
            }
            Ok(Some(HostEvent::Ready { packet })) => panic!(
                "host produced tick {} while waiting for {expected_tick}",
                packet.tick()
            ),
            Ok(Some(HostEvent::TransportError { error, .. })) => {
                panic!("transport error before host Ready({expected_tick}): {error}")
            }
            Ok(Some(HostEvent::ClientLeft { client_id }))
            | Ok(Some(HostEvent::ClientConnectionFailed { client_id })) => {
                panic!("client {client_id} left before host Ready({expected_tick})")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("host event stream ended before Ready({expected_tick})"),
            Err(_) => panic!("timed out waiting for host Ready({expected_tick})"),
        }
    }
}

fn player_info_registry_from_snapshot(
    snapshot: &PlayerInfoListSnapshot,
) -> ControlPlayerInfoRegistry {
    let mut registry = ControlPlayerInfoRegistry::default();
    registry.replace_snapshot(
        snapshot.last_player_id,
        snapshot
            .clients
            .iter()
            .map(|client| clonk_engine::PlayerInfoControlData {
                client_id: client.client_id,
                flags: client.flags,
                players: client.players.clone(),
                by_client: HOST_CLIENT_ID as i32,
            }),
    );
    registry
}

fn apply_observed_player_info(
    registry: &mut ControlPlayerInfoRegistry,
    info: clonk_engine::PlayerInfoControlData,
) {
    if let Some(last_player_id) = info.players.iter().map(|player| player.id).max() {
        registry.reserve_player_ids_through(last_player_id);
    }
    registry.apply(info);
}

async fn publish_player_info_snapshot(
    host: &clonk_network::HostHandle,
    snapshot: &mut clonk_network::HostJoinSnapshot,
    player_infos: &ControlPlayerInfoRegistry,
) {
    // Native SendJoinData copies the current Game.Parameters, whose PlayerInfos
    // list already contains all earlier admissions
    // (oracle src/C4Network2.cpp:1836-1860).
    let (last_player_id, clients) = player_infos.retained_rows_snapshot();
    snapshot.parameters.player_infos = PlayerInfoListSnapshot {
        last_player_id,
        clients: clients
            .into_iter()
            .map(|(client_id, flags, players)| ClientPlayerInfosSnapshot {
                client_id,
                flags,
                players,
            })
            .collect(),
    };
    host.publish_join_snapshot(snapshot.clone())
        .await
        .expect("publish current synthetic PlayerInfo roster");
    // The inspection command is an acknowledgement barrier behind the snapshot
    // publication on the same host command queue.
    let _ = host
        .runtime_connections()
        .await
        .expect("wait until the synthetic JoinData snapshot is published");
}

async fn wait_for_exact_synthetic_rosters(probes: &[ClientProbe]) {
    let deadline = Instant::now() + EVENT_WAIT;
    loop {
        let incomplete = probes
            .iter()
            .filter(|probe| !probe.has_exact_synthetic_roster())
            .map(|probe| probe.client_id)
            .collect::<Vec<_>>();
        if incomplete.is_empty() {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "synthetic clients missing the exact 24-profile roster before GO: {incomplete:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn has_exact_synthetic_roster(player_infos: &ControlPlayerInfoRegistry) -> bool {
    let (last_player_id, rows) = player_infos.retained_rows_snapshot();
    last_player_id == PLAYER_COUNT as i32
        && rows.len() == PLAYER_COUNT
        && rows.iter().zip(1..=PLAYER_COUNT as i32).all(
            |((client_id, _, players), expected_client_id)| {
                *client_id == expected_client_id
                    && players.as_slice()
                        == [ControlPlayerInfoEntry {
                            id: expected_client_id,
                            name: legacy_string(&format!("LoadPlayer{expected_client_id:02}")),
                            filename: legacy_string(&format!(
                                "LoadPlayer{expected_client_id:02}.c4p"
                            )),
                            league_progress_data_is_null: false,
                            ..Default::default()
                        }]
            },
        )
}

fn configure_host_transport(config: &mut HostConfig, topology: LoadTopology) {
    config.udp_bind_address = matches!(topology, LoadTopology::Udp)
        .then_some("127.0.0.1:0".parse().expect("static UDP host address"));
}

fn configure_client_transport(config: ClientConfig, topology: LoadTopology) -> ClientConfig {
    match topology {
        LoadTopology::Tcp => config
            .with_mesh_tcp_bind_address("127.0.0.1:0".parse().expect("static TCP mesh address")),
        LoadTopology::Udp => config
            .with_mesh_udp_bind_address("127.0.0.1:0".parse().expect("static UDP mesh address")),
        LoadTopology::Relay => config,
    }
}

async fn wait_for_full_mesh(
    host: &clonk_network::HostHandle,
    probes: &[ClientProbe],
    topology: LoadTopology,
) {
    assert!(topology.is_direct_mesh());
    let expected = expected_preferred_message_routes(topology);
    let deadline = Instant::now() + MESH_WAIT;
    loop {
        let observed = preferred_message_routes(host, probes).await;
        if observed == expected {
            return;
        }
        let first_mismatch = observed
            .iter()
            .zip(&expected)
            .find(|(observed, expected)| observed != expected)
            .map(|(observed, expected)| format!("observed {observed:?}, expected {expected:?}"));
        let observed_protocols = observed.iter().fold(BTreeMap::new(), |mut counts, route| {
            *counts.entry(route.protocol.as_str()).or_insert(0_usize) += 1;
            counts
        });
        assert!(
            Instant::now() < deadline,
            "host and 24 clients did not establish the expected {}-route direct \
             {topology:?} preferred-message mesh in {MESH_WAIT:?}; observed {} routes \
             with protocols {observed_protocols:?}; first mismatch: {first_mismatch:?}",
            expected.len(),
            observed.len(),
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn sample_runtime(
    host: &clonk_network::HostHandle,
    probes: &[ClientProbe],
    tick: Tick,
    measurement_started: Instant,
    native_wait_ms: &mut Vec<i64>,
    runtime_samples: &mut Vec<ProcessRuntimeSample>,
) {
    let elapsed_ms = measurement_started.elapsed().as_millis() as u64;
    let host_states = host
        .runtime_client_states(tick, false)
        .await
        .expect("inspect host client wait states");
    native_wait_ms.extend(
        host_states
            .into_iter()
            .map(|state| i64::from(state.wait_ms)),
    );
    let host_routes = host
        .runtime_connections()
        .await
        .expect("inspect host runtime routes");
    let host_io = host.io_statistics();
    host_io.generate_statistics(unix_time_ms());
    let host_io = host_io.snapshot();
    runtime_samples.push(ProcessRuntimeSample {
        elapsed_ms,
        process_client_id: HOST_CLIENT_ID,
        route_count: host_routes.len(),
        tcp_input_rate: host_io.tcp.input_rate,
        tcp_output_rate: host_io.tcp.output_rate,
        udp_input_rate: host_io.udp.input_rate,
        udp_output_rate: host_io.udp.output_rate,
    });

    for probe in probes {
        let states = probe
            .handle
            .runtime_client_states(tick, false)
            .await
            .expect("inspect client wait states");
        native_wait_ms.extend(states.into_iter().map(|state| i64::from(state.wait_ms)));
        let routes = probe
            .handle
            .runtime_connections()
            .await
            .expect("inspect client runtime routes");
        let io = probe.handle.io_statistics();
        io.generate_statistics(unix_time_ms());
        let io = io.snapshot();
        runtime_samples.push(ProcessRuntimeSample {
            elapsed_ms,
            process_client_id: probe.client_id,
            route_count: routes.len(),
            tcp_input_rate: io.tcp.input_rate,
            tcp_output_rate: io.tcp.output_rate,
            udp_input_rate: io.udp.input_rate,
            udp_output_rate: io.udp.output_rate,
        });
    }
}

async fn preferred_message_routes(
    host: &clonk_network::HostHandle,
    probes: &[ClientProbe],
) -> Vec<PreferredMessageRoute> {
    let mut routes = host
        .runtime_connections()
        .await
        .expect("inspect final host routes")
        .into_iter()
        .filter(|route| route.usage.contains("Msg"))
        .map(|route| PreferredMessageRoute {
            process_client_id: HOST_CLIENT_ID,
            peer_client_id: route.client_id,
            protocol: protocol_name(route.protocol).to_string(),
        })
        .collect::<Vec<_>>();
    for probe in probes {
        routes.extend(
            probe
                .handle
                .runtime_connections()
                .await
                .expect("inspect final client routes")
                .into_iter()
                .filter(|route| route.usage.contains("Msg"))
                .map(|route| PreferredMessageRoute {
                    process_client_id: probe.client_id,
                    peer_client_id: route.client_id,
                    protocol: protocol_name(route.protocol).to_string(),
                }),
        );
    }
    routes.sort_unstable_by(|left, right| {
        (
            left.process_client_id,
            left.peer_client_id,
            left.protocol.as_str(),
        )
            .cmp(&(
                right.process_client_id,
                right.peer_client_id,
                right.protocol.as_str(),
            ))
    });
    routes
}

fn route_peers(routes: &[PreferredMessageRoute]) -> Vec<(u32, Vec<u32>)> {
    let mut peers = (0..=PLAYER_COUNT as u32)
        .map(|process_client_id| (process_client_id, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for route in routes {
        peers
            .get_mut(&route.process_client_id)
            .expect("load route belongs to a synthetic endpoint")
            .push(route.peer_client_id);
    }
    for process_peers in peers.values_mut() {
        process_peers.sort_unstable();
    }
    peers.into_iter().collect()
}

async fn shutdown_session(
    host: clonk_network::HostHandle,
    probes: Vec<ClientProbe>,
) -> Result<(), String> {
    let mut shutdowns = tokio::task::JoinSet::new();
    shutdowns.spawn(async move {
        host.shutdown()
            .await
            .map_err(|error| format!("host shutdown failed: {error}"))
    });
    for probe in probes {
        let client_id = probe.client_id;
        shutdowns.spawn(async move { probe.shutdown().await });
        eprintln!("LC_NETWORK_LOAD_24 phase=cleanup scheduled_client={client_id}");
    }
    let mut failures = Vec::new();
    while let Some(result) = shutdowns.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failures.push(error),
            Err(error) => failures.push(format!("network shutdown task panicked: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("; "))
    }
}

fn summarize(samples: &[i64]) -> MetricSummary {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let percentile = |percent: usize| {
        (!sorted.is_empty()).then(|| {
            let rank = (percent * sorted.len()).div_ceil(100);
            sorted[rank.saturating_sub(1)]
        })
    };
    MetricSummary {
        samples: sorted.len(),
        p50: percentile(50),
        p95: percentile(95),
        p99: percentile(99),
        max: sorted.last().copied(),
    }
}

fn legacy_string(value: &str) -> LegacyCString {
    LegacyCString::from_bytes(value.as_bytes().to_vec()).expect("test string is NUL-free")
}

fn native_control_interval() -> Duration {
    NATIVE_GAME_TICK.saturating_mul(CONTROL_RATE)
}

fn minimum_native_control_ticks(duration: Duration) -> usize {
    let control_ticks = duration
        .as_nanos()
        .div_ceil(native_control_interval().as_nanos());
    usize::try_from(control_ticks).unwrap_or(usize::MAX)
}

fn measurement_elapsed_bounds(requested: Duration) -> (Duration, Duration) {
    let interval = native_control_interval();
    (
        requested.saturating_sub(interval),
        requested.saturating_add(interval),
    )
}

fn expected_route_peers(topology: LoadTopology) -> Vec<(u32, Vec<u32>)> {
    let host_peers = (1..=PLAYER_COUNT as u32).collect::<Vec<_>>();
    let mut expected = vec![(HOST_CLIENT_ID, host_peers)];
    expected.extend((1..=PLAYER_COUNT as u32).map(|process_id| {
        let peers = if topology.is_direct_mesh() {
            (0..=PLAYER_COUNT as u32)
                .filter(|peer_id| *peer_id != process_id)
                .collect()
        } else {
            vec![HOST_CLIENT_ID]
        };
        (process_id, peers)
    }));
    expected
}

fn expected_preferred_message_routes(topology: LoadTopology) -> Vec<PreferredMessageRoute> {
    let protocol = protocol_name(topology.preferred_message_protocol()).to_string();
    expected_route_peers(topology)
        .into_iter()
        .flat_map(|(process_client_id, peers)| {
            let protocol = protocol.clone();
            peers
                .into_iter()
                .map(move |peer_client_id| PreferredMessageRoute {
                    process_client_id,
                    peer_client_id,
                    protocol: protocol.clone(),
                })
        })
        .collect()
}

const fn protocol_name(protocol: NetworkProtocol) -> &'static str {
    match protocol {
        NetworkProtocol::Tcp => "tcp",
        NetworkProtocol::Udp => "udp",
        _ => "unknown",
    }
}

fn duration_us(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros()).unwrap_or(i64::MAX)
}

fn micros_to_millis(micros: Option<i64>) -> f64 {
    micros.map_or(-1.0, |value| value as f64 / 1_000.0)
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

fn evaluate_load_assertions(
    report: &NetworkLoadReport,
    requested_measurement: Duration,
    measurement_wall_elapsed: Duration,
    topology: LoadTopology,
) -> Vec<LoadAssertion> {
    let mut assertions = Vec::new();
    let mut push = |name: &str, passed: bool, detail: String| {
        assertions.push(LoadAssertion {
            name: name.to_string(),
            passed,
            detail,
        });
    };
    push(
        "every-participant-ready",
        report.observed_ready_deliveries == report.expected_ready_deliveries,
        format!(
            "observed {} of {} ready deliveries",
            report.observed_ready_deliveries, report.expected_ready_deliveries
        ),
    );
    push(
        "native-control-cadence",
        report.measured_ticks == report.minimum_native_control_ticks,
        format!(
            "measured {} ticks, expected {} at 28ms * ControlRate={CONTROL_RATE}",
            report.measured_ticks, report.minimum_native_control_ticks
        ),
    );
    let (minimum_elapsed, maximum_elapsed) = measurement_elapsed_bounds(requested_measurement);
    push(
        "measurement-wall-duration",
        (minimum_elapsed..=maximum_elapsed).contains(&measurement_wall_elapsed),
        format!(
            "observed {measurement_wall_elapsed:?}, expected within \
             {minimum_elapsed:?}..={maximum_elapsed:?}"
        ),
    );
    let expected_routes = expected_route_peers(topology);
    push(
        "exact-route-topology",
        report.final_route_peers == expected_routes,
        format!(
            "observed {:?}, expected {:?}",
            report.final_route_peers, expected_routes
        ),
    );
    let expected_message_routes = expected_preferred_message_routes(topology);
    push(
        "exact-preferred-message-routes",
        report.final_preferred_message_routes == expected_message_routes,
        format!(
            "observed {} {:?} routes, expected {} {:?} routes",
            report.final_preferred_message_routes.len(),
            report.preferred_message_protocol,
            expected_message_routes.len(),
            protocol_name(topology.preferred_message_protocol()),
        ),
    );
    push(
        "aggregate-rtt-samples",
        !report.client_to_host_round_trip.raw_samples.is_empty(),
        format!(
            "observed {} samples",
            report.client_to_host_round_trip.raw_samples.len()
        ),
    );
    push(
        "per-client-rtt-series",
        report.client_to_host_round_trip_by_client.len() == PLAYER_COUNT,
        format!(
            "observed {} client series, expected {PLAYER_COUNT}",
            report.client_to_host_round_trip_by_client.len()
        ),
    );
    for client in &report.client_to_host_round_trip_by_client {
        push(
            &format!("client-{}-rtt-samples", client.client_id),
            !client.metrics.raw_samples.is_empty(),
            format!("observed {} samples", client.metrics.raw_samples.len()),
        );
        push(
            &format!("client-{}-rtt-p99", client.client_id),
            client
                .metrics
                .summary
                .p99
                .is_some_and(|p99| p99 < LOOPBACK_RTT_P99_LIMIT_MS),
            format!(
                "p99={:?}, exclusive limit={}ms",
                client.metrics.summary.p99, LOOPBACK_RTT_P99_LIMIT_MS
            ),
        );
    }
    push(
        "aggregate-rtt-p99",
        report
            .client_to_host_round_trip
            .summary
            .p99
            .is_some_and(|p99| p99 < LOOPBACK_RTT_P99_LIMIT_MS),
        format!(
            "p99={:?}, exclusive limit={}ms",
            report.client_to_host_round_trip.summary.p99, LOOPBACK_RTT_P99_LIMIT_MS
        ),
    );
    push(
        "control-completion-p99",
        report
            .control_completion_wait
            .summary
            .p99
            .is_some_and(|p99| p99 < LOOPBACK_P99_LIMIT_US),
        format!(
            "p99={:?}, exclusive limit={}us",
            report.control_completion_wait.summary.p99, LOOPBACK_P99_LIMIT_US
        ),
    );
    assertions
}

fn load_assertion_result(assertions: &[LoadAssertion]) -> &'static str {
    if assertions.iter().all(|assertion| assertion.passed) {
        "pass"
    } else {
        "fail"
    }
}

fn write_report(report: &NetworkLoadReport) -> PathBuf {
    let path = std::env::var_os("LC_NETWORK_LOAD_METRICS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            workspace_root()
                .join("target/network-load")
                .join(format!("harpoonrace-shaped-24-{}.json", unix_time_ms()))
        });
    let parent = path.parent().expect("metrics path has a parent");
    std::fs::create_dir_all(parent).expect("create network-load artifact directory");
    let file = std::fs::File::create(&path).expect("create network-load metrics report");
    serde_json::to_writer_pretty(file, report).expect("serialize network-load metrics report");
    path
}

fn fingerprint() -> LoadFingerprint {
    let root = workspace_root();
    LoadFingerprint {
        source_commit: command_output(&root, "git", &["rev-parse", "HEAD"]),
        source_dirty: command_output(
            &root,
            "git",
            &["status", "--porcelain", "--untracked-files=normal"],
        )
        .is_some_and(|output| !output.is_empty()),
        content_revision: command_output(&root.join("content"), "git", &["rev-parse", "HEAD"]),
        rustc: command_output(&root, "rustc", &["--version"]),
        target_os: std::env::consts::OS,
        target_arch: std::env::consts::ARCH,
        cpu: command_output(&root, "sysctl", &["-n", "machdep.cpu.brand_string"]),
        os_version: command_output(&root, "sw_vers", &["-productVersion"]),
        cargo_profile: if cfg!(debug_assertions) {
            "test-with-debug-assertions"
        } else {
            "test"
        },
    }
}

fn command_output(directory: &Path, command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command)
        .args(args)
        .current_dir(directory)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("network crate is nested under workspace crates")
        .to_path_buf()
}

#[test]
fn load_metric_percentiles_use_nearest_rank_without_hiding_the_tail() {
    let summary = summarize(&[1_i64, 2, 3, 4, 100]);

    assert_eq!(summary.p50, Some(3));
    assert_eq!(summary.p95, Some(100));
    assert_eq!(summary.p99, Some(100));
}

#[test]
fn load_assertion_result_and_failure_detail_are_json_serializable() {
    let assertions = vec![
        LoadAssertion {
            name: "passing-gate".to_string(),
            passed: true,
            detail: "ok".to_string(),
        },
        LoadAssertion {
            name: "failed-gate".to_string(),
            passed: false,
            detail: "retained exact failure evidence".to_string(),
        },
    ];

    assert_eq!(load_assertion_result(&assertions), "fail");
    let serialized = serde_json::to_value(&assertions).expect("serialize assertions");
    assert_eq!(serialized[1]["passed"], false);
    assert_eq!(serialized[1]["detail"], "retained exact failure evidence");
}

#[test]
fn load_report_metadata_limits_harpoonrace_and_rtt_claims_to_measured_transport() {
    assert_eq!(
        LOAD_WORKLOAD,
        "same-process Tokio IPv4-loopback real-socket HarpoonRace-shaped control transport"
    );
    assert_eq!(
        LOAD_WORKLOAD_SCOPE,
        "HarpoonRace-shaped lobby/control parameters only; no scenario/resource loading or game simulation"
    );
    assert_eq!(
        LOAD_SEQUENCE,
        "synthetic max_players=24 JoinData -> 24 PlayerInfo joins -> activate all -> GO"
    );
    assert_eq!(
        LOAD_RTT_SCOPE,
        "client-to-host ping samples only; all endpoints run in one Tokio process over IPv4 loopback"
    );
    assert!(!LOAD_SEQUENCE.contains("/set"));
}

#[test]
fn sixty_second_budget_uses_cpp_native_control_cadence_and_bounds_wall_elapsed() {
    // C++ installs a 28ms game tick and executes one control every
    // ControlRate=2 frames. Network DefaultTargetFPS=38 only sizes PreSend and
    // is not the simulation clock (oracle src/C4Game.cpp:64,444;
    // src/C4GameControlNetwork.cpp:432-445).
    let requested = Duration::from_secs(60);
    let interval = native_control_interval();

    assert_eq!(interval, Duration::from_millis(56));
    assert_eq!(minimum_native_control_ticks(requested), 1_072);
    assert_eq!(
        measurement_elapsed_bounds(requested),
        (requested - interval, requested + interval)
    );
}

#[test]
fn expected_route_topology_counts_host_and_every_synthetic_client_exactly() {
    let expected_mesh = (0..=PLAYER_COUNT as u32)
        .map(|process_id| {
            (
                process_id,
                (0..=PLAYER_COUNT as u32)
                    .filter(|peer_id| *peer_id != process_id)
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let mut expected_host_spoke = vec![(
        HOST_CLIENT_ID,
        (1..=PLAYER_COUNT as u32).collect::<Vec<_>>(),
    )];
    expected_host_spoke
        .extend((1..=PLAYER_COUNT as u32).map(|client_id| (client_id, vec![HOST_CLIENT_ID])));

    assert_eq!(expected_route_peers(LoadTopology::Tcp), expected_mesh);
    assert_eq!(expected_route_peers(LoadTopology::Udp), expected_mesh);
    assert_eq!(
        expected_route_peers(LoadTopology::Relay),
        expected_host_spoke
    );
}

#[test]
fn load_topology_parser_preserves_legacy_direct_mesh_default_and_override() {
    assert_eq!(
        parse_load_topology(None, None),
        Ok(LoadTopology::Tcp),
        "the old harness defaulted to direct TCP mesh"
    );
    assert_eq!(
        parse_load_topology(None, Some("0")),
        Ok(LoadTopology::Relay)
    );
    assert_eq!(parse_load_topology(None, Some("1")), Ok(LoadTopology::Tcp));
    assert_eq!(
        parse_load_topology(None, Some("anything")),
        Ok(LoadTopology::Tcp),
        "the legacy flag treated every value except exactly zero as direct mesh"
    );
    assert_eq!(
        parse_load_topology(Some("udp"), Some("0")),
        Ok(LoadTopology::Udp),
        "the explicit topology supersedes the legacy compatibility flag"
    );
}

#[test]
fn load_topology_parser_accepts_only_named_transports() {
    assert_eq!(
        parse_load_topology(Some("tcp"), None),
        Ok(LoadTopology::Tcp)
    );
    assert_eq!(
        parse_load_topology(Some("udp"), None),
        Ok(LoadTopology::Udp)
    );
    assert_eq!(
        parse_load_topology(Some("relay"), None),
        Ok(LoadTopology::Relay)
    );
    assert_eq!(
        parse_load_topology(Some("UDP"), None),
        Err("LC_NETWORK_LOAD_TOPOLOGY must be tcp, udp, or relay; got \"UDP\"".to_string())
    );
}

#[test]
fn load_topology_configures_only_the_requested_direct_transport() {
    let mut tcp_host = HostConfig::default();
    configure_host_transport(&mut tcp_host, LoadTopology::Tcp);
    let tcp_client = configure_client_transport(
        ClientConfig::new("TCP", ParticipantKind::Player),
        LoadTopology::Tcp,
    );
    assert_eq!(tcp_host.udp_bind_address, None);
    assert_eq!(
        tcp_client.mesh_tcp_bind_address,
        Some("127.0.0.1:0".parse().unwrap())
    );
    assert_eq!(tcp_client.mesh_udp_bind_address, None);

    let mut udp_host = HostConfig::default();
    configure_host_transport(&mut udp_host, LoadTopology::Udp);
    let udp_client = configure_client_transport(
        ClientConfig::new("UDP", ParticipantKind::Player),
        LoadTopology::Udp,
    );
    assert_eq!(
        udp_host.udp_bind_address,
        Some("127.0.0.1:0".parse().unwrap())
    );
    assert_eq!(udp_client.mesh_tcp_bind_address, None);
    assert_eq!(
        udp_client.mesh_udp_bind_address,
        Some("127.0.0.1:0".parse().unwrap())
    );

    let mut relay_host = HostConfig::default();
    configure_host_transport(&mut relay_host, LoadTopology::Relay);
    let relay_client = configure_client_transport(
        ClientConfig::new("Relay", ParticipantKind::Player),
        LoadTopology::Relay,
    );
    assert_eq!(relay_host.udp_bind_address, None);
    assert_eq!(relay_client.mesh_tcp_bind_address, None);
    assert_eq!(relay_client.mesh_udp_bind_address, None);
}

#[test]
fn preferred_message_route_expectations_pin_protocol_and_report_labels() {
    let udp = expected_preferred_message_routes(LoadTopology::Udp);
    assert_eq!(udp.len(), (PLAYER_COUNT + 1) * PLAYER_COUNT);
    assert!(
        udp.iter().all(|route| route.protocol == "udp"),
        "every directed full-mesh message route must prefer reliable UDP"
    );
    let tcp = expected_preferred_message_routes(LoadTopology::Tcp);
    assert_eq!(tcp.len(), udp.len());
    assert!(tcp.iter().all(|route| route.protocol == "tcp"));
    let relay = expected_preferred_message_routes(LoadTopology::Relay);
    assert_eq!(relay.len(), PLAYER_COUNT * 2);
    assert!(relay.iter().all(|route| route.protocol == "tcp"));

    assert_eq!(
        serde_json::to_value(LoadTopology::Udp).unwrap(),
        serde_json::json!("udp")
    );
}

#[test]
fn every_synthetic_client_roster_requires_all_twenty_four_player_infos() {
    // Native JoinData copies current Game.Parameters, including PlayerInfos,
    // before it is sent (oracle src/C4Network2.cpp:1850-1860).
    let mut roster = ControlPlayerInfoRegistry::default();
    for client_id in 1..PLAYER_COUNT as i32 {
        roster.apply(clonk_engine::PlayerInfoControlData {
            client_id,
            players: vec![ControlPlayerInfoEntry {
                id: client_id,
                name: legacy_string(&format!("LoadPlayer{client_id:02}")),
                filename: legacy_string(&format!("LoadPlayer{client_id:02}.c4p")),
                league_progress_data_is_null: false,
                ..Default::default()
            }],
            ..Default::default()
        });
    }
    assert!(!has_exact_synthetic_roster(&roster));

    roster.apply(clonk_engine::PlayerInfoControlData {
        client_id: PLAYER_COUNT as i32,
        players: vec![ControlPlayerInfoEntry {
            id: PLAYER_COUNT as i32,
            name: legacy_string(&format!("LoadPlayer{PLAYER_COUNT:02}")),
            filename: legacy_string(&format!("LoadPlayer{PLAYER_COUNT:02}.c4p")),
            league_progress_data_is_null: false,
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(
        !has_exact_synthetic_roster(&roster),
        "complete rows with a stale LastPlayerID are not an exact JoinData roster"
    );
    let (_, rows) = roster.retained_rows_snapshot();
    roster.replace_snapshot(
        PLAYER_COUNT as i32,
        rows.into_iter().map(
            |(client_id, flags, players)| clonk_engine::PlayerInfoControlData {
                client_id,
                flags,
                players,
                by_client: HOST_CLIENT_ID as i32,
            },
        ),
    );
    assert!(has_exact_synthetic_roster(&roster));
}

#[test]
fn runtime_report_omits_structural_tcp_loss_counter() {
    // The public native statistic is not an end-to-end loss measurement, and
    // this harness's TCP route snapshots expose no measured loss signal
    // (oracle src/C4Network2IO.cpp:1502-1518).
    let sample = ProcessRuntimeSample {
        elapsed_ms: 1_000,
        process_client_id: 0,
        route_count: PLAYER_COUNT,
        tcp_input_rate: 1,
        tcp_output_rate: 2,
        udp_input_rate: 0,
        udp_output_rate: 0,
    };
    let serialized = serde_json::to_value(sample).expect("serialize runtime sample");

    assert!(serialized.get("packet_loss").is_none());
}
