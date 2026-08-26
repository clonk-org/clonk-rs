//! Host loop state: HostState, async-control waits, send helpers, runtime dynamic publishing.
//!
//! This child module shares the parent session's private protocol machinery;
//! `session.rs` re-exports its crate-facing surface under the original paths.

use super::*;

#[derive(Debug)]
pub(crate) struct ClientSetup {
    pub(crate) join_data: JoinDataEnvelope,
    pub(crate) addresses: Vec<crate::AddressPacket>,
    /// Oldest-first raw `CID_Message` controls queued after JoinData.
    pub(crate) lobby_chat_history: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) enum HostLoopMessage {
    ClientAccepted {
        connection_id: u32,
        remote_connection_id: u32,
        core: clonk_engine::ClientCoreControlData,
        peer_is_port: bool,
        peer_addr: SocketAddr,
        protocol: crate::NetworkProtocol,
        outbound: HostOutboundSender,
        setup_tx: oneshot::Sender<Result<(), String>>,
    },
    ClientMessage {
        connection_id: u32,
        client_id: ClientId,
        message: ControlMessage,
        ping_ms: i32,
    },
    /// Transport-task ping bookkeeping so the route mirrors the C++
    /// connection counters read by `getPingTime`/`getLag`.
    ConnectionPing {
        connection_id: u32,
        client_id: ClientId,
        update: RoutePingUpdate,
    },
    ClientDisconnected {
        connection_id: u32,
        client_id: ClientId,
        next_inbound_packet: u32,
        next_outbound_packet: u32,
        post_mortem: Option<crate::PostMortemPacket>,
        reason: Option<String>,
    },
    AdmissionFailed {
        connection_id: u32,
        error: String,
    },
    UnhandledPacket {
        client_id: Option<ClientId>,
        packet_type: u8,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct AsyncControlWait {
    pub(crate) tick: Tick,
    pub(crate) reached_at: tokio::time::Instant,
    pub(crate) control_rate: i32,
    pub(crate) target_fps: i32,
}

impl AsyncControlWait {
    pub(crate) fn deadline(self, async_max_wait_frames: i32) -> tokio::time::Instant {
        self.reached_at
            + strict_async_control_wait(self.control_rate, async_max_wait_frames, self.target_fps)
    }
}

fn strict_async_control_wait(
    control_rate: i32,
    async_max_wait_frames: i32,
    target_fps: i32,
) -> Duration {
    debug_assert!(target_fps > 0, "control target FPS must be positive");
    let max_wait_ms = i64::from(control_rate)
        .saturating_mul(i64::from(async_max_wait_frames).saturating_mul(1_000))
        / i64::from(target_fps.max(1));
    if max_wait_ms < 0 {
        Duration::ZERO
    } else {
        Duration::from_millis((max_wait_ms as u64).saturating_add(1))
    }
}

#[derive(Debug)]
pub(crate) struct HostState {
    pub(crate) config: HostConfig,
    pub(crate) coordinator: ControlCoordinator,
    /// The game-thread `C4GameControl::ControlTick` phase. The coordinator
    /// owns the next batch to collect and may already be one or more ticks
    /// ahead while the game is still executing an emitted batch.
    pub(crate) game_control_tick: Tick,
    /// First received C4ClientIDAll packet for each not-yet-ready tick.
    pub(crate) pending_complete: BTreeMap<Tick, ControlPacket>,
    pub(crate) backlog: ControlBacklog,
    pub(crate) client_performance: ClientPerformanceStats,
    pub(crate) local_control_backlog: ControlBacklog,
    pub(crate) scheduler: ResyncScheduler,
    pub(crate) clients: BTreeMap<ClientId, ClientConnection>,
    pub(crate) accepted_routes: BTreeMap<u32, AcceptedConnectionRoute>,
    #[cfg(test)]
    pub(crate) accepted_route_waiters: Vec<AcceptedRouteWaiter>,
    pub(crate) control_send_time_epoch: u64,
    pub(crate) closed_routes: crate::post_mortem::ClosedConnectionRouter,
    pub(crate) pending_sync: Vec<clonk_engine::ControlPacket>,
    pub(crate) status_barrier: StatusBarrier,
    pub(crate) last_chase_target_update: Option<tokio::time::Instant>,
    pub(crate) game_started: bool,
    pub(crate) control_mode: i32,
    /// Clients whose contribution was still missing when the host first
    /// reached each host-routed control tick.
    pub(crate) control_waiting_clients: BTreeMap<Tick, BTreeSet<ClientId>>,
    /// Clients whose contribution the async deadline gave up on, so the tick
    /// was packed without it and the input is gone rather than deferred
    /// (`force_expired_async_control`, mirroring `PackCompleteCtrl`,
    /// C4GameControlNetwork.cpp:741-784). Read once when the aggregate ships.
    pub(crate) control_discarded_clients: BTreeMap<Tick, BTreeSet<ClientId>>,
    /// Consecutive ticks each client has failed to deliver before the host
    /// packed without it.
    pub(crate) straggler_late: std::collections::BTreeMap<ClientId, u32>,
    /// What each connected peer announced it can do beyond the C++ protocol.
    /// A peer that never announced is assumed to be stock C++.
    pub(crate) peer_capabilities: crate::PeerCapabilityRegistry,
    pub(crate) async_control_wait: Option<AsyncControlWait>,
    pub(crate) admission: HostAdmission,
    pub(crate) client_cores: BTreeMap<i32, clonk_engine::ClientCoreControlData>,
    pub(crate) client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    pub(crate) netpuncher_game_ids: NetpuncherGameIds,
    pub(crate) pending_kinds: BTreeMap<i32, ParticipantKind>,
    pub(crate) join_snapshot: Option<HostJoinSnapshot>,
    /// Peers that received the current runtime dynamic in JoinData and have
    /// not yet reported every chunk. An already joined peer is not required
    /// to fetch a dynamic published solely for a later joiner.
    pub(crate) dynamic_required_clients: BTreeSet<ClientId>,
    pub(crate) resource_catalog: crate::ResourceCatalog,
    pub(crate) resource_backend: Option<crate::ResourceTransferBackend>,
    pub(crate) published_player_sources: BTreeMap<PathBuf, clonk_engine::NetworkResourceCore>,
    pub(crate) resource_resolver: crate::client_bootstrap::ClientBootstrapResolver,
    pub(crate) resource_epoch: Instant,
    pub(crate) next_connection_id: u32,
    pub(crate) pending_route_peers: BTreeMap<u32, SocketAddr>,
    pub(crate) pending_route_clients: BTreeMap<u32, ClientId>,
    pub(crate) pending_admissions: BTreeMap<u32, i32>,
    pub(crate) pending_post_mortems: BTreeMap<u32, (ClientId, crate::PostMortemPacket, i32)>,
    pub(crate) removing_clients: BTreeSet<ClientId>,
    /// Retained clients whose old-round ingress is quarantined until their
    /// fresh JoinData has reached and been installed by the runtime.
    pub(crate) round_restart_pending_clients: BTreeMap<ClientId, u64>,
    /// The single FIFO route on which each retained client received its
    /// marker and must return the matching acknowledgement.
    pub(crate) round_restart_routes: BTreeMap<ClientId, u32>,
    pub(crate) round_restart_nonce: u64,
    /// Bounded, presentation-only NORMAL/ME controls for late lobby joiners.
    pub(crate) lobby_chat_history: VecDeque<Vec<u8>>,
    pub(crate) event_tx: mpsc::Sender<HostEvent>,
}

#[cfg(test)]
#[derive(Debug)]
pub(crate) struct AcceptedRouteWaiter {
    pub(crate) initial_ids: BTreeSet<u32>,
    pub(crate) expected_count: usize,
    pub(crate) completion: oneshot::Sender<Vec<(u32, ClientId, u32)>>,
}

impl HostState {
    pub(crate) fn invalidate_control_send_time(&mut self) {
        self.control_send_time_epoch = self.control_send_time_epoch.wrapping_add(1);
    }
}

fn validate_host_round_resource_cores(snapshot: &HostJoinSnapshot) -> Result<(), String> {
    let mut cores_by_id = BTreeMap::<i32, clonk_engine::NetworkResourceCore>::new();
    let external_player_cores = snapshot
        .parameters
        .player_infos
        .clients
        .iter()
        .flat_map(|client| &client.players)
        .filter_map(|player| {
            let flags = player.flags;
            (flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
                && flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0
                && flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE == 0)
                .then_some(player.resource.as_ref())
                .flatten()
        });
    for core in std::iter::once(&snapshot.parameters.scenario)
        .chain(&snapshot.parameters.game_resources)
        .chain(std::iter::once(&snapshot.dynamic))
        .chain(external_player_cores)
    {
        if let Some(existing) = cores_by_id.get(&core.id) {
            if existing != core {
                return Err(format!(
                    "restarted round has conflicting resource ID {}",
                    core.id
                ));
            }
        } else {
            cores_by_id.insert(core.id, core.clone());
        }
    }
    Ok(())
}

pub(crate) fn validate_host_round_config(config: &HostConfig) -> Result<(), String> {
    if config.initial_status.state != NETWORK_STATE_LOBBY {
        return Err("restarted round must begin in the network lobby".to_string());
    }
    let snapshot = config
        .initial_join_snapshot
        .as_ref()
        .ok_or_else(|| "restarted round has no JoinData snapshot".to_string())?;
    validate_host_round_resource_cores(snapshot)?;
    let join_data_cores = round_resource_cores(&snapshot.dynamic, &snapshot.parameters);
    for resource in &config.resource_files {
        if join_data_cores
            .get(&resource.core.id)
            .is_some_and(|join_data_core| join_data_core != &resource.core)
        {
            return Err(format!(
                "restarted round resource file ID {} conflicts with its JoinData core",
                resource.core.id
            ));
        }
    }
    if snapshot.dynamic.resource_type == clonk_engine::NETWORK_RESOURCE_TYPE_NULL {
        return Err("restarted round has no loadable dynamic".to_string());
    }
    let start_tick = i32::try_from(config.start_tick)
        .map_err(|_| "restarted round start tick does not fit JoinData".to_string())?;
    if snapshot.dynamic_tick != start_tick {
        return Err(format!(
            "restarted round dynamic tick {} does not equal control tick {start_tick}",
            snapshot.dynamic_tick
        ));
    }

    let mut catalog = crate::ResourceCatalog::new(HOST_CLIENT_ID as i32);
    for registration in &config.resource_registrations {
        if !catalog.register(*registration) {
            return Err(format!(
                "restarted round repeats resource ID {}",
                registration.resource_id
            ));
        }
    }
    if config.resource_directory.is_none() && !config.resource_files.is_empty() {
        return Err(
            "restarted round resource files require a network working directory".to_string(),
        );
    }
    Ok(())
}

pub(crate) struct PreparedHostRoundConfig {
    config: HostConfig,
    coordinator: ControlCoordinator,
    resource_catalog: crate::ResourceCatalog,
    resource_backend: Option<crate::ResourceTransferBackend>,
    published_player_sources: BTreeMap<PathBuf, clonk_engine::NetworkResourceCore>,
    resource_resolver: crate::client_bootstrap::ClientBootstrapResolver,
    join_snapshot: Option<HostJoinSnapshot>,
    client_cores: BTreeMap<i32, clonk_engine::ClientCoreControlData>,
}

pub(crate) fn prepare_host_round_config(
    mut config: HostConfig,
    state: &HostState,
) -> Result<PreparedHostRoundConfig, String> {
    config.local_core.lobby_ready = false;
    let live_client_ids = state
        .clients
        .keys()
        .filter(|client_id| !state.removing_clients.contains(client_id))
        .copied()
        .collect::<BTreeSet<_>>();
    let mut client_cores = BTreeMap::from([(HOST_CLIENT_ID as i32, config.local_core.clone())]);
    client_cores.extend(live_client_ids.iter().filter_map(|client_id| {
        state.clients.get(client_id).map(|client| {
            let mut core = client.core.clone();
            core.activated = false;
            core.lobby_ready = false;
            (core.client_id, core)
        })
    }));
    if let Some(snapshot) = config.initial_join_snapshot.as_mut() {
        let live_wire_ids = client_cores.keys().copied().collect::<BTreeSet<_>>();
        snapshot.parameters.clients =
            JoinClientRegistrySnapshot::new(client_cores.values().cloned().collect());
        snapshot
            .parameters
            .player_infos
            .clients
            .retain(|client| live_wire_ids.contains(&client.client_id));
    }
    let referenced_resource_ids = config
        .initial_join_snapshot
        .as_ref()
        .map(|snapshot| round_resource_cores(&snapshot.dynamic, &snapshot.parameters))
        .unwrap_or_default()
        .into_keys()
        .collect::<BTreeSet<_>>();
    config
        .resource_registrations
        .retain(|registration| referenced_resource_ids.contains(&registration.resource_id));
    config
        .resource_files
        .retain(|resource| referenced_resource_ids.contains(&resource.core.id));
    config
        .player_resource_sources
        .retain(|(_, core)| referenced_resource_ids.contains(&core.id));
    validate_host_round_config(&config)?;
    let retained_resources = host_round_resources(&config);
    let retained_resource_ids = retained_resources.keys().copied().collect::<BTreeSet<_>>();
    let fresh_resource_ids = config
        .resource_registrations
        .iter()
        .map(|registration| registration.resource_id)
        .chain(
            config
                .resource_files
                .iter()
                .map(|resource| resource.core.id),
        )
        .collect::<BTreeSet<_>>();
    let mut resource_catalog = state.resource_catalog.clone();
    for resource_id in &fresh_resource_ids {
        resource_catalog.forget_resource(*resource_id);
    }
    for registration in &config.resource_registrations {
        if !resource_catalog.register(*registration) {
            return Err(format!(
                "could not install restarted round resource ID {}",
                registration.resource_id
            ));
        }
    }
    resource_catalog.retain_resource_ids(&retained_resource_ids);
    resource_catalog.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE);

    let backend_directory = config.resource_directory.as_deref().or_else(|| {
        state
            .resource_backend
            .as_ref()
            .map(crate::ResourceTransferBackend::resource_directory)
    });
    let mut resource_backend = match (&state.resource_backend, backend_directory) {
        (Some(backend), Some(directory)) => Some(
            backend
                .clone_for_round(directory)
                .map_err(|error| error.to_string())?,
        ),
        (None, Some(directory)) => {
            let mut backend = crate::ResourceTransferBackend::new(HOST_CLIENT_ID as i32, directory)
                .map_err(|error| error.to_string())?;
            backend.disarm_temporary_cleanup();
            Some(backend)
        }
        (_, None) => None,
    };
    for resource_id in &fresh_resource_ids {
        if let Some(backend) = resource_backend.as_mut() {
            backend.forget_resource(*resource_id);
        }
    }
    if !config.resource_files.is_empty() {
        let backend = resource_backend
            .as_mut()
            .ok_or_else(|| "restarted round has no filesystem resource backend".to_string())?;
        for resource in &config.resource_files {
            backend
                .register_hosted_resource(
                    resource.core.clone(),
                    &resource.path,
                    resource.ownership,
                    resource.binary_compatible,
                )
                .map_err(|error| error.to_string())?;
        }
    }
    if let Some(backend) = resource_backend.as_mut() {
        backend
            .retain_resources(&retained_resources)
            .map_err(|error| error.to_string())?;
        backend.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE);
    }

    let mut local_candidates = crate::ClientBootstrapLocalCandidates::default();
    local_candidates.extend_search_roots(&config.local_resource_roots);
    let resource_resolver = crate::client_bootstrap::ClientBootstrapResolver::new_with_group_maker(
        &local_candidates,
        config
            .resource_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("Network")),
        config.group_maker.clone(),
    );
    let mut published_player_sources = state.published_player_sources.clone();
    published_player_sources.retain(|_, core| retained_resource_ids.contains(&core.id));
    published_player_sources.extend(
        config
            .player_resource_sources
            .iter()
            .filter(|(_, core)| retained_resource_ids.contains(&core.id))
            .cloned(),
    );
    let join_snapshot = config.initial_join_snapshot.clone();

    let mut coordinator =
        ControlCoordinator::with_start_tick(config.backlog_limit, config.start_tick);
    coordinator
        .register_client(HOST_CLIENT_ID)
        .map_err(|error| error.to_string())?;

    // Transport fields describe the already-running listeners. Retaining the
    // fresh value is still useful because later publications consume its
    // round-scoped fields.
    config.initial_join_snapshot = join_snapshot.clone();
    Ok(PreparedHostRoundConfig {
        config,
        coordinator,
        resource_catalog,
        resource_backend,
        published_player_sources,
        resource_resolver,
        join_snapshot,
        client_cores,
    })
}

pub(crate) fn install_prepared_host_round_config(
    mut prepared: PreparedHostRoundConfig,
    state: &mut HostState,
) {
    if let Some(next_backend) = prepared.resource_backend.as_mut() {
        if let Some(previous_backend) = state.resource_backend.as_mut() {
            next_backend.arm_after_replacing(previous_backend);
        } else {
            next_backend.arm_temporary_cleanup();
        }
    }
    state.resource_backend = prepared.resource_backend.take();
    state.resource_catalog = prepared.resource_catalog;
    state.published_player_sources = prepared.published_player_sources;
    state.resource_resolver = prepared.resource_resolver;
    state.join_snapshot = prepared.join_snapshot;
    state.dynamic_required_clients.clear();
    state.client_cores = prepared.client_cores;
    let required_password =
        (!prepared.config.password.is_empty()).then(|| prepared.config.password.clone());
    state.admission = HostAdmission::new(
        state.admission.next_client_id(),
        prepared.config.allow_join,
        required_password,
        state.client_cores.values().map(|core| core.name.clone()),
    );
    state
        .client_addresses
        .retain(|client_id, _| state.client_cores.contains_key(client_id));
    state
        .pending_kinds
        .retain(|client_id, _| state.client_cores.contains_key(client_id));
    state.coordinator = prepared.coordinator;
    state.game_control_tick = prepared.config.start_tick;
    state.pending_complete.clear();
    state.backlog = ControlBacklog::new(prepared.config.backlog_limit);
    state.client_performance = ClientPerformanceStats::new(prepared.config.backlog_limit);
    state.local_control_backlog = ControlBacklog::new(prepared.config.backlog_limit);
    state.scheduler = ResyncScheduler::new(prepared.config.resync_cooldown);
    state.pending_sync.clear();
    state.status_barrier = StatusBarrier::stable(prepared.config.initial_status);
    state.last_chase_target_update = None;
    state.game_started = false;
    state.control_mode = prepared.config.initial_status.control_mode;
    state.control_waiting_clients.clear();
    state.control_discarded_clients.clear();
    state.straggler_late.clear();
    state.async_control_wait = None;
    state.lobby_chat_history.clear();
    state.invalidate_control_send_time();

    for client in state.clients.values_mut() {
        client.core.activated = false;
        client.core.lobby_ready = false;
        client.join_data_sent = false;
        client.join_data_needed_emitted = false;
    }
    state.round_restart_nonce = state.round_restart_nonce.wrapping_add(1).max(1);
    state.round_restart_pending_clients = state
        .clients
        .keys()
        .copied()
        .map(|client_id| (client_id, state.round_restart_nonce))
        .collect();
    state.round_restart_routes.clear();

    state.config = prepared.config;
}

#[cfg(test)]
pub(crate) fn install_host_round_config(
    config: HostConfig,
    state: &mut HostState,
) -> Result<(), String> {
    let prepared = prepare_host_round_config(config, state)?;
    install_prepared_host_round_config(prepared, state);
    Ok(())
}

fn host_round_resources(config: &HostConfig) -> BTreeMap<i32, clonk_engine::NetworkResourceCore> {
    let mut resources = config
        .initial_join_snapshot
        .as_ref()
        .map(|snapshot| round_resource_cores(&snapshot.dynamic, &snapshot.parameters))
        .unwrap_or_default();
    resources.extend(
        config
            .resource_files
            .iter()
            .map(|resource| (resource.core.id, resource.core.clone())),
    );
    resources
}

fn same_peer_host(left: SocketAddr, right: SocketAddr) -> bool {
    let left = crate::canonical_reliable_udp_peer_address(left);
    let right = crate::canonical_reliable_udp_peer_address(right);
    match (left, right) {
        (SocketAddr::V4(left), SocketAddr::V4(right)) => left.ip() == right.ip(),
        (SocketAddr::V6(left), SocketAddr::V6(right)) => {
            left.ip() == right.ip() && left.scope_id() == right.scope_id()
        }
        _ => false,
    }
}

pub(crate) fn invalidate_pending_client_routes(client_id: ClientId, state: &mut HostState) {
    let pending_route_ids = state
        .pending_route_clients
        .iter()
        .filter_map(|(connection_id, pending_client_id)| {
            (*pending_client_id == client_id).then_some(*connection_id)
        })
        .collect::<Vec<_>>();
    for connection_id in pending_route_ids {
        state.pending_route_clients.remove(&connection_id);
        state.pending_route_peers.remove(&connection_id);
        state.pending_admissions.remove(&connection_id);
    }
    state.pending_admissions.retain(|_, pending_client_id| {
        ClientId::try_from(*pending_client_id).ok() != Some(client_id)
    });
}

pub(crate) fn mark_client_removing(client_id: ClientId, state: &mut HostState) {
    state.removing_clients.insert(client_id);
    state.dynamic_required_clients.remove(&client_id);
    if let Some(remote) = state.status_barrier.remotes.get_mut(&client_id) {
        *remote = RemoteBarrierState::Removing;
    }
    invalidate_pending_client_routes(client_id, state);
}

pub(crate) fn secondary_route_matches_existing_host(
    state: &HostState,
    client_id: ClientId,
    connection_id: u32,
) -> bool {
    let Some(peer_addr) = state.pending_route_peers.get(&connection_id).copied() else {
        return false;
    };
    let mut existing_peers = state
        .accepted_routes
        .values()
        .filter(|route| route.client_id == client_id)
        .map(|route| route.peer_addr)
        .chain(
            state
                .pending_route_clients
                .iter()
                .filter(|(pending_id, pending_client_id)| {
                    **pending_id != connection_id && **pending_client_id == client_id
                })
                .filter_map(|(pending_id, _)| state.pending_route_peers.get(pending_id).copied()),
        )
        .peekable();
    if existing_peers.peek().is_none() {
        return true;
    }
    existing_peers.all(|existing_peer| same_peer_host(existing_peer, peer_addr))
}

pub(crate) fn preferred_host_route(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
) -> Option<&AcceptedConnectionRoute> {
    state
        .accepted_routes
        .values()
        .filter(|route| route.client_id == client_id && !route.outbound.is_closed())
        .min_by_key(|route| match (traffic, route.protocol) {
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Udp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Tcp) => 0,
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Tcp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Udp) => 1,
            _ => 2,
        })
}

pub(crate) fn prepare_host_restart_routes(
    state: &HostState,
) -> Result<BTreeMap<ClientId, u32>, String> {
    if !state.round_restart_pending_clients.is_empty() || !state.round_restart_routes.is_empty() {
        return Err(
            "the previous round restart is awaiting retained client acknowledgement".to_string(),
        );
    }
    state
        .clients
        .keys()
        .filter(|client_id| !state.removing_clients.contains(client_id))
        .copied()
        .map(|client_id| {
            state
                .accepted_routes
                .iter()
                .filter(|(_, route)| {
                    route.client_id == client_id && route.outbound.is_round_restart_route_live()
                })
                .min_by_key(|(connection_id, route)| {
                    let protocol_rank = match route.protocol {
                        crate::NetworkProtocol::Udp => 0_u8,
                        crate::NetworkProtocol::Tcp => 1_u8,
                        _ => 2_u8,
                    };
                    (protocol_rank, **connection_id)
                })
                .map(|(connection_id, _)| (client_id, *connection_id))
                .ok_or_else(|| format!("retained client {client_id} has no accepted message route"))
        })
        .collect()
}

pub(crate) fn retain_host_restart_routes(
    retained_routes: &BTreeMap<ClientId, u32>,
    state: &mut HostState,
) {
    // A connection which has not completed route setup has not joined the
    // stable roster being carried across the round boundary. Forget its
    // provisional association so a later Accepted message is released with
    // setup failure against the fresh round instead of reviving stale state.
    state.pending_route_peers.clear();
    state.pending_route_clients.clear();
    state.pending_admissions.clear();

    let retained_connection_ids = retained_routes.values().copied().collect::<BTreeSet<_>>();
    let retired_connection_ids = state
        .accepted_routes
        .keys()
        .copied()
        .filter(|connection_id| !retained_connection_ids.contains(connection_id))
        .collect::<Vec<_>>();
    for connection_id in retired_connection_ids {
        if let Some(route) = state.accepted_routes.remove(&connection_id) {
            drop(route.outbound.retire_and_take_post_failure());
        }
    }

    for (client_id, connection_id) in retained_routes {
        let route = state
            .accepted_routes
            .get(connection_id)
            .expect("prepared restart route remains accepted");
        if let Some(client) = state.clients.get_mut(client_id) {
            client.outbound = route.outbound.clone();
            client.peer_addr = route.peer_addr;
        }
        if route.protocol != crate::NetworkProtocol::Udp {
            state
                .peer_capabilities
                .clear(*client_id as i32, crate::PortCapabilities::VOICE_CHAT);
        }
    }

    state.pending_post_mortems.clear();
    state.closed_routes = crate::post_mortem::ClosedConnectionRouter::default();
    state.invalidate_control_send_time();
}

fn preferred_host_send_route(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
) -> Option<&AcceptedConnectionRoute> {
    state
        .accepted_routes
        .values()
        .filter(|route| route.client_id == client_id && route.outbound.accepts_post_failure_fifo())
        .min_by_key(|route| match (traffic, route.protocol) {
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Udp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Tcp) => 0,
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Tcp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Udp) => 1,
            _ => 2,
        })
}

fn host_control_send_time_topology(
    state: &HostState,
) -> (BTreeSet<ClientId>, BTreeMap<ClientId, i32>) {
    let known_clients = state
        .client_cores
        .keys()
        .filter_map(|client_id| ClientId::try_from(*client_id).ok())
        .collect::<BTreeSet<_>>();
    let mut preferred_message_routes = BTreeMap::<ClientId, (u8, u32, i32)>::new();

    for (connection_id, route) in &state.accepted_routes {
        if route.outbound.is_closed() || !known_clients.contains(&route.client_id) {
            continue;
        }
        let protocol_rank = match route.protocol {
            crate::NetworkProtocol::Udp => 0,
            crate::NetworkProtocol::Tcp => 1,
            _ => 2,
        };
        let candidate = (protocol_rank, *connection_id, route.ping.ping_ms());
        if preferred_message_routes
            .get(&route.client_id)
            .is_none_or(|best| {
                candidate.0 < best.0 || (candidate.0 == best.0 && candidate.1 < best.1)
            })
        {
            preferred_message_routes.insert(route.client_id, candidate);
        }
    }

    (
        known_clients,
        preferred_message_routes
            .into_iter()
            .map(|(client_id, (_, _, ping_ms))| (client_id, ping_ms))
            .collect(),
    )
}

pub(crate) fn publish_host_control_send_time(
    state: &HostState,
    snapshot: &ControlSendTimeSnapshot,
) {
    let (known_clients, preferred_message_ping_ms) = host_control_send_time_topology(state);
    snapshot.publish(
        state.control_mode,
        HOST_CLIENT_ID,
        known_clients,
        preferred_message_ping_ms,
    );
}

#[cfg(test)]
pub(crate) fn host_control_send_time_ms(
    state: &HostState,
    activated_client_ids: &[ClientId],
) -> i32 {
    let (known_clients, preferred_message_ping_ms) = host_control_send_time_topology(state);
    control_send_time_ms(
        state.control_mode,
        activated_client_ids
            .iter()
            .copied()
            .filter(|client_id| *client_id != HOST_CLIENT_ID)
            .filter(|client_id| known_clients.contains(client_id))
            .map(|client_id| {
                (
                    client_id,
                    preferred_message_ping_ms.get(&client_id).copied(),
                )
            }),
    )
}

fn preferred_host_outbound(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
) -> Option<HostOutboundSender> {
    preferred_host_send_route(state, client_id, traffic).map(|route| route.outbound.clone())
}

pub(crate) fn host_runtime_connections(state: &HostState) -> Vec<RuntimeNetworkConnection> {
    let now = Instant::now();
    state
        .accepted_routes
        .iter()
        .filter(|(_, route)| !route.outbound.is_closed())
        .filter_map(|(connection_id, route)| {
            let is_message =
                preferred_host_route(state, route.client_id, ConnectionTrafficClass::Message)
                    .is_some_and(|preferred| preferred.outbound.same_channel(&route.outbound));
            let is_data =
                preferred_host_route(state, route.client_id, ConnectionTrafficClass::Data)
                    .is_some_and(|preferred| preferred.outbound.same_channel(&route.outbound));
            Some(RuntimeNetworkConnection {
                connection_id: *connection_id,
                client_id: route.client_id,
                usage: runtime_connection_usage(is_message, is_data)?,
                protocol: route.protocol,
                peer_address: Some(route.peer_addr),
                packet_loss: 0,
                ping_ms: route.ping.ping_ms(),
                lag_ms: route.ping.lag_ms(now),
            })
        })
        .collect()
}

pub(crate) fn runtime_lobby_client_telemetry(
    connections: Vec<RuntimeNetworkConnection>,
    catalog: &crate::ResourceCatalog,
    client_ids: Vec<ClientId>,
) -> RuntimeLobbyClientTelemetry {
    RuntimeLobbyClientTelemetry {
        connections,
        resource_progress: client_ids
            .into_iter()
            .map(|client_id| {
                let peer_id = i32::try_from(client_id).unwrap_or(i32::MAX);
                (client_id, catalog.client_progress(peer_id))
            })
            .collect(),
    }
}

pub(crate) fn host_runtime_client_states(
    state: &HostState,
    tick: Tick,
) -> Vec<RuntimeNetworkClientState> {
    let client_ids = std::iter::once(HOST_CLIENT_ID)
        .chain(state.clients.keys().copied())
        .chain(
            state
                .client_cores
                .keys()
                .filter_map(|client_id| ClientId::try_from(*client_id).ok()),
        )
        .chain(state.status_barrier.remotes.keys().copied())
        .chain(state.removing_clients.iter().copied())
        .collect::<BTreeSet<_>>();

    client_ids
        .into_iter()
        .map(|client_id| {
            let lifecycle = if client_id == HOST_CLIENT_ID {
                state.status_barrier.local
            } else if state.removing_clients.contains(&client_id) {
                RemoteBarrierState::Removing
            } else if let Some(lifecycle) = state.status_barrier.remotes.get(&client_id) {
                *lifecycle
            } else {
                // A live client without a barrier acknowledgement is joining;
                // the synchronized client core can also become visible to the
                // roster before admission installs its transport entry.
                RemoteBarrierState::Joining
            };
            RuntimeNetworkClientState {
                client_id,
                status: lifecycle,
                control_ready: state.backlog.contains_packet(client_id, tick),
                wait_ms: state.client_performance.wait_ms(client_id),
            }
        })
        .collect()
}

pub(crate) fn disconnect_host_runtime_connection(
    connection_id: u32,
    state: &mut HostState,
) -> bool {
    let Some(route) = state
        .accepted_routes
        .get(&connection_id)
        .filter(|route| !route.outbound.is_retiring())
    else {
        return false;
    };
    route.outbound.retire();
    state.invalidate_control_send_time();
    true
}

pub(crate) async fn send_host_message(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
    message: ControlMessage,
) -> bool {
    try_send_host_message(state, client_id, traffic, message)
}

pub(crate) async fn send_host_raw(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
    mut packet: Vec<u8>,
) -> bool {
    loop {
        let Some(outbound) = preferred_host_outbound(state, client_id, traffic) else {
            return false;
        };
        match outbound.try_send_raw(packet) {
            Ok(()) => return true,
            Err(mpsc::error::SendError(closed)) => {
                packet = match closed {
                    HostOutboundMessage::Raw(packet) => packet,
                    HostOutboundMessage::Message(_) => unreachable!("sent a raw packet"),
                };
            }
        }
    }
}

pub(crate) fn try_send_host_message(
    state: &HostState,
    client_id: ClientId,
    traffic: ConnectionTrafficClass,
    mut message: ControlMessage,
) -> bool {
    loop {
        let Some(outbound) = preferred_host_outbound(state, client_id, traffic) else {
            return false;
        };
        match outbound.try_send(message) {
            Ok(()) => return true,
            Err(mpsc::error::SendError(closed)) => {
                message = match closed {
                    HostOutboundMessage::Message(message) => message,
                    HostOutboundMessage::Raw(_) => unreachable!("sent a logical message"),
                };
            }
        }
    }
}

/// Queues one logical message on each target client's preferred route.
///
/// C++ caches one message connection on every `C4Network2Client` and performs
/// a broadcast in one connection-list pass (src/C4Network2Client.cpp:497-541).
/// Resolve the equivalent Rust routes in one pass as well. Re-running
/// `preferred_host_send_route` for every client would scan the full route
/// registry once per peer.
pub(crate) fn broadcast_host_message(
    state: &HostState,
    traffic: ConnectionTrafficClass,
    message: ControlMessage,
    except_client_id: Option<ClientId>,
) -> Vec<ClientId> {
    let mut preferred = BTreeMap::<ClientId, (u8, u32, HostOutboundSender)>::new();
    for (connection_id, route) in &state.accepted_routes {
        if Some(route.client_id) == except_client_id
            || !state.clients.contains_key(&route.client_id)
            || !route.outbound.accepts_post_failure_fifo()
        {
            continue;
        }
        let protocol_rank = match (traffic, route.protocol) {
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Udp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Tcp) => 0,
            (ConnectionTrafficClass::Message, crate::NetworkProtocol::Tcp)
            | (ConnectionTrafficClass::Data, crate::NetworkProtocol::Udp) => 1,
            _ => 2,
        };
        let candidate = (protocol_rank, *connection_id);
        if preferred
            .get(&route.client_id)
            .is_none_or(|(best_rank, best_id, _)| candidate < (*best_rank, *best_id))
        {
            preferred.insert(
                route.client_id,
                (protocol_rank, *connection_id, route.outbound.clone()),
            );
        }
    }

    let selected = preferred
        .into_iter()
        .map(|(client_id, (_, _, outbound))| (client_id, outbound))
        .collect::<Vec<_>>();
    if let Some(results) = HostOutboundSender::try_send_many(
        &selected
            .iter()
            .map(|(_, outbound)| outbound.clone())
            .collect::<Vec<_>>(),
        message.clone(),
    ) {
        let mut sent = Vec::with_capacity(selected.len());
        for ((client_id, _), accepted) in selected.into_iter().zip(results) {
            if accepted || try_send_host_message(state, client_id, traffic, message.clone()) {
                sent.push(client_id);
            }
        }
        return sent;
    }

    let mut sent = Vec::with_capacity(selected.len());
    for (client_id, outbound) in selected {
        match outbound.try_send(message.clone()) {
            Ok(()) => sent.push(client_id),
            Err(mpsc::error::SendError(HostOutboundMessage::Message(message))) => {
                if try_send_host_message(state, client_id, traffic, message) {
                    sent.push(client_id);
                }
            }
            Err(mpsc::error::SendError(HostOutboundMessage::Raw(_))) => {
                unreachable!("logical broadcast only queues logical messages")
            }
        }
    }
    sent
}

pub(crate) fn resource_traffic_class(packet: &ResourcePacket) -> ConnectionTrafficClass {
    if matches!(packet, ResourcePacket::Data(_)) {
        ConnectionTrafficClass::Data
    } else {
        ConnectionTrafficClass::Message
    }
}

pub(crate) fn update_derived_resource_sources(
    sources: &mut BTreeMap<PathBuf, clonk_engine::NetworkResourceCore>,
    events: &[crate::ResourceTransferEvent],
) {
    for event in events {
        let crate::ResourceTransferEvent::Completed { core, .. } = event else {
            continue;
        };
        if core.derived_id < 0 {
            continue;
        }
        sources
            .values_mut()
            .filter(|source| source.id == core.derived_id)
            .for_each(|source| *source = core.clone());
    }
}

const MAX_RUNTIME_DYNAMIC_SUFFIX: u32 = 999;

pub(crate) struct PublishedRuntimeDynamic {
    pub(crate) core: clonk_engine::NetworkResourceCore,
    pub(crate) previous_dynamic_id: Option<i32>,
}

pub(crate) fn publish_host_runtime_dynamic(
    dynamic: crate::LiveNetworkDynamic,
    synchronized_control_tick: Tick,
    parameters: crate::JoinGameParametersEnvelope,
    state: &mut HostState,
) -> Result<PublishedRuntimeDynamic, String> {
    let dynamic_tick = i32::try_from(synchronized_control_tick)
        .map_err(|_| "synchronized runtime dynamic tick exceeds the C++ wire field".to_string())?;
    let current_tick = i32::try_from(state.game_control_tick).unwrap_or(i32::MAX);
    if dynamic_tick < current_tick {
        return Err(format!(
            "runtime dynamic tick {dynamic_tick} is stale at host control tick {current_tick}"
        ));
    }
    // Publication is the C4Network2::OnGameSynchronized callback executing
    // inside this exact ControlTick (src/C4Game.cpp:3707-3729;
    // src/C4Network2.cpp:1099-1115,1945-1971). It is authoritative even when
    // the async coordinator has already collected later batches.
    state.game_control_tick = synchronized_control_tick;
    let network_directory = state
        .config
        .resource_directory
        .clone()
        .ok_or_else(|| "host has no network resource directory".to_string())?;
    if state.resource_backend.is_none() {
        return Err("host has no filesystem resource backend".to_string());
    }

    let resource_id = loop {
        let candidate = state.resource_catalog.allocate_resource_id();
        let occupied_by_backend = state
            .resource_backend
            .as_ref()
            .is_some_and(|backend| backend.catalog().contains_resource(candidate));
        if !occupied_by_backend {
            break candidate;
        }
    };
    let source_path = materialize_runtime_dynamic(&network_directory, &dynamic)?;
    let wire_name =
        runtime_dynamic_wire_name(&dynamic.group_filename, &source_path).inspect_err(|_| {
            let _ = fs::remove_file(&source_path);
        })?;
    let core_spec = crate::HostResourceCoreSpec::new_with_raw_group_maker(
        crate::HostResourceType::Dynamic,
        resource_id,
        wire_name,
        state.config.group_maker.clone(),
    );
    let publication = crate::build_host_resource_core(&source_path, &network_directory, core_spec)
        .map_err(|error| {
            let _ = fs::remove_file(&source_path);
            format!("runtime dynamic resource core failed: {error}")
        })?;
    if publication.core.file_size != dynamic.file_size
        || publication.core.file_crc != dynamic.file_crc
        || publication.core.contents_crc != dynamic.contents_crc
        || publication.core.author.as_bytes() != dynamic.maker
    {
        discard_unregistered_runtime_dynamic(&publication, &source_path);
        return Err(format!(
            "runtime dynamic metadata differs: expected size/crc/contents/maker {}/{:08x}/{:08x}/{:?}, got {}/{:08x}/{:08x}/{:?}",
            dynamic.file_size,
            dynamic.file_crc,
            dynamic.contents_crc,
            dynamic.maker,
            publication.core.file_size,
            publication.core.file_crc,
            publication.core.contents_crc,
            publication.core.author.as_bytes(),
        ));
    }
    let Some(standalone_path) = publication.standalone_path.clone() else {
        discard_unregistered_runtime_dynamic(&publication, &source_path);
        return Err("runtime dynamic resource is unexpectedly non-loadable".to_string());
    };
    let Some(ownership) = publication.standalone_ownership else {
        discard_unregistered_runtime_dynamic(&publication, &source_path);
        return Err("runtime dynamic resource has no standalone ownership".to_string());
    };
    let core = publication.core;
    let registration = crate::ResourceRegistration::from_core(&core, true, false);
    let backend = state
        .resource_backend
        .as_mut()
        .ok_or_else(|| "host filesystem resource backend disappeared".to_string())?;
    if let Err(error) =
        backend.register_hosted_resource(core.clone(), &standalone_path, ownership, true)
    {
        let _ = fs::remove_file(&standalone_path);
        if standalone_path != source_path {
            let _ = fs::remove_file(&source_path);
        }
        return Err(format!(
            "runtime dynamic resource registration failed: {error}"
        ));
    }
    if !state.resource_catalog.register(registration) {
        backend.remove_resource(resource_id);
        return Err(format!(
            "resource ID {resource_id} became occupied during runtime dynamic publication"
        ));
    }

    let previous_dynamic_id = state.join_snapshot.as_ref().and_then(|snapshot| {
        (snapshot.dynamic.resource_type == crate::HostResourceType::Dynamic as u8)
            .then_some(snapshot.dynamic.id)
    });
    state.dynamic_required_clients.clear();
    state.join_snapshot = Some(HostJoinSnapshot {
        dynamic: core.clone(),
        dynamic_tick,
        parameters,
    });
    Ok(PublishedRuntimeDynamic {
        core,
        previous_dynamic_id,
    })
}

pub(crate) fn remove_host_runtime_dynamic(state: &mut HostState) -> Result<bool, String> {
    let Some(snapshot) = state.join_snapshot.as_mut() else {
        state.dynamic_required_clients.clear();
        return Ok(false);
    };
    if snapshot.dynamic.resource_type == clonk_engine::NETWORK_RESOURCE_TYPE_NULL {
        state.dynamic_required_clients.clear();
        return Ok(false);
    }
    if snapshot.dynamic.resource_type != crate::HostResourceType::Dynamic as u8 {
        return Err(format!(
            "join snapshot resource {} has non-dynamic type {}",
            snapshot.dynamic.id, snapshot.dynamic.resource_type
        ));
    }
    let resource_id = snapshot.dynamic.id;
    snapshot.dynamic = clonk_engine::NetworkResourceCore::default();
    snapshot.dynamic_tick = -1;
    state.dynamic_required_clients.clear();
    mark_host_resource_removed(resource_id, state);
    Ok(true)
}

pub(crate) fn remove_stale_host_runtime_dynamic(state: &mut HostState) -> bool {
    let current_tick = i32::try_from(state.game_control_tick).unwrap_or(i32::MAX);
    let stale_resource_id = state.join_snapshot.as_ref().and_then(|snapshot| {
        (snapshot.dynamic.resource_type == crate::HostResourceType::Dynamic as u8
            && current_tick > snapshot.dynamic_tick)
            .then_some(snapshot.dynamic.id)
    });
    let Some(resource_id) = stale_resource_id else {
        return false;
    };
    let catalog = state
        .resource_backend
        .as_ref()
        .map(crate::ResourceTransferBackend::catalog)
        .unwrap_or(&state.resource_catalog);
    let clients = &state.clients;
    let removing_clients = &state.removing_clients;
    state.dynamic_required_clients.retain(|client_id| {
        clients.contains_key(client_id)
            && !removing_clients.contains(client_id)
            && !i32::try_from(*client_id)
                .ok()
                .and_then(|client_id| catalog.peer_chunks(resource_id, client_id))
                .is_some_and(crate::ChunkSet::is_complete)
    });
    state.dynamic_required_clients.is_empty() && remove_host_runtime_dynamic(state).unwrap_or(false)
}

pub(crate) fn mark_host_resource_removed(resource_id: i32, state: &mut HostState) {
    state.resource_catalog.remove_resource(resource_id);
    if let Some(backend) = state.resource_backend.as_mut() {
        backend.remove_resource(resource_id);
    }
}

pub(crate) fn advance_shadow_resource_catalog_timer(
    catalog: &mut crate::ResourceCatalog,
    now_seconds: u64,
) {
    // The filesystem backend owns transport effects whenever it exists, but
    // the host also retains this catalog for union-ID allocation. Advance its
    // removal clock without dispatching duplicate discovery/status traffic so
    // IDs marked by `mark_host_resource_removed` are eventually reusable.
    let _ = catalog.on_timer(now_seconds);
}

fn materialize_runtime_dynamic(
    directory: &Path,
    dynamic: &crate::LiveNetworkDynamic,
) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("could not create network resource directory: {error}"))?;
    let basename =
        crate::host_resource_core::network_temp_basename(dynamic.group_filename.as_bytes());
    for suffix in 1..=MAX_RUNTIME_DYNAMIC_SUFFIX {
        let candidate = crate::host_resource_core::network_temp_candidate(&basename, suffix);
        let candidate = String::from_utf8(candidate).expect("FindTempResFileName produces ASCII");
        let path = directory.join(candidate);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(&dynamic.packed_bytes) {
                    let _ = fs::remove_file(&path);
                    return Err(format!("could not materialize runtime dynamic: {error}"));
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(format!("could not materialize runtime dynamic: {error}"));
            }
        }
    }
    Err("no free runtime dynamic filename from 1 through 999".to_string())
}

fn runtime_dynamic_wire_name(
    template: &str,
    materialized_path: &Path,
) -> Result<clonk_engine::LegacyCString, String> {
    let basename = materialized_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "runtime dynamic path has no UTF-8 basename".to_string())?;
    let prefix_length = template
        .as_bytes()
        .iter()
        .rposition(|byte| matches!(byte, b'/' | b'\\'))
        .map_or(0, |separator| separator + 1);
    let mut wire_name = Vec::with_capacity(prefix_length + basename.len());
    wire_name.extend_from_slice(&template.as_bytes()[..prefix_length]);
    wire_name.extend_from_slice(basename.as_bytes());
    clonk_engine::LegacyCString::from_bytes(wire_name)
        .ok_or_else(|| "runtime dynamic wire name contains a NUL".to_string())
}

fn discard_unregistered_runtime_dynamic(
    publication: &crate::HostResourcePublication,
    source_path: &Path,
) {
    if publication.standalone_ownership == Some(crate::ResourceFileOwnership::Temporary) {
        if let Some(path) = publication.standalone_path.as_ref() {
            let _ = fs::remove_file(path);
        }
    }
    if publication.standalone_path.as_deref() != Some(source_path) {
        let _ = fs::remove_file(source_path);
    }
}

pub(crate) fn publish_host_player_resource(
    request: crate::ClientPlayerResourceRequest,
    state: &mut HostState,
) -> Result<clonk_engine::NetworkResourceCore, String> {
    // C4PlayerInfo::LoadFromLocalFile asks getRefRes(source, local-only)
    // before AddByFile, so selecting the same local file reuses its core.
    if let Some(core) = state.published_player_sources.get(&request.source_path) {
        return Ok(core.clone());
    }
    let source_path = request.source_path.clone();
    let network_directory = state
        .config
        .resource_directory
        .clone()
        .ok_or_else(|| "host has no network resource directory".to_string())?;
    if state.resource_backend.is_none() {
        return Err("host has no filesystem resource backend".to_string());
    }

    // The host session retains the protocol catalog alongside the filesystem
    // backend's catalog. Allocate from their union: HostConfig permits a file
    // to be present in resource_files even when it is absent from the explicit
    // resource_registrations list.
    let resource_id = loop {
        let candidate = state.resource_catalog.allocate_resource_id();
        let occupied_by_backend = state
            .resource_backend
            .as_ref()
            .is_some_and(|backend| backend.catalog().contains_resource(candidate));
        if !occupied_by_backend {
            break candidate;
        }
    };
    let publication =
        crate::publish_client_player_resource(crate::ClientPlayerResourcePublicationSpec {
            resource_id,
            source_path: request.source_path,
            wire_name: request.wire_name,
            network_directory,
            group_maker: request.group_maker,
        })
        .map_err(|error| error.to_string())?;
    let crate::ClientPlayerResourcePublication {
        core,
        registration,
        resource_file,
    } = publication;
    let backend = state
        .resource_backend
        .as_mut()
        .ok_or_else(|| "host filesystem resource backend disappeared".to_string())?;
    if let Err(error) = backend.register_hosted_resource(
        resource_file.core,
        &resource_file.path,
        resource_file.ownership,
        resource_file.binary_compatible,
    ) {
        if resource_file.ownership == crate::ResourceFileOwnership::Temporary {
            let _ = std::fs::remove_file(resource_file.path);
        }
        return Err(error.to_string());
    }
    if !state.resource_catalog.register(registration) {
        backend.remove_resource(resource_id);
        return Err(format!(
            "resource ID {resource_id} became occupied during host player publication"
        ));
    }
    state
        .published_player_sources
        .insert(source_path, core.clone());
    Ok(core)
}

pub(crate) fn begin_host_resource_derive(
    resource_id: i32,
    source_path: PathBuf,
    ownership: crate::ResourceFileOwnership,
    state: &mut HostState,
) -> Result<crate::ResourceDerivation, String> {
    let now_seconds = state.resource_epoch.elapsed().as_secs();
    let backend = state
        .resource_backend
        .as_mut()
        .ok_or_else(|| "host has no filesystem resource backend".to_string())?;
    let derivation = backend
        .begin_derive(resource_id, source_path, ownership, now_seconds)
        .map_err(|error| error.to_string())?;
    state
        .resource_catalog
        .register_anonymous_derived_at(resource_id, true, now_seconds);
    Ok(derivation)
}

pub(crate) fn finish_host_resource_derive(
    derivation: crate::ResourceDerivation,
    state: &mut HostState,
) -> Result<
    (
        clonk_engine::NetworkResourceCore,
        Vec<crate::ResourceTransferEvent>,
    ),
    String,
> {
    let parent_resource_id = derivation.parent_resource_id();
    let resource_id = loop {
        let candidate = state.resource_catalog.allocate_resource_id();
        let occupied_by_backend = state
            .resource_backend
            .as_ref()
            .is_some_and(|backend| backend.catalog().contains_resource(candidate));
        if !occupied_by_backend {
            break candidate;
        }
    };
    let backend = state
        .resource_backend
        .as_mut()
        .ok_or_else(|| "host filesystem resource backend disappeared".to_string())?;
    let (core, events) = backend
        .finish_derive(derivation, resource_id)
        .map_err(|error| error.to_string())?;
    let shadow_actions = state.resource_catalog.finish_local_derived(&core);
    if shadow_actions.is_empty() {
        return Err(format!(
            "resource {parent_resource_id} has no session derivation"
        ));
    }
    state
        .published_player_sources
        .values_mut()
        .filter(|published| published.id == parent_resource_id)
        .for_each(|published| *published = core.clone());
    Ok((core, events))
}
