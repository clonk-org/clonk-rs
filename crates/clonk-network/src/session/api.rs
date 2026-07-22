//! Session config & public API: configs, handles, events, commands, errors.
//!
//! Moved byte-verbatim from `session.rs` (wave 2 of the decomposition
//! campaign, see REFACTOR_PLAN.md). Structural only.

use super::*;

pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
pub(crate) const CLIENT_ROUTE_RETRY_INTERVAL: Duration = Duration::from_secs(1);
pub(crate) const CHASE_TARGET_UPDATE_INTERVAL: Duration = Duration::from_secs(5);
pub(crate) const CONTROL_REQUEST_INTERVAL: Duration = Duration::from_secs(2);
pub(crate) const CLIENT_BACKLOG_LIMIT: usize = 256;
pub(crate) const CLIENT_MESH_PENDING_LIMIT: usize = 64;
#[cfg(test)]
pub(crate) const DEFAULT_CONTROL_TARGET_FPS: i32 = 38;
pub(crate) const HOST_CLIENT_ID: ClientId = 0;
static RESOURCE_RANDOM_STATE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn network_statistics_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

pub(crate) fn resource_safe_random(range: usize) -> usize {
    if range == 0 {
        return 0;
    }
    let mut current = RESOURCE_RANDOM_STATE.load(AtomicOrdering::Relaxed);
    loop {
        let next = current
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        match RESOURCE_RANDOM_STATE.compare_exchange_weak(
            current,
            next,
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => return (next as usize) % range,
            Err(observed) => current = observed,
        }
    }
}

/// Broadcast identifier that mirrors the legacy `C4ClientIDAll` constant.
pub const BROADCAST_CLIENT_ID: ClientId = u32::MAX;

/// One live transport route as shown by the runtime network client list.
/// `connection_id` is local to this process and remains stable for the life of
/// the route. A route may carry data, messages, or both according to the same
/// preference rules used by the session send path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeNetworkConnection {
    pub connection_id: u32,
    pub client_id: ClientId,
    pub usage: String,
    pub protocol: crate::NetworkProtocol,
    pub peer_address: Option<SocketAddr>,
    pub packet_loss: u32,
    /// `C4Network2IOConnection::getPingTime()`: the last measured round trip,
    /// `-1` until a pong arrived. The debug status text shows this value
    /// (src/C4Network2.cpp:1212-1218).
    pub ping_ms: i32,
    /// `C4Network2IOConnection::getLag()` at snapshot time: while a ping is
    /// unanswered, the elapsed wait once it exceeds the measurement
    /// (src/C4Network2IO.cpp:1283-1295). The lobby roster ping column
    /// (src/C4PlayerInfoListBox.cpp:885-908), the runtime client list
    /// (src/C4Network2Dialogs.cpp:357-369) and the stats graphs
    /// (src/C4Network2Stats.cpp:336-343) show this value.
    pub lag_ms: i32,
}

/// One atomic lobby snapshot of selected transport routes and each requested
/// remote client's chunk-weighted resource availability.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RuntimeLobbyClientTelemetry {
    pub connections: Vec<RuntimeNetworkConnection>,
    pub resource_progress: Vec<(ClientId, u8)>,
}

/// Receiver-side state used by the in-game network client list.
///
/// `wait_ms` mirrors `C4GameControlClient::getPerfStat`: it is the signed
/// control-arrival EWMA for this client, not a transport round-trip time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RuntimeNetworkClientState {
    pub client_id: ClientId,
    pub status: RemoteBarrierState,
    pub control_ready: bool,
    pub wait_ms: i32,
}

/// Local receive timestamps and the scaled C++ `ClientPerfStat` accumulator.
///
/// Native packets stamp `iTime` in the receiving process' packet constructor,
/// while `iWaitStart` is the first cadence at which that control tick is
/// attempted. Keeping both timestamps local preserves signed early/late
/// values without trusting a serialized clock or substituting route ping.
#[derive(Debug)]
pub(crate) struct ClientPerformanceStats {
    tick_limit: usize,
    pub(crate) arrivals: BTreeMap<Tick, BTreeMap<ClientId, tokio::time::Instant>>,
    cadences: BTreeMap<Tick, tokio::time::Instant>,
    consumed_at: BTreeMap<Tick, tokio::time::Instant>,
    consumed_clients: BTreeMap<Tick, BTreeSet<ClientId>>,
    sampled: BTreeSet<(Tick, ClientId)>,
    pub(crate) scaled_wait_ms: BTreeMap<ClientId, i32>,
}

impl ClientPerformanceStats {
    pub(crate) fn new(tick_limit: usize) -> Self {
        Self {
            tick_limit,
            arrivals: BTreeMap::new(),
            cadences: BTreeMap::new(),
            consumed_at: BTreeMap::new(),
            consumed_clients: BTreeMap::new(),
            sampled: BTreeSet::new(),
            scaled_wait_ms: BTreeMap::new(),
        }
    }

    pub(crate) fn record_arrival(
        &mut self,
        client_id: ClientId,
        tick: Tick,
        arrived_at: tokio::time::Instant,
    ) {
        if client_id == BROADCAST_CLIENT_ID
            || self
                .consumed_at
                .get(&tick)
                .is_some_and(|consumed_at| arrived_at > *consumed_at)
            || self
                .consumed_clients
                .get(&tick)
                .is_some_and(|clients| !clients.contains(&client_id))
        {
            return;
        }
        self.arrivals
            .entry(tick)
            .or_default()
            .entry(client_id)
            .or_insert(arrived_at);
        self.sample(tick, client_id);
        self.trim();
    }

    pub(crate) fn mark_consumed(
        &mut self,
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: impl IntoIterator<Item = ClientId>,
    ) {
        self.consumed_at.entry(tick).or_insert(consumed_at);
        let client_ids = self
            .consumed_clients
            .entry(tick)
            .or_insert_with(|| client_ids.into_iter().collect())
            .iter()
            .copied()
            .collect::<Vec<_>>();
        for client_id in client_ids {
            self.sample(tick, client_id);
        }
        self.trim();
    }

    pub(crate) fn reset_accumulators(&mut self) {
        self.sampled.extend(
            self.consumed_clients
                .iter()
                .flat_map(|(tick, clients)| clients.iter().map(|client_id| (*tick, *client_id))),
        );
        self.scaled_wait_ms.clear();
    }

    pub(crate) fn record_cadence(&mut self, tick: Tick, reached_at: tokio::time::Instant) {
        self.cadences.entry(tick).or_insert(reached_at);
        let client_ids = self
            .arrivals
            .get(&tick)
            .map(|arrivals| arrivals.keys().copied().collect::<Vec<_>>())
            .unwrap_or_default();
        for client_id in client_ids {
            self.sample(tick, client_id);
        }
        self.trim();
    }

    pub(crate) fn wait_ms(&self, client_id: ClientId) -> i32 {
        self.scaled_wait_ms.get(&client_id).copied().unwrap_or(0) / 100
    }

    fn sample(&mut self, tick: Tick, client_id: ClientId) {
        if self.sampled.contains(&(tick, client_id)) {
            return;
        }
        let Some(arrived_at) = self
            .arrivals
            .get(&tick)
            .and_then(|arrivals| arrivals.get(&client_id))
            .copied()
        else {
            return;
        };
        let Some(consumed_at) = self.consumed_at.get(&tick).copied() else {
            return;
        };
        if !self
            .consumed_clients
            .get(&tick)
            .is_some_and(|clients| clients.contains(&client_id))
        {
            return;
        }
        if arrived_at > consumed_at {
            return;
        }
        let Some(reached_at) = self.cadences.get(&tick).copied() else {
            return;
        };
        self.sampled.insert((tick, client_id));

        let wait_ms = signed_instant_millis(arrived_at, reached_at);
        let scaled = self.scaled_wait_ms.entry(client_id).or_default();
        // C4GameControlClient::AddPerf, including signed integer truncation.
        *scaled += wait_ms
            .saturating_mul(100)
            .saturating_sub(*scaled)
            / 100;
    }

    fn trim(&mut self) {
        if self.tick_limit == 0 {
            return;
        }
        let mut ticks = self
            .arrivals
            .keys()
            .chain(self.cadences.keys())
            .chain(self.consumed_at.keys())
            .chain(self.consumed_clients.keys())
            .copied()
            .collect::<BTreeSet<_>>();
        while ticks.len() > self.tick_limit {
            let Some(tick) = ticks.pop_first() else {
                break;
            };
            self.arrivals.remove(&tick);
            self.cadences.remove(&tick);
            self.consumed_at.remove(&tick);
            self.consumed_clients.remove(&tick);
            self.sampled
                .retain(|(sampled_tick, _)| *sampled_tick != tick);
        }
    }
}

fn signed_instant_millis(
    arrived_at: tokio::time::Instant,
    reached_at: tokio::time::Instant,
) -> i32 {
    let signed = if arrived_at >= reached_at {
        i128::try_from(arrived_at.duration_since(reached_at).as_millis()).unwrap_or(i128::MAX)
    } else {
        -i128::try_from(reached_at.duration_since(arrived_at).as_millis()).unwrap_or(i128::MAX)
    };
    signed.clamp(i128::from(i32::MIN), i128::from(i32::MAX)) as i32
}

pub(crate) fn runtime_connection_usage(message: bool, data: bool) -> Option<String> {
    match (message, data) {
        (true, true) => Some("Data/Msg".to_string()),
        (true, false) => Some("Msg".to_string()),
        (false, true) => Some("Data".to_string()),
        (false, false) => None,
    }
}

/// Configuration options for the multiplayer host.
#[derive(Debug, Clone)]
pub struct HostConfig {
    pub backlog_limit: usize,
    pub resync_interval: Duration,
    pub resync_cooldown: Duration,
    /// `Config.Network.AsyncMaxWait`, measured in extra control frames.
    pub async_max_wait_frames: i32,
    pub max_players: usize,
    pub start_tick: Tick,
    pub local_core: clonk_engine::ClientCoreControlData,
    /// Process-wide `Config.General.Name` used by C4Group rewrites. This is
    /// independent of the network-visible local client name.
    pub group_maker: clonk_engine::LegacyCString,
    pub initial_status: NetworkStatus,
    pub password: clonk_engine::LegacyCString,
    pub allow_join: bool,
    /// Optional C4NetIOUDP listener. TCP remains available through the
    /// separately prepared listener passed to the host startup API.
    pub udp_bind_address: Option<SocketAddr>,
    /// Resolved netpuncher endpoints in preference order. At most the first
    /// endpoint for each address family is connected through the shared UDP
    /// socket.
    pub netpuncher_addresses: Vec<SocketAddr>,
    /// Exact `Config.Network.PortTCP`/`PortUDP` values used for local address
    /// projection. `Some(0)` remains meaningful; `None` preserves the bound
    /// listener-port behavior of direct API callers.
    pub configured_tcp_port: Option<u16>,
    pub configured_udp_port: Option<u16>,
    /// Requests C++-style best-effort UPnP IGD mappings for each successfully
    /// bound host transport. Direct API callers opt in explicitly; the app
    /// applies the stock `Config.Network.EnableUPnP` default.
    pub enable_upnp: bool,
    pub initial_join_snapshot: Option<HostJoinSnapshot>,
    /// Resources in C++ publication order. `ResourceCatalog::register`
    /// prepends each entry, reproducing the linked-list discovery order.
    pub resource_registrations: Vec<crate::ResourceRegistration>,
    /// Stock network working directory (`Config.Network.WorkPath`).
    pub resource_directory: Option<PathBuf>,
    /// Local standalones and logical non-loadables in C++ publication order.
    pub resource_files: Vec<HostedResourceFile>,
    /// Original local player source paths and the cores published for them.
    /// C++ searches these before allocating another NRT_Player.
    pub player_resource_sources: Vec<(PathBuf, clonk_engine::NetworkResourceCore)>,
    /// C++ resource search roots retained for later authoritative PlayerInfo
    /// resources announced after JoinData.
    pub local_resource_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct HostedResourceFile {
    pub core: clonk_engine::NetworkResourceCore,
    pub path: PathBuf,
    pub ownership: crate::ResourceFileOwnership,
    pub binary_compatible: bool,
}

/// The synchronized dynamic/resource state frozen into a host's JoinData.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostJoinSnapshot {
    pub dynamic: clonk_engine::NetworkResourceCore,
    pub dynamic_tick: i32,
    pub parameters: crate::JoinGameParametersEnvelope,
}

impl Default for HostConfig {
    fn default() -> Self {
        let name = clonk_engine::LegacyCString::from_bytes(b"Host".to_vec())
            .expect("static host name is NUL-free");
        let local_core = clonk_engine::ClientCoreControlData {
            client_id: 0,
            activated: true,
            observer: false,
            name: name.clone(),
            nick: name,
            lobby_ready: false,
        };
        Self {
            backlog_limit: 256,
            resync_interval: Duration::from_millis(200),
            resync_cooldown: Duration::from_secs(2),
            async_max_wait_frames: 2,
            max_players: 8,
            start_tick: 0,
            local_core: local_core.clone(),
            group_maker: local_core.name.clone(),
            initial_status: NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 0,
                target_tick: -1,
            },
            password: clonk_engine::LegacyCString::default(),
            allow_join: true,
            udp_bind_address: None,
            netpuncher_addresses: Vec::new(),
            configured_tcp_port: None,
            configured_udp_port: None,
            enable_upnp: false,
            initial_join_snapshot: Some(synthetic_join_snapshot(local_core, 8)),
            resource_registrations: Vec::new(),
            resource_directory: None,
            resource_files: Vec::new(),
            player_resource_sources: Vec::new(),
            local_resource_roots: Vec::new(),
        }
    }
}

/// One resolved netpuncher endpoint and the host-advertised game ID for its
/// address family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientMeshPuncherConfig {
    pub address: SocketAddr,
    pub game_id: u32,
}

/// Client metadata supplied during handshake.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub name: String,
    pub group_maker: clonk_engine::LegacyCString,
    pub kind: ParticipantKind,
    /// Build advertised during `PID_Conn` and expected from every peer in the
    /// target game. Reference-backed joins set this from the host reference;
    /// unresolved direct joins retain [`CURRENT_GAME_BUILD`].
    pub compatibility_build: i32,
    pub password: clonk_engine::LegacyCString,
    pub resource_directory: Option<PathBuf>,
    pub bootstrap_local_candidates: crate::ClientBootstrapLocalCandidates,
    pub local_system_path: Option<PathBuf>,
    pub local_resource_roots: Vec<PathBuf>,
    /// TCP endpoint accepted for already joined client-to-client mesh routes.
    /// `None` keeps the listener disabled for embedders that do not expose a
    /// network endpoint.
    pub mesh_tcp_bind_address: Option<SocketAddr>,
    /// Reliable-UDP endpoint shared by outbound and inbound mesh routes.
    pub mesh_udp_bind_address: Option<SocketAddr>,
    /// Resolved per-family netpuncher endpoints advertised by the host.
    pub mesh_punchers: Vec<ClientMeshPuncherConfig>,
}

impl ClientConfig {
    pub fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        let name = name.into();
        let group_maker = clonk_engine::LegacyCString::from_bytes(name.as_bytes().to_vec())
            .unwrap_or_default();
        Self {
            name,
            group_maker,
            kind,
            compatibility_build: CURRENT_GAME_BUILD,
            password: clonk_engine::LegacyCString::default(),
            resource_directory: Some(default_client_resource_directory()),
            bootstrap_local_candidates: crate::ClientBootstrapLocalCandidates::default(),
            local_system_path: None,
            local_resource_roots: Vec::new(),
            mesh_tcp_bind_address: None,
            mesh_udp_bind_address: None,
            mesh_punchers: Vec::new(),
        }
    }

    pub fn with_password(mut self, password: clonk_engine::LegacyCString) -> Self {
        self.password = password;
        self
    }

    pub fn with_compatibility_build(mut self, build: i32) -> Self {
        self.compatibility_build = build;
        self
    }

    pub fn with_group_maker(mut self, group_maker: clonk_engine::LegacyCString) -> Self {
        self.group_maker = group_maker;
        self
    }

    pub fn with_resource_directory(mut self, resource_directory: impl Into<PathBuf>) -> Self {
        self.resource_directory = Some(resource_directory.into());
        self
    }

    pub fn with_bootstrap_local_candidates(
        mut self,
        candidates: crate::ClientBootstrapLocalCandidates,
    ) -> Self {
        self.bootstrap_local_candidates = candidates;
        self
    }

    pub fn with_local_system_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.local_system_path = Some(path.into());
        self
    }

    pub fn with_local_resource_roots(
        mut self,
        roots: impl IntoIterator<Item = impl Into<PathBuf>>,
    ) -> Self {
        self.local_resource_roots = roots.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_mesh_tcp_bind_address(mut self, address: SocketAddr) -> Self {
        self.mesh_tcp_bind_address = Some(address);
        self
    }

    pub fn with_mesh_udp_bind_address(mut self, address: SocketAddr) -> Self {
        self.mesh_udp_bind_address = Some(address);
        self
    }

    pub fn with_mesh_punchers(
        mut self,
        punchers: impl IntoIterator<Item = ClientMeshPuncherConfig>,
    ) -> Self {
        self.mesh_punchers = punchers.into_iter().collect();
        self
    }
}

fn default_client_resource_directory() -> PathBuf {
    // Application callers replace this with Config.Network.WorkPath. Library
    // callers still need a real ResList backend, so keep their default out of
    // the current source tree while preserving the stock `Network` role.
    std::env::temp_dir()
        .join(format!("legacyclonk-{}", std::process::id()))
        .join("Network")
}

pub(crate) async fn bind_client_mesh_tcp_listener(bind_address: SocketAddr) -> io::Result<TcpListener> {
    if !bind_address.ip().is_unspecified() {
        return TcpListener::bind(bind_address).await;
    }
    let socket = socket2::Socket::new(
        socket2::Domain::IPV6,
        socket2::Type::STREAM,
        Some(socket2::Protocol::TCP),
    )?;
    socket.set_only_v6(false)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    let dual_stack_address = SocketAddr::from(([0_u16; 8], bind_address.port()));
    socket.bind(&dual_stack_address.into())?;
    socket.listen(128)?;
    let listener = std::net::TcpListener::from(socket);
    TcpListener::from_std(listener)
}

/// Keeps in-process session tests operational. The app explicitly disables
/// this placeholder and must publish real scenario/dynamic resource cores
/// before admitting peers; these synthetic cores cannot boot a stock client.
pub(crate) fn synthetic_join_snapshot(
    local_core: clonk_engine::ClientCoreControlData,
    max_players: usize,
) -> HostJoinSnapshot {
    let empty_players = crate::PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
    HostJoinSnapshot {
        dynamic: clonk_engine::NetworkResourceCore {
            resource_type: 2,
            id: 1,
            derived_id: -1,
            loadable: true,
            file_size: 1,
            file_crc: 0,
            contents_crc: 0,
            filename: clonk_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec())
                .expect("static dynamic resource name is NUL-free"),
            ..Default::default()
        },
        dynamic_tick: 0,
        parameters: crate::JoinGameParametersEnvelope {
            random_seed: 0,
            startup_player_count: 0,
            max_players: i32::try_from(max_players).unwrap_or(i32::MAX),
            use_fair_crew: false,
            fair_crew_forced: false,
            fair_crew_strength: 0,
            allow_debug: true,
            is_network_game: true,
            control_rate: 1,
            auto_frame_skip: false,
            rules: Vec::new(),
            goals: Vec::new(),
            league: clonk_engine::LegacyCString::default(),
            league_address: clonk_engine::LegacyCString::default(),
            title: clonk_engine::LegacyCString::from_bytes(b"No title".to_vec())
                .expect("static title is NUL-free"),
            scenario: clonk_engine::NetworkResourceCore {
                resource_type: 1,
                id: 2,
                derived_id: -1,
                loadable: true,
                file_size: 1,
                file_crc: 0,
                contents_crc: 0,
                filename: clonk_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec())
                    .expect("static scenario resource name is NUL-free"),
                ..Default::default()
            },
            game_resources: Vec::new(),
            player_infos: empty_players.clone(),
            restore_player_infos: empty_players,
            teams: crate::JoinTeamListSnapshot {
                active: 1,
                custom: 0,
                allow_hostility_change: 1,
                allow_team_switch: 0,
                auto_generate_teams: 1,
                last_team_id: 0,
                team_distribution: 0,
                team_colors: 0,
                max_script_players: 0,
                script_player_names: clonk_engine::LegacyCString::default(),
                random_team_count: 0,
                teams: Vec::new(),
            },
            clients: JoinClientRegistrySnapshot {
                clients: vec![local_core],
                local_client_id: Some(0),
            },
        },
    }
}

/// Events emitted by the host loop.
#[derive(Debug)]
pub enum HostEvent {
    /// Complete current address list for the host client after an
    /// AddAddrFromPuncher update. Reference invalidation waits for AssID.
    LocalAddressesChanged {
        local_addresses: Vec<crate::NetworkAddress>,
    },
    /// A family-specific AssID arrived. The complete IDs and current address
    /// list let the application rebuild one exact advertised reference.
    NetpuncherStateChanged {
        game_ids: NetpuncherGameIds,
        local_addresses: Vec<crate::NetworkAddress>,
    },
    /// The authoritative host barrier was opened or retargeted. Runtime
    /// control owns the actual drive-to-target/stop operation, so surface
    /// every accepted `ChangeGameStatus` rather than inferring it from raw
    /// client acknowledgements.
    StatusChanged(NetworkStatus),
    StatusCommitted(NetworkStatus),
    StatusAck {
        client_id: ClientId,
        status: NetworkStatus,
    },
    ActivationRequest {
        client_id: ClientId,
        tick: i32,
        waited_for: bool,
        ping_ms: i32,
    },
    PlayerInfoUpdate {
        client_id: ClientId,
        request: crate::PlayerInfoUpdateRequest,
    },
    LobbyCountdown {
        packet: LobbyCountdownPacket,
    },
    ReadyCheck {
        packet: ReadyCheckPacket,
    },
    ResourceAction(crate::ResourceCatalogAction),
    ResourceProgress {
        resource_id: i32,
        present_percent: u8,
    },
    ResourceComplete {
        resource_id: i32,
        core: clonk_engine::NetworkResourceCore,
        path: PathBuf,
        local: bool,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
    ResourceDeriveUnsupported {
        core: clonk_engine::NetworkResourceCore,
    },
    ClientJoined {
        client_id: ClientId,
        name: String,
        kind: ParticipantKind,
    },
    ClientLeft {
        client_id: ClientId,
    },
    /// The final accepted route for a logical client was lost unexpectedly.
    /// Controlled synchronized removal, host shutdown, and loss of a route
    /// with a surviving fallback do not emit this event.
    ClientConnectionFailed {
        client_id: ClientId,
    },
    JoinDataNeeded {
        client_id: ClientId,
        current_control_tick: Tick,
    },
    Ready {
        packet: ControlPacket,
    },
    Direct {
        client_id: ClientId,
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    SyncScheduled {
        control_tick: Tick,
        controls: Vec<clonk_engine::ControlPacket>,
    },
    ExecSync {
        control_tick: Tick,
    },
    UnhandledPacket {
        client_id: Option<ClientId>,
        packet_type: u8,
    },
    TransportError {
        client_id: Option<ClientId>,
        error: String,
    },
}

/// Commands issued by the runtime to influence the host loop.
#[derive(Debug)]
pub enum HostCommand {
    ChangeStatus(NetworkStatus),
    BeginGo {
        status: NetworkStatus,
        join_allowed: bool,
        completion: oneshot::Sender<()>,
    },
    BroadcastStatusAck(NetworkStatus),
    ControlTickReached {
        tick: Tick,
        control_rate: i32,
        target_fps: i32,
        reached_at: tokio::time::Instant,
    },
    ControlTickConsumed {
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: Vec<ClientId>,
        reset_performance: bool,
    },
    StatusReachedCurrent,
    StatusReached {
        status: NetworkStatus,
        actual_control_tick: i32,
    },
    SubmitLocal(ControlPacket),
    SubmitLobbyCountdown(LobbyCountdownPacket),
    SubmitReadyCheck(ReadyCheckPacket),
    BroadcastLeagueRoundResults(crate::LeagueRoundResultsPacket),
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
    PublishJoinSnapshot(Box<HostJoinSnapshot>),
    PublishRuntimeDynamic {
        dynamic: Box<crate::LiveNetworkDynamic>,
        dynamic_tick: i32,
        parameters: Box<crate::JoinGameParametersEnvelope>,
        completion: oneshot::Sender<Result<clonk_engine::NetworkResourceCore, String>>,
    },
    RemoveRuntimeDynamic {
        completion: oneshot::Sender<Result<bool, String>>,
    },
    FailPendingJoinData {
        reason: clonk_engine::LegacyCString,
        completion: oneshot::Sender<usize>,
    },
    PublishPlayerResource {
        request: crate::ClientPlayerResourceRequest,
        completion: oneshot::Sender<Result<clonk_engine::NetworkResourceCore, String>>,
    },
    BeginResourceDerive {
        resource_id: i32,
        source_path: PathBuf,
        ownership: crate::ResourceFileOwnership,
        completion: oneshot::Sender<Result<crate::ResourceDerivation, String>>,
    },
    FinishResourceDerive {
        derivation: crate::ResourceDerivation,
        completion: oneshot::Sender<Result<clonk_engine::NetworkResourceCore, String>>,
    },
    SetJoinAllowed {
        allowed: bool,
        completion: oneshot::Sender<()>,
    },
    InspectRuntimeClientStates {
        tick: Tick,
        reset_performance: bool,
        completion: oneshot::Sender<Vec<RuntimeNetworkClientState>>,
    },
    SetPassword {
        password: Option<clonk_engine::LegacyCString>,
        completion: oneshot::Sender<()>,
    },
    InitNetpunchers {
        addresses: Vec<SocketAddr>,
        completion: oneshot::Sender<()>,
    },
    InspectRuntimeConnections {
        completion: oneshot::Sender<Vec<RuntimeNetworkConnection>>,
    },
    InspectLobbyClientTelemetry {
        client_ids: Vec<ClientId>,
        completion: oneshot::Sender<RuntimeLobbyClientTelemetry>,
    },
    DisconnectRuntimeConnection {
        connection_id: u32,
        completion: oneshot::Sender<bool>,
    },
    #[cfg(test)]
    InspectAcceptedRoutes {
        completion: oneshot::Sender<Vec<(u32, ClientId, u32)>>,
    },
    #[cfg(test)]
    InspectConnectedClients {
        completion: oneshot::Sender<Vec<ClientId>>,
    },
    Shutdown,
}

/// Handle for interacting with a running host loop.
#[derive(Debug)]
pub struct HostHandle {
    pub(crate) command_tx: mpsc::Sender<HostCommand>,
    pub(crate) event_rx: Option<mpsc::Receiver<HostEvent>>,
    pub(crate) shutdown_tx: Option<oneshot::Sender<()>>,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
    pub(crate) udp_local_addr: Option<SocketAddr>,
    pub(crate) io_statistics: crate::NetworkIoStatistics,
}

impl HostHandle {
    pub fn udp_local_addr(&self) -> Option<SocketAddr> {
        self.udp_local_addr
    }

    pub fn io_statistics(&self) -> crate::NetworkIoStatistics {
        self.io_statistics.clone()
    }

    pub fn events(&mut self) -> &mut mpsc::Receiver<HostEvent> {
        self.event_rx
            .as_mut()
            .expect("host event receiver already taken")
    }

    pub fn take_event_receiver(&mut self) -> mpsc::Receiver<HostEvent> {
        self.event_rx
            .take()
            .expect("host event receiver already taken")
    }

    pub async fn submit_local_control(&self, packet: ControlPacket) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::SubmitLocal(packet))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn submit_ready_check(&self, packet: ReadyCheckPacket) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::SubmitReadyCheck(packet))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn submit_lobby_countdown(
        &self,
        packet: LobbyCountdownPacket,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::SubmitLobbyCountdown(packet))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Broadcasts the host's final league result packet to every connected
    /// logical client as `PID_LeagueRoundResults`.
    pub async fn broadcast_league_round_results(
        &self,
        packet: crate::LeagueRoundResultsPacket,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::BroadcastLeagueRoundResults(packet))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn change_status(&self, status: NetworkStatus) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::ChangeStatus(status))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Applies the lobby's Go barrier and runtime admission policy as one
    /// host-loop operation. No pending admission request can be processed
    /// between the two state changes.
    pub async fn begin_go(
        &self,
        status: NetworkStatus,
        join_allowed: bool,
    ) -> Result<(), HostError> {
        let (completion, applied) = oneshot::channel();
        self.command_tx
            .send(HostCommand::BeginGo {
                status,
                join_allowed,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        applied.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn broadcast_status_ack(&self, status: NetworkStatus) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::BroadcastStatusAck(status))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Stamps the wall-clock instant at which the game first attempts this
    /// control frame. Repeated stalled-frame probes retain the first stamp.
    pub async fn control_tick_reached(
        &self,
        tick: Tick,
        control_rate: i32,
        target_fps: i32,
        reached_at: tokio::time::Instant,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::ControlTickReached {
                tick,
                control_rate,
                target_fps,
                reached_at,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn control_tick_consumed(
        &self,
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: Vec<ClientId>,
        reset_performance: bool,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::ControlTickConsumed {
                tick,
                consumed_at,
                client_ids,
                reset_performance,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Report local arrival only for the barrier the caller actually drove.
    /// A higher remote acknowledgement may retarget the session before this
    /// command is processed, in which case the stale arrival must be ignored.
    pub async fn status_reached(
        &self,
        status: NetworkStatus,
        actual_control_tick: i32,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::StatusReached {
                status,
                actual_control_tick,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Preserve the scenario-start path's existing ownership: FinalInit
    /// reaches whichever authoritative barrier is current when the command is
    /// processed. Runtime transitions use `status_reached` so stale reports
    /// cannot satisfy a later generation.
    pub async fn status_reached_current(&self) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::StatusReachedCurrent)
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn submit_packet(
        &self,
        delivery: ControlDelivery,
        data: Vec<u8>,
    ) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::SubmitPacket { delivery, data })
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn submit_exec_sync(&self, control_tick: Tick) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::ExecSync { control_tick })
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn publish_join_snapshot(&self, snapshot: HostJoinSnapshot) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::PublishJoinSnapshot(Box::new(snapshot)))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    /// Publishes the dynamic produced at a synchronized runtime-join save
    /// boundary and wakes every accepted client still waiting for JoinData.
    pub async fn publish_runtime_dynamic(
        &self,
        dynamic: crate::LiveNetworkDynamic,
        dynamic_tick: i32,
        parameters: crate::JoinGameParametersEnvelope,
    ) -> Result<clonk_engine::NetworkResourceCore, HostError> {
        let (completion, published) = oneshot::channel();
        self.command_tx
            .send(HostCommand::PublishRuntimeDynamic {
                dynamic: Box::new(dynamic),
                dynamic_tick,
                parameters: Box::new(parameters),
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        published
            .await
            .map_err(|_| HostError::HostLoopGone)?
            .map_err(HostError::Resource)
    }

    /// Clears the currently advertised runtime dynamic and schedules its
    /// retained temporary resource for C++-style delayed cleanup.
    pub async fn remove_runtime_dynamic(&self) -> Result<bool, HostError> {
        let (completion, removed) = oneshot::channel();
        self.command_tx
            .send(HostCommand::RemoveRuntimeDynamic { completion })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        removed
            .await
            .map_err(|_| HostError::HostLoopGone)?
            .map_err(HostError::Resource)
    }

    /// Emergency-removes every accepted client for which JoinData has not
    /// been sent. Removal is a host-authored synchronized control, not a raw
    /// transport close.
    pub async fn fail_pending_join_data(
        &self,
        reason: clonk_engine::LegacyCString,
    ) -> Result<usize, HostError> {
        let (completion, removed) = oneshot::channel();
        self.command_tx
            .send(HostCommand::FailPendingJoinData { reason, completion })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        removed.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn publish_player_resource(
        &self,
        request: crate::ClientPlayerResourceRequest,
    ) -> Result<clonk_engine::NetworkResourceCore, HostError> {
        let (completion, published) = oneshot::channel();
        self.command_tx
            .send(HostCommand::PublishPlayerResource {
                request,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        published
            .await
            .map_err(|_| HostError::HostLoopGone)?
            .map_err(HostError::Resource)
    }

    /// Protects a complete resource before its source file is rewritten.
    pub async fn begin_resource_derive(
        &self,
        resource_id: i32,
        source_path: impl Into<PathBuf>,
        ownership: crate::ResourceFileOwnership,
    ) -> Result<crate::ResourceDerivation, HostError> {
        let (completion, derived) = oneshot::channel();
        self.command_tx
            .send(HostCommand::BeginResourceDerive {
                resource_id,
                source_path: source_path.into(),
                ownership,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        derived
            .await
            .map_err(|_| HostError::HostLoopGone)?
            .map_err(HostError::Resource)
    }

    /// Publishes the rewritten source under a fresh ID and broadcasts its
    /// `PID_NetResDerive` core to every connected peer.
    pub async fn finish_resource_derive(
        &self,
        derivation: crate::ResourceDerivation,
    ) -> Result<clonk_engine::NetworkResourceCore, HostError> {
        let (completion, finished) = oneshot::channel();
        self.command_tx
            .send(HostCommand::FinishResourceDerive {
                derivation,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        finished
            .await
            .map_err(|_| HostError::HostLoopGone)?
            .map_err(HostError::Resource)
    }

    pub async fn set_join_allowed(&self, allowed: bool) -> Result<(), HostError> {
        let (completion, applied) = oneshot::channel();
        self.command_tx
            .send(HostCommand::SetJoinAllowed {
                allowed,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        applied.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn set_password(
        &self,
        password: Option<clonk_engine::LegacyCString>,
    ) -> Result<(), HostError> {
        let (completion, applied) = oneshot::channel();
        self.command_tx
            .send(HostCommand::SetPassword {
                password,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        applied.await.map_err(|_| HostError::HostLoopGone)
    }

    /// Re-runs the host's NAT-puncher handshake for the first address of
    /// each resolved family. Live Internet signup uses the same reliable-UDP
    /// socket as startup so the externally observed port remains stable.
    pub async fn init_netpunchers(&self, addresses: Vec<SocketAddr>) -> Result<(), HostError> {
        let (completion, initialized) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InitNetpunchers {
                addresses,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        initialized.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn runtime_connections(&self) -> Result<Vec<RuntimeNetworkConnection>, HostError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectRuntimeConnections { completion })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        inspected.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn lobby_client_telemetry(
        &self,
        client_ids: Vec<ClientId>,
    ) -> Result<RuntimeLobbyClientTelemetry, HostError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectLobbyClientTelemetry {
                client_ids,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        inspected.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn runtime_client_states(
        &self,
        tick: Tick,
        reset_performance: bool,
    ) -> Result<Vec<RuntimeNetworkClientState>, HostError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectRuntimeClientStates {
                tick,
                reset_performance,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        inspected.await.map_err(|_| HostError::HostLoopGone)
    }

    pub async fn disconnect_runtime_connection(&self, connection_id: u32) -> Result<(), HostError> {
        let (completion, disconnected) = oneshot::channel();
        self.command_tx
            .send(HostCommand::DisconnectRuntimeConnection {
                connection_id,
                completion,
            })
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        if disconnected.await.map_err(|_| HostError::HostLoopGone)? {
            Ok(())
        } else {
            Err(HostError::ConnectionNotFound(connection_id))
        }
    }

    #[cfg(test)]
    pub(crate) async fn accepted_routes(&self) -> Vec<(u32, ClientId, u32)> {
        let (completion, routes) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectAcceptedRoutes { completion })
            .await
            .expect("test host loop accepts route inspection");
        routes
            .await
            .expect("test host loop returns route inspection")
    }

    #[cfg(test)]
    pub(crate) async fn connected_clients(&self) -> Vec<ClientId> {
        let (completion, clients) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectConnectedClients { completion })
            .await
            .expect("test host loop accepts client inspection");
        clients
            .await
            .expect("test host loop returns client inspection")
    }

    pub async fn shutdown(mut self) -> Result<(), HostError> {
        self.event_rx.take();
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        drop(self.command_tx);
        self.join_handle
            .await
            .map_err(|_| HostError::HostLoopGone)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("failed to bind listener: {0}")]
    Bind(#[from] io::Error),
    #[error("no host transport is available")]
    NoTransport,
    #[error("host loop terminated unexpectedly")]
    HostLoopGone,
    #[error("host resource initialization failed: {0}")]
    Resource(String),
    #[error("runtime connection {0} is not active")]
    ConnectionNotFound(u32),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("failed to connect to host: {0}")]
    Connect(#[from] io::Error),
    #[error("host rejected the client password: {message:?}")]
    WrongPassword { message: clonk_engine::LegacyCString },
    #[error("handshake rejected: {0}")]
    Handshake(String),
    #[error("client resource publication failed: {0}")]
    Resource(String),
    #[error("failed to notify host before leaving: {0}")]
    GracefulPart(String),
    #[error("client loop terminated unexpectedly")]
    ClientLoopGone,
    #[error("runtime connection {0} is not active")]
    ConnectionNotFound(u32),
}

pub(crate) enum ClientAttemptError {
    Retryable(ClientError),
    WrongPassword(ClientError),
    Terminal(ClientError),
}

impl ClientAttemptError {
    pub(crate) fn into_error(self) -> ClientError {
        match self {
            Self::Retryable(error) | Self::WrongPassword(error) | Self::Terminal(error) => error,
        }
    }
}

impl From<ClientError> for ClientAttemptError {
    fn from(error: ClientError) -> Self {
        Self::Terminal(error)
    }
}

/// Events observed by a connected client.
#[derive(Debug)]
pub enum ClientEvent {
    /// Complete current address list for the local logical client after an
    /// AddAddrFromPuncher update.
    LocalAddressesChanged {
        local_addresses: Vec<crate::NetworkAddress>,
    },
    PingMeasured {
        round_trip_ms: i32,
    },
    Status(NetworkStatus),
    StatusAck(NetworkStatus),
    LobbyCountdown {
        packet: LobbyCountdownPacket,
    },
    ReadyCheck {
        packet: ReadyCheckPacket,
    },
    Ready {
        packet: ControlPacket,
    },
    Direct {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    SyncScheduled {
        control_tick: Tick,
        controls: Vec<clonk_engine::ControlPacket>,
    },
    ExecSync {
        control_tick: Tick,
    },
    ResourceAction(crate::ResourceCatalogAction),
    ResourceProgress {
        resource_id: i32,
        present_percent: u8,
    },
    ResourceComplete {
        resource_id: i32,
        core: clonk_engine::NetworkResourceCore,
        path: PathBuf,
        local: bool,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
    ResourceDeriveUnsupported {
        core: clonk_engine::NetworkResourceCore,
    },
    LeagueRoundResults {
        packet: crate::LeagueRoundResultsPacket,
    },
    UnhandledPacket {
        packet_type: u8,
    },
    Disconnected {
        reason: Option<String>,
    },
}

/// Commands available to a connected client.
#[derive(Debug)]
pub enum ClientCommand {
    SubmitStatusAck(NetworkStatus),
    SubmitReadyCheck(ReadyCheckPacket),
    RequestActivation(i32),
    SubmitPlayerInfoUpdate(crate::PlayerInfoUpdateRequest),
    SubmitControl(ControlPacket),
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
    SubmitResource(ResourcePacket),
    RemoveResource {
        resource_id: i32,
        completion: oneshot::Sender<Result<(), String>>,
    },
    PublishPlayerResource {
        request: crate::ClientPlayerResourceRequest,
        completion: oneshot::Sender<Result<clonk_engine::NetworkResourceCore, String>>,
    },
    BeginResourceDerive {
        resource_id: i32,
        source_path: PathBuf,
        ownership: crate::ResourceFileOwnership,
        completion: oneshot::Sender<Result<crate::ResourceDerivation, String>>,
    },
    GracefulPart {
        completion: oneshot::Sender<Result<(), String>>,
    },
    ControlTickReached {
        tick: Tick,
        reached_at: tokio::time::Instant,
    },
    ControlTickConsumed {
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: Vec<ClientId>,
        reset_performance: bool,
    },
    InspectRuntimeClientStates {
        tick: Tick,
        reset_performance: bool,
        completion: oneshot::Sender<Vec<RuntimeNetworkClientState>>,
    },
    InspectRuntimeConnections {
        completion: oneshot::Sender<Vec<RuntimeNetworkConnection>>,
    },
    InspectLobbyClientTelemetry {
        client_ids: Vec<ClientId>,
        completion: oneshot::Sender<RuntimeLobbyClientTelemetry>,
    },
    DisconnectRuntimeConnection {
        connection_id: u32,
        completion: oneshot::Sender<bool>,
    },
    #[cfg(test)]
    InspectMeshPeers {
        completion: oneshot::Sender<Vec<ClientId>>,
    },
    #[cfg(test)]
    InspectMeshAddressCount {
        peer_id: i32,
        completion: oneshot::Sender<usize>,
    },
    #[cfg(test)]
    ForceMeshAttempt {
        peer_id: i32,
        completion: oneshot::Sender<()>,
    },
    Shutdown,
}

/// Handle for a connected client.
#[derive(Debug)]
pub struct ClientHandle {
    pub(crate) command_tx: mpsc::Sender<ClientCommand>,
    pub(crate) event_rx: Option<mpsc::Receiver<ClientEvent>>,
    pub(crate) shutdown_tx: Option<oneshot::Sender<()>>,
    pub(crate) join_handle: tokio::task::JoinHandle<()>,
    pub(crate) client_id: ClientId,
    pub(crate) join_data: Option<JoinDataEnvelope>,
    pub(crate) io_statistics: crate::NetworkIoStatistics,
}

impl ClientHandle {
    pub fn io_statistics(&self) -> crate::NetworkIoStatistics {
        self.io_statistics.clone()
    }

    pub fn events(&mut self) -> &mut mpsc::Receiver<ClientEvent> {
        self.event_rx
            .as_mut()
            .expect("client event receiver already taken")
    }

    pub fn take_event_receiver(&mut self) -> mpsc::Receiver<ClientEvent> {
        self.event_rx
            .take()
            .expect("client event receiver already taken")
    }

    pub fn client_id(&self) -> ClientId {
        self.client_id
    }

    pub fn take_join_data(&mut self) -> Option<JoinDataEnvelope> {
        self.join_data.take()
    }

    pub async fn submit_resource(&self, packet: ResourcePacket) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitResource(packet))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn remove_resource(&self, resource_id: i32) -> Result<(), ClientError> {
        let (completion, removed) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::RemoveResource {
                resource_id,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        removed
            .await
            .map_err(|_| ClientError::ClientLoopGone)?
            .map_err(ClientError::Resource)
    }

    pub async fn publish_player_resource(
        &self,
        request: crate::ClientPlayerResourceRequest,
    ) -> Result<clonk_engine::NetworkResourceCore, ClientError> {
        let (completion, published) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::PublishPlayerResource {
                request,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        published
            .await
            .map_err(|_| ClientError::ClientLoopGone)?
            .map_err(ClientError::Resource)
    }

    /// Protects a local complete resource and waits for the control host's
    /// matching derive announcement after the source is rewritten.
    pub async fn begin_resource_derive(
        &self,
        resource_id: i32,
        source_path: impl Into<PathBuf>,
        ownership: crate::ResourceFileOwnership,
    ) -> Result<crate::ResourceDerivation, ClientError> {
        let (completion, derived) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::BeginResourceDerive {
                resource_id,
                source_path: source_path.into(),
                ownership,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        derived
            .await
            .map_err(|_| ClientError::ClientLoopGone)?
            .map_err(ClientError::Resource)
    }

    pub async fn submit_player_info_update(
        &self,
        request: crate::PlayerInfoUpdateRequest,
    ) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitPlayerInfoUpdate(request))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn request_activation(&self, tick: i32) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::RequestActivation(tick))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn submit_status_ack(&self, status: NetworkStatus) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitStatusAck(status))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn submit_ready_check(&self, packet: ReadyCheckPacket) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitReadyCheck(packet))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn submit_control(&self, packet: ControlPacket) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitControl(packet))
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn submit_packet(
        &self,
        delivery: ControlDelivery,
        data: Vec<u8>,
    ) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitPacket { delivery, data })
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn submit_exec_sync(&self, control_tick: Tick) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::ExecSync { control_tick })
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    /// Stamps the first local cadence at which the game attempts this control
    /// tick. It is paired only with receiver-local packet arrival instants.
    pub async fn control_tick_reached(
        &self,
        tick: Tick,
        reached_at: tokio::time::Instant,
    ) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::ControlTickReached { tick, reached_at })
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn control_tick_consumed(
        &self,
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: Vec<ClientId>,
        reset_performance: bool,
    ) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::ControlTickConsumed {
                tick,
                consumed_at,
                client_ids,
                reset_performance,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn runtime_client_states(
        &self,
        tick: Tick,
        reset_performance: bool,
    ) -> Result<Vec<RuntimeNetworkClientState>, ClientError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::InspectRuntimeClientStates {
                tick,
                reset_performance,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        inspected.await.map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn runtime_connections(&self) -> Result<Vec<RuntimeNetworkConnection>, ClientError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::InspectRuntimeConnections { completion })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        inspected.await.map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn lobby_client_telemetry(
        &self,
        client_ids: Vec<ClientId>,
    ) -> Result<RuntimeLobbyClientTelemetry, ClientError> {
        let (completion, inspected) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::InspectLobbyClientTelemetry {
                client_ids,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        inspected.await.map_err(|_| ClientError::ClientLoopGone)
    }

    pub async fn disconnect_runtime_connection(
        &self,
        connection_id: u32,
    ) -> Result<(), ClientError> {
        let (completion, disconnected) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::DisconnectRuntimeConnection {
                connection_id,
                completion,
            })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        if disconnected
            .await
            .map_err(|_| ClientError::ClientLoopGone)?
        {
            Ok(())
        } else {
            Err(ClientError::ConnectionNotFound(connection_id))
        }
    }

    pub async fn graceful_part(mut self) -> Result<(), ClientError> {
        self.event_rx.take();
        let (completion, sent) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::GracefulPart { completion })
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        let sent = sent
            .await
            .map_err(|_| ClientError::ClientLoopGone)?
            .map_err(ClientError::GracefulPart);
        self.join_handle
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        sent
    }

    #[cfg(test)]
    pub(crate) async fn mesh_peer_ids(&self) -> Vec<ClientId> {
        let (completion, peers) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::InspectMeshPeers { completion })
            .await
            .expect("client loop accepts mesh inspection");
        peers.await.expect("client loop returns mesh inspection")
    }

    #[cfg(test)]
    pub(crate) async fn mesh_address_count(&self, peer_id: ClientId) -> usize {
        let (completion, count) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::InspectMeshAddressCount {
                peer_id: i32::try_from(peer_id).unwrap(),
                completion,
            })
            .await
            .expect("client loop accepts mesh-address inspection");
        count
            .await
            .expect("client loop returns mesh-address inspection")
    }

    #[cfg(test)]
    pub(crate) async fn force_mesh_attempt(&self, peer_id: ClientId) {
        let (completion, forced) = oneshot::channel();
        self.command_tx
            .send(ClientCommand::ForceMeshAttempt {
                peer_id: i32::try_from(peer_id).unwrap(),
                completion,
            })
            .await
            .expect("client loop accepts forced mesh attempt");
        forced.await.expect("client loop completes forced mesh attempt");
    }

    pub async fn shutdown(mut self) -> Result<(), ClientError> {
        self.event_rx.take();
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        drop(self.command_tx);
        self.join_handle
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        Ok(())
    }
}

