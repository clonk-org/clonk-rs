use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

use crate::legacy::{
    aggregate_control_packets_for_tick, decode_control_entry_payload, validate_control_envelope,
};
use crate::{
    aggregate_ready_batch, reconcile_join_client_registry, run_client_connection_handshake,
    run_host_connection_handshake, AdmissionDecision, BarrierEffect, ClientId, ConnectionAction,
    ConnectionLivenessState, ControlBacklog, ControlCoordinator, ControlDelivery, ControlMessage,
    ControlOutcome, ControlPacket, HostAdmission, HostAdmissionRequest, JoinClientRegistrySnapshot,
    JoinDataEnvelope, LobbyCountdownPacket, MissingRange, NetworkStatus, ParticipantKind,
    ReadyBatch, ReadyCheckPacket, RemoteBarrierState, ResourcePacket, ResyncScheduler,
    StatusBarrier, Tick, TransportError, CURRENT_GAME_BUILD, NETWORK_STATE_GO, NETWORK_STATE_LOBBY,
    NETWORK_STATE_PAUSE,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_BACKLOG_LIMIT: usize = 256;
const HOST_CLIENT_ID: ClientId = 0;
static RESOURCE_RANDOM_STATE: AtomicU64 = AtomicU64::new(1);

fn resource_safe_random(range: usize) -> usize {
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

/// Configuration options for the multiplayer host.
#[derive(Debug, Clone)]
pub struct HostConfig {
    pub backlog_limit: usize,
    pub resync_interval: Duration,
    pub resync_cooldown: Duration,
    pub max_players: usize,
    pub start_tick: Tick,
    pub local_core: lc_engine::ClientCoreControlData,
    pub initial_status: NetworkStatus,
    pub password: lc_engine::LegacyCString,
    pub allow_join: bool,
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
    pub player_resource_sources: Vec<(PathBuf, lc_engine::NetworkResourceCore)>,
    /// C++ resource search roots retained for later authoritative PlayerInfo
    /// resources announced after JoinData.
    pub local_resource_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct HostedResourceFile {
    pub core: lc_engine::NetworkResourceCore,
    pub path: PathBuf,
    pub ownership: crate::ResourceFileOwnership,
    pub binary_compatible: bool,
}

/// The synchronized dynamic/resource state frozen into a host's JoinData.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostJoinSnapshot {
    pub dynamic: lc_engine::NetworkResourceCore,
    pub dynamic_tick: i32,
    pub parameters: crate::JoinGameParametersEnvelope,
}

impl Default for HostConfig {
    fn default() -> Self {
        let name = lc_engine::LegacyCString::from_bytes(b"Host".to_vec())
            .expect("static host name is NUL-free");
        let local_core = lc_engine::ClientCoreControlData {
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
            max_players: 8,
            start_tick: 0,
            local_core: local_core.clone(),
            initial_status: NetworkStatus {
                state: NETWORK_STATE_LOBBY,
                control_mode: 0,
                target_tick: -1,
            },
            password: lc_engine::LegacyCString::default(),
            allow_join: true,
            initial_join_snapshot: Some(synthetic_join_snapshot(local_core, 8)),
            resource_registrations: Vec::new(),
            resource_directory: None,
            resource_files: Vec::new(),
            player_resource_sources: Vec::new(),
            local_resource_roots: Vec::new(),
        }
    }
}

/// Client metadata supplied during handshake.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub name: String,
    pub kind: ParticipantKind,
    pub password: lc_engine::LegacyCString,
    pub resource_directory: Option<PathBuf>,
    pub bootstrap_local_candidates: crate::ClientBootstrapLocalCandidates,
    pub local_system_path: Option<PathBuf>,
    pub local_resource_roots: Vec<PathBuf>,
}

impl ClientConfig {
    pub fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        Self {
            name: name.into(),
            kind,
            password: lc_engine::LegacyCString::default(),
            resource_directory: Some(default_client_resource_directory()),
            bootstrap_local_candidates: crate::ClientBootstrapLocalCandidates::default(),
            local_system_path: None,
            local_resource_roots: Vec::new(),
        }
    }

    pub fn with_password(mut self, password: lc_engine::LegacyCString) -> Self {
        self.password = password;
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
}

fn default_client_resource_directory() -> PathBuf {
    // Application callers replace this with Config.Network.WorkPath. Library
    // callers still need a real ResList backend, so keep their default out of
    // the current source tree while preserving the stock `Network` role.
    std::env::temp_dir()
        .join(format!("legacyclonk-{}", std::process::id()))
        .join("Network")
}

/// Keeps in-process session tests operational. The app explicitly disables
/// this placeholder and must publish real scenario/dynamic resource cores
/// before admitting peers; these synthetic cores cannot boot a stock client.
fn synthetic_join_snapshot(
    local_core: lc_engine::ClientCoreControlData,
    max_players: usize,
) -> HostJoinSnapshot {
    let empty_players = crate::PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    };
    HostJoinSnapshot {
        dynamic: lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 1,
            derived_id: -1,
            loadable: true,
            file_size: 1,
            file_crc: 0,
            contents_crc: 0,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec())
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
            league: lc_engine::LegacyCString::default(),
            league_address: lc_engine::LegacyCString::default(),
            title: lc_engine::LegacyCString::from_bytes(b"No title".to_vec())
                .expect("static title is NUL-free"),
            scenario: lc_engine::NetworkResourceCore {
                resource_type: 1,
                id: 2,
                derived_id: -1,
                loadable: true,
                file_size: 1,
                file_crc: 0,
                contents_crc: 0,
                filename: lc_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec())
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
                script_player_names: lc_engine::LegacyCString::default(),
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
    ResourceComplete {
        resource_id: i32,
        core: lc_engine::NetworkResourceCore,
        path: PathBuf,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
    ResourceDeriveUnsupported {
        core: lc_engine::NetworkResourceCore,
    },
    ClientJoined {
        client_id: ClientId,
        name: String,
        kind: ParticipantKind,
    },
    ClientLeft {
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
        controls: Vec<lc_engine::ControlPacket>,
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
    BroadcastStatusAck(NetworkStatus),
    StatusReached,
    SubmitLocal(ControlPacket),
    SubmitLobbyCountdown(LobbyCountdownPacket),
    SubmitReadyCheck(ReadyCheckPacket),
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
    PublishJoinSnapshot(Box<HostJoinSnapshot>),
    PublishPlayerResource {
        request: crate::ClientPlayerResourceRequest,
        completion: oneshot::Sender<Result<lc_engine::NetworkResourceCore, String>>,
    },
    SetJoinAllowed {
        allowed: bool,
        completion: oneshot::Sender<()>,
    },
    #[cfg(test)]
    InspectAcceptedRoutes {
        completion: oneshot::Sender<Vec<(u32, ClientId, u32)>>,
    },
    Shutdown,
}

/// Handle for interacting with a running host loop.
#[derive(Debug)]
pub struct HostHandle {
    command_tx: mpsc::Sender<HostCommand>,
    event_rx: Option<mpsc::Receiver<HostEvent>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: tokio::task::JoinHandle<()>,
}

impl HostHandle {
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

    pub async fn change_status(&self, status: NetworkStatus) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::ChangeStatus(status))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn broadcast_status_ack(&self, status: NetworkStatus) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::BroadcastStatusAck(status))
            .await
            .map_err(|_| HostError::HostLoopGone)
    }

    pub async fn status_reached(&self) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::StatusReached)
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

    pub async fn publish_player_resource(
        &self,
        request: crate::ClientPlayerResourceRequest,
    ) -> Result<lc_engine::NetworkResourceCore, HostError> {
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

    #[cfg(test)]
    async fn accepted_routes(&self) -> Vec<(u32, ClientId, u32)> {
        let (completion, routes) = oneshot::channel();
        self.command_tx
            .send(HostCommand::InspectAcceptedRoutes { completion })
            .await
            .expect("test host loop accepts route inspection");
        routes.await.expect("test host loop returns route inspection")
    }

    pub async fn shutdown(mut self) -> Result<(), HostError> {
        let _ = self.command_tx.send(HostCommand::Shutdown).await;
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
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
    #[error("host loop terminated unexpectedly")]
    HostLoopGone,
    #[error("host resource initialization failed: {0}")]
    Resource(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("failed to connect to host: {0}")]
    Connect(#[from] io::Error),
    #[error("handshake rejected: {0}")]
    Handshake(String),
    #[error("client resource publication failed: {0}")]
    Resource(String),
    #[error("failed to notify host before leaving: {0}")]
    GracefulPart(String),
    #[error("client loop terminated unexpectedly")]
    ClientLoopGone,
}

/// Starts the multiplayer host loop.
pub async fn start_host(
    listener: TcpListener,
    config: HostConfig,
) -> Result<HostHandle, HostError> {
    let resource_backend = build_host_resource_backend(&config)?;
    let (command_tx, command_rx) = mpsc::channel::<HostCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<HostEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_handle = tokio::spawn(run_host(
        listener,
        config,
        resource_backend,
        command_rx,
        event_tx.clone(),
        shutdown_rx,
    ));
    Ok(HostHandle {
        command_tx,
        event_rx: Some(event_rx),
        shutdown_tx: Some(shutdown_tx),
        join_handle,
    })
}

fn build_host_resource_backend(
    config: &HostConfig,
) -> Result<Option<crate::ResourceTransferBackend>, HostError> {
    let Some(directory) = config.resource_directory.as_ref() else {
        if config.resource_files.is_empty() {
            return Ok(None);
        }
        return Err(HostError::Resource(
            "host resource files require a network working directory".to_string(),
        ));
    };
    let mut backend = crate::ResourceTransferBackend::new(0, directory)
        .map_err(|error| HostError::Resource(error.to_string()))?;
    for resource in &config.resource_files {
        backend
            .register_hosted_resource(
                resource.core.clone(),
                &resource.path,
                resource.ownership,
                resource.binary_compatible,
            )
            .map_err(|error| HostError::Resource(error.to_string()))?;
    }
    Ok(Some(backend))
}

/// Connects to an existing host and returns a handle for interaction.
pub async fn connect_client(
    addr: SocketAddr,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError> {
    connect_client_from(TcpStream::connect(addr), config).await
}

async fn connect_client_from<F>(
    connection: F,
    config: ClientConfig,
) -> Result<ClientHandle, ClientError>
where
    F: Future<Output = Result<TcpStream, io::Error>>,
{
    connect_client_from_inner(connection, config, None).await
}

async fn connect_client_from_inner<F>(
    connection: F,
    config: ClientConfig,
    liveness: Option<ConnectionLivenessState>,
) -> Result<ClientHandle, ClientError>
where
    F: Future<Output = Result<TcpStream, io::Error>>,
{
    let stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection)
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();
    let host_peer_addr = stream.peer_addr().ok();

    let ClientConfig {
        name,
        kind,
        password,
        resource_directory,
        mut bootstrap_local_candidates,
        local_system_path,
        local_resource_roots,
    } = config;
    let wire_name = lc_engine::LegacyCString::from_bytes(name.into_bytes()).ok_or_else(|| {
        ClientError::Handshake("client name contains an interior NUL".to_string())
    })?;
    let local_core = lc_engine::ClientCoreControlData {
        client_id: -1,
        activated: matches!(kind, ParticipantKind::Player),
        observer: matches!(kind, ParticipantKind::Observer),
        name: wire_name.clone(),
        nick: wire_name,
        lobby_ready: false,
    };
    let request = crate::ConnectionRequest {
        core: local_core.clone(),
        build: CURRENT_GAME_BUILD,
        password,
        connection_id: 0,
    };
    let mut transport = crate::ControlTransport::new(stream);
    let bootstrap = match liveness {
        Some(liveness) => {
            crate::connection_handshake::run_client_connection_handshake_with_liveness(
                &mut transport,
                request,
                liveness,
            )
            .await
        }
        None => run_client_connection_handshake(&mut transport, request).await,
    }
    .map_err(|error| ClientError::Handshake(error.to_string()))?;
    let mut join_data = bootstrap.join_data;
    if join_data.client_id < 0 {
        return Err(ClientError::Handshake(
            "host did not assign a client id in JoinData".to_string(),
        ));
    }
    if !matches!(
        join_data.status.state,
        NETWORK_STATE_LOBBY | NETWORK_STATE_PAUSE | NETWORK_STATE_GO
    ) {
        return Err(ClientError::Handshake(format!(
            "host sent invalid JoinData status {}",
            join_data.status.state
        )));
    }
    let existing_clients = JoinClientRegistrySnapshot {
        clients: vec![local_core],
        local_client_id: Some(-1),
    };
    join_data.parameters.clients = reconcile_join_client_registry(
        &existing_clients,
        join_data.parameters.clients.clone(),
        join_data.client_id,
    )
    .ok_or_else(|| {
        ClientError::Handshake("assigned local client is missing from JoinData".to_string())
    })?;
    let start_control_tick = Tick::try_from(join_data.start_control_tick).map_err(|_| {
        ClientError::Handshake(format!(
            "host sent negative JoinData control tick {}",
            join_data.start_control_tick
        ))
    })?;
    let mut resource_state = ClientResourceState::new(
        &join_data,
        bootstrap.peer_core.client_id,
        bootstrap.pending_resources,
        bootstrap.pending_controls,
        bootstrap.liveness,
        resource_directory.clone(),
    )
    .map_err(ClientError::Handshake)?;
    resource_state.initial_ready_checks = bootstrap.pending_ready_checks;
    resource_state.initial_lobby_countdowns = bootstrap.pending_lobby_countdowns;
    send_client_control_request(&mut transport, start_control_tick)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!(
                "failed to initialize control after JoinData: {error}"
            ))
        })?;
    bootstrap_local_candidates.extend_from_roots(&join_data, &local_resource_roots);
    if let Some(system_path) = local_system_path {
        for system in join_data
            .parameters
            .game_resources
            .iter()
            .filter(|core| core.resource_type == crate::HostResourceType::System as u8)
        {
            bootstrap_local_candidates.prioritize(system.id, system_path.clone());
        }
    }
    let standalone_directory = resource_directory
        .as_deref()
        .unwrap_or_else(|| std::path::Path::new("Network"));
    let bootstrap_resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
        &bootstrap_local_candidates,
        standalone_directory.to_path_buf(),
    );
    let mut initialized_game_resources = 0;
    for core in &join_data.parameters.game_resources {
        if resource_state
            .resolve_and_add_bootstrap_resource(
                &bootstrap_resolver,
                crate::ClientBootstrapResourceRole::GameResource,
                core,
            )
            .is_err()
        {
            break;
        }
        initialized_game_resources += 1;
    }
    resource_state
        .resolve_and_add_bootstrap_resource(
            &bootstrap_resolver,
            crate::ClientBootstrapResourceRole::Dynamic,
            &join_data.dynamic,
        )
        .map_err(ClientError::Handshake)?;
    for player in join_data
        .parameters
        .player_infos
        .clients
        .iter_mut()
        .flat_map(|client| &mut client.players)
    {
        let flags = player.flags;
        if flags & lc_engine::PLAYER_INFO_FLAG_REMOVED != 0
            || flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
        {
            continue;
        }
        if flags & lc_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        }
        let Some(core) = player.resource.clone() else {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        };
        match resource_state.resolve_and_add_bootstrap_resource(
            &bootstrap_resolver,
            crate::ClientBootstrapResourceRole::Player,
            &core,
        ) {
            Ok(
                ClientBootstrapRegistration::AlreadyPresent
                | ClientBootstrapRegistration::Registered,
            ) => {}
            Ok(ClientBootstrapRegistration::UnavailableNonLoadable) | Err(_) => {
                crate::client_bootstrap::clear_player_resource(player);
            }
        }
    }
    let client_id = join_data.client_id as ClientId;
    let mut client_addresses = join_data
        .parameters
        .clients
        .clients
        .iter()
        .map(|core| (core.client_id, Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for packet in &bootstrap.pending_addresses {
        if !client_addresses.contains_key(&packet.client_id) {
            continue;
        }
        let packet = host_peer_addr
            .map(|peer| packet.announcement_for_peer(peer))
            .unwrap_or(*packet);
        crate::append_received_address(
            client_addresses.entry(packet.client_id).or_default(),
            packet.address,
        );
    }
    if let Some(host_peer_addr) = host_peer_addr {
        let host_address = crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, host_peer_addr);
        crate::append_received_address(
            client_addresses
                .entry(bootstrap.peer_core.client_id)
                .or_default(),
            host_address,
        );
    }
    let address_announcements = client_addresses
        .iter()
        .flat_map(|(client_id, addresses)| {
            addresses.iter().filter_map(move |address| {
                let address = host_peer_addr
                    .and_then(|peer| address_for_peer(*address, peer))
                    .or((host_peer_addr.is_none()).then_some(*address))?;
                Some(crate::AddressPacket {
                    client_id: *client_id,
                    address,
                })
            })
        })
        .collect();
    send_client_address_announcements(&mut transport, address_announcements)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!(
                "failed to announce addresses after JoinData: {error}"
            ))
        })?;
    resource_state
        .resolve_and_add_bootstrap_resource(
            &bootstrap_resolver,
            crate::ClientBootstrapResourceRole::Scenario,
            &join_data.parameters.scenario,
        )
        .map_err(ClientError::Handshake)?;
    for core in join_data
        .parameters
        .game_resources
        .iter()
        .skip(initialized_game_resources)
    {
        resource_state
            .resolve_and_add_bootstrap_resource(
                &bootstrap_resolver,
                crate::ClientBootstrapResourceRole::GameResource,
                core,
            )
            .map_err(ClientError::Handshake)?;
    }
    resource_state.retain_resource_resolver(bootstrap_resolver);

    let (command_tx, command_rx) = mpsc::channel::<ClientCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_handle = tokio::spawn(run_client_loop_with_addresses(
        transport,
        command_rx,
        event_tx,
        shutdown_rx,
        host_peer_addr,
        client_addresses,
        resource_state,
    ));

    Ok(ClientHandle {
        command_tx,
        event_rx: Some(event_rx),
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        client_id,
        join_data: Some(join_data),
    })
}

#[cfg(test)]
async fn send_client_post_join_packets<S>(
    transport: &mut crate::ControlTransport<S>,
    start_control_tick: Tick,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    send_client_control_request(transport, start_control_tick).await?;
    send_client_address_announcements(transport, address_announcements).await
}

async fn send_client_control_request<S>(
    transport: &mut crate::ControlTransport<S>,
    start_control_tick: Tick,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // C4GameControlNetwork::Init asks connected peers for the first control
    // tick before any JoinData resource initialization
    // (src/C4GameControlNetwork.cpp:46-62; src/C4Network2.cpp:1603-1613).
    transport
        .send_message(ControlMessage::Request {
            from_tick: start_control_tick,
        })
        .await
}

async fn send_client_address_announcements<S>(
    transport: &mut crate::ControlTransport<S>,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // HandleJoinData announces addresses only after early GameRes, Dynamic,
    // and player-resource setup (src/C4Network2.cpp:1612-1622).
    for packet in address_announcements {
        transport
            .send_message(ControlMessage::Address(packet))
            .await?;
    }
    Ok(())
}

/// Events observed by a connected client.
#[derive(Debug)]
pub enum ClientEvent {
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
        controls: Vec<lc_engine::ControlPacket>,
    },
    ExecSync {
        control_tick: Tick,
    },
    ResourceAction(crate::ResourceCatalogAction),
    ResourceComplete {
        resource_id: i32,
        core: lc_engine::NetworkResourceCore,
        path: PathBuf,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
    ResourceDeriveUnsupported {
        core: lc_engine::NetworkResourceCore,
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
        completion: oneshot::Sender<Result<lc_engine::NetworkResourceCore, String>>,
    },
    GracefulPart {
        completion: oneshot::Sender<Result<(), String>>,
    },
    Shutdown,
}

/// Handle for a connected client.
#[derive(Debug)]
pub struct ClientHandle {
    command_tx: mpsc::Sender<ClientCommand>,
    event_rx: Option<mpsc::Receiver<ClientEvent>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    join_handle: tokio::task::JoinHandle<()>,
    client_id: ClientId,
    join_data: Option<JoinDataEnvelope>,
}

impl ClientHandle {
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
    ) -> Result<lc_engine::NetworkResourceCore, ClientError> {
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

    pub async fn graceful_part(self) -> Result<(), ClientError> {
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

    pub async fn shutdown(mut self) -> Result<(), ClientError> {
        let _ = self.command_tx.send(ClientCommand::Shutdown).await;
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        self.join_handle
            .await
            .map_err(|_| ClientError::ClientLoopGone)?;
        Ok(())
    }
}

#[derive(Debug)]
enum HostOutboundMessage {
    Message(ControlMessage),
    Raw(Vec<u8>),
}

#[derive(Clone, Debug)]
struct HostOutboundSender {
    sender: mpsc::Sender<HostOutboundMessage>,
}

impl HostOutboundSender {
    fn channel(capacity: usize) -> (Self, mpsc::Receiver<HostOutboundMessage>) {
        let (sender, receiver) = mpsc::channel(capacity);
        (Self { sender }, receiver)
    }

    async fn send(
        &self,
        message: ControlMessage,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        self.sender.send(HostOutboundMessage::Message(message)).await
    }

    async fn send_raw(
        &self,
        packet: Vec<u8>,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        self.sender.send(HostOutboundMessage::Raw(packet)).await
    }

    fn try_send(
        &self,
        message: ControlMessage,
    ) -> Result<(), mpsc::error::TrySendError<HostOutboundMessage>> {
        self.sender.try_send(HostOutboundMessage::Message(message))
    }

    fn same_channel(&self, other: &Self) -> bool {
        self.sender.same_channel(&other.sender)
    }
}

#[derive(Debug)]
struct ClientConnection {
    outbound: HostOutboundSender,
    core: lc_engine::ClientCoreControlData,
    peer_addr: SocketAddr,
    join_data_sent: bool,
    join_data_needed_emitted: bool,
}

/// One accepted transport route, separate from its logical network client.
/// C++ keeps every route in `C4Network2IO::pConnList` and assigns message/data
/// ownership on `C4Network2Client` (`src/C4Network2IO.h:69-74,228-264`;
/// `src/C4Network2Client.h:82-84,127-133`).
#[derive(Debug)]
struct AcceptedConnectionRoute {
    client_id: ClientId,
    remote_connection_id: u32,
    peer_addr: SocketAddr,
    outbound: HostOutboundSender,
}

#[derive(Debug)]
struct ClientResourceState {
    catalog: crate::ResourceCatalog,
    backend: Option<crate::ResourceTransferBackend>,
    local_resource_sources: BTreeMap<PathBuf, lc_engine::NetworkResourceCore>,
    host_peer_id: i32,
    initial_complete_resources: Vec<(lc_engine::NetworkResourceCore, PathBuf)>,
    initial_packets: Vec<ResourcePacket>,
    initial_controls: Vec<ControlPacket>,
    initial_ready_checks: Vec<ReadyCheckPacket>,
    initial_lobby_countdowns: Vec<LobbyCountdownPacket>,
    liveness: ConnectionLivenessState,
    resource_epoch: Instant,
    resource_directory: Option<PathBuf>,
    resource_resolver: crate::client_bootstrap::ClientBootstrapResolver,
    control: ClientControlState,
}

#[derive(Debug)]
struct ClientControlState {
    mode: i32,
    coordinator: ControlCoordinator,
    pending_unregistered: BTreeMap<ClientId, BTreeMap<Tick, ControlPacket>>,
}

impl ClientControlState {
    #[cfg(test)]
    fn central(start_tick: Tick) -> Self {
        Self {
            mode: 1,
            coordinator: ControlCoordinator::with_start_tick(CLIENT_BACKLOG_LIMIT, start_tick),
            pending_unregistered: BTreeMap::new(),
        }
    }

    fn from_join_data(join_data: &JoinDataEnvelope) -> Result<Self, String> {
        let start_tick = Tick::try_from(join_data.start_control_tick).map_err(|_| {
            format!(
                "host sent negative JoinData control tick {}",
                join_data.start_control_tick
            )
        })?;
        let mut state = Self {
            mode: join_data.status.control_mode,
            coordinator: ControlCoordinator::with_start_tick(CLIENT_BACKLOG_LIMIT, start_tick),
            pending_unregistered: BTreeMap::new(),
        };
        for core in join_data
            .parameters
            .clients
            .clients
            .iter()
            .filter(|core| core.activated)
        {
            let client_id = ClientId::try_from(core.client_id).map_err(|_| {
                format!("active JoinData client has negative ID {}", core.client_id)
            })?;
            state.register(client_id)?;
        }
        Ok(state)
    }

    fn set_mode(&mut self, mode: i32) {
        self.mode = mode;
    }

    fn register(&mut self, client_id: ClientId) -> Result<Vec<ControlPacket>, String> {
        if self.coordinator.client_ids().any(|id| id == client_id) {
            return Ok(Vec::new());
        }
        self.coordinator
            .register_client(client_id)
            .map_err(|error| error.to_string())?;
        let mut ready = Vec::new();
        for packet in self
            .pending_unregistered
            .remove(&client_id)
            .into_iter()
            .flat_map(BTreeMap::into_values)
        {
            ready.extend(
                self.coordinator
                    .ingest(packet)
                    .map_err(|error| error.to_string())?
                    .ready,
            );
        }
        Self::aggregate(ready)
    }

    fn unregister(&mut self, client_id: ClientId) -> Result<Vec<ControlPacket>, String> {
        if !self.coordinator.client_ids().any(|id| id == client_id) {
            return Ok(Vec::new());
        }
        let ready = self
            .coordinator
            .remove_client(client_id)
            .map_err(|error| error.to_string())?;
        Self::aggregate(ready)
    }

    fn apply_membership(
        &mut self,
        control: &lc_engine::ControlPacket,
    ) -> Result<Vec<ControlPacket>, String> {
        match control {
            lc_engine::ControlPacket::ClientJoin(join)
                if join.by_client == HOST_CLIENT_ID as i32 =>
            {
                let Ok(client_id) = ClientId::try_from(join.core.client_id) else {
                    return Ok(Vec::new());
                };
                if join.core.activated {
                    self.register(client_id)
                } else {
                    Ok(Vec::new())
                }
            }
            lc_engine::ControlPacket::ClientUpdate(update)
                if update.by_client == HOST_CLIENT_ID as i32 =>
            {
                let Ok(client_id) = ClientId::try_from(update.client_id) else {
                    return Ok(Vec::new());
                };
                match update.update_type {
                    lc_engine::CLIENT_UPDATE_ACTIVATE if update.data != 0 => {
                        self.register(client_id)
                    }
                    lc_engine::CLIENT_UPDATE_ACTIVATE | lc_engine::CLIENT_UPDATE_SET_OBSERVER => {
                        self.unregister(client_id)
                    }
                    _ => Ok(Vec::new()),
                }
            }
            lc_engine::ControlPacket::ClientRemove(remove)
                if remove.by_client == HOST_CLIENT_ID as i32 =>
            {
                ClientId::try_from(remove.client_id)
                    .map_or_else(|_| Ok(Vec::new()), |client_id| self.unregister(client_id))
            }
            _ => Ok(Vec::new()),
        }
    }

    fn ingest_contribution(&mut self, packet: ControlPacket) -> Result<Vec<ControlPacket>, String> {
        if self.mode != 0 || packet.client_id() == BROADCAST_CLIENT_ID {
            return Ok(Vec::new());
        }
        validate_control_envelope(&packet).map_err(|error| error.to_string())?;
        if !self
            .coordinator
            .client_ids()
            .any(|id| id == packet.client_id())
        {
            self.pending_unregistered
                .entry(packet.client_id())
                .or_default()
                .entry(packet.tick())
                .or_insert(packet);
            return Ok(Vec::new());
        }
        let outcome = self
            .coordinator
            .ingest(packet)
            .map_err(|error| error.to_string())?;
        Self::aggregate(outcome.ready)
    }

    fn accept_network(&mut self, packet: ControlPacket) -> Result<Vec<ControlPacket>, String> {
        if self.mode == 0 {
            return self.ingest_contribution(packet);
        }
        if packet.client_id() != BROADCAST_CLIENT_ID {
            return Ok(Vec::new());
        }
        validate_control_envelope(&packet).map_err(|error| error.to_string())?;
        Ok(vec![packet])
    }

    fn aggregate(ready: Vec<ReadyBatch>) -> Result<Vec<ControlPacket>, String> {
        ready
            .iter()
            .map(|batch| aggregate_ready_batch(batch).map_err(|error| error.to_string()))
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClientBootstrapRegistration {
    AlreadyPresent,
    Registered,
    UnavailableNonLoadable,
}

fn add_resolved_resource(
    catalog: &mut crate::ResourceCatalog,
    backend: Option<&mut crate::ResourceTransferBackend>,
    resource: &crate::ClientBootstrapResourcePlan,
) -> Result<ClientBootstrapRegistration, String> {
    if catalog.contains_resource(resource.core.id) {
        return Ok(ClientBootstrapRegistration::AlreadyPresent);
    }
    let (binary_compatible, loading) = match &resource.source {
        crate::ClientBootstrapResourceSource::Local(local) => {
            if let Some(backend) = backend {
                local
                    .clone()
                    .register(backend)
                    .map_err(|error| error.to_string())?;
            }
            (local.binary_compatible(), false)
        }
        crate::ClientBootstrapResourceSource::Download => {
            if let Some(backend) = backend {
                backend
                    .register_remote_loadable(resource.core.clone())
                    .map_err(|error| error.to_string())?;
            }
            (true, true)
        }
        crate::ClientBootstrapResourceSource::UnavailableNonLoadable(_) => {
            return Ok(ClientBootstrapRegistration::UnavailableNonLoadable);
        }
    };
    if !catalog.register(crate::ResourceRegistration::from_core(
        &resource.core,
        binary_compatible,
        loading,
    )) {
        return Ok(ClientBootstrapRegistration::AlreadyPresent);
    }
    Ok(ClientBootstrapRegistration::Registered)
}

fn local_resource_lookup_path(local: &crate::LocalResourceMatch) -> Option<PathBuf> {
    if local.source_path().is_dir() {
        local
            .standalone_path()
            .map(std::path::Path::to_path_buf)
    } else {
        Some(local.source_path().to_path_buf())
    }
}

fn load_authoritative_player_resources(
    resolver: &crate::client_bootstrap::ClientBootstrapResolver,
    catalog: &mut crate::ResourceCatalog,
    mut backend: Option<&mut crate::ResourceTransferBackend>,
    info: &mut lc_engine::PlayerInfoControlData,
) -> Vec<(PathBuf, lc_engine::NetworkResourceCore)> {
    let mut local_sources = Vec::new();
    for player in &mut info.players {
        let flags = player.flags;
        if flags & lc_engine::PLAYER_INFO_FLAG_REMOVED != 0
            || flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
        {
            continue;
        }
        if flags & lc_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        }
        let Some(core) = player.resource.as_ref() else {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        };
        // AddByCore returns an existing ID before comparing cores or probing
        // local files (src/C4Network2Res.cpp:1473-1477).
        if catalog.contains_resource(core.id) {
            continue;
        }
        let registered = resolver
            .resolve(crate::ClientBootstrapResourceRole::Player, core)
            .ok()
            .and_then(|resource| {
                let registration =
                    add_resolved_resource(catalog, backend.as_deref_mut(), &resource).ok()?;
                if registration == ClientBootstrapRegistration::Registered {
                    if let crate::ClientBootstrapResourceSource::Local(local) = &resource.source {
                        if let Some(path) = local_resource_lookup_path(local) {
                            local_sources.push((path, resource.core.clone()));
                        }
                    }
                }
                Some(registration)
            })
            .is_some_and(|registration| {
                registration != ClientBootstrapRegistration::UnavailableNonLoadable
            });
        if !registered {
            crate::client_bootstrap::clear_player_resource(player);
        }
    }
    local_sources
}

impl ClientResourceState {
    #[cfg(test)]
    fn empty() -> Self {
        let local_candidates = crate::ClientBootstrapLocalCandidates::default();
        Self {
            catalog: crate::ResourceCatalog::new(-1),
            backend: None,
            local_resource_sources: BTreeMap::new(),
            host_peer_id: 0,
            initial_complete_resources: Vec::new(),
            initial_packets: Vec::new(),
            initial_controls: Vec::new(),
            initial_ready_checks: Vec::new(),
            initial_lobby_countdowns: Vec::new(),
            liveness: ConnectionLivenessState::new_accepted_system(),
            resource_epoch: Instant::now(),
            resource_directory: None,
            resource_resolver: crate::client_bootstrap::ClientBootstrapResolver::new(
                &local_candidates,
                PathBuf::from("Network"),
            ),
            control: ClientControlState::central(0),
        }
    }

    fn new(
        join_data: &JoinDataEnvelope,
        host_peer_id: i32,
        initial_packets: Vec<ResourcePacket>,
        initial_controls: Vec<ControlPacket>,
        liveness: ConnectionLivenessState,
        resource_directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        let standalone_directory = resource_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("Network"));
        let local_candidates = crate::ClientBootstrapLocalCandidates::default();
        let backend = resource_directory
            .as_ref()
            .map(|directory| {
                crate::ResourceTransferBackend::new(join_data.client_id, directory)
                    .map_err(|error| error.to_string())
            })
            .transpose()?;
        let control = ClientControlState::from_join_data(join_data)?;
        Ok(Self {
            catalog: crate::ResourceCatalog::new(join_data.client_id),
            backend,
            local_resource_sources: BTreeMap::new(),
            host_peer_id,
            initial_complete_resources: Vec::new(),
            initial_packets,
            initial_controls,
            initial_ready_checks: Vec::new(),
            initial_lobby_countdowns: Vec::new(),
            liveness,
            resource_epoch: Instant::now(),
            resource_directory,
            resource_resolver: crate::client_bootstrap::ClientBootstrapResolver::new(
                &local_candidates,
                standalone_directory,
            ),
            control,
        })
    }

    fn retain_resource_resolver(
        &mut self,
        resolver: crate::client_bootstrap::ClientBootstrapResolver,
    ) {
        self.resource_resolver = resolver;
    }

    fn publish_player_resource(
        &mut self,
        request: crate::ClientPlayerResourceRequest,
    ) -> Result<lc_engine::NetworkResourceCore, String> {
        if let Some(core) = self.local_resource_sources.get(&request.source_path) {
            return Ok(core.clone());
        }
        if self.backend.is_none() {
            return Err("client has no filesystem resource backend".to_string());
        }
        let source_path = request.source_path.clone();
        let source_is_directory = source_path.is_dir();
        let network_directory = self
            .resource_directory
            .clone()
            .ok_or_else(|| "client has no network resource directory".to_string())?;
        let resource_id = self.catalog.allocate_resource_id();
        let publication = crate::publish_client_player_resource(
            crate::ClientPlayerResourcePublicationSpec {
                resource_id,
                source_path: request.source_path,
                wire_name: request.wire_name,
                network_directory,
                group_maker: request.group_maker,
            },
        )
        .map_err(|error| error.to_string())?;
        let crate::ClientPlayerResourcePublication {
            core,
            registration,
            resource_file,
        } = publication;
        let effective_source_path = if source_is_directory {
            resource_file.path.clone()
        } else {
            source_path
        };
        let backend = self
            .backend
            .as_mut()
            .ok_or_else(|| "client filesystem resource backend disappeared".to_string())?;
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
        if !self.catalog.register(registration) {
            return Err(format!(
                "resource ID {resource_id} became occupied during player publication"
            ));
        }
        self.local_resource_sources
            .insert(effective_source_path, core.clone());
        Ok(core)
    }

    fn remove_resource(&mut self, resource_id: i32) -> Result<(), String> {
        let removed_from_catalog = self.catalog.remove_resource(resource_id);
        let removed_from_backend = self
            .backend
            .as_mut()
            .is_some_and(|backend| backend.remove_resource(resource_id));
        (removed_from_catalog || removed_from_backend)
            .then_some(())
            .ok_or_else(|| format!("resource ID {resource_id} is not registered"))
    }

    fn contains_bootstrap_resource(&self, resource_id: i32) -> bool {
        self.catalog.contains_resource(resource_id)
    }

    fn add_bootstrap_resource(
        &mut self,
        resource: &crate::ClientBootstrapResourcePlan,
    ) -> Result<ClientBootstrapRegistration, String> {
        let registration =
            add_resolved_resource(&mut self.catalog, self.backend.as_mut(), resource)?;
        if let (
            ClientBootstrapRegistration::Registered,
            crate::ClientBootstrapResourceSource::Local(local),
        ) = (registration, &resource.source)
        {
            self.initial_complete_resources
                .push((resource.core.clone(), local.path().to_path_buf()));
            if let Some(path) = local_resource_lookup_path(local) {
                self.local_resource_sources
                    .insert(path, resource.core.clone());
            }
        }
        Ok(registration)
    }

    fn resolve_and_add_bootstrap_resource(
        &mut self,
        resolver: &crate::client_bootstrap::ClientBootstrapResolver,
        role: crate::ClientBootstrapResourceRole,
        core: &lc_engine::NetworkResourceCore,
    ) -> Result<ClientBootstrapRegistration, String> {
        // C4Network2ResList::AddByCore returns an existing ID before probing
        // local files or starting a download (src/C4Network2Res.cpp:1473-1477).
        if self.contains_bootstrap_resource(core.id) {
            return Ok(ClientBootstrapRegistration::AlreadyPresent);
        }
        let resource = resolver
            .resolve(role, core)
            .map_err(|error| error.to_string())?;
        self.add_bootstrap_resource(&resource)
    }

    fn load_authoritative_player_resources(
        &mut self,
        info: &mut lc_engine::PlayerInfoControlData,
    ) -> Vec<(PathBuf, lc_engine::NetworkResourceCore)> {
        let local_sources = load_authoritative_player_resources(
            &self.resource_resolver,
            &mut self.catalog,
            self.backend.as_mut(),
            info,
        );
        self.local_resource_sources
            .extend(local_sources.iter().cloned());
        local_sources
    }

    #[cfg(test)]
    fn from_join_data(
        join_data: &JoinDataEnvelope,
        host_peer_id: i32,
        initial_packets: Vec<ResourcePacket>,
        initial_controls: Vec<ControlPacket>,
        liveness: ConnectionLivenessState,
        bootstrap_plan: &crate::ClientBootstrapPlan,
        resource_directory: Option<PathBuf>,
    ) -> Result<Self, String> {
        let mut state = Self::new(
            join_data,
            host_peer_id,
            initial_packets,
            initial_controls,
            liveness,
            resource_directory,
        )?;
        for resource in bootstrap_plan.resources() {
            state.add_bootstrap_resource(resource)?;
        }
        Ok(state)
    }
}

#[derive(Debug)]
struct ClientSetup {
    join_data: JoinDataEnvelope,
    addresses: Vec<crate::AddressPacket>,
}

#[derive(Debug)]
enum HostLoopMessage {
    ClientAccepted {
        connection_id: u32,
        remote_connection_id: u32,
        core: lc_engine::ClientCoreControlData,
        peer_addr: SocketAddr,
        outbound: HostOutboundSender,
        setup_tx: oneshot::Sender<Result<Option<ClientSetup>, String>>,
    },
    ClientMessage {
        connection_id: u32,
        client_id: ClientId,
        message: ControlMessage,
        ping_ms: i32,
    },
    ClientDisconnected {
        connection_id: u32,
        client_id: ClientId,
        next_inbound_packet: u32,
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
    TransportError {
        client_id: Option<ClientId>,
        error: String,
    },
}

#[derive(Debug)]
struct HostState {
    config: HostConfig,
    coordinator: ControlCoordinator,
    backlog: ControlBacklog,
    scheduler: ResyncScheduler,
    clients: BTreeMap<ClientId, ClientConnection>,
    accepted_routes: BTreeMap<u32, AcceptedConnectionRoute>,
    closed_routes: crate::post_mortem::ClosedConnectionRouter,
    pending_sync: Vec<lc_engine::ControlPacket>,
    status_barrier: StatusBarrier,
    control_mode: i32,
    admission: HostAdmission,
    client_cores: BTreeMap<i32, lc_engine::ClientCoreControlData>,
    client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    pending_kinds: BTreeMap<i32, ParticipantKind>,
    join_snapshot: Option<HostJoinSnapshot>,
    resource_catalog: crate::ResourceCatalog,
    resource_backend: Option<crate::ResourceTransferBackend>,
    published_player_sources: BTreeMap<PathBuf, lc_engine::NetworkResourceCore>,
    resource_resolver: crate::client_bootstrap::ClientBootstrapResolver,
    resource_epoch: Instant,
    next_connection_id: u32,
    pending_admissions: BTreeMap<u32, i32>,
    event_tx: mpsc::Sender<HostEvent>,
}

fn publish_host_player_resource(
    request: crate::ClientPlayerResourceRequest,
    state: &mut HostState,
) -> Result<lc_engine::NetworkResourceCore, String> {
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
    let publication = crate::publish_client_player_resource(
        crate::ClientPlayerResourcePublicationSpec {
            resource_id,
            source_path: request.source_path,
            wire_name: request.wire_name,
            network_directory,
            group_maker: request.group_maker,
        },
    )
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

async fn run_host(
    listener: TcpListener,
    config: HostConfig,
    resource_backend: Option<crate::ResourceTransferBackend>,
    mut commands: mpsc::Receiver<HostCommand>,
    event_tx: mpsc::Sender<HostEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let listener_addr = listener.local_addr().ok();
    let backlog_limit = config.backlog_limit;
    let mut coordinator = ControlCoordinator::with_start_tick(backlog_limit, config.start_tick);
    // The host is an active lockstep participant from session start. C++ keeps
    // it in the ordered control-client list used by PackCompleteCtrl, which
    // waits for every client at the requested tick
    // (src/C4GameControlNetwork.cpp:741-769). Registering lazily on first input
    // lets a client-first packet incorrectly complete a tick by itself.
    coordinator
        .register_client(HOST_CLIENT_ID)
        .expect("new host coordinator must accept client ID 0");
    let required_password = (!config.password.is_empty()).then(|| config.password.clone());
    let admission = HostAdmission::new(
        1,
        config.allow_join,
        required_password,
        [config.local_core.name.clone()],
    );
    let client_cores = BTreeMap::from([(0, config.local_core.clone())]);
    let mut host_addresses = Vec::new();
    if let Some(listener_addr) = listener_addr {
        host_addresses.push(crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            SocketAddr::from(([0, 0, 0, 0], listener_addr.port())),
        ));
        if !listener_addr.ip().is_unspecified() {
            crate::append_received_address(
                &mut host_addresses,
                crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, listener_addr),
            );
        }
    }
    let client_addresses = BTreeMap::from([(0, host_addresses)]);
    let mut resource_catalog = crate::ResourceCatalog::new(0);
    config
        .resource_registrations
        .iter()
        .copied()
        .for_each(|registration| {
            resource_catalog.register(registration);
        });
    let mut local_candidates = crate::ClientBootstrapLocalCandidates::default();
    local_candidates.extend_search_roots(&config.local_resource_roots);
    let resource_resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
        &local_candidates,
        config
            .resource_directory
            .clone()
            .unwrap_or_else(|| PathBuf::from("Network")),
    );
    let published_player_sources = config
        .player_resource_sources
        .iter()
        .cloned()
        .collect();
    let mut state = HostState {
        coordinator,
        backlog: ControlBacklog::new(backlog_limit),
        scheduler: ResyncScheduler::new(config.resync_cooldown),
        clients: BTreeMap::new(),
        accepted_routes: BTreeMap::new(),
        closed_routes: crate::post_mortem::ClosedConnectionRouter::default(),
        pending_sync: Vec::new(),
        status_barrier: StatusBarrier::stable(config.initial_status),
        control_mode: config.initial_status.control_mode,
        admission,
        client_cores,
        client_addresses,
        pending_kinds: BTreeMap::new(),
        join_snapshot: config.initial_join_snapshot.clone(),
        resource_catalog,
        resource_backend,
        published_player_sources,
        resource_resolver,
        resource_epoch: Instant::now(),
        next_connection_id: 0,
        pending_admissions: BTreeMap::new(),
        event_tx: event_tx.clone(),
        config,
    };

    let (client_tx, mut client_rx) = mpsc::channel::<HostLoopMessage>(128);
    let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(32);
    let mut resync_timer = interval(state.config.resync_interval);
    let mut resource_timer = interval(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS));

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                break;
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        let connection_id = state.next_connection_id;
                        state.next_connection_id = state.next_connection_id.wrapping_add(1);
                        spawn_host_accept(
                            stream,
                            addr,
                            state.config.local_core.clone(),
                            connection_id,
                            admission_tx.clone(),
                            client_tx.clone(),
                        );
                    }
                    Err(error) => {
                        let _ = state.event_tx
                            .send(HostEvent::TransportError {
                                client_id: None,
                                error: format!("failed to accept connection: {error}"),
                            })
                            .await;
                        break;
                    }
                }
            }
            Some(request) = admission_rx.recv() => {
                handle_host_admission_request(request, &mut state).await;
            }
            Some(message) = client_rx.recv() => {
                match message {
                    HostLoopMessage::ClientAccepted {
                        connection_id,
                        remote_connection_id,
                        core,
                        peer_addr,
                        outbound,
                        setup_tx,
                    } => {
                        handle_client_accepted(
                            connection_id,
                            remote_connection_id,
                            core,
                            peer_addr,
                            outbound,
                            setup_tx,
                            &mut state,
                        )
                        .await;
                    }
                    HostLoopMessage::ClientMessage {
                        connection_id,
                        client_id,
                        message,
                        ping_ms,
                    } => {
                        handle_client_message(
                            connection_id,
                            client_id,
                            message,
                            ping_ms,
                            &mut state,
                        )
                        .await;
                    }
                    HostLoopMessage::ClientDisconnected {
                        connection_id,
                        client_id,
                        next_inbound_packet,
                        post_mortem,
                        reason,
                    } => {
                        handle_client_disconnected(
                            connection_id,
                            client_id,
                            next_inbound_packet,
                            post_mortem,
                            reason,
                            &mut state,
                        )
                        .await;
                    }
                    HostLoopMessage::AdmissionFailed { connection_id, error } => {
                        handle_admission_failed(connection_id, error, &mut state).await;
                    }
                    HostLoopMessage::UnhandledPacket {
                        client_id,
                        packet_type,
                    } => {
                        let _ = state
                            .event_tx
                            .send(HostEvent::UnhandledPacket {
                                client_id,
                                packet_type,
                            })
                            .await;
                    }
                    HostLoopMessage::TransportError { client_id, error } => {
                        let _ = state.event_tx.send(HostEvent::TransportError { client_id, error }).await;
                    }
                }
            }
            Some(command) = commands.recv() => {
                match command {
                    HostCommand::ChangeStatus(status) => {
                        let effects = state.status_barrier.change_status(status);
                        apply_barrier_effects(effects, &mut state).await;
                    }
                    HostCommand::BroadcastStatusAck(status) => {
                        broadcast_status(status, true, &mut state).await;
                    }
                    HostCommand::StatusReached => {
                        let effects = state.status_barrier.local_reached();
                        apply_barrier_effects(effects, &mut state).await;
                    }
                    HostCommand::SubmitLocal(packet) => {
                        ingest_control(packet, ControlIngress::Local, &mut state).await
                    }
                    HostCommand::SubmitLobbyCountdown(packet) => {
                        let _ = state.event_tx.send(HostEvent::LobbyCountdown { packet }).await;
                        broadcast_lobby_countdown(packet, &mut state).await;
                    }
                    HostCommand::SubmitReadyCheck(packet) => {
                        apply_ready_check_to_host_state(packet, &mut state);
                        broadcast_ready_check(packet, None, &mut state).await;
                    }
                    HostCommand::SubmitPacket { delivery, data } => broadcast_packet(delivery, data, None, &mut state).await,
                    HostCommand::ExecSync { control_tick } => broadcast_exec_sync(control_tick, &mut state).await,
                    HostCommand::PublishJoinSnapshot(snapshot) => {
                        state.join_snapshot = Some(*snapshot);
                        publish_pending_join_data(&mut state).await;
                    }
                    HostCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = publish_host_player_resource(request, &mut state);
                        let _ = completion.send(result);
                    }
                    HostCommand::SetJoinAllowed {
                        allowed,
                        completion,
                    } => {
                        state.admission.set_allow_join(allowed);
                        let _ = completion.send(());
                    }
                    #[cfg(test)]
                    HostCommand::InspectAcceptedRoutes { completion } => {
                        let routes = state
                            .accepted_routes
                            .iter()
                            .map(|(connection_id, route)| {
                                (
                                    *connection_id,
                                    route.client_id,
                                    route.remote_connection_id,
                                )
                            })
                            .collect();
                        let _ = completion.send(routes);
                    }
                    HostCommand::Shutdown => break,
                }
            }
            _ = resync_timer.tick() => {
                request_missing_controls(&mut state).await;
            }
            _ = resource_timer.tick() => {
                let now_seconds = state.resource_epoch.elapsed().as_secs();
                if let Some(backend) = state.resource_backend.as_mut() {
                    let mut random = resource_safe_random;
                    match backend.on_timer(now_seconds, &mut random) {
                        Ok(events) => dispatch_host_resource_events(events, &mut state).await,
                        Err(error) => report_host_resource_error(error, &state).await,
                    }
                } else {
                    let actions = state.resource_catalog.on_timer(now_seconds);
                    dispatch_host_resource_actions(actions, &mut state).await;
                }
            }
        }
    }

    for client_id in state.clients.keys() {
        let _ = state
            .event_tx
            .send(HostEvent::ClientLeft {
                client_id: *client_id,
            })
            .await;
    }
}

fn spawn_host_accept(
    stream: TcpStream,
    addr: SocketAddr,
    local_core: lc_engine::ClientCoreControlData,
    connection_id: u32,
    admission_tx: mpsc::Sender<HostAdmissionRequest>,
    host_tx: mpsc::Sender<HostLoopMessage>,
) {
    tokio::spawn(async move {
        if let Err(error) = stream.set_nodelay(true) {
            let _ = host_tx
                .send(HostLoopMessage::TransportError {
                    client_id: None,
                    error: format!("failed to configure connection {addr}: {error}"),
                })
                .await;
            return;
        }

        let request = crate::ConnectionRequest {
            core: local_core,
            build: CURRENT_GAME_BUILD,
            password: lc_engine::LegacyCString::default(),
            connection_id,
        };
        let mut transport = crate::ControlTransport::new(stream);
        let handshake =
            match run_host_connection_handshake(&mut transport, request, &admission_tx).await {
                Ok(handshake) => handshake,
                Err(error) => {
                    let _ = host_tx
                        .send(HostLoopMessage::AdmissionFailed {
                            connection_id,
                            error: format!("connection admission from {addr} failed: {error}"),
                        })
                        .await;
                    return;
                }
            };
        let crate::HostConnectionHandshake {
            local_connection_id,
            remote_connection_id,
            peer_core,
            liveness,
        } = handshake;
        debug_assert_eq!(local_connection_id, connection_id);
        let Ok(client_id) = ClientId::try_from(peer_core.client_id) else {
            let _ = host_tx
                .send(HostLoopMessage::AdmissionFailed {
                    connection_id,
                    error: "accepted peer has a negative client id".to_string(),
                })
                .await;
            return;
        };
        let (outbound, outbound_rx) = HostOutboundSender::channel(64);
        let (setup_tx, setup_rx) = oneshot::channel();
        if host_tx
            .send(HostLoopMessage::ClientAccepted {
                connection_id,
                remote_connection_id,
                core: peer_core,
                peer_addr: addr,
                outbound,
                setup_tx,
            })
            .await
            .is_err()
        {
            return;
        }
        let setup = match setup_rx.await {
            Ok(Ok(setup)) => setup,
            Ok(Err(error)) => {
                let _ = host_tx
                    .send(HostLoopMessage::ClientDisconnected {
                        connection_id,
                        client_id,
                        next_inbound_packet: liveness.connection().inbound_packet_counter(),
                        post_mortem: None,
                        reason: Some(error),
                    })
                    .await;
                return;
            }
            Err(_) => {
                let _ = host_tx
                    .send(HostLoopMessage::ClientDisconnected {
                        connection_id,
                        client_id,
                        next_inbound_packet: liveness.connection().inbound_packet_counter(),
                        post_mortem: None,
                        reason: Some("host setup coordinator stopped".to_string()),
                    })
                    .await;
                return;
            }
        };
        if let Some(setup) = setup {
            if let Err(error) = transport
                .send_message(ControlMessage::JoinData(Box::new(setup.join_data)))
                .await
            {
                let _ = host_tx
                    .send(HostLoopMessage::ClientDisconnected {
                        connection_id,
                        client_id,
                        next_inbound_packet: liveness.connection().inbound_packet_counter(),
                        post_mortem: None,
                        reason: Some(format!("JoinData send failed: {error}")),
                    })
                    .await;
                return;
            }
            for address in setup.addresses {
                if let Err(error) = transport
                    .send_message(ControlMessage::Address(address))
                    .await
                {
                    let _ = host_tx
                        .send(HostLoopMessage::ClientDisconnected {
                            connection_id,
                            client_id,
                            next_inbound_packet: liveness.connection().inbound_packet_counter(),
                            post_mortem: None,
                            reason: Some(format!("address send failed: {error}")),
                        })
                        .await;
                    return;
                }
            }
        }

        ClientTask {
            local_connection_id: connection_id,
            remote_connection_id,
            client_id,
            transport,
            outbound_rx,
            host_tx,
            liveness,
        }
        .run()
        .await;
    });
}

async fn handle_host_admission_request(request: HostAdmissionRequest, state: &mut HostState) {
    let requested_kind = if request.request.core.observer {
        ParticipantKind::Observer
    } else {
        ParticipantKind::Player
    };
    let canonical_peer = state
        .client_cores
        .get(&request.request.core.client_id)
        .filter(|core| {
            core.client_id != HOST_CLIENT_ID as i32
                && core.name == request.request.core.name
                && core.nick == request.request.core.nick
        })
        .cloned();
    let mut decision = canonical_peer.as_ref().map_or_else(
        || state.admission.admit_new_peer(&request.request),
        |core| crate::KnownPeerAdmission::admit(&request.request, core, false),
    );
    if let AdmissionDecision::Accept {
        before_reply,
        peer_core,
        ..
    } = &mut decision
    {
        for action in std::mem::take(before_reply) {
            let ConnectionAction::EmitDirectClientJoin(join) = action else {
                let _ = request.decision_tx.send(AdmissionDecision::Reject {
                    message: lc_engine::LegacyCString::from_bytes(
                        b"invalid host admission action".to_vec(),
                    )
                    .unwrap_or_default(),
                    wrong_password: false,
                });
                return;
            };
            state
                .client_cores
                .insert(join.core.client_id, join.core.clone());
            state
                .pending_kinds
                .insert(join.core.client_id, requested_kind);
            if let Ok(data) =
                crate::encode_control_entry_payload(&lc_engine::ControlPacket::ClientJoin(join))
            {
                for client in state.clients.values() {
                    let _ = client
                        .outbound
                        .send(ControlMessage::Packet {
                            delivery: ControlDelivery::Direct,
                            data: data.clone(),
                        })
                        .await;
                }
                let _ = state
                    .event_tx
                    .send(HostEvent::Direct {
                        client_id: HOST_CLIENT_ID,
                        delivery: ControlDelivery::Direct,
                        data,
                    })
                    .await;
            }
        }
        debug_assert_eq!(
            state.client_cores.get(&peer_core.client_id),
            Some(&*peer_core)
        );
        if canonical_peer.is_none() {
            state
                .pending_admissions
                .insert(request.connection_id, peer_core.client_id);
        }
    }
    let _ = request.decision_tx.send(decision);
}

async fn handle_client_accepted(
    connection_id: u32,
    remote_connection_id: u32,
    core: lc_engine::ClientCoreControlData,
    peer_addr: SocketAddr,
    outbound: HostOutboundSender,
    setup_tx: oneshot::Sender<Result<Option<ClientSetup>, String>>,
    state: &mut HostState,
) {
    state.pending_admissions.remove(&connection_id);
    let Ok(client_id) = ClientId::try_from(core.client_id) else {
        let _ = setup_tx.send(Err("accepted peer has a negative client id".to_string()));
        return;
    };
    let replaced_route = state.accepted_routes.insert(
        connection_id,
        AcceptedConnectionRoute {
            client_id,
            remote_connection_id,
            peer_addr,
            outbound: outbound.clone(),
        },
    );
    debug_assert!(replaced_route.is_none());
    if state.clients.contains_key(&client_id) {
        if setup_tx.send(Ok(None)).is_err() {
            state.accepted_routes.remove(&connection_id);
        }
        return;
    }
    let kind = state
        .pending_kinds
        .remove(&core.client_id)
        .unwrap_or(ParticipantKind::Player);
    state.clients.insert(
        client_id,
        ClientConnection {
            outbound: outbound.clone(),
            core: core.clone(),
            peer_addr,
            join_data_sent: false,
            join_data_needed_emitted: false,
        },
    );
    state.client_addresses.entry(core.client_id).or_default();
    let _ = state
        .event_tx
        .send(HostEvent::ClientJoined {
            client_id,
            name: lc_resources::decode_legacy_script_text(core.name.as_bytes()),
            kind,
        })
        .await;

    let setup_result = match build_client_setup(client_id, state) {
        Ok(Some(setup)) => {
            mark_join_data_sent(client_id, state);
            Ok(Some(setup))
        }
        Ok(None) => {
            emit_join_data_needed(client_id, state).await;
            Ok(None)
        }
        Err(error) => Err(error),
    };
    let setup_error = setup_result.as_ref().err().cloned();
    let setup_delivered = setup_tx.send(setup_result).is_ok();
    if setup_error.is_some() || !setup_delivered {
        handle_client_disconnected(
            connection_id,
            client_id,
            0,
            None,
            setup_error.or_else(|| Some("accepted connection setup was dropped".to_string())),
            state,
        )
        .await;
        return;
    }
    let now_seconds = state.resource_epoch.elapsed().as_secs();
    if let Some(backend) = state.resource_backend.as_mut() {
        let mut random = resource_safe_random;
        match backend.on_peer_connected(core.client_id, now_seconds, &mut random) {
            Ok(events) => dispatch_host_resource_events(events, state).await,
            Err(error) => report_host_resource_error(error, state).await,
        }
    } else {
        let actions = state.resource_catalog.on_peer_connected(core.client_id);
        dispatch_host_resource_actions(actions, state).await;
    }
}

fn build_client_setup(
    client_id: ClientId,
    state: &HostState,
) -> Result<Option<ClientSetup>, String> {
    let Some(client) = state.clients.get(&client_id) else {
        return Err(format!("accepted client {client_id} is missing"));
    };
    if client.join_data_sent {
        return Ok(None);
    }
    let Some(mut snapshot) = state.join_snapshot.clone() else {
        return Ok(None);
    };
    let current_tick = i32::try_from(state.coordinator.current_tick()).unwrap_or(i32::MAX);
    if snapshot.dynamic.resource_type == lc_engine::NETWORK_RESOURCE_TYPE_NULL
        || snapshot.dynamic_tick < current_tick
    {
        return Ok(None);
    }

    snapshot.parameters.clients =
        JoinClientRegistrySnapshot::new(state.client_cores.values().cloned().collect());
    let join_data = JoinDataEnvelope {
        client_id: client.core.client_id,
        start_control_tick: snapshot.dynamic_tick,
        status: state.status_barrier.status,
        dynamic: snapshot.dynamic,
        parameters: snapshot.parameters,
    };
    let addresses = address_packets_for_peer(&state.client_addresses, client.peer_addr);
    Ok(Some(ClientSetup {
        join_data,
        addresses,
    }))
}

fn mark_join_data_sent(client_id: ClientId, state: &mut HostState) {
    if let Some(client) = state.clients.get_mut(&client_id) {
        client.join_data_sent = true;
    }
    state
        .status_barrier
        .set_remote_state(client_id, RemoteBarrierState::Chasing);
}

async fn emit_join_data_needed(client_id: ClientId, state: &mut HostState) {
    let Some(client) = state.clients.get_mut(&client_id) else {
        return;
    };
    if client.join_data_needed_emitted {
        return;
    }
    client.join_data_needed_emitted = true;
    let _ = state
        .event_tx
        .send(HostEvent::JoinDataNeeded {
            client_id,
            current_control_tick: state.coordinator.current_tick(),
        })
        .await;
}

async fn publish_pending_join_data(state: &mut HostState) {
    let pending = state
        .clients
        .iter()
        .filter_map(|(client_id, client)| (!client.join_data_sent).then_some(*client_id))
        .collect::<Vec<_>>();
    for client_id in pending {
        let setup = match build_client_setup(client_id, state) {
            Ok(Some(setup)) => setup,
            Ok(None) => {
                emit_join_data_needed(client_id, state).await;
                continue;
            }
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(client_id),
                        error,
                    })
                    .await;
                continue;
            }
        };
        let Some(outbound) = state
            .clients
            .get(&client_id)
            .map(|client| client.outbound.clone())
        else {
            continue;
        };
        if outbound
            .send(ControlMessage::JoinData(Box::new(setup.join_data)))
            .await
            .is_err()
        {
            continue;
        }
        let mut failed = false;
        for address in setup.addresses {
            if outbound
                .send(ControlMessage::Address(address))
                .await
                .is_err()
            {
                failed = true;
                break;
            }
        }
        if failed {
            continue;
        }
        mark_join_data_sent(client_id, state);
    }
}

fn address_packets_for_peer(
    client_addresses: &BTreeMap<i32, Vec<crate::NetworkAddress>>,
    peer_addr: SocketAddr,
) -> Vec<crate::AddressPacket> {
    client_addresses
        .iter()
        .flat_map(|(client_id, addresses)| {
            addresses.iter().filter_map(move |address| {
                address_for_peer(*address, peer_addr).map(|address| crate::AddressPacket {
                    client_id: *client_id,
                    address,
                })
            })
        })
        .collect()
}

fn address_for_peer(
    mut address: crate::NetworkAddress,
    peer_addr: SocketAddr,
) -> Option<crate::NetworkAddress> {
    let SocketAddr::V6(mut endpoint) = address.endpoint else {
        return Some(address);
    };
    if endpoint.scope_id() == 0 {
        return Some(address);
    }
    let SocketAddr::V6(peer) = peer_addr else {
        return None;
    };
    if peer.scope_id() != endpoint.scope_id() {
        return None;
    }
    endpoint.set_scope_id(0);
    address.endpoint = SocketAddr::V6(endpoint);
    Some(address)
}

impl HostState {
    fn coordination_register(&mut self, client_id: ClientId) -> Result<(), crate::ControlError> {
        if !self.coordinator.client_ids().any(|id| id == client_id) {
            self.coordinator.register_client(client_id)?;
        }
        Ok(())
    }
}

async fn handle_client_message(
    connection_id: u32,
    client_id: ClientId,
    message: ControlMessage,
    ping_ms: i32,
    state: &mut HostState,
) {
    match message {
        ControlMessage::Ping(packet) => {
            if let Some(client) = state.clients.get(&client_id) {
                let _ = client.outbound.send(ControlMessage::Pong(packet)).await;
            }
        }
        ControlMessage::Pong(_) => {}
        ControlMessage::ConnectionRequest(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "accepted client sent a duplicate connection request".to_string(),
                })
                .await;
        }
        ControlMessage::ConnectionReply(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "accepted client sent a duplicate connection reply".to_string(),
                })
                .await;
        }
        ControlMessage::ForwardRequest(packet) => {
            handle_forward_request(connection_id, client_id, packet, ping_ms, state).await;
        }
        ControlMessage::Forward(packet) => {
            handle_forwarded_packet_for_host(connection_id, client_id, packet, ping_ms, state)
                .await;
        }
        ControlMessage::PostMortem(packet) => {
            handle_post_mortem_recovery(packet, ping_ms, state).await;
        }
        // PID_JoinData is host-to-client only; C++ silently ignores it on a
        // host (src/C4Network2.cpp:938-946).
        ControlMessage::JoinData(_) => {}
        ControlMessage::Address(packet) => {
            handle_received_host_address(client_id, packet, state).await;
        }
        ControlMessage::Resource(packet) => {
            let now_seconds = state.resource_epoch.elapsed().as_secs();
            if let Some(backend) = state.resource_backend.as_mut() {
                let mut random = resource_safe_random;
                match backend.on_packet(client_id as i32, &packet, now_seconds, &mut random) {
                    Ok(events) => dispatch_host_resource_events(events, state).await,
                    Err(error) => report_host_resource_error(error, state).await,
                }
            } else {
                let actions = state.resource_catalog.on_packet(client_id as i32, &packet);
                dispatch_host_resource_actions(actions, state).await;
            }
        }
        ControlMessage::Status(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "client attempted to originate host Status".to_string(),
                })
                .await;
        }
        ControlMessage::StatusAck(status) => {
            let _ = state
                .event_tx
                .send(HostEvent::StatusAck { client_id, status })
                .await;
            let effects = state.status_barrier.remote_ack(client_id, status);
            apply_barrier_effects(effects, state).await;
        }
        ControlMessage::LobbyCountdown(packet) => {
            let _ = state
                .event_tx
                .send(HostEvent::LobbyCountdown { packet })
                .await;
        }
        // A Request is host-authored. C++ rejects every network-origin
        // Request while running as the host, regardless of packet.Client
        // (src/C4Network2.cpp:1642-1654).
        ControlMessage::ReadyCheck(packet) if packet.data.vote_requested() => {}
        ControlMessage::ReadyCheck(packet) => {
            apply_ready_check_to_host_state(packet, state);
            let _ = state.event_tx.send(HostEvent::ReadyCheck { packet }).await;
        }
        ControlMessage::ActivationRequest { tick } => {
            let waited_for = matches!(
                state.status_barrier.remotes.get(&client_id),
                Some(RemoteBarrierState::NotReady | RemoteBarrierState::Ready)
            );
            let _ = state
                .event_tx
                .send(HostEvent::ActivationRequest {
                    client_id,
                    tick,
                    waited_for,
                    ping_ms,
                })
                .await;
        }
        ControlMessage::PlayerInfoUpdate(request) => {
            let _ = state
                .event_tx
                .send(HostEvent::PlayerInfoUpdate { client_id, request })
                .await;
        }
        ControlMessage::Control(packet) => {
            // C4GameControlNetwork::HandleControl receives the source client
            // ID but deliberately does not authenticate the packet envelope
            // against it. Only PID_ControlPkt checks its embedded ByClient.
            ingest_control(packet, ControlIngress::Network, state).await;
        }
        ControlMessage::Request { from_tick } => {
            fulfill_resync_request(client_id, from_tick, state).await;
        }
        ControlMessage::Packet { delivery, data } => {
            broadcast_packet(delivery, data, Some(client_id), state).await;
        }
        ControlMessage::ExecSync { control_tick } => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: format!(
                        "client attempted to release synchronized controls at tick {control_tick}"
                    ),
                })
                .await;
        }
    }
}

async fn handle_post_mortem_recovery(
    packet: crate::PostMortemPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    let Some(replay) = state.closed_routes.recover(&packet) else {
        return;
    };
    for nested_packet in replay.packets {
        match crate::transport::parse_complete_packet(&nested_packet) {
            Ok(Some(message)) => {
                Box::pin(handle_client_message(
                    replay.connection_id,
                    replay.client_id,
                    message,
                    ping_ms,
                    state,
                ))
                .await;
            }
            Ok(None) => {
                report_unhandled_forwarded_packet(replay.client_id, &nested_packet, state).await;
            }
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(replay.client_id),
                        error: format!(
                            "invalid post-mortem packet for closed connection {}: {error}",
                            replay.connection_id
                        ),
                    })
                    .await;
            }
        }
    }
}

fn forward_selects(packet: &crate::ForwardPacket, client_id: i32) -> bool {
    let listed = packet.clients.contains(&client_id);
    if listed {
        !packet.negative_list
    } else {
        packet.negative_list
    }
}

async fn report_forward_error(source: ClientId, error: String, state: &HostState) {
    let _ = state
        .event_tx
        .send(HostEvent::TransportError {
            client_id: Some(source),
            error,
        })
        .await;
}

async fn report_unhandled_forwarded_packet(
    source: ClientId,
    nested_packet: &[u8],
    state: &HostState,
) {
    let Some(&packet_type) = nested_packet.first() else {
        return;
    };
    let _ = state
        .event_tx
        .send(HostEvent::UnhandledPacket {
            client_id: Some(source),
            packet_type,
        })
        .await;
}

async fn handle_forward_request(
    connection_id: u32,
    source: ClientId,
    packet: crate::ForwardPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    // C4Network2IO keeps connection-list order, excludes the requester's
    // client ID, and deduplicates targets into a positive list. Rust assigns
    // monotonically increasing IDs, so reverse ID order mirrors the current
    // head-inserted C++ connection list (src/C4Network2IO.cpp:1066-1082).
    let target_ids = state
        .clients
        .keys()
        .rev()
        .copied()
        .filter(|client_id| *client_id != source)
        .filter(|client_id| {
            i32::try_from(*client_id).is_ok_and(|client_id| forward_selects(&packet, client_id))
        })
        .collect::<Vec<_>>();
    let targets = target_ids
        .iter()
        .filter_map(|client_id| {
            state
                .clients
                .get(client_id)
                .map(|client| client.outbound.clone())
        })
        .collect::<Vec<_>>();
    if target_ids.len() <= 2 {
        for outbound in targets {
            let _ = outbound.send_raw(packet.nested_packet.clone()).await;
        }
    } else {
        let forwarded = ControlMessage::Forward(crate::ForwardPacket {
            negative_list: false,
            clients: target_ids
                .iter()
                .filter_map(|client_id| i32::try_from(*client_id).ok())
                .collect(),
            nested_packet: packet.nested_packet.clone(),
        });
        for outbound in targets {
            let _ = outbound.send(forwarded.clone()).await;
        }
    }
    if forward_selects(&packet, HOST_CLIENT_ID as i32) {
        dispatch_forwarded_packet_for_host(
            connection_id,
            source,
            &packet.nested_packet,
            ping_ms,
            state,
        )
        .await;
    }
}

async fn handle_forwarded_packet_for_host(
    connection_id: u32,
    source: ClientId,
    packet: crate::ForwardPacket,
    ping_ms: i32,
    state: &mut HostState,
) {
    if !forward_selects(&packet, HOST_CLIENT_ID as i32) {
        return;
    }
    dispatch_forwarded_packet_for_host(
        connection_id,
        source,
        &packet.nested_packet,
        ping_ms,
        state,
    )
    .await;
}

async fn dispatch_forwarded_packet_for_host(
    connection_id: u32,
    source: ClientId,
    nested_packet: &[u8],
    ping_ms: i32,
    state: &mut HostState,
) {
    let message = match crate::transport::parse_complete_packet(nested_packet) {
        Ok(Some(message)) => message,
        Ok(None) => {
            report_unhandled_forwarded_packet(source, nested_packet, state).await;
            return;
        }
        Err(error) => {
            report_forward_error(source, format!("invalid forwarded packet: {error}"), state).await;
            return;
        }
    };
    match message {
        ControlMessage::Packet { delivery, data }
            if matches!(delivery, ControlDelivery::Direct | ControlDelivery::Private) =>
        {
            dispatch_packet(delivery, data, Some(source), false, state).await;
        }
        message => {
            Box::pin(handle_client_message(
                connection_id,
                source,
                message,
                ping_ms,
                state,
            ))
            .await;
        }
    }
}

async fn dispatch_host_resource_actions(
    actions: Vec<crate::ResourceCatalogAction>,
    state: &mut HostState,
) {
    for action in actions {
        match action {
            crate::ResourceCatalogAction::SendToPeer { peer_id, packet } => {
                let Ok(client_id) = ClientId::try_from(peer_id) else {
                    continue;
                };
                if let Some(client) = state.clients.get(&client_id) {
                    let _ = client.outbound.send(ControlMessage::Resource(packet)).await;
                }
            }
            crate::ResourceCatalogAction::Broadcast { packet } => {
                for client in state.clients.values() {
                    let _ = client
                        .outbound
                        .send(ControlMessage::Resource(packet.clone()))
                        .await;
                }
            }
            external => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceAction(external))
                    .await;
            }
        }
    }
}

async fn dispatch_host_resource_events(
    events: Vec<crate::ResourceTransferEvent>,
    state: &mut HostState,
) {
    for event in events {
        match event {
            crate::ResourceTransferEvent::Transport(action) => {
                dispatch_host_resource_actions(vec![action], state).await;
            }
            crate::ResourceTransferEvent::Completed {
                resource_id,
                core,
                path,
            } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceComplete {
                        resource_id,
                        core,
                        path,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::LoadFailed { resource_id } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceLoadFailed { resource_id })
                    .await;
            }
            crate::ResourceTransferEvent::FinishDerivedUnsupported { core } => {
                let _ = state
                    .event_tx
                    .send(HostEvent::ResourceDeriveUnsupported { core })
                    .await;
            }
        }
    }
}

async fn report_host_resource_error(error: crate::ResourceTransferError, state: &HostState) {
    let _ = state
        .event_tx
        .send(HostEvent::TransportError {
            client_id: None,
            error: format!("resource transfer failed: {error}"),
        })
        .await;
}

async fn handle_received_host_address(
    source_client_id: ClientId,
    packet: crate::AddressPacket,
    state: &mut HostState,
) {
    if !state.client_cores.contains_key(&packet.client_id) {
        return;
    }
    let Some(peer_addr) = state
        .clients
        .get(&source_client_id)
        .map(|client| client.peer_addr)
    else {
        return;
    };
    let packet = packet.announcement_for_peer(peer_addr);
    let insertion = crate::append_received_address(
        state.client_addresses.entry(packet.client_id).or_default(),
        packet.address,
    );
    if !matches!(insertion, crate::AddressInsertion::Added { .. }) {
        return;
    }

    // AddAddr(..., true) re-announces a newly learned address to every
    // connected client, including the source connection. The source then
    // suppresses the duplicate on receipt (src/C4Network2Client.cpp:259-278,
    // 581-597).
    for client in state.clients.values() {
        let _ = client.outbound.send(ControlMessage::Address(packet)).await;
    }
}

async fn handle_client_disconnected(
    connection_id: u32,
    client_id: ClientId,
    next_inbound_packet: u32,
    post_mortem: Option<crate::PostMortemPacket>,
    reason: Option<String>,
    state: &mut HostState,
) {
    let disconnected_route = state.accepted_routes.remove(&connection_id);
    if let Some(route) = &disconnected_route {
        state
            .closed_routes
            .retain(connection_id, route.client_id, next_inbound_packet);
    }
    let is_secondary_route = disconnected_route.as_ref().is_some_and(|route| {
        state
            .clients
            .get(&client_id)
            .is_some_and(|client| !route.outbound.same_channel(&client.outbound))
    });
    let promoted_route = disconnected_route
        .as_ref()
        .filter(|route| {
            state
                .clients
                .get(&client_id)
                .is_some_and(|client| route.outbound.same_channel(&client.outbound))
        })
        .and_then(|_| {
            state
                .accepted_routes
                .values()
                .find(|route| route.client_id == client_id)
        })
        .map(|route| (route.outbound.clone(), route.peer_addr));
    if let Some(AcceptedConnectionRoute {
        client_id: route_client_id,
        remote_connection_id: _remote_connection_id,
        peer_addr: _peer_addr,
        outbound: _outbound,
    }) = disconnected_route
    {
        debug_assert_eq!(route_client_id, client_id);
    }
    if is_secondary_route {
        if let (Some(post_mortem), Some(outbound)) = (
            post_mortem,
            state
                .clients
                .get(&client_id)
                .map(|client| client.outbound.clone()),
        ) {
            let _ = outbound.send(ControlMessage::PostMortem(post_mortem)).await;
        }
        if let Some(reason) = reason {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: reason,
                })
                .await;
        }
        return;
    }
    if let Some((outbound, peer_addr)) = promoted_route {
        if let Some(client) = state.clients.get_mut(&client_id) {
            client.outbound = outbound.clone();
            client.peer_addr = peer_addr;
        }
        if let Some(post_mortem) = post_mortem {
            let _ = outbound.send(ControlMessage::PostMortem(post_mortem)).await;
        }
        if let Some(reason) = reason {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: reason,
                })
                .await;
        }
        return;
    }
    let disconnected = state.clients.remove(&client_id);
    if let Some(client) = &disconnected {
        state.pending_kinds.remove(&client.core.client_id);
    }
    let barrier_effects = state.status_barrier.remove_remote(client_id);

    let _ = state
        .event_tx
        .send(HostEvent::ClientLeft { client_id })
        .await;

    if let Some(client) = disconnected {
        // Socket loss stops waiting for this peer's status acknowledgement,
        // but the running control client remains active until the synchronized
        // ClientRemove executes. C4ClientList::CtrlRemove only flags the net
        // client before queuing CDT_Sync; C4GameControlNetwork refreshes its
        // active-client copy at that synchronization boundary
        // (src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220,260-297,329-345).
        queue_disconnected_client_remove(&client.core, state).await;
    }
    apply_barrier_effects(barrier_effects, state).await;

    if let Some(reason) = reason {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: Some(client_id),
                error: reason,
            })
            .await;
    }
}

async fn handle_admission_failed(connection_id: u32, error: String, state: &mut HostState) {
    let provisional_client_id = state.pending_admissions.remove(&connection_id);
    if let Some(core) =
        provisional_client_id.and_then(|client_id| state.client_cores.get(&client_id).cloned())
    {
        queue_disconnected_client_remove(&core, state).await;
    }
    let _ = state
        .event_tx
        .send(HostEvent::TransportError {
            client_id: provisional_client_id.and_then(|id| ClientId::try_from(id).ok()),
            error,
        })
        .await;
}

async fn queue_disconnected_client_remove(
    core: &lc_engine::ClientCoreControlData,
    state: &mut HostState,
) {
    let Ok(data) = crate::encode_control_entry_payload(&lc_engine::ControlPacket::ClientRemove(
        lc_engine::ClientRemoveControlData {
            client_id: core.client_id,
            reason: lc_engine::LegacyCString::from_bytes(b"disconnected".to_vec())
                .unwrap_or_default(),
            by_client: 0,
        },
    )) else {
        return;
    };
    broadcast_packet(ControlDelivery::Sync, data, None, state).await;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlIngress {
    Local,
    Network,
}

async fn ingest_control(
    packet: ControlPacket,
    ingress: ControlIngress,
    state: &mut HostState,
) {
    let client_id = packet.client_id();
    // Validate everything PackCompleteCtrl needs before the coordinator
    // consumes contributions and advances its tick. A malformed frame must
    // not create a permanent hole in the lockstep stream.
    if let Err(error) = validate_control_envelope(&packet) {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: Some(client_id),
                error: format!("invalid control packet: {error}"),
            })
            .await;
        return;
    }
    if let Err(error) = validate_queued_control_authors(&packet) {
        let _ = state
            .event_tx
            .send(HostEvent::TransportError {
                client_id: Some(client_id),
                error,
            })
            .await;
        return;
    }
    // A host's own DoInput broadcasts before AddCtrl in CNM_Decentral. A raw
    // network PID_Control goes straight to HandleControl and is only stored;
    // client fallback fanout belongs to PID_FwdReq instead (pristine C++
    // src/C4GameControlNetwork.cpp:156-179,517-529).
    if ingress == ControlIngress::Local && state.control_mode == 0 {
        broadcast_control(&packet, state).await;
    }
    match state.coordinator.ingest(packet) {
        Ok(ControlOutcome { ready, missing, .. }) => {
            for batch in ready {
                publish_ready_batch(batch, state).await;
            }
            if !missing.is_empty() {
                schedule_missing(missing, state);
            }
        }
        Err(error) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: error.to_string(),
                })
                .await;
        }
    }
}

/// Authenticate security-sensitive inner authors in a queued contribution.
///
/// Complete packets deliberately remain opaque when they contain an
/// unsupported control type, because the host must still aggregate legacy
/// controls it does not execute. Such a packet cannot become a typed
/// `ReadyTick` in the current app decoder. Every fully decodable frame that
/// can reach that path is checked here before the coordinator consumes it.
fn validate_queued_control_authors(packet: &ControlPacket) -> Result<(), String> {
    let frame = match crate::decode_control_packet(packet) {
        Ok(frame) => frame,
        Err(crate::LegacyControlError::UnsupportedPacket(_)) => return Ok(()),
        Err(error) => return Err(format!("invalid control packet: {error}")),
    };
    let expected_author = i32::try_from(frame.client_id).map_err(|_| {
        format!(
            "queued control packet has unsupported author id {}",
            frame.client_id
        )
    })?;
    for control in &frame.controls {
        let (name, author) = match control {
            lc_engine::ControlPacket::Script(script) => ("CID_Script", script.by_client),
            lc_engine::ControlPacket::MessageBoardAnswer(answer) => {
                ("CID_MessageBoardAnswer", answer.by_client)
            }
            lc_engine::ControlPacket::Message(message) => ("CID_Message", message.by_client),
            lc_engine::ControlPacket::CustomCommand(command) => {
                ("CID_CustomCommand", command.by_client)
            }
            lc_engine::ControlPacket::EmMoveObject(control) => {
                ("CID_EMMoveObj", control.by_client)
            }
            lc_engine::ControlPacket::EmDrawTool(control) => {
                ("CID_EMDrawTool", control.by_client)
            }
            lc_engine::ControlPacket::EmDropDef(control) => {
                ("CID_EMDropDef", control.by_client)
            }
            lc_engine::ControlPacket::ActivateGameGoalMenu(control) => {
                ("CID_ActivateGameGoalMenu", control.by_client)
            }
            lc_engine::ControlPacket::ToggleHostility(control) => {
                ("CID_ToggleHostility", control.by_client)
            }
            lc_engine::ControlPacket::ActivateGameGoalRule(control) => {
                ("CID_ActivateGameGoalRule", control.by_client)
            }
            lc_engine::ControlPacket::SetPlayerTeam(control) => {
                ("CID_SetPlayerTeam", control.by_client)
            }
            lc_engine::ControlPacket::EliminatePlayer(control) => {
                ("CID_EliminatePlayer", control.by_client)
            }
            lc_engine::ControlPacket::RemovePlayer(remove) => {
                ("CID_RemovePlr", remove.by_client)
            }
            control => {
                let Some(set) = crate::LegacyControlSet::from_control_packet(control) else {
                    continue;
                };
                ("CID_Set", set.by_client)
            }
        };
        if author != expected_author {
            return Err(format!(
                "queued {name} claimed author {author}, but authenticated author is {expected_author}"
            ));
        }
    }
    Ok(())
}

fn schedule_missing(missing: Vec<MissingRange>, state: &mut HostState) {
    let requests = state.scheduler.schedule(missing.iter(), Instant::now());
    for request in requests {
        if let Some(client) = state.clients.get(&request.client_id) {
            if !client.join_data_sent {
                continue;
            }
            let _ = client.outbound.try_send(ControlMessage::Request {
                from_tick: request.from_tick,
            });
        }
    }
}

async fn request_missing_controls(state: &mut HostState) {
    let missing = state.coordinator.missing_ranges();
    if missing.is_empty() {
        return;
    }
    schedule_missing(missing, state);
}

async fn fulfill_resync_request(client_id: ClientId, from_tick: Tick, state: &mut HostState) {
    let resend = state.backlog.fulfill_request(from_tick);
    let Some(outbound) = state
        .clients
        .get(&client_id)
        .map(|client| client.outbound.clone())
    else {
        return;
    };

    let mut packets_by_tick = BTreeMap::<Tick, Vec<ControlPacket>>::new();
    for packet in resend {
        packets_by_tick
            .entry(packet.tick())
            .or_default()
            .push(packet);
    }
    for (tick, packets) in packets_by_tick {
        // A client requesting old host control needs the same C4ClientIDAll
        // packet used during live play. C++ HandleControlReq sends the stored
        // complete control packet for every contiguous tick
        // (src/C4GameControlNetwork.cpp:531-544).
        let aggregated = match aggregate_control_packets_for_tick(tick, &packets) {
            Ok(packet) => packet,
            Err(error) => {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(client_id),
                        error: format!("failed to aggregate resync tick {tick}: {error}"),
                    })
                    .await;
                return;
            }
        };
        if outbound
            .send(ControlMessage::Control(aggregated))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn publish_ready_batch(batch: ReadyBatch, state: &mut HostState) {
    let aggregated = match aggregate_ready_batch(&batch) {
        Ok(packet) => packet,
        Err(error) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: None,
                    error: format!("failed to aggregate ready tick {}: {error}", batch.tick()),
                })
                .await;
            return;
        }
    };

    state.backlog.record_ready_batch(&batch);
    // Only central/async hosts transmit C4ClientIDAll. Decentralized peers
    // already received each contribution and pack this packet themselves
    // (src/C4GameControlNetwork.cpp:763-777).
    if state.control_mode != 0 {
        broadcast_control(&aggregated, state).await;
    }
    let _ = state
        .event_tx
        .send(HostEvent::Ready { packet: aggregated })
        .await;
}

async fn broadcast_control(packet: &ControlPacket, state: &mut HostState) {
    for client in state.clients.values() {
        let _ = client
            .outbound
            .send(ControlMessage::Control(packet.clone()))
            .await;
    }
}

async fn broadcast_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    origin: Option<ClientId>,
    state: &mut HostState,
) {
    dispatch_packet(delivery, data, origin, true, state).await;
}

async fn dispatch_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    origin: Option<ClientId>,
    relay_to_clients: bool,
    state: &mut HostState,
) {
    match delivery {
        ControlDelivery::Sync => {
            let expected_author = origin
                .and_then(|client_id| i32::try_from(client_id).ok())
                .unwrap_or(0);
            let control = match authenticated_single_control(&data, expected_author) {
                Ok(control) => control,
                Err(error) => {
                    let _ = state
                        .event_tx
                        .send(HostEvent::TransportError {
                            client_id: origin,
                            error,
                        })
                        .await;
                    return;
                }
            };
            // The client that originated a Sync packet deleted its local copy
            // and waits for the host echo, so include every client here
            // (src/C4GameControlNetwork.cpp:181-220,568-572).
            if relay_to_clients {
                for client in state.clients.values() {
                    let _ = client
                        .outbound
                        .send(ControlMessage::Packet {
                            delivery,
                            data: data.clone(),
                        })
                        .await;
                }
            }
            state.pending_sync.push(control);
            if state.status_barrier.is_frozen() {
                execute_frozen_sync(state.coordinator.current_tick(), state).await;
            } else if let Ok(next_control_tick) = i32::try_from(state.coordinator.current_tick()) {
                let effects = state.status_barrier.sync(next_control_tick);
                apply_barrier_effects(effects, state).await;
            }
        }
        ControlDelivery::Queue | ControlDelivery::Decide => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: origin,
                    error: format!("single control packet cannot use {delivery:?} delivery"),
                })
                .await;
        }
        ControlDelivery::Direct | ControlDelivery::Private => {
            let expected_author = origin
                .and_then(|client_id| i32::try_from(client_id).ok())
                .unwrap_or(0);
            let mut control = match authenticated_single_control(&data, expected_author) {
                Ok(control) => control,
                Err(error) => {
                    let _ = state
                        .event_tx
                        .send(HostEvent::TransportError {
                            client_id: origin,
                            error,
                        })
                        .await;
                    return;
                }
            };
            let mut local_data = data.clone();
            if let lc_engine::ControlPacket::PlayerInfo(info) = &mut control {
                let local_sources = load_authoritative_player_resources(
                    &state.resource_resolver,
                    &mut state.resource_catalog,
                    state.resource_backend.as_mut(),
                    info,
                );
                for (path, core) in &local_sources {
                    let _ = state
                        .event_tx
                        .send(HostEvent::ResourceComplete {
                            resource_id: core.id,
                            core: core.clone(),
                            path: path.clone(),
                        })
                        .await;
                }
                state.published_player_sources.extend(local_sources);
                if let Ok(normalized) = crate::encode_control_entry_payload(&control) {
                    local_data = normalized;
                }
            }
            if relay_to_clients {
                for (client_id, client) in state.clients.iter() {
                    if Some(*client_id) == origin {
                        continue;
                    }
                    let _ = client
                        .outbound
                        .send(ControlMessage::Packet {
                            delivery,
                            data: data.clone(),
                        })
                        .await;
                }
            }
            let _ = state
                .event_tx
                .send(HostEvent::Direct {
                    client_id: origin.unwrap_or(BROADCAST_CLIENT_ID),
                    delivery,
                    data: local_data,
                })
                .await;
        }
    }
}

fn authenticated_single_control(
    data: &[u8],
    expected_author: i32,
) -> Result<lc_engine::ControlPacket, String> {
    let control = decode_control_entry_payload(data)
        .map_err(|error| format!("invalid single control packet: {error}"))?;
    let author = match &control {
        lc_engine::ControlPacket::ClientJoin(data) => data.by_client,
        lc_engine::ControlPacket::ClientUpdate(data) => data.by_client,
        lc_engine::ControlPacket::ClientRemove(data) => data.by_client,
        lc_engine::ControlPacket::PlayerSelect(data) => data.by_client,
        lc_engine::ControlPacket::PlayerControl(data) => data.by_client,
        lc_engine::ControlPacket::PlayerCommand(data) => data.by_client,
        lc_engine::ControlPacket::Script(data) => data.by_client,
        lc_engine::ControlPacket::MessageBoardAnswer(data) => data.by_client,
        lc_engine::ControlPacket::Message(data) => data.by_client,
        lc_engine::ControlPacket::CustomCommand(data) => data.by_client,
        lc_engine::ControlPacket::EmMoveObject(data) => data.by_client,
        lc_engine::ControlPacket::EmDrawTool(data) => data.by_client,
        lc_engine::ControlPacket::EmDropDef(data) => data.by_client,
        lc_engine::ControlPacket::ActivateGameGoalMenu(data) => data.by_client,
        lc_engine::ControlPacket::ToggleHostility(data) => data.by_client,
        lc_engine::ControlPacket::ActivateGameGoalRule(data) => data.by_client,
        lc_engine::ControlPacket::SetPlayerTeam(data) => data.by_client,
        lc_engine::ControlPacket::EliminatePlayer(data) => data.by_client,
        lc_engine::ControlPacket::InitScenarioPlayer(data) => data.by_client,
        lc_engine::ControlPacket::SurrenderPlayer(data) => data.by_client,
        lc_engine::ControlPacket::Synchronize(data) => data.by_client,
        lc_engine::ControlPacket::SyncCheck(data) => data.by_client,
        lc_engine::ControlPacket::JoinPlayer(data) => data.by_client,
        lc_engine::ControlPacket::RemovePlayer(data) => data.by_client,
        lc_engine::ControlPacket::PlayerInfo(data) => data.by_client,
        lc_engine::ControlPacket::Vote(data) | lc_engine::ControlPacket::VoteEnd(data) => {
            data.by_client
        }
        lc_engine::ControlPacket::Unknown { .. } => {
            crate::LegacyControlSet::from_control_packet(&control)
                .map(|set| set.by_client)
                .ok_or_else(|| "unsupported single control packet".to_string())?
        }
    };
    if author != expected_author {
        return Err(format!(
            "single control claimed author {author}, but authenticated author is {expected_author}"
        ));
    }
    Ok(control)
}

async fn broadcast_exec_sync(control_tick: Tick, state: &mut HostState) {
    if state.pending_sync.is_empty() {
        return;
    }
    for client in state.clients.values() {
        let _ = client
            .outbound
            .send(ControlMessage::ExecSync { control_tick })
            .await;
    }
    let controls = std::mem::take(&mut state.pending_sync);
    apply_host_membership_controls(&controls, state).await;
    let _ = state
        .event_tx
        .send(HostEvent::SyncScheduled {
            control_tick,
            controls,
        })
        .await;
}

async fn execute_frozen_sync(control_tick: Tick, state: &mut HostState) {
    if state.pending_sync.is_empty() {
        return;
    }
    let controls = std::mem::take(&mut state.pending_sync);
    apply_host_membership_controls(&controls, state).await;
    let _ = state
        .event_tx
        .send(HostEvent::SyncScheduled {
            control_tick,
            controls,
        })
        .await;
    for client in state.clients.values() {
        let _ = client
            .outbound
            .send(ControlMessage::ExecSync { control_tick })
            .await;
    }
}

async fn apply_host_membership_controls(
    controls: &[lc_engine::ControlPacket],
    state: &mut HostState,
) {
    for control in controls {
        match control {
            lc_engine::ControlPacket::ClientUpdate(update)
                if update.by_client == HOST_CLIENT_ID as i32 =>
            {
                let Ok(client_id) = ClientId::try_from(update.client_id) else {
                    continue;
                };
                match update.update_type {
                    lc_engine::CLIENT_UPDATE_ACTIVATE => {
                        let activated = update.data != 0;
                        if let Some(core) = state.client_cores.get_mut(&update.client_id) {
                            core.activated = activated;
                            core.observer = false;
                        } else {
                            continue;
                        }
                        if let Some(client) = state.clients.get_mut(&client_id) {
                            client.core.activated = activated;
                            client.core.observer = false;
                        }
                        if activated {
                            let _ = state.coordination_register(client_id);
                        } else {
                            coordination_unregister(client_id, state).await;
                        }
                    }
                    lc_engine::CLIENT_UPDATE_SET_OBSERVER => {
                        if let Some(core) = state.client_cores.get_mut(&update.client_id) {
                            core.activated = false;
                            core.observer = true;
                        } else {
                            continue;
                        }
                        if let Some(client) = state.clients.get_mut(&client_id) {
                            client.core.activated = false;
                            client.core.observer = true;
                        }
                        coordination_unregister(client_id, state).await;
                    }
                    _ => {}
                }
            }
            lc_engine::ControlPacket::ClientRemove(remove)
                if remove.by_client == HOST_CLIENT_ID as i32 =>
            {
                if let Ok(client_id) = ClientId::try_from(remove.client_id) {
                    coordination_unregister(client_id, state).await;
                }
                if let Some(core) = state.client_cores.remove(&remove.client_id) {
                    state.admission.remove_client_name(&core.name);
                }
                state.client_addresses.remove(&remove.client_id);
                state.resource_catalog.remove_at_client(remove.client_id);
                if let Some(backend) = state.resource_backend.as_mut() {
                    backend.remove_at_client(remove.client_id);
                }
                state.pending_kinds.remove(&remove.client_id);
            }
            _ => {}
        }
    }
}

async fn coordination_unregister(client_id: ClientId, state: &mut HostState) {
    let ready_batches = state
        .coordinator
        .remove_client(client_id)
        .unwrap_or_default();
    state.scheduler.remove_client(client_id);
    for batch in ready_batches {
        publish_ready_batch(batch, state).await;
    }
}

async fn broadcast_status(status: NetworkStatus, acknowledgement: bool, state: &mut HostState) {
    for client in state.clients.values() {
        let message = if acknowledgement {
            ControlMessage::StatusAck(status)
        } else {
            ControlMessage::Status(status)
        };
        let _ = client.outbound.send(message).await;
    }
}

async fn broadcast_ready_check(
    packet: ReadyCheckPacket,
    except_client_id: Option<ClientId>,
    state: &mut HostState,
) {
    for (client_id, client) in &state.clients {
        if Some(*client_id) != except_client_id {
            let _ = client
                .outbound
                .send(ControlMessage::ReadyCheck(packet))
                .await;
        }
    }
}

async fn broadcast_lobby_countdown(packet: LobbyCountdownPacket, state: &mut HostState) {
    for client in state.clients.values() {
        let _ = client
            .outbound
            .send(ControlMessage::LobbyCountdown(packet))
            .await;
    }
}

fn apply_ready_check_to_host_state(packet: ReadyCheckPacket, state: &mut HostState) {
    if packet.data.vote_requested() {
        return;
    }
    let ready = packet.data.is_ready();
    if let Some(core) = state.client_cores.get_mut(&packet.client_id) {
        core.lobby_ready = ready;
    }
    if let Ok(client_id) = ClientId::try_from(packet.client_id) {
        if let Some(client) = state.clients.get_mut(&client_id) {
            client.core.lobby_ready = ready;
        }
    }
}

async fn apply_barrier_effects(effects: Vec<BarrierEffect>, state: &mut HostState) {
    let mut committed = false;
    for effect in effects {
        match effect {
            BarrierEffect::InvalidateReference
            | BarrierEffect::DriveControlTo(_)
            | BarrierEffect::StopControl
            | BarrierEffect::SweepUnjoinedPlayers
            | BarrierEffect::StartControl => {}
            BarrierEffect::SetControlMode(mode) => state.control_mode = mode,
            BarrierEffect::BroadcastStatus(status) => {
                broadcast_status(status, false, state).await;
            }
            BarrierEffect::ExecutePendingSyncControls => {
                if let Ok(control_tick) = Tick::try_from(state.status_barrier.status.target_tick) {
                    broadcast_exec_sync(control_tick, state).await;
                }
            }
            BarrierEffect::BroadcastStatusAck(status) => {
                broadcast_status(status, true, state).await;
                committed = true;
            }
            BarrierEffect::SendStatusAck { client_id, status } => {
                if let Some(client) = state.clients.get(&client_id) {
                    let _ = client
                        .outbound
                        .send(ControlMessage::StatusAck(status))
                        .await;
                }
            }
        }
    }
    if committed {
        let _ = state
            .event_tx
            .send(HostEvent::StatusCommitted(state.status_barrier.status))
            .await;
    }
}

struct ClientTask<S> {
    local_connection_id: u32,
    remote_connection_id: u32,
    client_id: ClientId,
    transport: crate::ControlTransport<S>,
    outbound_rx: mpsc::Receiver<HostOutboundMessage>,
    host_tx: mpsc::Sender<HostLoopMessage>,
    liveness: ConnectionLivenessState,
}

impl<S> ClientTask<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    async fn notify_disconnected(&mut self, reason: Option<String>) {
        let post_mortem = self
            .transport
            .create_post_mortem(self.remote_connection_id);
        let _ = self
            .host_tx
            .send(HostLoopMessage::ClientDisconnected {
                connection_id: self.local_connection_id,
                client_id: self.client_id,
                next_inbound_packet: self.liveness.connection().inbound_packet_counter(),
                post_mortem,
                reason,
            })
            .await;
    }

    async fn run(mut self) {
        loop {
            let liveness_deadline = self.liveness.next_timer_at();
            tokio::select! {
                Some(message) = self.outbound_rx.recv() => {
                    let result = match message {
                        HostOutboundMessage::Message(message) => {
                            self.transport.send_message(message).await
                        }
                        HostOutboundMessage::Raw(packet) => {
                            self.transport.send_complete_packet_bytes(&packet).await
                        }
                    };
                    if let Err(error) = result {
                        self.notify_disconnected(Some(format!("send failed: {error}")))
                            .await;
                        break;
                    }
                }
                packet = self.transport.read_packet() => {
                    let result = match packet {
                        Ok(crate::transport::InboundPacket::Message(message)) => {
                            self.liveness.record_inbound_message(&message);
                            Ok(message)
                        }
                        Ok(crate::transport::InboundPacket::Ignored(packet_type)) => {
                            self.liveness.record_inbound_packet(packet_type);
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::UnhandledPacket {
                                    client_id: Some(self.client_id),
                                    packet_type,
                                })
                                .await;
                            continue;
                        }
                        Ok(crate::transport::InboundPacket::Empty) => continue,
                        Ok(crate::transport::InboundPacket::Invalid {
                            packet_type,
                            error,
                        }) => {
                            self.liveness.record_inbound_packet(packet_type);
                            Err(error)
                        }
                        Err(error) => Err(error),
                    };
                    match result {
                        Ok(ControlMessage::Ping(packet)) => {
                            if let Err(error) = self
                                .transport
                                .send_message(ControlMessage::Pong(packet))
                                .await
                            {
                                self.notify_disconnected(Some(format!("pong send failed: {error}")))
                                    .await;
                                break;
                            }
                        }
                        Ok(ControlMessage::Pong(packet)) => {
                            self.liveness.record_pong(packet);
                        }
                        Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                            self.notify_disconnected(Some(
                                lc_resources::decode_legacy_script_text(reply.message.as_bytes()),
                            ))
                            .await;
                            break;
                        }
                        Ok(message) => {
                            let ping_ms = self
                                .liveness
                                .connection()
                                .measured_ping_ms()
                                .unwrap_or(-1);
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ClientMessage {
                                    connection_id: self.local_connection_id,
                                    client_id: self.client_id,
                                    message,
                                    ping_ms,
                                })
                                .await;
                        }
                        Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                            self.notify_disconnected(None).await;
                            break;
                        }
                        Err(error) => {
                            self.notify_disconnected(Some(format!("read failed: {error}"))).await;
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep_until(liveness_deadline) => {
                    if let Err(reason) = drive_session_liveness_timer(
                        &mut self.transport,
                        &mut self.liveness,
                    )
                    .await
                    {
                        self.notify_disconnected(Some(reason)).await;
                        break;
                    }
                }
            }
        }
    }
}

async fn drive_session_liveness_timer<S>(
    transport: &mut crate::ControlTransport<S>,
    liveness: &mut ConnectionLivenessState,
) -> Result<(), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let ping = liveness
        .timer_tick()
        .map_err(|timeout| format!("connection {timeout:?} timeout"))?;
    if let Some(ping) = ping {
        let result = transport.send_message(ControlMessage::Ping(ping)).await;
        // C4Network2IO calls OnPing after the send attempt even on failure
        // (src/C4Network2IO.cpp:1141-1151).
        liveness.record_ping_dispatched();
        result.map_err(|error| format!("ping send failed: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
async fn run_client_loop<S>(
    transport: crate::ControlTransport<S>,
    commands: mpsc::Receiver<ClientCommand>,
    event_tx: mpsc::Sender<ClientEvent>,
    shutdown_rx: oneshot::Receiver<()>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    run_client_loop_with_addresses(
        transport,
        commands,
        event_tx,
        shutdown_rx,
        None,
        BTreeMap::new(),
        ClientResourceState::empty(),
    )
    .await;
}

async fn run_client_loop_with_addresses<S>(
    mut transport: crate::ControlTransport<S>,
    mut commands: mpsc::Receiver<ClientCommand>,
    event_tx: mpsc::Sender<ClientEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
    host_peer_addr: Option<SocketAddr>,
    mut client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    mut resource_state: ClientResourceState,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut backlog = ControlBacklog::new(CLIENT_BACKLOG_LIMIT);
    let mut pending_sync = Vec::<lc_engine::ControlPacket>::new();
    let mut received_controls = BTreeSet::<(ClientId, Tick)>::new();
    let mut highest_received_tick = None::<Tick>;
    let mut resource_timer = interval(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS));

    for (core, path) in std::mem::take(&mut resource_state.initial_complete_resources) {
        let _ = event_tx
            .send(ClientEvent::ResourceComplete {
                resource_id: core.id,
                core,
                path,
            })
            .await;
    }

    for packet in std::mem::take(&mut resource_state.initial_controls) {
        let key = (packet.client_id(), packet.tick());
        if received_controls.insert(key) {
            highest_received_tick =
                Some(highest_received_tick.map_or(packet.tick(), |tick| tick.max(packet.tick())));
            let ready = match resource_state.control.accept_network(packet) {
                Ok(ready) => ready,
                Err(error) => {
                    let _ = event_tx
                        .send(ClientEvent::Disconnected {
                            reason: Some(format!("invalid initial control packet: {error}")),
                        })
                        .await;
                    return;
                }
            };
            for packet in ready {
                let _ = event_tx.send(ClientEvent::Ready { packet }).await;
            }
        }
    }

    for packet in std::mem::take(&mut resource_state.initial_ready_checks) {
        if packet.data.vote_requested() && packet.client_id != 0 {
            continue;
        }
        let _ = event_tx.send(ClientEvent::ReadyCheck { packet }).await;
    }

    for packet in std::mem::take(&mut resource_state.initial_lobby_countdowns) {
        let _ = event_tx
            .send(ClientEvent::LobbyCountdown { packet })
            .await;
    }

    for packet in std::mem::take(&mut resource_state.initial_packets) {
        let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
        let result = if let Some(backend) = resource_state.backend.as_mut() {
            let mut random = resource_safe_random;
            match backend.on_packet(
                resource_state.host_peer_id,
                &packet,
                now_seconds,
                &mut random,
            ) {
                Ok(events) => dispatch_client_resource_events(
                    events,
                    &mut transport,
                    &event_tx,
                    resource_state.host_peer_id,
                )
                .await
                .map_err(|error| error.to_string()),
                Err(error) => Err(error.to_string()),
            }
        } else {
            let actions = resource_state
                .catalog
                .on_packet(resource_state.host_peer_id, &packet);
            dispatch_client_resource_actions(
                actions,
                &mut transport,
                &event_tx,
                resource_state.host_peer_id,
            )
            .await
            .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            let _ = event_tx
                .send(ClientEvent::Disconnected {
                    reason: Some(format!("resource bootstrap failed: {error}")),
                })
                .await;
            return;
        }
    }

    'outer: loop {
        let liveness_deadline = resource_state.liveness.next_timer_at();
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break,
            Some(command) = commands.recv() => {
                match command {
                    ClientCommand::SubmitStatusAck(status) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::StatusAck(status))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitReadyCheck(packet) => {
                        let raw_result = transport
                            .send_message(ControlMessage::ReadyCheck(packet))
                            .await;
                        let forward_result = match raw_result {
                            Ok(()) => {
                                let nested_packet =
                                    crate::transport::encode_complete_ready_check_packet(packet);
                                transport
                                    .send_message(ControlMessage::ForwardRequest(
                                        crate::ForwardPacket {
                                            negative_list: true,
                                            clients: vec![resource_state.host_peer_id],
                                            nested_packet,
                                        },
                                    ))
                                    .await
                            }
                            Err(error) => Err(error),
                        };
                        if let Err(error) = forward_result {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::RequestActivation(tick) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::ActivationRequest { tick })
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitPlayerInfoUpdate(request) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::PlayerInfoUpdate(request))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitControl(packet) => {
                        let clone = packet.clone();
                        let message = if resource_state.control.mode == 0 {
                            match crate::transport::encode_complete_control_packet(&packet) {
                                Ok(nested_packet) => {
                                    ControlMessage::ForwardRequest(crate::ForwardPacket {
                                        negative_list: true,
                                        clients: Vec::new(),
                                        nested_packet,
                                    })
                                }
                                Err(error) => {
                                    let _ = event_tx
                                        .send(ClientEvent::Disconnected {
                                            reason: Some(format!("send failed: {error}")),
                                        })
                                        .await;
                                    break;
                                }
                            }
                        } else {
                            ControlMessage::Control(packet)
                        };
                        match transport.send_message(message).await {
                            Ok(()) => {
                                backlog.record_packet(&clone);
                                match resource_state.control.ingest_contribution(clone) {
                                    Ok(ready) => {
                                        for packet in ready {
                                            let _ = event_tx
                                                .send(ClientEvent::Ready { packet })
                                                .await;
                                        }
                                    }
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "invalid local control packet: {error}"
                                                )),
                                            })
                                            .await;
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("send failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    ClientCommand::SubmitPacket { delivery, data } => {
                        let message = if matches!(
                            delivery,
                            ControlDelivery::Direct | ControlDelivery::Private
                        ) {
                            ControlMessage::ForwardRequest(crate::ForwardPacket {
                                negative_list: true,
                                clients: Vec::new(),
                                nested_packet:
                                    crate::transport::encode_complete_control_delivery_packet(
                                        delivery, &data,
                                    ),
                            })
                        } else {
                            ControlMessage::Packet { delivery, data }
                        };
                        if let Err(error) = transport.send_message(message).await {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::ExecSync { control_tick } => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::ExecSync { control_tick })
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::SubmitResource(packet) => {
                        if let Err(error) = transport
                            .send_message(ControlMessage::Resource(packet))
                            .await
                        {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("resource send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    ClientCommand::RemoveResource {
                        resource_id,
                        completion,
                    } => {
                        let _ = completion.send(resource_state.remove_resource(resource_id));
                    }
                    ClientCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = resource_state.publish_player_resource(request);
                        let _ = completion.send(result);
                    }
                    ClientCommand::GracefulPart { completion } => {
                        let result = transport
                            .send_message(ControlMessage::ConnectionReply(
                                crate::ConnectionReply {
                                    ok: false,
                                    message: lc_engine::LegacyCString::from_bytes(
                                        b"removing client".to_vec(),
                                    )
                                    .unwrap_or_default(),
                                    wrong_password: false,
                                },
                            ))
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                        break;
                    }
                    ClientCommand::Shutdown => break,
                }
            }
            _ = resource_timer.tick() => {
                let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
                if let Some(backend) = resource_state.backend.as_mut() {
                    let mut random = resource_safe_random;
                    match backend.on_timer(now_seconds, &mut random) {
                        Ok(events) => {
                            if let Err(error) = dispatch_client_resource_events(
                                events,
                                &mut transport,
                                &event_tx,
                                resource_state.host_peer_id,
                            )
                            .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("resource timer send failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("resource timer failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                } else {
                    let actions = resource_state.catalog.on_timer(now_seconds);
                    if let Err(error) = dispatch_client_resource_actions(
                        actions,
                        &mut transport,
                        &event_tx,
                        resource_state.host_peer_id,
                    )
                    .await
                    {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(format!("resource timer send failed: {error}")),
                            })
                            .await;
                        break;
                    }
                }
            }
            _ = tokio::time::sleep_until(liveness_deadline) => {
                if let Err(reason) = drive_session_liveness_timer(
                    &mut transport,
                    &mut resource_state.liveness,
                )
                .await
                {
                    let _ = event_tx
                        .send(ClientEvent::Disconnected {
                            reason: Some(reason),
                        })
                        .await;
                    break;
                }
            }
            packet = transport.read_packet() => {
                let result = match packet {
                    Ok(crate::transport::InboundPacket::Message(message)) => {
                        resource_state.liveness.record_inbound_message(&message);
                        Ok(message)
                    }
                    Ok(crate::transport::InboundPacket::Ignored(packet_type)) => {
                        resource_state.liveness.record_inbound_packet(packet_type);
                        let _ = event_tx
                            .send(ClientEvent::UnhandledPacket { packet_type })
                            .await;
                        continue;
                    }
                    Ok(crate::transport::InboundPacket::Empty) => continue,
                    Ok(crate::transport::InboundPacket::Invalid {
                        packet_type,
                        error,
                    }) => {
                        resource_state.liveness.record_inbound_packet(packet_type);
                        Err(error)
                    }
                    Err(error) => Err(error),
                };
                let result = match result {
                    Ok(ControlMessage::Forward(packet)) => {
                        let local_client_id = resource_state.catalog.local_client_id();
                        if !forward_selects(&packet, local_client_id) {
                            continue;
                        }
                        match crate::transport::parse_complete_packet(&packet.nested_packet) {
                            Ok(Some(message)) => Ok(message),
                            Ok(None) => {
                                let packet_type = packet.nested_packet[0];
                                let _ = event_tx
                                    .send(ClientEvent::UnhandledPacket { packet_type })
                                    .await;
                                continue;
                            }
                            Err(error) => Err(error),
                        }
                    }
                    other => other,
                };
                match result {
                    Ok(ControlMessage::Ping(packet)) => {
                        if let Err(error) = transport.send_message(ControlMessage::Pong(packet)).await {
                            let _ = event_tx
                                .send(ClientEvent::Disconnected {
                                    reason: Some(format!("pong send failed: {error}")),
                                })
                                .await;
                            break;
                        }
                    }
                    Ok(ControlMessage::Pong(packet)) => {
                        resource_state.liveness.record_pong(packet);
                    }
                    Ok(ControlMessage::ConnectionRequest(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some("host sent a duplicate connection request".to_string()),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::ConnectionReply(reply)) if !reply.ok => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(lc_resources::decode_legacy_script_text(
                                    reply.message.as_bytes(),
                                )),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::ConnectionReply(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some("host sent a duplicate connection reply".to_string()),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::ForwardRequest(_)) | Ok(ControlMessage::Forward(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(
                                    "recursive forwarding packet is not accepted".to_string(),
                                ),
                            })
                            .await;
                        break;
                    }
                    Ok(ControlMessage::PostMortem(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(
                                    "post-mortem recovery has not reached the connection router"
                                        .to_string(),
                                ),
                            })
                            .await;
                        break;
                    }
                    // Admission consumes the only valid GS_Init JoinData.
                    // C++ merely logs/ignores later packets rather than
                    // disconnecting (src/C4Network2.cpp:1574-1580).
                    Ok(ControlMessage::JoinData(_)) => {}
                    Ok(ControlMessage::Address(packet)) => {
                        if !client_addresses.contains_key(&packet.client_id) {
                            continue;
                        }
                        let packet = host_peer_addr
                            .map(|peer| packet.announcement_for_peer(peer))
                            .unwrap_or(packet);
                        let insertion = crate::append_received_address(
                            client_addresses.entry(packet.client_id).or_default(),
                            packet.address,
                        );
                        if matches!(insertion, crate::AddressInsertion::Added { .. }) {
                            // A newly learned address is announced through all
                            // connected message links. The Rust client has one
                            // host link at this stage, so echo it there exactly
                            // once (src/C4Network2Client.cpp:259-278,581-597).
                            if let Err(error) = transport
                                .send_message(ControlMessage::Address(packet))
                                .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!(
                                            "address announcement failed: {error}"
                                        )),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    Ok(ControlMessage::Resource(packet)) => {
                        let now_seconds = resource_state.resource_epoch.elapsed().as_secs();
                        if let Some(backend) = resource_state.backend.as_mut() {
                            let mut random = resource_safe_random;
                            match backend.on_packet(
                                resource_state.host_peer_id,
                                &packet,
                                now_seconds,
                                &mut random,
                            ) {
                                Ok(events) => {
                                    if let Err(error) = dispatch_client_resource_events(
                                        events,
                                        &mut transport,
                                        &event_tx,
                                        resource_state.host_peer_id,
                                    )
                                    .await
                                    {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!("resource response failed: {error}")),
                                            })
                                            .await;
                                        break;
                                    }
                                }
                                Err(error) => {
                                    let _ = event_tx
                                        .send(ClientEvent::Disconnected {
                                            reason: Some(format!("resource response failed: {error}")),
                                        })
                                        .await;
                                    break;
                                }
                            }
                        } else {
                            let actions = resource_state
                                .catalog
                                .on_packet(resource_state.host_peer_id, &packet);
                            if let Err(error) = dispatch_client_resource_actions(
                                actions,
                                &mut transport,
                                &event_tx,
                                resource_state.host_peer_id,
                            )
                            .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("resource response failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    Ok(ControlMessage::Status(status)) => {
                        let _ = event_tx.send(ClientEvent::Status(status)).await;
                    }
                    Ok(ControlMessage::StatusAck(status)) => {
                        if status.state == NETWORK_STATE_GO {
                            resource_state.control.set_mode(status.control_mode);
                        }
                        let _ = event_tx.send(ClientEvent::StatusAck(status)).await;
                    }
                    Ok(ControlMessage::LobbyCountdown(packet)) => {
                        let _ = event_tx
                            .send(ClientEvent::LobbyCountdown { packet })
                            .await;
                    }
                    // A client accepts a Request only when packet.Client is
                    // the host. Other ReadyCheck values keep their claimed
                    // client unchanged (src/C4Network2.cpp:1625-1646).
                    Ok(ControlMessage::ReadyCheck(packet))
                        if packet.data.vote_requested() && packet.client_id != 0 => {}
                    Ok(ControlMessage::ReadyCheck(packet)) => {
                        let _ = event_tx.send(ClientEvent::ReadyCheck { packet }).await;
                    }
                    Ok(ControlMessage::ActivationRequest { .. }) => {
                        // PID_ClientActReq is accepted by the host only
                        // (src/C4Network2.cpp:982-991).
                    }
                    Ok(ControlMessage::Control(packet)) => {
                        let key = (packet.client_id(), packet.tick());
                        if !received_controls.insert(key) {
                            continue;
                        }
                        highest_received_tick = Some(
                            highest_received_tick.map_or(packet.tick(), |tick| tick.max(packet.tick())),
                        );
                        if let Some(highest) = highest_received_tick {
                            let threshold = highest.saturating_sub(CLIENT_BACKLOG_LIMIT as Tick);
                            received_controls.retain(|(_, tick)| *tick >= threshold);
                        }
                        match resource_state.control.accept_network(packet) {
                            Ok(ready) => {
                                for packet in ready {
                                    let _ = event_tx.send(ClientEvent::Ready { packet }).await;
                                }
                            }
                            Err(error) => {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!(
                                            "invalid synchronized control packet: {error}"
                                        )),
                                    })
                                    .await;
                                break;
                            }
                        }
                    }
                    Ok(ControlMessage::PlayerInfoUpdate(_)) => {
                        // PID_PlayerInfoUpdReq is accepted by the host only
                        // (src/C4Network2Players.cpp:405-411).
                    }
                    Ok(ControlMessage::Packet { delivery, data }) => {
                        match delivery {
                            ControlDelivery::Direct | ControlDelivery::Private => {
                                let mut local_data = data;
                                if let Ok(mut control) = decode_control_entry_payload(&local_data) {
                                    let local_sources =
                                        if let lc_engine::ControlPacket::PlayerInfo(info) =
                                            &mut control
                                        {
                                            let local_sources = resource_state
                                                .load_authoritative_player_resources(info);
                                            if let Ok(normalized) =
                                                crate::encode_control_entry_payload(&control)
                                            {
                                                local_data = normalized;
                                            }
                                            local_sources
                                        } else {
                                            Vec::new()
                                        };
                                    for (path, core) in local_sources {
                                        let _ = event_tx
                                            .send(ClientEvent::ResourceComplete {
                                                resource_id: core.id,
                                                core,
                                                path,
                                            })
                                            .await;
                                    }
                                    let ready = match resource_state
                                        .control
                                        .apply_membership(&control)
                                    {
                                        Ok(ready) => ready,
                                        Err(error) => {
                                            let _ = event_tx
                                                .send(ClientEvent::Disconnected {
                                                    reason: Some(format!(
                                                        "control membership update failed: {error}"
                                                    )),
                                                })
                                                .await;
                                            break;
                                        }
                                    };
                                    apply_client_membership(
                                        &mut client_addresses,
                                        &mut resource_state.catalog,
                                        resource_state.backend.as_mut(),
                                        &control,
                                    );
                                    for packet in ready {
                                        let _ = event_tx
                                            .send(ClientEvent::Ready { packet })
                                            .await;
                                    }
                                }
                                let _ = event_tx
                                    .send(ClientEvent::Direct {
                                        delivery,
                                        data: local_data,
                                    })
                                    .await;
                            }
                            ControlDelivery::Queue
                            | ControlDelivery::Sync
                            | ControlDelivery::Decide => {
                                match decode_control_entry_payload(&data) {
                                    Ok(control) => pending_sync.push(control),
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "invalid synchronized control packet: {error}"
                                                )),
                                            })
                                            .await;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    Ok(ControlMessage::ExecSync { control_tick }) => {
                        if pending_sync.is_empty() {
                            // Temporary compatibility for the session's
                            // pre-status startup marker. Real empty releases
                            // are suppressed by the host.
                            let _ = event_tx.send(ClientEvent::ExecSync { control_tick }).await;
                        } else {
                            let controls = std::mem::take(&mut pending_sync);
                            for control in &controls {
                                let ready = match resource_state
                                    .control
                                    .apply_membership(control)
                                {
                                    Ok(ready) => ready,
                                    Err(error) => {
                                        let _ = event_tx
                                            .send(ClientEvent::Disconnected {
                                                reason: Some(format!(
                                                    "control membership update failed: {error}"
                                                )),
                                            })
                                            .await;
                                        break 'outer;
                                    }
                                };
                                apply_client_membership(
                                    &mut client_addresses,
                                    &mut resource_state.catalog,
                                    resource_state.backend.as_mut(),
                                    control,
                                );
                                for packet in ready {
                                    let _ = event_tx
                                        .send(ClientEvent::Ready { packet })
                                        .await;
                                }
                            }
                            let _ = event_tx
                                .send(ClientEvent::SyncScheduled {
                                    control_tick,
                                    controls,
                                })
                                .await;
                        }
                    }
                    Ok(ControlMessage::Request { from_tick }) => {
                        let resend = backlog.fulfill_request(from_tick);
                        for packet in resend {
                            if let Err(error) = transport
                                .send_message(ControlMessage::Control(packet))
                                .await
                            {
                                let _ = event_tx
                                    .send(ClientEvent::Disconnected {
                                        reason: Some(format!("send failed: {error}")),
                                    })
                                    .await;
                                break 'outer;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some(format!("read failed: {error}")),
                            })
                            .await;
                        break;
                    }
                }
            }
        }
    }
}

async fn dispatch_client_resource_actions<S>(
    actions: Vec<crate::ResourceCatalogAction>,
    transport: &mut crate::ControlTransport<S>,
    event_tx: &mpsc::Sender<ClientEvent>,
    host_peer_id: i32,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for action in actions {
        match action {
            crate::ResourceCatalogAction::SendToPeer { peer_id, packet }
                if peer_id == host_peer_id =>
            {
                transport
                    .send_message(ControlMessage::Resource(packet))
                    .await?;
            }
            crate::ResourceCatalogAction::Broadcast { packet } => {
                // The current Rust session has its host message link here. As
                // P2P links are established, the same action must fan out over
                // every connected peer just like C++ BroadcastMsg.
                transport
                    .send_message(ControlMessage::Resource(packet))
                    .await?;
            }
            external => {
                let _ = event_tx.send(ClientEvent::ResourceAction(external)).await;
            }
        }
    }
    Ok(())
}

async fn dispatch_client_resource_events<S>(
    events: Vec<crate::ResourceTransferEvent>,
    transport: &mut crate::ControlTransport<S>,
    event_tx: &mpsc::Sender<ClientEvent>,
    host_peer_id: i32,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for event in events {
        match event {
            crate::ResourceTransferEvent::Transport(action) => {
                dispatch_client_resource_actions(vec![action], transport, event_tx, host_peer_id)
                    .await?;
            }
            crate::ResourceTransferEvent::Completed {
                resource_id,
                core,
                path,
            } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceComplete {
                        resource_id,
                        core,
                        path,
                    })
                    .await;
            }
            crate::ResourceTransferEvent::LoadFailed { resource_id } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceLoadFailed { resource_id })
                    .await;
            }
            crate::ResourceTransferEvent::FinishDerivedUnsupported { core } => {
                let _ = event_tx
                    .send(ClientEvent::ResourceDeriveUnsupported { core })
                    .await;
            }
        }
    }
    Ok(())
}

fn apply_client_membership(
    client_addresses: &mut BTreeMap<i32, Vec<crate::NetworkAddress>>,
    resource_catalog: &mut crate::ResourceCatalog,
    resource_backend: Option<&mut crate::ResourceTransferBackend>,
    control: &lc_engine::ControlPacket,
) {
    match control {
        lc_engine::ControlPacket::ClientJoin(join) => {
            client_addresses.entry(join.core.client_id).or_default();
        }
        lc_engine::ControlPacket::ClientRemove(remove) => {
            client_addresses.remove(&remove.client_id);
            resource_catalog.remove_at_client(remove.client_id);
            if let Some(backend) = resource_backend {
                backend.remove_at_client(remove.client_id);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode_control_packet, encode_control_entry_payload, encode_control_packet,
        LegacyControlFrame, NetworkStatus, ParticipantKind, NETWORK_STATE_GO,
    };
    use lc_engine::{
        ClientUpdateControlData, ControlPacket as EngineControlPacket, PlayerControlData,
        CLIENT_UPDATE_ACTIVATE,
    };
    use lc_resources::{c4group_file_crc, MutableGroup};
    use std::fs;
    use std::future::{pending, ready};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};
    use tokio::time::{timeout, timeout_at};

    fn tcp_frame(payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![0xff];
        frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
        frame.extend_from_slice(payload);
        frame
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_unwraps_a_selected_forwarded_control() {
        // PID_Fwd dispatches its complete nested packet exactly once when the
        // local client matches the positive list (pristine C++
        // src/C4Network2IO.cpp:1026-1033).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.set_mode(0);
        resource_state.control.register(0).unwrap();
        resource_state.control.register(1).unwrap();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let local = legacy_packet(1, 0, 0x22);
        command_tx
            .send(ClientCommand::SubmitControl(local))
            .await
            .unwrap();
        assert!(matches!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::ForwardRequest(_)
        ));

        let host = legacy_packet(0, 0, 0x11);
        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: crate::transport::encode_complete_control_packet(&host).unwrap(),
            }))
            .await
            .unwrap();

        let ready = match timeout(EVENT_WAIT, event_rx.recv()).await.unwrap() {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected forwarded aggregate, got {other:?}"),
        };
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_reports_a_selected_forwarded_unhandled_packet() {
        // PID_Fwd recursively enters HandlePacket. A structurally valid packet
        // without an enabled handler is logged just like a direct packet and
        // does not close the connection (src/C4Network2IO.cpp:1037-1045,
        // 856-899).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let league_results = vec![0x17, 0x01, b'O', b'K', 0x00, 0x00];

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: league_results,
            }))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::UnhandledPacket { packet_type: 0x17 })
        ));

        let ping = crate::PingPacket {
            sent_at: 29,
            packet_counter: 0,
        };
        host_transport
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            host_transport.read_message().await.unwrap(),
            ControlMessage::Pong(ping)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_unselected_malformed_forward_and_bounds_recursion() {
        // DoFwdTo is evaluated before the nested packet is unpacked. A selected
        // recursive PID_Fwd is bounded instead of reproducing C++'s unbounded
        // recursive HandlePacket call (pristine C++
        // src/C4Network2IO.cpp:1026-1033,1626-1636).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, mut event_rx) = mpsc::channel(2);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![2],
                nested_packet: vec![0x40],
            }))
            .await
            .unwrap();
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Status(received))) if received == status
        ));

        let mut recursive = vec![crate::PID_FORWARD];
        recursive.extend(
            crate::encode_forward_packet_payload(&crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: vec![0xff],
            })
            .unwrap(),
        );
        host_transport
            .send_message(ControlMessage::Forward(crate::ForwardPacket {
                negative_list: false,
                clients: vec![1],
                nested_packet: recursive,
            }))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::Disconnected { reason: Some(reason) }))
                if reason == "recursive forwarding packet is not accepted"
        ));
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decentral_client_sends_cpp_forward_request_for_local_control() {
        // BroadcastMsgToClients excludes the directly connected host, records
        // no other direct peers in the negative list, and sends the complete
        // PID_Control inside PID_FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameControlNetwork.cpp:156-174).
        let (client_stream, mut host_stream) = duplex(128);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.catalog.set_local_client_id(1);
        resource_state.control.set_mode(0);
        resource_state.control.register(0).unwrap();
        resource_state.control.register(1).unwrap();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        command_tx
            .send(ClientCommand::SubmitControl(
                ControlPacket::builder(1, 0).payload(vec![0xff]),
            ))
            .await
            .unwrap();
        let mut bytes = vec![0; 64];
        let count = timeout(EVENT_WAIT, host_stream.read(&mut bytes))
            .await
            .expect("forward request send wait")
            .unwrap();
        bytes.truncate(count);
        assert_eq!(
            bytes,
            [
                0xff, 0x08, 0x00, 0x00, 0x00, 0x04, 0x01, 0x00, 0x04, 0x40, 0x01, 0x00,
                0xff,
            ]
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_direct_and_private_packets_send_only_forward_request() {
        // CDT_Direct and CDT_Private exclude the host from the direct leg.
        // With no peer mesh, only the host FwdReq remains and its negative
        // list is empty (pristine C++ src/C4Network2Client.cpp:515-541;
        // src/C4GameControlNetwork.cpp:224-240).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(2);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        for delivery in [ControlDelivery::Direct, ControlDelivery::Private] {
            command_tx
                .send(ClientCommand::SubmitPacket {
                    delivery,
                    data: vec![0xaa, 0xbb],
                })
                .await
                .unwrap();
            assert_eq!(
                timeout(EVENT_WAIT, host_transport.read_message())
                    .await
                    .expect("forward request send wait")
                    .expect("read forward request"),
                ControlMessage::ForwardRequest(crate::ForwardPacket {
                    negative_list: true,
                    clients: Vec::new(),
                    nested_packet: vec![0x42, u8::from(delivery), 0xaa, 0xbb],
                })
            );
        }
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err(),
            "direct/private submission emitted an extra raw packet"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ready_check_sends_raw_then_host_excluding_forward_request() {
        // ReadyCheck uses includeHost=true: the host receives the raw packet
        // first and is then excluded from the fallback FwdReq (pristine C++
        // src/C4Network2Client.cpp:515-541; src/C4GameLobby.cpp:329-343).
        let (client_stream, host_stream) = duplex(512);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, _event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let mut resource_state = ClientResourceState::empty();
        resource_state.host_peer_id = 7;
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let packet = ReadyCheckPacket {
            client_id: 12,
            data: crate::ReadyCheckData::Ready,
        };

        command_tx
            .send(ClientCommand::SubmitReadyCheck(packet))
            .await
            .unwrap();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("raw ready-check send wait")
                .expect("read raw ready-check"),
            ControlMessage::ReadyCheck(packet)
        );
        let mut nested_packet = vec![0x21];
        nested_packet.extend_from_slice(&packet.client_id.to_ne_bytes());
        nested_packet.extend_from_slice(&i32::from(packet.data).to_ne_bytes());
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("ready-check forward request send wait")
                .expect("read ready-check forward request"),
            ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: vec![7],
                nested_packet,
            })
        );
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err(),
            "ready-check submission emitted an extra packet"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn rust_client_direct_packet_reaches_rust_host_and_observer_once() {
        // The client now reaches the host only through FwdReq for direct
        // packets. Preserve Rust-host interoperability while the generic
        // opaque forwarding router remains separate work.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let source = connect_client(
            address,
            ClientConfig::new("Source", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_a = connect_client(
            address,
            ClientConfig::new("Observer A", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_a_events = observer_a.take_event_receiver();
        let mut observer_b = connect_client(
            address,
            ClientConfig::new("Observer B", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut observer_b_events = observer_b.take_event_receiver();
        let source_id = source.client_id();
        let data =
            encode_control_entry_payload(&EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x22,
                data: 0x33,
                by_client: i32::try_from(source_id).unwrap(),
            }))
            .unwrap();

        source
            .submit_packet(ControlDelivery::Direct, data.clone())
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data: received,
                }) if client_id == source_id && received == data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => {
                    panic!("source forwarding failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before direct packet"),
            }
        }
        for (name, events) in [
            ("observer A", &mut observer_a_events),
            ("observer B", &mut observer_b_events),
        ] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::Direct {
                        delivery: ControlDelivery::Direct,
                        data: received,
                    }) if received == data => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("{name} disconnected during direct forwarding: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("{name} event stream ended before direct packet"),
                }
            }
        }

        let host_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(host_duplicate_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::Direct {
                        client_id,
                        delivery: ControlDelivery::Direct,
                        data: ref received,
                    } if client_id == source_id && *received == data
                ),
                "host executed the forwarded direct packet twice"
            );
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == source_id
                ),
                "host rejected the direct forwarding leg"
            );
        }
        for (name, events) in [
            ("observer A", &mut observer_a_events),
            ("observer B", &mut observer_b_events),
        ] {
            let duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            while let Ok(Some(event)) = timeout_at(duplicate_deadline, events.recv()).await {
                assert!(
                    !matches!(
                        event,
                        ClientEvent::Direct {
                            delivery: ControlDelivery::Direct,
                            data: ref received,
                        } if *received == data
                    ),
                    "{name} received the forwarded direct packet twice"
                );
            }
        }

        source.shutdown().await.unwrap();
        observer_a.shutdown().await.unwrap();
        observer_b.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_does_not_rebroadcast_a_raw_client_control() {
        // PID_Control dispatches only to HandleControl, which stores the
        // contribution; only HandleFwdReq performs fallback fanout (pristine
        // C++ src/C4GameControlNetwork.cpp:517-529;
        // src/C4Network2IO.cpp:1066-1117).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, _) = raw_client_transport(address, b"Observer A").await;
        let (mut observer_b, _) = raw_client_transport(address, b"Observer B").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer_a).await;
        drain_raw_client(&mut observer_b).await;

        let packet = ControlPacket::builder(source_id, 0).payload(vec![0xff]);
        source
            .send_message(ControlMessage::Control(packet.clone()))
            .await
            .unwrap();
        for observer in [&mut observer_a, &mut observer_b] {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(100);
            let mut rebroadcast = false;
            while let Ok(Ok(message)) = timeout_at(deadline, observer.read_message()).await {
                if message == ControlMessage::Control(packet.clone()) {
                    rebroadcast = true;
                    break;
                }
            }
            assert!(!rebroadcast, "raw PID_Control was incorrectly relayed");
        }

        drop(source);
        drop(observer_a);
        drop(observer_b);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forward_request_without_echoing_its_origin() {
        // HandleFwdReq excludes the requester from remote targets, sends the
        // nested packet directly when at most two remote clients remain, then
        // dispatches it locally when the negative list selects the host
        // (pristine C++ src/C4Network2IO.cpp:1066-1117).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        activate_joined_client(&host, &mut host_events, source_id).await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;

        let host_packet = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let source_packet = legacy_packet(source_id, 0, 0x22);
        host.submit_local_control(host_packet).await.unwrap();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&source_packet)
                    .unwrap(),
            }))
            .await
            .unwrap();

        assert!(raw_client_received_control(
            &mut observer,
            &source_packet,
            EVENT_WAIT
        )
        .await);
        assert!(
            !raw_client_received_control(
                &mut source,
                &source_packet,
                Duration::from_millis(100)
            )
            .await,
            "forward request echoed its nested control to the origin"
        );
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        drop(source);
        drop(observer);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_routes_forwarded_direct_control_and_checks_self_dispatch_author() {
        // HandleFwdReq relays the opaque ControlPkt before its independent
        // self leg applies C4GameControlNetwork's ByClient check
        // (src/C4Network2IO.cpp:1077-1128;
        // src/C4GameControlNetwork.cpp:477-492).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, _) = raw_client_transport(address, b"Observer A").await;
        let (mut observer_b, _) = raw_client_transport(address, b"Observer B").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer_a).await;
        drain_raw_client(&mut observer_b).await;

        let direct_data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x22,
                data: 0x33,
                by_client: i32::try_from(source_id).unwrap(),
            },
        ))
        .unwrap();
        let mut nested_packet = vec![0x42, u8::from(ControlDelivery::Direct)];
        nested_packet.extend_from_slice(&direct_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet,
            }))
            .await
            .unwrap();

        let expected_direct = ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: direct_data.clone(),
        };
        for observer in [&mut observer_a, &mut observer_b] {
            assert!(raw_client_received_message(observer, &expected_direct, EVENT_WAIT).await);
            assert!(
                !raw_client_received_message(
                    observer,
                    &expected_direct,
                    Duration::from_millis(50)
                )
                .await,
                "direct ControlPkt was relayed more than once"
            );
        }
        let host_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(host_deadline, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data,
                }) if client_id == source_id && data == direct_data => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => panic!("valid self dispatch failed: {error}"),
                Some(_) => continue,
                None => panic!("host event stream ended before direct self dispatch"),
            }
        }

        let spoofed_data = encode_control_entry_payload(&EngineControlPacket::PlayerControl(
            PlayerControlData {
                player: i32::try_from(source_id).unwrap(),
                command: 0x44,
                data: 0x55,
                by_client: i32::try_from(source_id + 1).unwrap(),
            },
        ))
        .unwrap();
        let mut spoofed_nested = vec![0x42, u8::from(ControlDelivery::Direct)];
        spoofed_nested.extend_from_slice(&spoofed_data);
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: spoofed_nested,
            }))
            .await
            .unwrap();

        let expected_spoofed = ControlMessage::Packet {
            delivery: ControlDelivery::Direct,
            data: spoofed_data.clone(),
        };
        for observer in [&mut observer_a, &mut observer_b] {
            assert!(raw_client_received_message(observer, &expected_spoofed, EVENT_WAIT).await);
        }
        let error_deadline = tokio::time::Instant::now() + EVENT_WAIT;
        let error = loop {
            match timeout_at(error_deadline, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct {
                    client_id,
                    delivery: ControlDelivery::Direct,
                    data,
                }) if client_id == source_id && data == spoofed_data => {
                    panic!("spoofed ControlPkt executed before its author error")
                }
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => break error,
                Some(_) => continue,
                None => panic!("host event stream ended before ControlPkt author error"),
            }
        };
        assert!(error.contains("claimed author"));
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::Direct {
                        client_id,
                        delivery: ControlDelivery::Direct,
                        ref data,
                    } if client_id == source_id && *data == spoofed_data
                ),
                "spoofed ControlPkt executed on the host"
            );
        }

        drop(source);
        drop(observer_a);
        drop(observer_b);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_forwarded_ready_check_opaquely_without_self_dispatch() {
        // A ReadyCheck can be selected for remote peers while the negative
        // list excludes the host. Its trailing bytes survive the direct relay
        // (src/C4Network2IO.cpp:1077-1128; src/C4GameLobby.cpp:329-343).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer, _) = raw_client_transport(address, b"Observer").await;
        drain_raw_client(&mut source).await;
        drain_raw_client(&mut observer).await;
        let mut observer = observer.into_inner();
        let ready = ReadyCheckPacket {
            client_id: i32::try_from(source_id).unwrap(),
            data: crate::ReadyCheckData::Ready,
        };
        let mut nested_packet = vec![0x21];
        nested_packet.extend_from_slice(&ready.client_id.to_ne_bytes());
        nested_packet.extend_from_slice(&i32::from(ready.data).to_ne_bytes());
        nested_packet.extend_from_slice(&[0xde, 0xad]);

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: vec![HOST_CLIENT_ID as i32],
                nested_packet: nested_packet.clone(),
            }))
            .await
            .unwrap();
        assert!(raw_tcp_received_frame(
            &mut observer,
            &nested_packet,
            EVENT_WAIT
        )
        .await);
        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(quiet_deadline, host_events.recv()).await {
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == source_id
                ),
                "opaque ReadyCheck relay was reported as an error"
            );
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == ready),
                "host was excluded from the forwarding list"
            );
        }

        drop(source);
        drop(observer);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_reports_a_self_forwarded_unhandled_packet() {
        // HandleFwdReq relays first and then recursively handles the self leg.
        // A valid packet with no enabled handler is reported without closing
        // its source connection (src/C4Network2IO.cpp:856-899,1077-1129).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        drain_raw_client(&mut source).await;
        let league_results = vec![0x17, 0x01, b'O', b'K', 0x00, 0x00];

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: league_results,
            }))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::UnhandledPacket {
                    client_id: Some(client_id),
                    packet_type: 0x17,
                }) if client_id == source_id => break,
                Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                }) if client_id == source_id => {
                    panic!("forwarded unhandled packet failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before unhandled packet"),
            }
        }

        let ping = crate::PingPacket {
            sent_at: 31,
            packet_counter: 0,
        };
        source
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, source.read_message()).await.unwrap() {
                Ok(ControlMessage::Pong(received)) if received == ping => break,
                Ok(_) => continue,
                Err(error) => panic!("connection closed after unhandled forwarding: {error}"),
            }
        }

        drop(source);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_reports_malformed_nested_forward_requests_without_closing() {
        // Selected nested packets recursively use the full packet pipeline;
        // malformed bytes are reported without preventing later traffic.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;

        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: vec![0x40],
            }))
            .await
            .unwrap();
        assert!(wait_for_host_error(&mut host_events, source_id)
            .await
            .contains("invalid forwarded packet"));

        let mut recursive = vec![crate::PID_FORWARD_REQUEST];
        recursive.extend(
            crate::encode_forward_packet_payload(&crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: vec![0xff],
            })
            .unwrap(),
        );
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: recursive,
            }))
            .await
            .unwrap();
        assert!(wait_for_host_error(&mut host_events, source_id)
            .await
            .contains("invalid forwarded packet"));

        let ping = crate::PingPacket {
            sent_at: 17,
            packet_counter: 3,
        };
        source
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, source.read_message()).await.unwrap() {
                Ok(ControlMessage::Pong(received)) if received == ping => break,
                Ok(_) => continue,
                Err(error) => panic!("connection closed after rejected forwarding: {error}"),
            }
        }

        drop(source);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_uses_cpp_forward_wrapper_for_more_than_two_remote_targets() {
        // HandleFwdReq switches from direct nested sends to one positive-list
        // PID_Fwd broadcast when more than two remote client IDs are selected
        // (pristine C++ src/C4Network2IO.cpp:1083-1112).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let (mut source, source_id) = raw_client_transport(address, b"Source").await;
        let (mut observer_a, observer_a_id) = raw_client_transport(address, b"A").await;
        let (mut observer_b, observer_b_id) = raw_client_transport(address, b"B").await;
        let (mut observer_c, observer_c_id) = raw_client_transport(address, b"C").await;
        for transport in [
            &mut source,
            &mut observer_a,
            &mut observer_b,
            &mut observer_c,
        ] {
            drain_raw_client(transport).await;
        }

        let control = ControlPacket::builder(source_id, 0).payload(vec![0xff]);
        let nested_packet = crate::transport::encode_complete_control_packet(&control).unwrap();
        source
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: nested_packet.clone(),
            }))
            .await
            .unwrap();
        let expected = crate::ForwardPacket {
            negative_list: false,
            clients: vec![observer_c_id, observer_b_id, observer_a_id]
                .into_iter()
                .map(|client_id| i32::try_from(client_id).unwrap())
                .collect(),
            nested_packet,
        };
        for transport in [&mut observer_a, &mut observer_b, &mut observer_c] {
            assert!(raw_client_received_forward(transport, &expected, EVENT_WAIT).await);
            assert!(
                !raw_client_received_control(transport, &control, Duration::from_millis(50)).await,
                "more-than-two target routing also sent a raw nested packet"
            );
        }
        assert!(
            !raw_client_received_forward(&mut source, &expected, Duration::from_millis(100)).await,
            "wrapper broadcast echoed to its origin"
        );

        drop(source);
        drop(observer_a);
        drop(observer_b);
        drop(observer_c);
        host.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn host_join_gate_returns_only_after_the_live_state_applies() {
        // C4Network2::AllowJoin mutates fAllowJoin before returning; callers
        // enter DoLobby only after that synchronous transition
        // (src/C4Network2.cpp:835-843; src/C4Game.cpp:3874-3880).
        let (command_tx, mut commands) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = HostHandle {
            command_tx,
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(async {}),
        };
        let setter = tokio::spawn(async move { handle.set_join_allowed(true).await });

        let HostCommand::SetJoinAllowed {
            allowed,
            completion,
        } = commands.recv().await.expect("gate command")
        else {
            panic!("expected gate command");
        };
        assert!(allowed);
        assert!(!setter.is_finished(), "host state has not applied the gate");
        completion.send(()).expect("acknowledge applied gate");
        setter
            .await
            .expect("setter task")
            .expect("gate acknowledgement");
    }

    #[test]
    fn loading_resource_advertises_received_chunks_for_cpp_peer_sharing() {
        // SetLoad assigns szStandalone immediately, so IsBinaryCompatible is
        // true while the file is still loading. Discovery therefore receives
        // a status containing the currently present chunk ranges
        // (src/C4Network2Res.cpp:496-523,553-567,831-845,1557-1568).
        let host = HostConfig::default();
        let core = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 8,
            chunk_size: 4,
            ..Default::default()
        };
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .unwrap();
        let mut state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .unwrap();

        assert_eq!(
            state.catalog.on_packet(
                0,
                &ResourcePacket::Discover(crate::ResourceDiscoverPacket {
                    resource_ids: vec![core.id],
                }),
            ),
            vec![crate::ResourceCatalogAction::SendToPeer {
                peer_id: 0,
                packet: ResourcePacket::Status(crate::ResourceStatusPacket {
                    resource_id: core.id,
                    chunks: crate::ResourceChunkAvailability {
                        chunk_count: 2,
                        ranges: Vec::new(),
                    },
                }),
            }]
        );
    }

    #[test]
    fn post_join_resource_registration_includes_scenario_last() {
        // HandleJoinData first registers GameRes, dynamic and players. After
        // InitClient returns, C4GameParameters::InitNetwork adds Scenario;
        // C4Network2ResList::Add prepends it to discovery order
        // (src/C4Network2.cpp:329-331,1612-1620;
        // src/C4GameParameters.cpp:541-549;
        // src/C4Network2Res.cpp:1431-1441).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.dynamic = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            chunk_size: 1,
            ..Default::default()
        };
        snapshot.parameters.scenario = lc_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            chunk_size: 1,
            ..Default::default()
        };
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            std::env::temp_dir(),
        )
        .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            None,
        )
        .unwrap();

        assert_eq!(state.catalog.discovery_packet().resource_ids, vec![8, 7]);
    }

    #[test]
    fn client_bootstrap_installs_an_exact_local_loadable_without_redownloading_it() {
        // SetByCore keeps a contents-identical binary-compatible local file;
        // AddByCore must not replace it with SetLoad or a Network temporary
        // (src/C4Network2Res.cpp:441-493,1473-1516).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();

        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();

        let backend = state.backend.expect("filesystem resource backend");
        assert_eq!(backend.path(core.id), Some(local_dynamic.as_path()));
        assert_eq!(backend.core(core.id), Some(&core));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_bootstrap_reports_an_exact_local_resource_as_complete() {
        // SetByCore leaves a contents-identical resource complete with its
        // local file immediately available through getFile; AddByCore then
        // returns that complete resource without starting SetLoad
        // (pristine 9ffa0a5d src/C4Network2Res.h:238-244;
        // src/C4Network2Res.cpp:441-457,1473-1496).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![local_dynamic.clone()]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();
        let (client_stream, _host_stream) = duplex(4096);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            state,
        ));

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("local resource completion event stalled")
            .expect("client event stream closed");
        let ClientEvent::ResourceComplete {
            resource_id,
            core: completed_core,
            path,
        } = event
        else {
            panic!("unexpected client bootstrap event: {event:?}");
        };
        assert_eq!(resource_id, core.id);
        assert_eq!(completed_core, core);
        assert_eq!(path, local_dynamic);

        shutdown_tx.send(()).unwrap();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn buffered_bootstrap_chunks_are_persisted_before_completion_is_reported() {
        // Once HandleJoinData registers the dynamic, resource Status and Data
        // packets run through C4Network2Res::OnStatus/OnChunk. OnChunk writes
        // the bytes before marking the chunk present and ending the load
        // (pristine 9ffa0a5d src/C4Network2.cpp:1612-1617;
        // src/C4Network2Res.cpp:886-940,1263-1318,1571-1615).
        let directories = SessionResourceDirectories::new();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let core = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            derived_id: -1,
            loadable: true,
            file_size: 5,
            chunk_size: 5,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = core.clone();
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let plan = crate::plan_client_bootstrap(
            &join_data,
            &crate::ClientBootstrapLocalCandidates::default(),
            directories.client.clone(),
        )
        .unwrap();
        let initial_packets = vec![
            ResourcePacket::Status(crate::ResourceStatusPacket {
                resource_id: core.id,
                chunks: crate::ResourceChunkAvailability {
                    chunk_count: 1,
                    ranges: vec![crate::ResourceChunkRange {
                        start: 0,
                        length: 1,
                    }],
                },
            }),
            ResourcePacket::Data(crate::ResourceDataPacket {
                resource_id: core.id,
                chunk: 0,
                data: b"early".to_vec(),
            }),
        ];
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            initial_packets,
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();
        let (client_stream, _host_stream) = duplex(4096);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            state,
        ));

        let event = timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("buffered resource completion stalled")
            .expect("client event stream closed");
        let ClientEvent::ResourceComplete {
            resource_id,
            core: completed_core,
            path,
        } = event
        else {
            panic!("unexpected buffered resource event: {event:?}");
        };
        assert_eq!(resource_id, core.id);
        assert_eq!(completed_core, core);
        assert_eq!(fs::read(&path).unwrap(), b"early");
        assert!(path.is_file());

        shutdown_tx.send(()).unwrap();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_removes_the_merged_dynamic_before_next_discovery() {
        // RetrieveScenario marks the dynamic resource removed immediately
        // after its files merge successfully; removed resources stay retained
        // but are excluded from subsequent discovery packets
        // (pristine 9ffa0a5d src/C4Network2.cpp:656-669;
        // src/C4Network2Res.cpp:825-829,1677-1688).
        let directories = SessionResourceDirectories::new();
        let local_dynamic = directories.root.join("local-dynamic.c4d");
        fs::write(&local_dynamic, b"local").unwrap();
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        let dynamic = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            contents_crc: 0x8bd6_88e8,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.dynamic = dynamic.clone();
        let scenario_id = snapshot.parameters.scenario.id;
        let join_data = JoinDataEnvelope {
            client_id: 1,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(dynamic.id, vec![local_dynamic]);
        let plan =
            crate::plan_client_bootstrap(&join_data, &candidates, directories.client.clone())
                .unwrap();
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            &plan,
            Some(directories.client.clone()),
        )
        .unwrap();
        let (client_stream, host_stream) = duplex(4096);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(2);
        let handle = ClientHandle {
            command_tx,
            event_rx: Some(event_rx),
            shutdown_tx: None,
            join_handle: tokio::spawn(async {}),
            client_id: 1,
            join_data: None,
        };
        let dynamic_id = dynamic.id;
        let removal = tokio::spawn(async move { handle.remove_resource(dynamic_id).await });
        tokio::task::yield_now().await;
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            state,
        ));

        removal
            .await
            .expect("resource-removal task")
            .expect("registered dynamic removal");
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let message = timeout(EVENT_WAIT, host_transport.read_message())
            .await
            .expect("post-removal discovery stalled")
            .expect("post-removal discovery transport");
        let ControlMessage::Resource(ResourcePacket::Discover(discovery)) = message else {
            panic!("unexpected post-removal message: {message:?}");
        };
        assert_eq!(discovery.resource_ids, vec![scenario_id]);

        shutdown_tx.send(()).unwrap();
        client_loop.await.unwrap();
    }

    #[test]
    fn client_player_publication_reuses_the_same_source_with_different_wire_metadata() {
        // LoadFromLocalFile and AddByFile search the resource list by the
        // normalized source path before allocating an ID. A hit reuses the
        // existing core even when the requested resource name or maker differs
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
        // src/C4Network2Res.cpp:1397-1417,1443-1471).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let request = |wire_name: &[u8], maker: &[u8]| crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: lc_engine::LegacyCString::from_bytes(wire_name.to_vec()).unwrap(),
            group_maker: lc_engine::LegacyCString::from_bytes(maker.to_vec()).unwrap(),
        };

        let original = state
            .publish_player_resource(request(b"First.c4p", b"First maker"))
            .unwrap();
        let reused = state
            .publish_player_resource(request(b"Second.c4p", b"Second maker"))
            .unwrap();

        assert_eq!(reused, original);
        assert_eq!(state.catalog.allocate_resource_id(), (7 << 16) + 1);
    }

    #[test]
    fn client_player_publication_reuses_a_locally_resolved_bootstrap_source() {
        // Received player resources are first admitted through AddByCore. If
        // that resolves to a local file, a later AddByFile lookup by the same
        // path reuses the existing core before allocating a client resource ID
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(
                crate::ClientBootstrapResourceRole::Player,
                &publication.core,
            )
            .unwrap();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let reused = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: lc_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec()).unwrap(),
                group_maker: lc_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

        assert_eq!(reused, publication.core);
        assert_eq!(state.catalog.allocate_resource_id(), 7 << 16);
    }

    #[test]
    fn client_bootstrap_keeps_a_nested_player_source_as_the_lookup_key() {
        // C4Group::Open retains a packed child's full mother/child name in
        // szFile. GetStandalone copies that child to a temporary file but,
        // unlike the directory branch, does not replace szFile. AddByFile
        // therefore still finds it by the original nested path (pristine
        // 9ffa0a5d src/C4Group.cpp:656-715,1792-1816,2408-2419;
        // src/C4Network2Res.cpp:431-449,516-588,1397-1417).
        let directories = SessionResourceDirectories::new();
        let mother_path = directories.root.join("Players.c4f");
        let mut player = MutableGroup::new("Shared.c4p");
        player
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        let contents_crc = player.contents_crc();
        let player_raw = player.pack_raw().unwrap();
        let mut mother = MutableGroup::new("Players.c4f");
        mother
            .add_child_with_metadata("Shared.c4p", player, 1, false)
            .unwrap();
        fs::write(&mother_path, mother.pack().unwrap()).unwrap();
        let nested_player = mother_path.join("Shared.c4p");
        let core = lc_engine::NetworkResourceCore {
            resource_type: crate::HostResourceType::Player as u8,
            id: 1 << 16,
            derived_id: -1,
            loadable: true,
            file_size: player_raw.len() as u32,
            file_crc: c4group_file_crc(&player_raw),
            chunk_size: 100 * 1024,
            contents_crc,
            filename: lc_engine::LegacyCString::from_bytes(
                b"Players.c4f/Shared.c4p".to_vec(),
            )
            .unwrap(),
            ..Default::default()
        };
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(core.id, vec![nested_player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(
                crate::ClientBootstrapResourceRole::Player,
                &core,
            )
            .unwrap();
        let standalone_path = match &resource.source {
            crate::ClientBootstrapResourceSource::Local(local) => {
                assert!(local.binary_compatible());
                assert_eq!(local.source_path(), nested_player);
                assert_ne!(local.path(), nested_player);
                local.path().to_path_buf()
            }
            source => panic!("expected a local packed child, got {source:?}"),
        };
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        assert_eq!(state.local_resource_sources.get(&nested_player), Some(&core));
        assert!(!state
            .local_resource_sources
            .contains_key(&standalone_path));
    }

    #[test]
    fn client_bootstrap_does_not_reuse_the_original_player_directory_path() {
        // SetByCore packs a directory and replaces szFile with the temporary
        // standalone before checking physical compatibility. Therefore a
        // later AddByFile of the original directory path does not find that
        // resource and allocates a new client ID (pristine 9ffa0a5d
        // src/C4Network2Res.cpp:431-449,516-588,1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        fs::create_dir(&player).unwrap();
        fs::write(player.join("Player.txt"), b"player core").unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "Host maker",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        let resolver = crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        );
        let resource = resolver
            .resolve(
                crate::ClientBootstrapResourceRole::Player,
                &publication.core,
            )
            .unwrap();
        assert_eq!(
            state.add_bootstrap_resource(&resource).unwrap(),
            ClientBootstrapRegistration::Registered
        );

        let published = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: lc_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                group_maker: lc_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

        assert_ne!(published, publication.core);
        assert_eq!(published.id, 7 << 16);
    }

    #[test]
    fn client_player_publication_reuses_an_authoritative_local_player_source() {
        // HandlePlayerInfo immediately loads each received player resource via
        // AddByCore. If that resolves to a local file, a later AddByFile of
        // the same path reuses the core before allocating a client resource ID
        // (pristine 9ffa0a5d src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1477).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Shared.c4p");
        let mut group = MutableGroup::new("Shared.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        fs::write(&player, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &player,
            directories.host.clone(),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Shared.c4p".to_vec()).unwrap(),
                "",
            ),
        )
        .unwrap();
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        let mut state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(directories.client.clone()),
        )
        .unwrap();
        let mut candidates = crate::ClientBootstrapLocalCandidates::default();
        candidates.insert(publication.core.id, vec![player.clone()]);
        state.retain_resource_resolver(crate::client_bootstrap::ClientBootstrapResolver::new(
            &candidates,
            directories.client.clone(),
        ));
        let mut info = lc_engine::PlayerInfoControlData {
            players: vec![lc_engine::ControlPlayerInfoEntry {
                flags: lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(publication.core.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let completed = state.load_authoritative_player_resources(&mut info);
        assert_eq!(completed, vec![(player.clone(), publication.core.clone())]);

        let reused = state
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: player,
                wire_name: lc_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec()).unwrap(),
                group_maker: lc_engine::LegacyCString::from_bytes(b"Client maker".to_vec())
                    .unwrap(),
            })
            .unwrap();

        assert_eq!(reused, publication.core);
        assert_eq!(state.catalog.allocate_resource_id(), 7 << 16);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_handle_publishes_the_selected_player_into_both_resource_registries() {
        // After SetLocalID, AddByFile allocates from the assigned client's
        // high-word namespace. NRT_Player publication protects the persistent
        // source with a temporary copy before OptimizeStandalone, and Add
        // makes that complete file visible to discovery and chunk requests
        // (pristine 9ffa0a5d src/C4Network2Res.cpp:1168-1205,1361-1385,
        // 1431-1471; src/C4PlayerInfo.cpp:70-104).
        let directories = SessionResourceDirectories::new();
        let player = directories.root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        group
            .add_file_with_metadata("Portrait.png", b"portrait".to_vec(), 2, false)
            .unwrap();
        let original = group.pack().unwrap();
        fs::write(&player, &original).unwrap();
        let request = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: lc_engine::LegacyCString::from_bytes(
                b"Players.c4f/Alice.c4p".to_vec(),
            )
            .unwrap(),
            group_maker: lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
        };
        let host = HostConfig::default();
        let snapshot = synthetic_join_snapshot(host.local_core, 8);
        let join_data = JoinDataEnvelope {
            client_id: 7,
            start_control_tick: snapshot.dynamic_tick,
            status: host.initial_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };

        let direct_directory = directories.root.join("direct");
        let mut direct_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(direct_directory),
        )
        .unwrap();
        let direct_core = direct_state
            .publish_player_resource(request.clone())
            .unwrap();
        assert_eq!(direct_core.id, 7 << 16);
        assert!(direct_state.catalog.contains_resource(direct_core.id));
        let direct_backend = direct_state.backend.as_ref().unwrap();
        assert_eq!(direct_backend.core(direct_core.id), Some(&direct_core));
        assert!(direct_backend.path(direct_core.id).unwrap().is_file());

        let (client_stream, host_stream) = duplex(4096);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let loop_directory = directories.root.join("loop");
        let resource_state = ClientResourceState::new(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(loop_directory),
        )
        .unwrap();
        let join_handle = tokio::spawn(run_client_loop_with_addresses(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));
        let handle = ClientHandle {
            command_tx,
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle,
            client_id: 7,
            join_data: Some(join_data),
        };

        let core = handle.publish_player_resource(request).await.unwrap();
        assert_eq!(core.id, 7 << 16);
        assert_eq!(core.resource_type, crate::HostResourceType::Player as u8);
        assert_eq!(fs::read(&player).unwrap(), original);

        host_transport
            .send_message(ControlMessage::Resource(ResourcePacket::Discover(
                crate::ResourceDiscoverPacket {
                    resource_ids: vec![core.id],
                },
            )))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Status(status))
                    if status.resource_id == core.id =>
                {
                    assert_eq!(status.chunks.ranges[0].start, 0);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    host_transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => {}
            }
        }
        host_transport
            .send_message(ControlMessage::Resource(ResourcePacket::Request(
                crate::ResourceRequestPacket {
                    resource_id: core.id,
                    chunk: 0,
                },
            )))
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert!(!data.data.is_empty());
                    break;
                }
                ControlMessage::Ping(ping) => {
                    host_transport
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => {}
            }
        }

        handle.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_handle_reuses_initial_and_serves_runtime_player_resources() {
        // LoadFromLocalFile searches the entire local resource list by source
        // path before AddByFile, including players published during InitHost.
        // A miss registers the new NRT_Player so an already-connected peer can
        // discover its complete chunks and ask for their bytes (pristine
        // 9ffa0a5d src/C4PlayerInfo.cpp:91-104; src/C4Network2Res.cpp:831-865,
        // 1168-1205,1431-1471,1557-1615).
        let directories = SessionResourceDirectories::new();
        let initial_player = directories.root.join("HostInitial.c4p");
        let mut initial_group = MutableGroup::new("HostInitial.c4p");
        initial_group
            .add_file_with_metadata("Player.txt", b"host initial player".to_vec(), 1, false)
            .unwrap();
        fs::write(&initial_player, initial_group.pack().unwrap()).unwrap();
        let initial_wire =
            lc_engine::LegacyCString::from_bytes(b"HostInitial.c4p".to_vec()).unwrap();
        let maker = lc_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
        let initial_request = crate::ClientPlayerResourceRequest {
            source_path: initial_player.clone(),
            wire_name: initial_wire.clone(),
            group_maker: maker.clone(),
        };
        let initial_publication = crate::publish_client_player_resource(
            crate::ClientPlayerResourcePublicationSpec {
                resource_id: 0,
                source_path: initial_player.clone(),
                wire_name: initial_wire,
                network_directory: directories.host.clone(),
                group_maker: maker.clone(),
            },
        )
        .unwrap();
        let initial_core = initial_publication.core.clone();

        let player = directories.root.join("HostRuntime.c4p");
        let mut group = MutableGroup::new("HostRuntime.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host runtime player".to_vec(), 1, false)
            .unwrap();
        let original = group.pack().unwrap();
        fs::write(&player, &original).unwrap();
        let publication = crate::ClientPlayerResourceRequest {
            source_path: player.clone(),
            wire_name: lc_engine::LegacyCString::from_bytes(b"HostRuntime.c4p".to_vec())
                .unwrap(),
            group_maker: maker,
        };

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![initial_publication.registration],
                resource_directory: Some(directories.host.clone()),
                resource_files: vec![initial_publication.resource_file],
                player_resource_sources: vec![(initial_player, initial_core.clone())],
                ..HostConfig::default()
            },
        )
        .await
        .unwrap();
        let stream = TcpStream::connect(address).await.unwrap();
        let mut peer = crate::ControlTransport::new(stream);
        let peer_name = lc_engine::LegacyCString::from_bytes(b"Peer".to_vec()).unwrap();
        run_client_connection_handshake(
            &mut peer,
            crate::ConnectionRequest {
                core: lc_engine::ClientCoreControlData {
                    client_id: -1,
                    name: peer_name.clone(),
                    nick: peer_name,
                    ..Default::default()
                },
                build: CURRENT_GAME_BUILD,
                password: lc_engine::LegacyCString::default(),
                connection_id: 0,
            },
        )
        .await
        .expect("peer joins before runtime publication");

        assert_eq!(
            host.publish_player_resource(initial_request).await.unwrap(),
            initial_core,
            "an InitHost player source reuses its existing core"
        );
        let core = host
            .publish_player_resource(publication.clone())
            .await
            .unwrap();
        let reused = host.publish_player_resource(publication).await.unwrap();
        assert_eq!(reused, core, "the same source path reuses one resource");
        assert_eq!(core.id, 1);
        assert_eq!(core.resource_type, crate::HostResourceType::Player as u8);
        assert_eq!(fs::read(&player).unwrap(), original);

        peer.send_message(ControlMessage::Resource(ResourcePacket::Discover(
            crate::ResourceDiscoverPacket {
                resource_ids: vec![core.id],
            },
        )))
        .await
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, peer.read_message())
                .await
                .expect("host runtime resource discovery stalled")
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Status(status))
                    if status.resource_id == core.id =>
                {
                    assert_eq!(status.chunks.ranges[0].start, 0);
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                _ => {}
            }
        }
        peer.send_message(ControlMessage::Resource(ResourcePacket::Request(
            crate::ResourceRequestPacket {
                resource_id: core.id,
                chunk: 0,
            },
        )))
        .await
        .unwrap();
        loop {
            match timeout(EVENT_WAIT, peer.read_message())
                .await
                .expect("host runtime resource chunk stalled")
                .unwrap()
            {
                ControlMessage::Resource(ResourcePacket::Data(data))
                    if data.resource_id == core.id =>
                {
                    assert_eq!(data.chunk, 0);
                    assert!(!data.data.is_empty());
                    break;
                }
                ControlMessage::Ping(ping) => {
                    peer.send_message(ControlMessage::Pong(ping)).await.unwrap();
                }
                _ => {}
            }
        }

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authoritative_player_info_loads_remote_resources_and_preserves_cpp_flag_rules() {
        // HandlePlayerInfo merges the authoritative list and immediately calls
        // LoadResources. Each eligible PIF_HasRes entry uses AddByCore(true):
        // an existing ID is reused, an identical local file wins, otherwise a
        // loadable core starts a download. Removed entries are untouched;
        // InScenario and unavailable non-loadable entries lose HasResource
        // locally (pristine 9ffa0a5d src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:275-292; src/C4Network2Res.cpp:1473-1516).
        let directories = SessionResourceDirectories::new();
        let source = directories.root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"player core".to_vec(), 1, false)
            .unwrap();
        fs::write(&source, group.pack().unwrap()).unwrap();
        let publication = crate::build_host_resource_core(
            &source,
            directories.root.join("published"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap();
        let valid_core = publication.core.clone();
        let hosted_path = publication.standalone_path.unwrap();
        let mut removed_core = valid_core.clone();
        removed_core.id += 1;
        let mut scenario_core = valid_core.clone();
        scenario_core.id += 2;
        let mut nonloadable_core = valid_core.clone();
        nonloadable_core.id += 3;
        nonloadable_core.loadable = false;
        nonloadable_core.file_size = u32::MAX;
        nonloadable_core.file_crc = u32::MAX;

        let local_host = HostConfig::default();
        let local_snapshot = synthetic_join_snapshot(local_host.local_core, 8);
        let local_join_data = JoinDataEnvelope {
            client_id: 2,
            start_control_tick: local_snapshot.dynamic_tick,
            status: local_host.initial_status,
            dynamic: local_snapshot.dynamic,
            parameters: local_snapshot.parameters,
        };
        let local_work_path = directories.root.join("client-local");
        let mut local_state = ClientResourceState::new(
            &local_join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
            Some(local_work_path.clone()),
        )
        .unwrap();
        let mut local_candidates = crate::ClientBootstrapLocalCandidates::default();
        local_candidates.extend_search_roots([directories.root.clone()]);
        local_state.retain_resource_resolver(
            crate::client_bootstrap::ClientBootstrapResolver::new(
                &local_candidates,
                local_work_path,
            ),
        );
        let mut local_info = lc_engine::PlayerInfoControlData {
            players: vec![lc_engine::ControlPlayerInfoEntry {
                flags: lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(valid_core.clone()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let local_sources = local_state.load_authoritative_player_resources(&mut local_info);
        assert_eq!(local_sources, vec![(source.clone(), valid_core.clone())]);
        assert!(local_state.catalog.contains_resource(valid_core.id));
        let local_backend = local_state.backend.as_ref().unwrap();
        assert_eq!(local_backend.core(valid_core.id), Some(&valid_core));
        assert_eq!(local_backend.path(valid_core.id), Some(source.as_path()));
        assert!(local_backend
            .catalog()
            .local_chunks(valid_core.id)
            .unwrap()
            .is_complete());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            resource_registrations: vec![crate::ResourceRegistration::from_core(
                &valid_core,
                true,
                false,
            )],
            resource_files: vec![HostedResourceFile {
                core: valid_core.clone(),
                path: hosted_path,
                ownership: crate::ResourceFileOwnership::Temporary,
                binary_compatible: true,
            }],
            ..HostConfig::default()
        };
        let host = start_host(listener, host_config).await.unwrap();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();

        let resource_player = |id: i32, flags: u16, core: lc_engine::NetworkResourceCore| {
            lc_engine::ControlPlayerInfoEntry {
                id,
                flags,
                resource: Some(core),
                ..Default::default()
            }
        };
        let info = lc_engine::PlayerInfoControlData {
            client_id: 1,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![
                resource_player(
                    1,
                    lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    valid_core.clone(),
                ),
                resource_player(
                    2,
                    lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE | lc_engine::PLAYER_INFO_FLAG_REMOVED,
                    removed_core.clone(),
                ),
                resource_player(
                    3,
                    lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE
                        | lc_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE,
                    scenario_core,
                ),
                resource_player(
                    4,
                    lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    nonloadable_core,
                ),
            ],
            by_client: 0,
        };
        let encoded = crate::encode_control_entry_payload(&lc_engine::ControlPacket::PlayerInfo(
            info.clone(),
        ))
        .unwrap();
        host.submit_packet(ControlDelivery::Direct, encoded.clone())
            .await
            .unwrap();

        let mut delivered = None;
        let mut completed = None;
        while delivered.is_none() || completed.is_none() {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct { data, .. }) => {
                    if let Ok(lc_engine::ControlPacket::PlayerInfo(actual)) =
                        decode_control_entry_payload(&data)
                    {
                        delivered = Some(actual);
                    }
                }
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core,
                    path,
                }) if resource_id == valid_core.id => {
                    completed = Some((core, path));
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected while loading PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }
        let delivered = delivered.unwrap();
        assert_ne!(
            delivered.players[0].flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0
        );
        assert_ne!(
            delivered.players[1].flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
            0,
            "removed players return before LoadResource mutates their flags"
        );
        for player in &delivered.players[2..] {
            assert_eq!(player.flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
            assert_eq!(player.resource, None);
        }
        let (completed_core, completed_path) = completed.unwrap();
        assert_eq!(completed_core, valid_core);
        assert!(completed_path.is_file());

        host.submit_packet(ControlDelivery::Direct, encoded)
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(lc_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(ClientEvent::ResourceComplete { resource_id, .. })
                    if resource_id == valid_core.id =>
                {
                    panic!("an already-registered PlayerInfo resource restarted its download");
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("duplicate PlayerInfo disconnected client: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_resolves_authoritative_player_resource_before_direct_broadcast() {
        // The host executes CID_PlrInfo locally as a direct control before
        // peers consume it. HandlePlayerInfo calls LoadResources there, and
        // AddByCore first searches for an identical local file before falling
        // back to AddLoad. A later AddByFile of that path reuses the resolved
        // resource before allocating a new host ID (pristine 9ffa0a5d
        // src/C4Network2Players.cpp:245-260;
        // src/C4PlayerInfo.cpp:70-104,275-292;
        // src/C4Network2Res.cpp:1397-1417,1443-1516).
        let directories = SessionResourceDirectories::new();
        let local_root = directories.root.join("local");
        fs::create_dir_all(&local_root).unwrap();
        let source = local_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"host-local player".to_vec(), 1, false)
            .unwrap();
        fs::write(&source, group.pack().unwrap()).unwrap();
        let core = crate::build_host_resource_core(
            &source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap()
        .core;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            local_resource_roots: vec![local_root],
            ..HostConfig::default()
        };
        let mut host = start_host(listener, host_config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();
        let info = lc_engine::PlayerInfoControlData {
            client_id: 1,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            by_client: 0,
        };
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&lc_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .unwrap();

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert_eq!(path, source);
                    break;
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("host could not resolve local PlayerInfo resource: {error}");
                }
                Some(HostEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(lc_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    panic!("host exposed PlayerInfo before its local resource completion");
                }
                Some(_) => {}
                None => panic!("host event stream ended"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(lc_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("host could not expose local PlayerInfo: {error}");
                }
                Some(_) => {}
                None => panic!("host event stream ended"),
            }
        }

        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert!(path.is_file());
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("host could not serve its local PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        let reused = host
            .publish_player_resource(crate::ClientPlayerResourceRequest {
                source_path: source,
                wire_name: lc_engine::LegacyCString::from_bytes(b"Renamed.c4p".to_vec()).unwrap(),
                group_maker: lc_engine::LegacyCString::from_bytes(b"Host maker".to_vec()).unwrap(),
            })
            .await
            .unwrap();
        assert_eq!(
            reused, core,
            "AddByFile reuses the locally resolved authoritative resource"
        );

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resolves_local_player_resource_before_exposing_direct_control() {
        let directories = SessionResourceDirectories::new();
        let host_root = directories.root.join("host-local");
        let client_root = directories.root.join("client-local");
        fs::create_dir_all(&host_root).unwrap();
        fs::create_dir_all(&client_root).unwrap();
        let host_source = host_root.join("Alice.c4p");
        let client_source = client_root.join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata("Player.txt", b"shared local player".to_vec(), 1, false)
            .unwrap();
        let player_bytes = group.pack().unwrap();
        fs::write(&host_source, &player_bytes).unwrap();
        fs::write(&client_source, player_bytes).unwrap();
        let core = crate::build_host_resource_core(
            &host_source,
            directories.root.join("core"),
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::Player,
                1 << 16,
                lc_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec()).unwrap(),
                "Host",
            ),
        )
        .unwrap()
        .core;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host_config = HostConfig {
            resource_directory: Some(directories.host.clone()),
            local_resource_roots: vec![host_root],
            ..HostConfig::default()
        };
        let host = start_host(listener, host_config).await.unwrap();
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_resource_roots([client_root]),
        )
        .await
        .unwrap();
        let mut client_events = client.take_event_receiver();
        let info = lc_engine::PlayerInfoControlData {
            client_id: 1,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry {
                id: 1,
                flags: lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(core.clone()),
                ..Default::default()
            }],
            by_client: 0,
        };
        host.submit_packet(
            ControlDelivery::Direct,
            crate::encode_control_entry_payload(&lc_engine::ControlPacket::PlayerInfo(info))
                .unwrap(),
        )
        .await
        .unwrap();

        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed,
                    path,
                }) if resource_id == core.id => {
                    assert_eq!(completed, core);
                    assert_eq!(path, client_source);
                    break;
                }
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(lc_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    panic!("client exposed PlayerInfo before its local resource completion");
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client could not resolve local PlayerInfo resource: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct { data, .. })
                    if matches!(
                        decode_control_entry_payload(&data),
                        Ok(lc_engine::ControlPacket::PlayerInfo(_))
                    ) =>
                {
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client could not expose local PlayerInfo: {reason:?}");
                }
                Some(_) => {}
                None => panic!("client event stream ended"),
            }
        }

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn default_client_config_transfers_a_cpp_resource_file_to_completion() {
        // C4Network2ResList handles Dis/Stat/Req/Data inside the network
        // session: OnStatus starts one request, SendChunk reads the standalone,
        // and OnChunk writes/refills until OnResComplete fires. ResList is
        // always initialized even when a caller does not override WorkPath
        // (src/C4Network2.cpp:358-362;
        // src/C4Network2Res.cpp:831-940,1017-1122,1546-1620).
        let directories = SessionResourceDirectories::new();
        let source = directories.host.join("Dynamic.c4d");
        fs::write(&source, b"local").unwrap();
        let core = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 5,
            file_crc: 0x8bd6_88e8,
            chunk_size: 2,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = core.clone();
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: core.clone(),
            path: source,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: true,
        }];

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let mut client =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();

        let completed_path = loop {
            match timeout(EVENT_WAIT, client.events().recv())
                .await
                .expect("resource transfer stalled")
                .expect("client event stream closed")
            {
                ClientEvent::ResourceComplete {
                    resource_id,
                    core: completed_core,
                    path,
                } => {
                    assert_eq!(resource_id, core.id);
                    assert_eq!(completed_core, core);
                    break path;
                }
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected during resource transfer: {reason:?}")
                }
                _ => continue,
            }
        };

        assert_eq!(fs::read(&completed_path).unwrap(), b"local");
        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rejects_an_unmatched_required_nonloadable_system_before_lobby() {
        // InitClient reruns GameRes.InitNetwork after HandleJoinData and fails
        // before Control.InitNetwork, Players.Init, or DoLobby when a required
        // non-loadable System core has no contents-identical local candidate
        // (src/C4Network2.cpp:281-344; src/C4GameParameters.cpp:125-160;
        // src/C4Network2Res.cpp:441-493,1473-1516).
        let directories = SessionResourceDirectories::new();
        let system_path = directories.host.join("System.c4g");
        let mismatched_system_path = directories.client.join("System.c4g");
        fs::write(&system_path, b"host system").unwrap();
        fs::write(&mismatched_system_path, b"different client system").unwrap();
        let publication = crate::build_host_resource_core(
            &system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                lc_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = lc_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: publication.core,
            path: system_path,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(mismatched_system_path),
        )
        .await;
        host.shutdown().await.unwrap();

        let error = result.expect_err("client must fail before entering the lobby");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("System.c4g") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rejects_nonloadable_dynamic_when_game_resources_are_empty() {
        // HandleJoinData requires ResDynamic independently of GameRes. A
        // non-loadable dynamic core with no contents-identical local file
        // clears the client after control initialization but before DoLobby
        // (src/C4Network2.cpp:1574-1618).
        let directories = SessionResourceDirectories::new();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: false,
            file_size: u32::MAX,
            file_crc: u32::MAX,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = lc_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
        assert!(snapshot.parameters.game_resources.is_empty());
        host_config.initial_join_snapshot = Some(snapshot);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let result = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone()),
        )
        .await;
        host.shutdown().await.unwrap();

        let error = result.expect_err("missing non-loadable dynamic must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Dynamic.c4d") && message.contains("non-loadable")),
            "unexpected client bootstrap failure: {error:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_accepts_a_contents_identical_local_nonloadable_system() {
        // SetByCore accepts a contents-identical local System even though its
        // non-loadable core has no transferable standalone; InitClient may
        // then continue into control/player initialization and the lobby
        // (src/C4Network2Res.cpp:441-493,1473-1516;
        // src/C4Network2.cpp:329-344).
        let directories = SessionResourceDirectories::new();
        let system_bytes = b"shared system";
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        fs::write(&host_system_path, system_bytes).unwrap();
        fs::write(&client_system_path, system_bytes).unwrap();
        let publication = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                lc_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.dynamic = lc_engine::NetworkResourceCore {
            resource_type: 2,
            id: 7,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Dynamic.c4d".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot.parameters.scenario = lc_engine::NetworkResourceCore {
            resource_type: 1,
            id: 8,
            loadable: true,
            file_size: 1,
            file_crc: 1,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(b"Scenario.c4s".to_vec()).unwrap(),
            ..Default::default()
        };
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![HostedResourceFile {
            core: publication.core,
            path: host_system_path,
            ownership: crate::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path),
        )
        .await
        .expect("contents-identical local System permits client bootstrap");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_search_roots_accept_contents_identical_nonloadable_definitions() {
        // SetByCore searches the executable roots for every core, not only
        // System. An over-limit Definitions resource remains non-loadable but
        // is accepted when a local Objects.c4d has the same contents CRC
        // (src/C4Network2Res.cpp:441-493,1443-1516;
        // src/C4GameParameters.cpp:125-160).
        let directories = SessionResourceDirectories::new();
        let system_bytes = b"shared system";
        let definitions_bytes = b"shared definitions";
        let host_system_path = directories.host.join("System.c4g");
        let client_system_path = directories.client.join("System.c4g");
        let host_definitions_path = directories.host.join("Objects.c4d");
        let client_definitions_path = directories.client.join("Objects.c4d");
        fs::write(&host_system_path, system_bytes).unwrap();
        fs::write(&client_system_path, system_bytes).unwrap();
        fs::write(&host_definitions_path, definitions_bytes).unwrap();
        fs::write(&client_definitions_path, definitions_bytes).unwrap();
        let system = crate::build_host_resource_core(
            &host_system_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                9,
                lc_engine::LegacyCString::from_bytes(b"System.c4g".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        let mut definitions = crate::build_host_resource_core(
            &host_definitions_path,
            &directories.host,
            crate::HostResourceCoreSpec::new(
                crate::HostResourceType::System,
                10,
                lc_engine::LegacyCString::from_bytes(b"Objects.c4d".to_vec()).unwrap(),
                "Test host",
            ),
        )
        .unwrap();
        definitions.core.resource_type = crate::HostResourceType::Definitions as u8;

        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.parameters.game_resources = vec![system.core.clone(), definitions.core.clone()];
        host_config.initial_join_snapshot = Some(snapshot);
        host_config.resource_directory = Some(directories.host.clone());
        host_config.resource_files = vec![
            HostedResourceFile {
                core: system.core,
                path: host_system_path,
                ownership: crate::ResourceFileOwnership::Persistent,
                binary_compatible: false,
            },
            HostedResourceFile {
                core: definitions.core,
                path: host_definitions_path,
                ownership: crate::ResourceFileOwnership::Persistent,
                binary_compatible: false,
            },
        ];

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player)
                .with_resource_directory(directories.client.clone())
                .with_local_system_path(client_system_path)
                .with_local_resource_roots([directories.client.clone()]),
        )
        .await
        .expect("contents-identical non-loadable definitions permit bootstrap");

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_clears_an_unavailable_optional_player_resource_before_exposing_join_data() {
        // Player resource failure is nonfatal, but LoadResource clears
        // PIF_HasRes before HandleJoinData returns and before the parameters
        // become visible to the rest of the client
        // (src/C4PlayerInfo.cpp:275-292; src/C4Network2.cpp:1595-1622).
        let mut host_config = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host_config.local_core.clone(), 8);
        snapshot.parameters.player_infos = crate::PlayerInfoListSnapshot {
            last_player_id: 1,
            clients: vec![crate::ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![lc_engine::ControlPlayerInfoEntry {
                    id: 1,
                    flags: lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: Some(nonloadable_core(3, 9, b"Host.c4p")),
                    ..Default::default()
                }],
            }],
        };
        host_config.initial_join_snapshot = Some(snapshot);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let host = start_host(listener, host_config).await.unwrap();
        let mut client =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .expect("an unavailable player resource must not abort the join");

        let join_data = client.take_join_data().expect("initial JoinData");
        let player = &join_data.parameters.player_infos.clients[0].players[0];
        assert_eq!(player.flags & lc_engine::PLAYER_INFO_FLAG_HAS_RESOURCE, 0);
        assert_eq!(player.resource, None);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    static NEXT_RESOURCE_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    struct SessionResourceDirectories {
        root: std::path::PathBuf,
        host: std::path::PathBuf,
        client: std::path::PathBuf,
    }

    impl SessionResourceDirectories {
        fn new() -> Self {
            let unique = NEXT_RESOURCE_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "legacyclonk-session-resource-{}-{unique}",
                std::process::id()
            ));
            let host = root.join("host");
            let client = root.join("client");
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&host).unwrap();
            fs::create_dir_all(&client).unwrap();
            Self { root, host, client }
        }
    }

    impl Drop for SessionResourceDirectories {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Upper bound for a single event wait. Generous so loaded parallel test
    /// runs do not trip it; a genuine failure still fails fast because the
    /// expected event never arrives at all.
    const EVENT_WAIT: Duration = Duration::from_secs(5);

    #[test]
    fn direct_client_join_authenticates_the_embedded_host_author() {
        let payload = encode_control_entry_payload(&EngineControlPacket::ClientJoin(
            lc_engine::ClientJoinControlData {
                core: lc_engine::ClientCoreControlData {
                    client_id: 3,
                    ..Default::default()
                },
                by_client: 0,
            },
        ))
        .expect("encode ClientJoin");

        assert!(authenticated_single_control(&payload, 0).is_ok());
        assert!(authenticated_single_control(&payload, 3).is_err());
    }

    #[test]
    fn scenario_player_init_authenticates_the_selecting_client() {
        // PID_ControlPkt rejects a non-host packet whose embedded ByClient
        // differs from the authenticated connection (src/C4GameControlNetwork.cpp:478-490).
        let payload = encode_control_entry_payload(
            &EngineControlPacket::InitScenarioPlayer(
                lc_engine::InitScenarioPlayerControlData {
                    team: 2,
                    player: 4,
                    by_client: 7,
                },
            ),
        )
        .expect("encode InitScenarioPlayer");

        assert!(authenticated_single_control(&payload, 7).is_ok());
        assert!(authenticated_single_control(&payload, 3).is_err());
    }

    #[test]
    fn single_control_authentication_uses_control_set_by_client() {
        let control = crate::LegacyControlSet {
            value_type: 0,
            data: 1,
            by_client: 7,
        }
        .into_control_packet();
        let payload = encode_control_entry_payload(&control).expect("encode CID_Set");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8).expect_err("reject spoofed author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_control_set_authentication_uses_frame_client_id() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![crate::LegacyControlSet {
                    value_type: 1,
                    data: 0,
                    by_client,
                }
                .into_control_packet()],
            })
            .expect("encode queued CID_Set")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_Set");
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn remove_player_control_cannot_forge_host_author() {
        let control = EngineControlPacket::RemovePlayer(
            lc_engine::RemovePlayerControlData {
                player: 4,
                disconnected: false,
                by_client: 0,
            },
        );
        let payload = encode_control_entry_payload(&control).expect("encode CID_RemovePlr");
        assert_eq!(
            authenticated_single_control(&payload, 0).expect("host author matches"),
            control
        );
        assert!(authenticated_single_control(&payload, 7).is_err());

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control],
        })
        .expect("encode queued CID_RemovePlr");
        let error = validate_queued_control_authors(&packet)
            .expect_err("queued client may not forge host CID_RemovePlr");
        assert!(error.contains("queued CID_RemovePlr"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_script_control_authenticates_embedded_author() {
        let control = EngineControlPacket::Script(lc_engine::ScriptControlData {
            target_object: lc_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: lc_engine::ScriptStrictness::Strict3,
            script: lc_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_Script");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8)
            .expect_err("reject spoofed script author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_script_control_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::Script(lc_engine::ScriptControlData {
                    target_object: lc_engine::SCRIPT_SCOPE_GLOBAL,
                    strictness: lc_engine::ScriptStrictness::Strict3,
                    script: lc_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                        .expect("fixture is NUL-free"),
                    by_client,
                })],
            })
            .expect("encode queued CID_Script")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_Script");
        assert!(error.contains("queued CID_Script"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_message_board_answer_authenticates_embedded_author() {
        let control = EngineControlPacket::MessageBoardAnswer(
            lc_engine::MessageBoardAnswerControlData {
                object: 42,
                answer: lc_engine::LegacyCString::from_bytes(b"answer".to_vec())
                    .expect("fixture is NUL-free"),
                player: 3,
                by_client: 7,
            },
        );
        let payload =
            encode_control_entry_payload(&control).expect("encode CID_MessageBoardAnswer");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8)
            .expect_err("reject spoofed message-board answer author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn single_message_control_authenticates_embedded_author() {
        let control = EngineControlPacket::Message(lc_engine::MessageControlData {
            message_type: lc_engine::MESSAGE_TYPE_PRIVATE,
            player: 3,
            to_player: 5,
            message: lc_engine::LegacyCString::from_bytes(b"secret".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_Message");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error =
            authenticated_single_control(&payload, 8).expect_err("reject spoofed message author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_message_board_answer_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::MessageBoardAnswer(
                    lc_engine::MessageBoardAnswerControlData {
                        object: 42,
                        answer: lc_engine::LegacyCString::from_bytes(b"answer".to_vec())
                            .expect("fixture is NUL-free"),
                        player: 3,
                        by_client,
                    },
                )],
            })
            .expect("encode queued CID_MessageBoardAnswer")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_MessageBoardAnswer");
        assert!(error.contains("queued CID_MessageBoardAnswer"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn single_custom_command_authenticates_embedded_author() {
        let control = EngineControlPacket::CustomCommand(lc_engine::CustomCommandControlData {
            command: lc_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("fixture is NUL-free"),
            argument: lc_engine::LegacyCString::from_bytes(b"argument".to_vec())
                .expect("fixture is NUL-free"),
            player: 3,
            by_client: 7,
        });
        let payload = encode_control_entry_payload(&control).expect("encode CID_CustomCommand");

        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching author"),
            control
        );
        let error = authenticated_single_control(&payload, 8)
            .expect_err("reject spoofed custom-command author");
        assert!(error.contains("claimed author 7"));
        assert!(error.contains("authenticated author is 8"));
    }

    #[test]
    fn queued_custom_command_cannot_forge_host_author() {
        let packet = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![EngineControlPacket::CustomCommand(
                    lc_engine::CustomCommandControlData {
                        command: lc_engine::LegacyCString::from_bytes(b"push".to_vec())
                            .expect("fixture is NUL-free"),
                        argument: lc_engine::LegacyCString::from_bytes(b"argument".to_vec())
                            .expect("fixture is NUL-free"),
                        player: 3,
                        by_client,
                    },
                )],
            })
            .expect("encode queued CID_CustomCommand")
        };

        validate_queued_control_authors(&packet(7)).expect("matching queued author");
        let error = validate_queued_control_authors(&packet(0))
            .expect_err("queued client may not forge host CID_CustomCommand");
        assert!(error.contains("queued CID_CustomCommand"));
        assert!(error.contains("claimed author 0"));
        assert!(error.contains("authenticated author is 7"));
    }

    #[test]
    fn em_move_object_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmMoveObject(lc_engine::EmMoveObjectControlData {
                action: lc_engine::EMMO_SCRIPT,
                tx: -12,
                ty: 34,
                target_object: 42,
                objects: vec![7, 9],
                strictness: lc_engine::ScriptStrictness::Strict2,
                script: lc_engine::LegacyCString::from_bytes(b"SetXDir(0)".to_vec())
                    .expect("fixture is NUL-free"),
                by_client,
            })
        };

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMMoveObj");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMMoveObj");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMMoveObj");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMMoveObj"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
    }

    #[test]
    fn em_draw_tool_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmDrawTool(lc_engine::EmDrawToolControlData {
                action: lc_engine::EMDT_LINE,
                mode: 3,
                x: -12,
                y: 34,
                x2: 56,
                y2: -78,
                grade: 9,
                ift: true,
                material: lc_engine::LegacyCString::from_bytes(b"Earth".to_vec())
                    .expect("fixture is NUL-free"),
                texture: lc_engine::LegacyCString::from_bytes(b"Rough".to_vec())
                    .expect("fixture is NUL-free"),
                by_client,
            })
        };

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMDrawTool");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor draw control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMDrawTool");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMDrawTool");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor draw control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMDrawTool"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
    }

    #[test]
    fn em_drop_def_control_authenticates_direct_and_queued_authors() {
        let control = |by_client| {
            EngineControlPacket::EmDropDef(lc_engine::EmDropDefControlData {
                id: *b"HUT2",
                x: -130,
                y: 130,
                by_client,
            })
        };

        let direct = control(7);
        let payload = encode_control_entry_payload(&direct).expect("encode CID_EMDropDef");
        assert_eq!(
            authenticated_single_control(&payload, 7).expect("matching direct author"),
            direct
        );
        let direct_error = authenticated_single_control(&payload, 8)
            .expect_err("direct editor drop control may not spoof its author");
        assert!(direct_error.contains("claimed author 7"));
        assert!(direct_error.contains("authenticated author is 8"));

        let packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(7)],
        })
        .expect("encode queued CID_EMDropDef");
        validate_queued_control_authors(&packet).expect("matching queued author");

        let forged_packet = encode_control_packet(&LegacyControlFrame {
            client_id: 7,
            tick: 12,
            timestamp_ms: 0,
            controls: vec![control(0)],
        })
        .expect("encode forged queued CID_EMDropDef");
        let queued_error = validate_queued_control_authors(&forged_packet)
            .expect_err("queued editor drop control may not forge the host author");
        assert!(queued_error.contains("queued CID_EMDropDef"));
        assert!(queued_error.contains("claimed author 0"));
        assert!(queued_error.contains("authenticated author is 7"));
    }

    #[test]
    fn internal_player_script_controls_authenticate_direct_and_queued_authors() {
        fn controls(by_client: i32) -> [EngineControlPacket; 5] {
            [
                EngineControlPacket::ActivateGameGoalMenu(
                    lc_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client,
                    },
                ),
                EngineControlPacket::ToggleHostility(lc_engine::ToggleHostilityControlData {
                    opponent: 4,
                    player: 3,
                    by_client,
                }),
                EngineControlPacket::ActivateGameGoalRule(
                    lc_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client,
                    },
                ),
                EngineControlPacket::SetPlayerTeam(lc_engine::SetPlayerTeamControlData {
                    team: 5,
                    player: 3,
                    by_client,
                }),
                EngineControlPacket::EliminatePlayer(lc_engine::EliminatePlayerControlData {
                    player: 3,
                    by_client,
                }),
            ]
        }

        let names = [
            "CID_ActivateGameGoalMenu",
            "CID_ToggleHostility",
            "CID_ActivateGameGoalRule",
            "CID_SetPlayerTeam",
            "CID_EliminatePlayer",
        ];
        for (name, control) in names.into_iter().zip(controls(7)) {
            let payload = encode_control_entry_payload(&control).expect("encode direct control");
            assert_eq!(
                authenticated_single_control(&payload, 7).expect("matching direct author"),
                control
            );
            let direct_error = authenticated_single_control(&payload, 8)
                .expect_err("direct author spoof must fail");
            assert!(direct_error.contains("claimed author 7"), "{name}: {direct_error}");

            let packet = encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![control],
            })
            .expect("encode queued control");
            validate_queued_control_authors(&packet).expect("matching queued author");

            let forged = controls(0)
                .into_iter()
                .zip(names)
                .find_map(|(candidate, candidate_name)| (candidate_name == name).then_some(candidate))
                .expect("fixture name exists");
            let forged_packet = encode_control_packet(&LegacyControlFrame {
                client_id: 7,
                tick: 12,
                timestamp_ms: 0,
                controls: vec![forged],
            })
            .expect("encode forged queued control");
            let queued_error = validate_queued_control_authors(&forged_packet)
                .expect_err("queued author spoof must fail");
            assert!(queued_error.contains(name), "{name}: {queued_error}");
            assert!(queued_error.contains("claimed author 0"), "{name}: {queued_error}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn pending_connection_attempt_times_out() {
        // C4Network2IO::CheckTimeout closes unaccepted connections after
        // C4NetAcceptTimeout (src/C4Network2IO.cpp:1155-1170).
        let result = timeout(
            HANDSHAKE_TIMEOUT + Duration::from_secs(1),
            connect_client_from(
                pending::<Result<TcpStream, io::Error>>(),
                ClientConfig::new("Alice", ParticipantKind::Player),
            ),
        )
        .await;

        match result {
            Ok(Err(ClientError::Connect(error))) => {
                assert_eq!(error.kind(), io::ErrorKind::TimedOut);
                assert_eq!(error.to_string(), "connection attempt timed out");
            }
            other => panic!("expected bounded connection timeout, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_first_packet_is_cpp_connection_request_not_json() {
        // C4Network2IO sends PID_Conn through the ordinary C4NetIOTCP frame as
        // soon as the socket opens (src/C4Network2IO.cpp:478-525,1223-1252;
        // src/C4NetIO.cpp:1287-1323).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(connect_client_from(
            TcpStream::connect(addr),
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));
        let (mut peer, _) = listener.accept().await.unwrap();
        let mut header_and_pid = [0; 6];
        peer.read_exact(&mut header_and_pid).await.unwrap();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        client.abort();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_first_packet_is_cpp_connection_request_without_blocking_listener() {
        // An accepted C++ TCP socket sends its own PID_Conn immediately; the
        // listener/main loop does not wait for the peer's request first
        // (src/C4Network2IO.cpp:479-530,1223-1252).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut peer = TcpStream::connect(addr).await.unwrap();
        let mut header_and_pid = [0; 6];
        timeout(Duration::from_secs(1), peer.read_exact(&mut header_and_pid))
            .await
            .expect("host must not wait for a client JSON/request prefix")
            .unwrap();

        assert_eq!(header_and_pid[0], 0xff);
        assert_eq!(header_and_pid[5], 0x02);
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_accepts_a_canonical_existing_client_connection_request() {
        // HandleConn selects an existing client before the new-client Join path;
        // CheckConn accepts status-only core differences and replies
        // "connection accepted" (src/C4Network2.cpp:1286-1334,1366-1380;
        // src/C4Client.cpp:58-70).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);

        assert!(matches!(
            timeout(EVENT_WAIT, transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        transport
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: lc_engine::ClientCoreControlData {
                        client_id: i32::try_from(client.client_id()).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: lc_engine::LegacyCString::default(),
                    connection_id: 17,
                },
            ))
            .await
            .unwrap();

        let reply = timeout(EVENT_WAIT, transport.read_message())
            .await
            .expect("host existing-client admission stalled")
            .unwrap();
        let accepted_message =
            lc_engine::LegacyCString::from_bytes(b"connection accepted".to_vec()).unwrap();
        assert_eq!(
            reply,
            ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: accepted_message,
                wrong_password: false,
            })
        );

        host.shutdown().await.unwrap();
        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secondary_route_does_not_rejoin_replace_or_remove_the_logical_client() {
        // HandleConnRe records whether this is the client's first connection;
        // only that first connection runs OnClientConnect and its JoinData,
        // lobby, and resource setup (src/C4Network2.cpp:1479-1498,1734-1743,
        // 1768-1783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![crate::ResourceRegistration {
                    resource_id: 3,
                    chunk_count: 1,
                    binary_compatible: true,
                    loading: false,
                }],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();
        while host_events.try_recv().is_ok() {}
        while canonical_events.try_recv().is_ok() {}

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut secondary = crate::ControlTransport::new(stream);
        let host_request = match secondary.read_message().await.unwrap() {
            ControlMessage::ConnectionRequest(request) => request,
            other => panic!("expected host connection request, got {other:?}"),
        };
        let local_connection_id = host_request.connection_id;
        let remote_connection_id = 29;
        let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        secondary
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: lc_engine::ClientCoreControlData {
                        client_id: i32::try_from(canonical_id).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: lc_engine::LegacyCString::default(),
                    connection_id: remote_connection_id,
                },
            ))
            .await
            .unwrap();
        loop {
            match secondary.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                other => panic!("expected positive host connection reply, got {other:?}"),
            }
        }
        secondary
            .send_message(ControlMessage::ConnectionReply(
                crate::ConnectionReply {
                    ok: true,
                    message: lc_engine::LegacyCString::from_bytes(
                        b"connection accepted".to_vec(),
                    )
                    .unwrap(),
                    wrong_password: false,
                },
            ))
            .await
            .unwrap();

        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 2 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("secondary accepted route was not retained");
        assert!(routes.contains(&(
            local_connection_id,
            canonical_id,
            remote_connection_id,
        )));

        while let Ok(event) = host_events.try_recv() {
            assert!(
                !matches!(
                    event,
                    HostEvent::ClientJoined { client_id, .. } if client_id == canonical_id
                ),
                "secondary route emitted duplicate ClientJoined"
            );
        }

        let quiet_deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        loop {
            match timeout_at(quiet_deadline, secondary.read_message()).await {
                Err(_) => break,
                Ok(Ok(ControlMessage::Ping(ping))) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                Ok(Ok(message)) => {
                    panic!("secondary route received duplicate first-connect setup: {message:?}")
                }
                Ok(Err(error)) => panic!("secondary route closed unexpectedly: {error}"),
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(7);
        host.submit_lobby_countdown(countdown).await.unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet }) if packet == countdown => break,
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended before lobby countdown"),
                }
            }
        })
        .await
        .expect("secondary route replaced the logical client's primary sender");

        // RemoveConn clears only the failed route. OnDisconnect removes the
        // logical client only when no message route remains
        // (src/C4Network2.cpp:1758-1783;
        // src/C4Network2Client.cpp:78-102).
        while host_events.try_recv().is_ok() {}
        drop(secondary);
        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 1 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("secondary route was not removed");
        assert!(routes.iter().all(|(connection_id, client_id, _)| {
            *connection_id != local_connection_id && *client_id == canonical_id
        }));

        while let Ok(event) = host_events.try_recv() {
            match event {
                HostEvent::ClientLeft { client_id } if client_id == canonical_id => {
                    panic!("secondary disconnect emitted ClientLeft for the logical client")
                }
                HostEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| matches!(
                        control,
                        EngineControlPacket::ClientRemove(remove)
                            if remove.client_id == i32::try_from(canonical_id).unwrap()
                    )) => panic!("secondary disconnect queued ClientRemove for the logical client"),
                _ => {}
            }
        }

        let after_disconnect = crate::LobbyCountdownPacket::new(6);
        host.submit_lobby_countdown(after_disconnect).await.unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet })
                        if packet == after_disconnect =>
                    {
                        break;
                    }
                    Some(ClientEvent::SyncScheduled { controls, .. })
                        if controls.iter().any(|control| matches!(
                            control,
                            EngineControlPacket::ClientRemove(remove)
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        )) => panic!("canonical client executed a secondary-route ClientRemove"),
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended after secondary disconnect"),
                }
            }
        })
        .await
        .expect("primary route stopped receiving after secondary disconnect");

        host.shutdown().await.unwrap();
        canonical.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn primary_route_loss_promotes_the_surviving_secondary() {
        // RemoveConn promotes the remaining data route to the message route;
        // OnDisconnect removes the logical client only when that fallback is
        // absent (src/C4Network2Client.cpp:78-102;
        // src/C4Network2.cpp:1758-1783).
        async fn connect_secondary(
            addr: SocketAddr,
            client_id: ClientId,
            remote_connection_id: u32,
        ) -> (crate::ControlTransport<TcpStream>, u32) {
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.unwrap() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
            transport
                .send_message(ControlMessage::ConnectionRequest(
                    crate::ConnectionRequest {
                        core: lc_engine::ClientCoreControlData {
                            client_id: i32::try_from(client_id).unwrap(),
                            activated: true,
                            observer: false,
                            name: name.clone(),
                            nick: name,
                            lobby_ready: true,
                        },
                        build: CURRENT_GAME_BUILD,
                        password: lc_engine::LegacyCString::default(),
                        connection_id: remote_connection_id,
                    },
                ))
                .await
                .unwrap();
            loop {
                match transport.read_message().await.unwrap() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(
                    crate::ConnectionReply {
                        ok: true,
                        message: lc_engine::LegacyCString::from_bytes(
                            b"connection accepted".to_vec(),
                        )
                        .unwrap(),
                        wrong_password: false,
                    },
                ))
                .await
                .unwrap();
            (transport, host_request.connection_id)
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();
        let remote_connection_id = 31;
        let (mut secondary, secondary_connection_id) =
            connect_secondary(addr, canonical_id, remote_connection_id).await;

        timeout(EVENT_WAIT, async {
            loop {
                if host.accepted_routes().await.len() == 2 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("secondary route was not accepted");
        while host_events.try_recv().is_ok() {}

        let dead_route_countdown = crate::LobbyCountdownPacket::new(9);
        host.submit_lobby_countdown(dead_route_countdown)
            .await
            .unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match canonical_events.recv().await {
                    Some(ClientEvent::LobbyCountdown { packet })
                        if packet == dead_route_countdown =>
                    {
                        break;
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("canonical route disconnected unexpectedly: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("canonical event stream ended before the test packet"),
                }
            }
        })
        .await
        .expect("dead route did not receive the recoverable test packet");

        canonical.shutdown().await.unwrap();
        let routes = timeout(EVENT_WAIT, async {
            loop {
                let routes = host.accepted_routes().await;
                if routes.len() == 1 {
                    break routes;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("primary route was not removed");
        assert_eq!(
            routes,
            vec![(
                secondary_connection_id,
                canonical_id,
                remote_connection_id,
            )]
        );

        // OnDisconn first removes the dead route (promoting the remaining
        // data route), then sends that dead route's exact packet backlog to the
        // same logical client through its new message route
        // (src/C4Network2.cpp:884-905;
        // src/C4Network2Client.cpp:90-102).
        let recovery = timeout(EVENT_WAIT, async {
            loop {
                match secondary.read_message().await {
                    Ok(ControlMessage::PostMortem(packet)) => break packet,
                    Ok(ControlMessage::Ping(ping)) => {
                        secondary
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    Ok(_) => continue,
                    Err(error) => panic!("surviving route closed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("dead route backlog was not rerouted over the promoted survivor");
        assert_eq!(recovery.connection_id, 0);
        assert!(recovery.packets.iter().any(|packet| {
            matches!(
                crate::transport::parse_complete_packet(packet),
                Ok(Some(ControlMessage::LobbyCountdown(packet))) if packet == dead_route_countdown
            )
        }));

        while let Ok(event) = host_events.try_recv() {
            match event {
                HostEvent::ClientLeft { client_id } if client_id == canonical_id => {
                    panic!("primary disconnect emitted ClientLeft despite a surviving route")
                }
                HostEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| matches!(
                        control,
                        EngineControlPacket::ClientRemove(remove)
                            if remove.client_id == i32::try_from(canonical_id).unwrap()
                    )) => panic!("primary disconnect queued ClientRemove despite a surviving route"),
                _ => {}
            }
        }

        let countdown = crate::LobbyCountdownPacket::new(5);
        host.submit_lobby_countdown(countdown).await.unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match secondary.read_message().await {
                    Ok(ControlMessage::LobbyCountdown(packet)) if packet == countdown => break,
                    Ok(ControlMessage::Ping(ping)) => {
                        secondary
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    Ok(ControlMessage::Packet { data, .. })
                        if matches!(
                            decode_control_entry_payload(&data),
                            Ok(EngineControlPacket::ClientRemove(remove))
                                if remove.client_id == i32::try_from(canonical_id).unwrap()
                        ) => panic!("surviving route received ClientRemove"),
                    Ok(_) => continue,
                    Err(error) => panic!("surviving route closed unexpectedly: {error}"),
                }
            }
        })
        .await
        .expect("host traffic did not use the promoted secondary route");

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_replays_a_dead_routes_post_mortem_suffix_once() {
        // OnDisconn retains the closed connection and its iInPacketCounter.
        // PID_PostMortem received over another route looks up the dead local
        // ConnID, dispatches only the consecutive suffix beginning at that
        // counter under the dead connection's CCore, and removes it afterward
        // (src/C4Network2IO.cpp:520-570,594-597,1036-1055,1351-1356).
        async fn connect_existing_route(
            addr: SocketAddr,
            client_id: ClientId,
            remote_connection_id: u32,
        ) -> (crate::ControlTransport<TcpStream>, u32) {
            let stream = TcpStream::connect(addr).await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_request = match transport.read_message().await.unwrap() {
                ControlMessage::ConnectionRequest(request) => request,
                other => panic!("expected host connection request, got {other:?}"),
            };
            let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
            transport
                .send_message(ControlMessage::ConnectionRequest(
                    crate::ConnectionRequest {
                        core: lc_engine::ClientCoreControlData {
                            client_id: i32::try_from(client_id).unwrap(),
                            activated: true,
                            observer: false,
                            name: name.clone(),
                            nick: name,
                            lobby_ready: true,
                        },
                        build: CURRENT_GAME_BUILD,
                        password: lc_engine::LegacyCString::default(),
                        connection_id: remote_connection_id,
                    },
                ))
                .await
                .unwrap();
            loop {
                match transport.read_message().await.unwrap() {
                    ControlMessage::ConnectionReply(reply) if reply.ok => break,
                    ControlMessage::Ping(ping) => {
                        transport
                            .send_message(ControlMessage::Pong(ping))
                            .await
                            .unwrap();
                    }
                    other => panic!("expected positive host connection reply, got {other:?}"),
                }
            }
            transport
                .send_message(ControlMessage::ConnectionReply(
                    crate::ConnectionReply {
                        ok: true,
                        message: lc_engine::LegacyCString::from_bytes(
                            b"connection accepted".to_vec(),
                        )
                        .unwrap(),
                        wrong_password: false,
                    },
                ))
                .await
                .unwrap();
            (transport, host_request.connection_id)
        }

        async fn encode_nested(message: ControlMessage) -> Vec<u8> {
            let (writer, mut reader) = duplex(256);
            let mut transport = crate::ControlTransport::new(writer);
            transport.send_message(message).await.unwrap();
            let mut header = [0; 5];
            reader.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], 0xff);
            let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut packet = vec![0; length];
            reader.read_exact(&mut packet).await.unwrap();
            packet
        }

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let client_id = canonical.client_id();
        let (mut dead_route, dead_connection_id) =
            connect_existing_route(addr, client_id, 29).await;
        let (mut surviving_route, _surviving_connection_id) =
            connect_existing_route(addr, client_id, 30).await;

        timeout(EVENT_WAIT, async {
            while host.accepted_routes().await.len() != 3 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("additional routes were not accepted");
        while host_events.try_recv().is_ok() {}

        for tick in [100, 101] {
            dead_route
                .send_message(ControlMessage::ActivationRequest { tick })
                .await
                .unwrap();
        }
        let mut received_before_close = Vec::new();
        timeout(EVENT_WAIT, async {
            while received_before_close.len() != 2 {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick,
                        ..
                    }) if source == client_id => received_before_close.push(tick),
                    Some(_) => {}
                    None => panic!("host event stream ended before route close"),
                }
            }
        })
        .await
        .expect("host did not dispatch the pre-close packets");
        assert_eq!(received_before_close, vec![100, 101]);

        drop(dead_route);
        timeout(EVENT_WAIT, async {
            while host.accepted_routes().await.len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dead route was not removed");
        while host_events.try_recv().is_ok() {}

        let recovery = crate::PostMortemPacket {
            connection_id: dead_connection_id,
            packet_counter: 4,
            packets: vec![
                encode_nested(ControlMessage::ActivationRequest { tick: 100 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 101 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 102 }).await,
                encode_nested(ControlMessage::ActivationRequest { tick: 103 }).await,
            ],
        };
        surviving_route
            .send_message(ControlMessage::PostMortem(recovery.clone()))
            .await
            .unwrap();

        let mut recovered = Vec::new();
        timeout(EVENT_WAIT, async {
            while recovered.len() != 2 {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick,
                        ..
                    }) if source == client_id => recovered.push(tick),
                    Some(HostEvent::TransportError { error, .. }) => {
                        panic!("post-mortem recovery failed: {error}")
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended during recovery"),
                }
            }
        })
        .await
        .expect("host did not dispatch the recovered suffix");
        assert_eq!(recovered, vec![102, 103]);

        surviving_route
            .send_message(ControlMessage::PostMortem(recovery))
            .await
            .unwrap();
        surviving_route
            .send_message(ControlMessage::ActivationRequest { tick: 104 })
            .await
            .unwrap();
        timeout(EVENT_WAIT, async {
            loop {
                match host_events.recv().await {
                    Some(HostEvent::ActivationRequest {
                        client_id: source,
                        tick: 104,
                        ..
                    }) if source == client_id => break,
                    Some(HostEvent::ActivationRequest { tick, .. }) => {
                        panic!("retired dead route replayed packet {tick} twice")
                    }
                    Some(HostEvent::TransportError { error, .. }) => {
                        panic!("duplicate recovery was rejected noisily: {error}")
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended after recovery"),
                }
            }
        })
        .await
        .expect("host did not process the duplicate-recovery barrier");

        drop(surviving_route);
        canonical.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn nonresponsive_server_handshake_times_out() {
        // C4Network2IO::CheckTimeout closes connections which do not reach the
        // accepted state after C4NetAcceptTimeout (src/C4Network2IO.cpp:1155-1170).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let (connection, accepted) = tokio::join!(TcpStream::connect(addr), listener.accept());
        let client_stream = connection.expect("connect client socket");
        let (_server_stream, _) = accepted.expect("accept client socket");

        let result = timeout(
            HANDSHAKE_TIMEOUT + Duration::from_secs(1),
            connect_client_from_inner(
                ready(Ok(client_stream)),
                ClientConfig::new("Alice", ParticipantKind::Player),
                Some(ConnectionLivenessState::new_test(0, 0)),
            ),
        )
        .await;

        match result {
            Ok(Err(ClientError::Handshake(message))) => {
                assert_eq!(message, "connection admission timed out");
            }
            other => panic!("expected bounded handshake timeout, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_emits_one_decodable_ready_packet_for_host_and_client() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(
            listener,
            HostConfig {
                max_players: 4,
                ..Default::default()
            },
        )
        .await
        .expect("start host");

        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut events = host.take_event_receiver();
        activate_joined_client(&host, &mut events, client.client_id()).await;

        client
            .submit_control(legacy_packet(1, 0, 0x12))
            .await
            .expect("submit client control");
        host.submit_local_control(legacy_packet(0, 0, 0x34))
            .await
            .expect("submit host control");

        let packet = wait_for_host_ready(&mut events, EVENT_WAIT).await;
        assert_eq!(packet.tick(), 0);
        assert_eq!(packet.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn decentralized_host_and_two_clients_pack_the_same_ordered_tick() {
        // Every participant stores its own input, receives each other active
        // client's contribution through direct/forwarded broadcast, and runs
        // PackCompleteCtrl in client-ID order (pristine C++
        // src/C4GameControlNetwork.cpp:156-179,741-783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut alpha = connect_client(
            address,
            ClientConfig::new("Alpha", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        activate_joined_client(&host, &mut host_events, alpha.client_id()).await;
        let mut beta = connect_client(
            address,
            ClientConfig::new("Beta", ParticipantKind::Player),
        )
        .await
        .unwrap();
        let mut beta_events = beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, beta.client_id()).await;

        host.submit_local_control(legacy_packet(0, 0, 0x10))
            .await
            .unwrap();
        alpha
            .submit_control(legacy_packet(alpha.client_id(), 0, 0x20))
            .await
            .unwrap();
        beta.submit_control(legacy_packet(beta.client_id(), 0, 0x30))
            .await
            .unwrap();

        let host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        let alpha_ready = wait_for_client_ready(&mut alpha_events, EVENT_WAIT).await;
        let beta_ready = wait_for_client_ready(&mut beta_events, EVENT_WAIT).await;
        assert_eq!(host_ready, alpha_ready);
        assert_eq!(host_ready, beta_ready);
        assert_eq!(control_commands(&host_ready), vec![0x10, 0x20, 0x30]);
        for events in [&mut alpha_events, &mut beta_events] {
            while let Ok(Some(event)) = timeout(Duration::from_millis(50), events.recv()).await {
                assert!(
                    !matches!(event, ClientEvent::Ready { .. }),
                    "one decentralized contribution emitted more than one complete tick"
                );
            }
        }

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn join_data_rebinds_local_client_and_host_emits_direct_join_first() {
        // Host Join inserts the canonical client before ConnRe/JoinData; the
        // client then rebinds its unknown local object to the assigned ID
        // (src/C4Network2.cpp:1395-1445,1574-1604;
        // src/C4Client.cpp:284-290,321-350).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let client_id = client.client_id();
        let join_data = client.take_join_data().expect("bootstrap is retained once");

        assert_eq!(join_data.client_id, i32::try_from(client_id).unwrap());
        assert_eq!(
            join_data
                .parameters
                .clients
                .clients
                .iter()
                .map(|core| core.client_id)
                .collect::<Vec<_>>(),
            vec![0, i32::try_from(client_id).unwrap()]
        );
        assert_eq!(
            join_data.parameters.clients.local_client_id,
            Some(i32::try_from(client_id).unwrap())
        );
        assert!(client.take_join_data().is_none());

        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await.unwrap(),
            Some(HostEvent::Direct {
                delivery: ControlDelivery::Direct,
                ..
            })
        ));
        assert!(matches!(
            timeout(EVENT_WAIT, host_events.recv()).await.unwrap(),
            Some(HostEvent::ClientJoined {
                client_id: joined,
                ..
            }) if joined == client_id
        ));

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_sends_cpp_address_packets_immediately_after_join_data() {
        // SendJoinData writes PID_JoinData and then every known PID_Addr on the
        // accepted message connection before resource discovery begins
        // (src/C4Network2.cpp:1810-1850;
        // src/C4Network2Client.cpp:319-337,616-621).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(
            listener,
            HostConfig {
                resource_registrations: vec![
                    crate::ResourceRegistration {
                        resource_id: 3,
                        chunk_count: 1,
                        binary_compatible: true,
                        loading: false,
                    },
                    crate::ResourceRegistration {
                        resource_id: 4,
                        chunk_count: 2,
                        binary_compatible: true,
                        loading: false,
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let stream = TcpStream::connect(addr).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        let request = crate::ConnectionRequest {
            core: lc_engine::ClientCoreControlData {
                client_id: -1,
                name: name.clone(),
                nick: name,
                ..Default::default()
            },
            build: CURRENT_GAME_BUILD,
            password: lc_engine::LegacyCString::default(),
            connection_id: 0,
        };

        let bootstrap = run_client_connection_handshake(&mut transport, request)
            .await
            .expect("binary admission and JoinData");
        assert_eq!(bootstrap.join_data.client_id, 1);

        let packet = timeout(EVENT_WAIT, transport.read_message())
            .await
            .expect("host must follow JoinData with PID_Addr")
            .unwrap();
        match packet {
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address:
                    crate::NetworkAddress {
                        protocol: crate::NetworkProtocol::Tcp,
                        endpoint,
                    },
            }) => assert_eq!(
                endpoint,
                format!("0.0.0.0:{}", addr.port()).parse().unwrap()
            ),
            other => panic!("expected host PID_Addr after JoinData, got {other:?}"),
        }
        loop {
            match timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("resource discovery follows JoinData addresses")
                .unwrap()
            {
                ControlMessage::Address(crate::AddressPacket { client_id: 0, .. }) => continue,
                ControlMessage::Resource(ResourcePacket::Discover(discover)) => {
                    assert_eq!(discover.resource_ids, vec![4, 3]);
                    break;
                }
                other => panic!("expected PID_Addr* then PID_NetResDis, got {other:?}"),
            }
        }

        let client_address = crate::AddressPacket {
            client_id: 1,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "0.0.0.0:11112".parse().unwrap(),
            ),
        };
        transport
            .send_message(ControlMessage::Address(client_address))
            .await
            .unwrap();
        let mut saw_reannouncement = false;
        for _ in 0..8 {
            let message = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("host address propagation stalled")
                .unwrap();
            if message == ControlMessage::Address(client_address) {
                saw_reannouncement = true;
                break;
            }
        }
        assert!(
            saw_reannouncement,
            "host did not re-announce the newly learned client address"
        );

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_requests_the_join_tick_before_announcing_addresses() {
        // HandleJoinData initializes C4GameControlNetwork, whose Init sends
        // PID_ControlReq(start tick), before SendAddresses emits PID_Addr
        // (src/C4Network2.cpp:1603-1623;
        // src/C4GameControlNetwork.cpp:46-62).
        let (client_stream, host_stream) = duplex(512);
        let mut client = crate::ControlTransport::new(client_stream);
        let mut host = crate::ControlTransport::new(host_stream);
        let host_address = crate::NetworkAddress::new(
            crate::NetworkProtocol::Tcp,
            "192.0.2.4:11112".parse().unwrap(),
        );

        send_client_post_join_packets(
            &mut client,
            17,
            vec![crate::AddressPacket {
                client_id: 0,
                address: host_address,
            }],
        )
        .await
        .unwrap();

        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Request { from_tick: 17 }
        );
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address: host_address,
            })
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_control_request_precedes_dynamic_failure_even_after_bad_game_resource() {
        // HandleJoinData initializes network control first, ignores the first
        // GameRes.InitNetwork failure, and only then treats Dynamic failure as
        // fatal (src/C4Network2.cpp:1603-1618).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.game_resources.push(nonloadable_core(
            crate::HostResourceType::System as u8,
            9,
            b"System.c4g",
        ));
        snapshot.dynamic = nonloadable_core(2, 7, b"Dynamic.c4d");
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let messages = server.await.unwrap();

        let error = result.expect_err("missing non-loadable Dynamic must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Dynamic.c4d") && message.contains("non-loadable")),
            "the ignored early GameRes failure masked Dynamic: {error:?}"
        );
        assert_eq!(
            messages,
            vec![ControlMessage::Request { from_tick: 0 }],
            "control initialization must precede Dynamic retrieval, but addresses must not"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_addresses_before_final_scenario_validation_failure() {
        // HandleJoinData sends known addresses before outer InitClient calls
        // Parameters.InitNetwork, whose first required resource is Scenario
        // (src/C4Network2.cpp:1620-1622,329-331;
        // src/C4GameParameters.cpp:539-547).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.scenario = nonloadable_core(1, 8, b"Scenario.c4s");
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let messages = server.await.unwrap();

        let error = result.expect_err("missing non-loadable Scenario must abort bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Scenario.c4s") && message.contains("non-loadable")),
            "unexpected Scenario bootstrap failure: {error:?}"
        );
        assert_eq!(
            messages.first(),
            Some(&ControlMessage::Request { from_tick: 0 })
        );
        assert!(
            matches!(messages.get(1), Some(ControlMessage::Address(packet)) if
            packet.client_id == 0 && packet.address.endpoint == address)
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_rechecks_failed_game_resource_after_announcing_addresses() {
        // The early GameRes result is ignored. After addresses, the outer
        // Parameters.InitNetwork retries GameRes after Scenario and makes the
        // same missing non-loadable core fatal
        // (src/C4Network2.cpp:1612-1622,329-331;
        // src/C4GameParameters.cpp:237-247,539-547).
        let host = HostConfig::default();
        let mut snapshot = synthetic_join_snapshot(host.local_core, 8);
        snapshot.parameters.game_resources.push(nonloadable_core(
            crate::HostResourceType::Definitions as u8,
            9,
            b"Objects.c4d",
        ));
        let (address, server) = start_client_bootstrap_probe(snapshot).await;

        let result =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player)).await;
        let messages = server.await.unwrap();

        let error = result.expect_err("final GameRes retry must fail bootstrap");
        assert!(
            matches!(&error, ClientError::Handshake(message) if
                message.contains("Objects.c4d") && message.contains("non-loadable")),
            "unexpected GameRes bootstrap failure: {error:?}"
        );
        assert_eq!(
            messages.first(),
            Some(&ControlMessage::Request { from_tick: 0 })
        );
        assert!(
            matches!(messages.get(1), Some(ControlMessage::Address(packet)) if
            packet.client_id == 0 && packet.address.endpoint == address)
        );
    }

    fn nonloadable_core(
        resource_type: u8,
        id: i32,
        filename: &[u8],
    ) -> lc_engine::NetworkResourceCore {
        lc_engine::NetworkResourceCore {
            resource_type,
            id,
            derived_id: -1,
            loadable: false,
            file_size: u32::MAX,
            file_crc: u32::MAX,
            contents_crc: 1,
            filename: lc_engine::LegacyCString::from_bytes(filename.to_vec()).unwrap(),
            ..Default::default()
        }
    }

    async fn start_client_bootstrap_probe(
        mut snapshot: HostJoinSnapshot,
    ) -> (SocketAddr, tokio::task::JoinHandle<Vec<ControlMessage>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = lc_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = lc_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: lc_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: lc_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();

            let mut messages = Vec::new();
            while messages.len() < 4 {
                match timeout(Duration::from_millis(250), transport.read_message()).await {
                    Ok(Ok(message)) => messages.push(message),
                    Ok(Err(_)) | Err(_) => break,
                }
            }
            messages
        });
        (address, server)
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_client_continues_the_cpp_ping_timer_after_bootstrap() {
        // C4Network2IO's 500 ms timer and strict one-second ping gate continue
        // on the accepted connection after JoinData
        // (src/C4Network2IO.cpp:605-617,1141-1151).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, _event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match host.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(ping.packet_counter, 0);
        host.send_message(ControlMessage::Pong(ping)).await.unwrap();

        shutdown_tx.send(()).unwrap();
        drop(command_tx);
        task.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_host_connection_continues_the_cpp_ping_timer() {
        // The host's accepted connection remains on the same C4Network2IO
        // timer after mutual admission (src/C4Network2IO.cpp:605-617,
        // 1141-1177).
        let (host_stream, client_stream) = duplex(512);
        let mut client = crate::ControlTransport::new(client_stream);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel(4);
        let (host_tx, mut host_rx) = mpsc::channel(4);
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 1,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );

        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(1_500)).await;
        let ping = match client.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected host accepted-session PID_Ping, got {other:?}"),
        };
        client
            .send_message(ControlMessage::Pong(ping))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        assert!(host_rx.try_recv().is_err());

        drop(client);
        drop(outbound_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn accepted_host_reports_tcp_sim_open_and_keeps_the_connection() {
        let (host_stream, mut client_stream) = duplex(512);
        let (outbound_tx, outbound_rx) = mpsc::channel(4);
        let (host_tx, mut host_rx) = mpsc::channel(4);
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 7,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );

        // Packed client 7 plus a TCP IPv6 endpoint, matching the native
        // C4PacketTCPSimOpen binary layout.
        let tcp_sim_open = [
            0x14, 0x07, 0x01, b'[', b'2', b'0', b'0', b'1', b':', b'd', b'b', b'8', b':', b':',
            b'7', b']', b':', b'1', b'1', b'1', b'1', b'2', 0x00,
        ];
        client_stream
            .write_all(&tcp_frame(&tcp_sim_open))
            .await
            .unwrap();

        assert!(matches!(
            timeout(EVENT_WAIT, host_rx.recv()).await.unwrap(),
            Some(HostLoopMessage::UnhandledPacket {
                client_id: Some(7),
                packet_type: 0x14,
            })
        ));

        let mut client = crate::ControlTransport::new(client_stream);
        let ping = crate::PingPacket {
            sent_at: 17,
            packet_counter: 0,
        };
        client
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            client.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the ignored packet must not terminate the accepted connection"
        );

        drop(client);
        drop(outbound_tx);
        task.await.unwrap();
        assert!(matches!(
            host_rx.recv().await,
            Some(HostLoopMessage::ClientDisconnected {
                connection_id: 3,
                client_id: 7,
                next_inbound_packet: 1,
                ..
            })
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn accepted_client_reports_league_results_and_keeps_the_connection() {
        let (client_stream, mut host_stream) = duplex(512);
        let (command_tx, command_rx) = mpsc::channel(4);
        let (event_tx, mut event_rx) = mpsc::channel(4);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let task = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        // Success, "OK", and zero result players, matching the native
        // C4PacketLeagueRoundResults binary layout.
        let league_results = [0x17, 0x01, b'O', b'K', 0x00, 0x00];
        host_stream
            .write_all(&tcp_frame(&league_results))
            .await
            .unwrap();

        assert!(matches!(
            timeout(Duration::from_millis(100), event_rx.recv())
                .await
                .unwrap(),
            Some(ClientEvent::UnhandledPacket { packet_type: 0x17 })
        ));

        let mut host = crate::ControlTransport::new(host_stream);
        let ping = crate::PingPacket {
            sent_at: 23,
            packet_counter: 0,
        };
        host.send_message(ControlMessage::Ping(ping)).await.unwrap();
        assert_eq!(
            host.read_message().await.unwrap(),
            ControlMessage::Pong(ping),
            "the ignored packet must not terminate the accepted connection"
        );

        tokio::time::advance(Duration::from_millis(1_500)).await;
        let liveness_ping = match host.read_message().await.unwrap() {
            ControlMessage::Ping(ping) => ping,
            other => panic!("expected accepted-session PID_Ping, got {other:?}"),
        };
        assert_eq!(
            liveness_ping.packet_counter, 1,
            "the unhandled PID must advance the recoverable inbound counter"
        );
        host.send_message(ControlMessage::Pong(liveness_ping))
            .await
            .unwrap();

        shutdown_tx.send(()).unwrap();
        drop(command_tx);
        task.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn client_announces_known_host_address_after_applying_join_data() {
        // HandleJoinData finishes by sending every address already known by
        // the client list. At this point the outgoing host ConnectAddr is
        // known, so it is re-announced as a host-owned PID_Addr
        // (src/C4Network2.cpp:1448-1499,1574-1623;
        // src/C4Network2Client.cpp:319-337,616-621).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut transport = crate::ControlTransport::new(stream);
            let host_name = lc_engine::LegacyCString::from_bytes(b"Host".to_vec()).unwrap();
            let host_core = lc_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: host_name.clone(),
                nick: host_name,
                ..Default::default()
            };
            let request = crate::ConnectionRequest {
                core: host_core.clone(),
                build: CURRENT_GAME_BUILD,
                password: lc_engine::LegacyCString::default(),
                connection_id: 9,
            };
            let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(1);
            let admission = tokio::spawn(async move {
                let request = admission_rx.recv().await.unwrap();
                let mut assigned = request.request.core.clone();
                assigned.client_id = 1;
                request
                    .decision_tx
                    .send(AdmissionDecision::Accept {
                        peer_core: assigned.clone(),
                        before_reply: Vec::new(),
                        message: lc_engine::LegacyCString::from_bytes(b"join accepted".to_vec())
                            .unwrap(),
                    })
                    .unwrap();
                assigned
            });
            run_host_connection_handshake(&mut transport, request, &admission_tx)
                .await
                .unwrap();
            let assigned = admission.await.unwrap();
            let mut snapshot = synthetic_join_snapshot(host_core.clone(), 8);
            snapshot.parameters.clients =
                JoinClientRegistrySnapshot::new(vec![host_core, assigned.clone()]);
            transport
                .send_message(ControlMessage::JoinData(Box::new(JoinDataEnvelope {
                    client_id: assigned.client_id,
                    start_control_tick: snapshot.dynamic_tick,
                    status: NetworkStatus {
                        state: NETWORK_STATE_LOBBY,
                        control_mode: 0,
                        target_tick: -1,
                    },
                    dynamic: snapshot.dynamic,
                    parameters: snapshot.parameters,
                })))
                .await
                .unwrap();

            let control_request = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("client must request its JoinData control tick")
                .unwrap();
            let initial = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("client must announce addresses after JoinData")
                .unwrap();
            let learned = crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    "198.51.100.7:11112".parse().unwrap(),
                ),
            };
            transport
                .send_message(ControlMessage::Address(learned))
                .await
                .unwrap();
            let mut echoed = None;
            for _ in 0..8 {
                let message = timeout(EVENT_WAIT, transport.read_message())
                    .await
                    .expect("client must re-announce a newly learned address")
                    .unwrap();
                if message == ControlMessage::Address(learned) {
                    echoed = Some(message);
                    break;
                }
            }
            let echoed = echoed.expect("client never re-announced the newly learned address");
            (control_request, initial, learned, echoed)
        });

        let client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .unwrap();
        let (control_request, packet, learned, echoed) = server.await.unwrap();
        assert_eq!(control_request, ControlMessage::Request { from_tick: 0 });
        assert_eq!(
            packet,
            ControlMessage::Address(crate::AddressPacket {
                client_id: 0,
                address: crate::NetworkAddress::new(crate::NetworkProtocol::Tcp, addr),
            })
        );
        assert_eq!(echoed, ControlMessage::Address(learned));

        client.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn accepted_client_waits_for_a_fresh_published_join_snapshot() {
        // SendJoinData retains an accepted NCS_Joining client when no current
        // dynamic exists. OnGameSynchronized later publishes the fresh
        // dynamic and sends JoinData/Addr without re-running admission
        // (src/C4Network2.cpp:1099-1115,1768-1784,1820-1849).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut config = HostConfig::default();
        let snapshot = synthetic_join_snapshot(config.local_core.clone(), config.max_players);
        config.initial_join_snapshot = None;
        let mut host = start_host(listener, config).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client_task = tokio::spawn(connect_client(
            addr,
            ClientConfig::new("Alice", ParticipantKind::Player),
        ));

        let mut needed = false;
        for _ in 0..4 {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::JoinDataNeeded {
                    client_id: 1,
                    current_control_tick: 0,
                }) => {
                    needed = true;
                    break;
                }
                Some(HostEvent::Direct { .. }) | Some(HostEvent::ClientJoined { .. }) => {}
                other => panic!("unexpected event while waiting for JoinData: {other:?}"),
            }
        }
        assert!(
            needed,
            "host did not retain the joining client for a dynamic"
        );
        assert!(timeout(Duration::from_millis(50), &mut client_task)
            .await
            .is_err());

        host.publish_join_snapshot(snapshot.clone()).await.unwrap();
        let mut client = timeout(EVENT_WAIT, client_task)
            .await
            .expect("published JoinData did not release the client")
            .unwrap()
            .unwrap();
        let join_data = client.take_join_data().unwrap();
        assert_eq!(join_data.dynamic, snapshot.dynamic);
        assert_eq!(join_data.start_control_tick, snapshot.dynamic_tick);

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn direct_client_join_reaches_already_connected_clients_before_new_join_finishes() {
        // CtrlAdd executes CID_ClientJoin as direct control before the host
        // sends positive ConnRe, so every existing client learns the newcomer
        // before normal synchronized traffic continues
        // (src/C4Network2.cpp:1395-1445; src/C4Control.cpp:554-573).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        let beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();

        let data = loop {
            match timeout(EVENT_WAIT, alpha_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    data,
                }) => break data,
                Some(ClientEvent::Ready { .. }) => continue,
                other => panic!("expected direct ClientJoin for Beta, got {other:?}"),
            }
        };
        let lc_engine::ControlPacket::ClientJoin(join) =
            decode_control_entry_payload(&data).unwrap()
        else {
            panic!("direct packet was not ClientJoin");
        };
        assert_eq!(
            join.core.client_id,
            i32::try_from(beta.client_id()).unwrap()
        );
        assert_eq!(join.core.name.as_bytes(), b"Beta");

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connecting_without_pending_sync_emits_no_exec_sync_marker() {
        // PID_ExecSyncCtrl is emitted only when SyncControl is non-empty;
        // connection establishment is not a synchronization release
        // (src/C4GameControlNetwork.cpp:260-276).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut events = client.take_event_receiver();

        assert!(timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err());

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn status_and_ack_round_trip_over_real_tcp() {
        // PID_Status is host-authored; a client answers with PID_StatusAck and
        // the host later broadcasts the final ACK
        // (src/C4Network2.cpp:1501-1534,1994-2012,2062-2077).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let client_id = client.client_id();
        let mut client_events = client.take_event_receiver();
        let status = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 195_995,
        };

        host.change_status(status).await.expect("broadcast status");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client status wait")
            {
                Some(ClientEvent::Status(received)) => {
                    assert_eq!(received, status);
                    break;
                }
                Some(ClientEvent::Ready { .. }) | Some(ClientEvent::Direct { .. }) => continue,
                other => panic!("expected client status event, got {other:?}"),
            }
        }

        client
            .submit_status_ack(status)
            .await
            .expect("submit status ack");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host status ack wait")
            {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status: received,
                }) => {
                    assert_eq!((received_id, received), (client_id, status));
                    break;
                }
                Some(HostEvent::ClientJoined { .. }) | Some(HostEvent::Direct { .. }) => continue,
                other => panic!("expected host status ack event, got {other:?}"),
            }
        }

        assert!(timeout(Duration::from_millis(50), client_events.recv())
            .await
            .is_err());
        host.status_reached()
            .await
            .expect("host reached status target");
        match timeout(EVENT_WAIT, client_events.recv())
            .await
            .expect("client final status ack wait")
        {
            Some(ClientEvent::StatusAck(received)) => assert_eq!(received, status),
            other => panic!("expected client final status ack, got {other:?}"),
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host status commit wait")
            {
                Some(HostEvent::StatusCommitted(committed)) => {
                    assert_eq!(committed, status);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before status commit"),
            }
        }

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn higher_client_status_ack_retargets_real_tcp_barrier_before_commit() {
        // CheckStatusReached replaces a client's requested target with its
        // current control tick. HandleStatusAck must rebroadcast that higher
        // target before the barrier can commit
        // (src/C4Network2.cpp:1994-2012,2062-2077).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let client_id = client.client_id();
        let initial_status = client.take_join_data().expect("client JoinData").status;
        let mut client_events = client.take_event_receiver();

        // Send the JoinData status acknowledgement first so the host advances
        // this client from Chasing to Ready before opening a fresh barrier.
        client
            .submit_status_ack(initial_status)
            .await
            .expect("acknowledge JoinData status");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host initial status ack wait")
            {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == initial_status => break,
                Some(_) => continue,
                None => panic!("host event stream ended before initial status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client initial status ack wait")
            {
                Some(ClientEvent::StatusAck(status)) if status == initial_status => break,
                Some(_) => continue,
                None => panic!("client event stream ended before initial status ack"),
            }
        }

        let requested = NetworkStatus {
            state: NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 41,
        };
        host.change_status(requested)
            .await
            .expect("broadcast requested Pause");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client requested Pause wait")
            {
                Some(ClientEvent::Status(status)) if status == requested => break,
                Some(_) => continue,
                None => panic!("client event stream ended before requested Pause"),
            }
        }

        let retargeted = NetworkStatus {
            target_tick: 44,
            ..requested
        };
        client
            .submit_status_ack(retargeted)
            .await
            .expect("submit retargeted Pause acknowledgement");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host retargeted status ack wait")
            {
                Some(HostEvent::StatusAck {
                    client_id: received_id,
                    status,
                }) if received_id == client_id && status == retargeted => break,
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status ack"),
            }
        }

        match timeout(EVENT_WAIT, client_events.recv())
            .await
            .expect("client retargeted Pause wait")
        {
            Some(ClientEvent::Status(status)) => assert_eq!(status, retargeted),
            other => panic!("expected retargeted Pause before final ack, got {other:?}"),
        }
        assert!(
            timeout(Duration::from_millis(50), async {
                loop {
                    match client_events.recv().await {
                        Some(ClientEvent::StatusAck(status)) => break Some(status),
                        Some(_) => continue,
                        None => break None,
                    }
                }
            })
            .await
            .is_err(),
            "retargeted barrier committed before the host reached tick 44"
        );

        host.status_reached()
            .await
            .expect("host reached retargeted Pause");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client final retargeted status ack wait")
            {
                Some(ClientEvent::StatusAck(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before final retargeted status ack"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host retargeted status commit wait")
            {
                Some(HostEvent::StatusCommitted(status)) => {
                    assert_eq!(status, retargeted);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before retargeted status commit"),
            }
        }

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_controls_wait_for_status_barrier_and_keep_fifo_order() {
        // In running games, CDT_Sync packets accumulate in SyncControl and do
        // not execute until PID_ExecSyncCtrl is emitted after the status
        // barrier (src/C4GameControlNetwork.cpp:181-220,260-297,558-588).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut client_events = client.take_event_receiver();

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running)
            .await
            .expect("enter running status");
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("initial Go status wait")
            {
                Some(ClientEvent::Status(status)) => {
                    assert_eq!(status, running);
                    break;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before initial Go"),
            }
        }
        client
            .submit_status_ack(running)
            .await
            .expect("acknowledge initial Go");
        host.status_reached()
            .await
            .expect("host reached initial Go");
        let mut host_running = false;
        let mut client_running = false;
        while !host_running || !client_running {
            if !host_running {
                match timeout(EVENT_WAIT, host_events.recv())
                    .await
                    .expect("host initial Go commit wait")
                {
                    Some(HostEvent::StatusCommitted(status)) => {
                        assert_eq!(status, running);
                        host_running = true;
                    }
                    Some(_) => {}
                    None => panic!("host event stream ended before initial Go commit"),
                }
            }
            if !client_running {
                match timeout(EVENT_WAIT, client_events.recv())
                    .await
                    .expect("client initial Go ack wait")
                {
                    Some(ClientEvent::StatusAck(status)) => {
                        assert_eq!(status, running);
                        client_running = true;
                    }
                    Some(_) => {}
                    None => panic!("client event stream ended before initial Go ack"),
                }
            }
        }

        let first = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x41,
            data: 0,
            by_client: 0,
        });
        let second = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x42,
            data: 0,
            by_client: 0,
        });
        for control in [&first, &second] {
            host.submit_packet(
                ControlDelivery::Sync,
                encode_control_entry_payload(control).expect("encode sync control"),
            )
            .await
            .expect("submit sync control");
        }

        let sync_status = loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client synchronization status wait")
            {
                Some(ClientEvent::Status(status)) => break status,
                Some(ClientEvent::SyncScheduled { .. }) => {
                    panic!("client released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before synchronization status"),
            }
        };
        assert_eq!(sync_status.state, NETWORK_STATE_GO);

        // A complete ordinary lockstep tick is not the C++ status barrier.
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x11))
            .await
            .expect("submit client tick");
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x22))
            .await
            .expect("submit host tick");
        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host ready wait")
            {
                Some(HostEvent::Ready { .. }) => break,
                Some(HostEvent::SyncScheduled { .. }) => {
                    panic!("host released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client ready wait")
            {
                Some(ClientEvent::Ready { .. }) => break,
                Some(ClientEvent::SyncScheduled { .. }) => {
                    panic!("client released Sync before the status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before ready"),
            }
        }

        client
            .submit_status_ack(sync_status)
            .await
            .expect("acknowledge synchronization status");
        host.status_reached()
            .await
            .expect("host reached synchronization target");
        let mut host_controls = None;
        let mut host_committed = false;
        while host_controls.is_none() || !host_committed {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host sync release wait")
            {
                Some(HostEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(
                        i32::try_from(control_tick).ok(),
                        Some(sync_status.target_tick)
                    );
                    host_controls = Some(controls);
                }
                Some(HostEvent::StatusCommitted(status)) => {
                    assert_eq!(status, sync_status);
                    host_committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before sync release"),
            }
        }
        let mut client_controls = None;
        let mut client_committed = false;
        while client_controls.is_none() || !client_committed {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client sync release wait")
            {
                Some(ClientEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(
                        i32::try_from(control_tick).ok(),
                        Some(sync_status.target_tick)
                    );
                    client_controls = Some(controls);
                }
                Some(ClientEvent::StatusAck(status)) => {
                    assert_eq!(status, sync_status);
                    client_committed = true;
                }
                Some(_) => continue,
                None => panic!("client event stream ended before sync release"),
            }
        }
        assert_eq!(host_controls, Some(vec![first.clone(), second.clone()]));
        assert_eq!(client_controls, Some(vec![first, second]));

        host.submit_exec_sync(2)
            .await
            .expect("empty sync release is accepted");
        assert!(timeout(Duration::from_millis(50), host_events.recv())
            .await
            .is_err());
        assert!(timeout(Duration::from_millis(50), client_events.recv())
            .await
            .is_err());

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sync_control_executes_immediately_in_frozen_lobby() {
        // Lobby is frozen without a status round trip, so the host executes a
        // CDT_Sync control immediately and then emits PID_ExecSyncCtrl
        // (src/C4Network2.cpp:1982-1991;
        // src/C4GameControlNetwork.cpp:204-213).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");
        let mut client_events = client.take_event_receiver();
        let control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 0x51,
            data: 0,
            by_client: 0,
        });

        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&control).expect("encode lobby sync control"),
        )
        .await
        .expect("submit lobby sync control");

        loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host frozen sync wait")
            {
                Some(HostEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(control_tick, 0);
                    assert_eq!(controls, vec![control.clone()]);
                    break;
                }
                Some(HostEvent::StatusAck { .. }) | Some(HostEvent::StatusCommitted(_)) => {
                    panic!("frozen lobby Sync must not open a status barrier")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before frozen sync"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv())
                .await
                .expect("client frozen sync wait")
            {
                Some(ClientEvent::SyncScheduled {
                    control_tick,
                    controls,
                }) => {
                    assert_eq!(control_tick, 0);
                    assert_eq!(controls, vec![control]);
                    break;
                }
                Some(ClientEvent::Status(_)) | Some(ClientEvent::StatusAck(_)) => {
                    panic!("frozen lobby Sync must not open a status barrier")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before frozen sync"),
            }
        }

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_matches_cpp_pid_control_source_id_semantics() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let (mut client, client_id) = raw_client_transport(addr, b"spoof-check").await;
        activate_joined_client(&host, &mut host_events, client_id).await;
        drain_raw_client(&mut client).await;
        let spoofed = legacy_packet(HOST_CLIENT_ID, 0, 0x66);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&spoofed)
                    .unwrap(),
            }))
            .await
            .expect("submit spoofed host control");
        raw_client_ping_barrier(&mut client).await;
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit real host control");
        let contribution = legacy_packet(client_id, 0, 0x22);
        client
            .send_message(ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet: crate::transport::encode_complete_control_packet(&contribution)
                    .unwrap(),
            }))
            .await
            .expect("submit real client control");

        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("C++ accepts PID_Control independently of its source connection: {error}")
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert_eq!(
            control_commands(&ready),
            vec![0x66, 0x22],
            "HandleControl ignores iByClientID and retains the first contribution for a slot"
        );

        drop(client);
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forged_queued_control_set_author_does_not_consume_the_tick() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let client = connect_client(
            addr,
            ClientConfig::new("set-spoof-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;
        let client_author = i32::try_from(client_id).expect("test client id fits i32");
        let queued_set = |by_client| {
            encode_control_packet(&LegacyControlFrame {
                client_id,
                tick: 0,
                timestamp_ms: 0,
                controls: vec![crate::LegacyControlSet {
                    value_type: 5,
                    data: 10_000,
                    by_client,
                }
                .into_control_packet()],
            })
            .expect("encode queued CID_Set")
        };

        client
            .submit_control(queued_set(0))
            .await
            .expect("submit forged host-authored Set");
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit host contribution");
        client
            .submit_control(queued_set(client_author))
            .await
            .expect("replace with authenticated Set");

        let mut saw_rejection = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
                Some(HostEvent::TransportError {
                    client_id: Some(rejected_id),
                    error,
                }) if error.contains("queued CID_Set claimed author") => {
                    assert_eq!(rejected_id, client_id);
                    assert!(error.contains("claimed author 0"));
                    saw_rejection = true;
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert!(
            saw_rejection,
            "forged CID_Set contribution was not rejected"
        );
        let frame = decode_control_packet(&ready).expect("ready packet remains decodable");
        let sets = frame
            .controls
            .iter()
            .filter_map(crate::LegacyControlSet::from_control_packet)
            .collect::<Vec<_>>();
        assert_eq!(
            sets,
            vec![crate::LegacyControlSet {
                value_type: 5,
                data: 10_000,
                by_client: client_author,
            }]
        );

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_contribution_does_not_consume_the_synchronized_tick() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let client = connect_client(
            addr,
            ClientConfig::new("validation-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x22))
            .await
            .expect("submit valid client control");
        let valid_host = legacy_packet(HOST_CLIENT_ID, 0, 0x11);
        let mut malformed_payload = valid_host.payload().to_vec();
        *malformed_payload.last_mut().expect("control terminator") = 0x7f;
        let malformed_host = ControlPacket::builder(HOST_CLIENT_ID, 0).payload(malformed_payload);
        host.submit_local_control(malformed_host)
            .await
            .expect("submit malformed host control");
        host.submit_local_control(valid_host)
            .await
            .expect("replace malformed host control");

        let mut saw_validation_error = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
                Some(HostEvent::TransportError { error, .. }) => {
                    assert!(error.contains("PID_NONE"));
                    saw_validation_error = true;
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert!(saw_validation_error, "malformed input was not diagnosed");
        assert_eq!(control_commands(&ready), vec![0x11, 0x22]);

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn control_sync_and_reconnect_smoke() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let config = HostConfig {
            max_players: 4,
            ..Default::default()
        };
        let mut host = start_host(listener, config.clone())
            .await
            .expect("start host");

        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        let mut client_events = client.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;

        submit_control_pair(&mut host, &client, 0, 0xAA, 0x11).await;

        let first_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(first_host_ready.tick(), 0);

        let first_client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(first_client_ready.tick(), 0);

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.unwrap();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .expect("publish runtime-join dynamic");

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect second client");
        let mut client_beta_events = client_beta.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client_beta.client_id()).await;

        submit_control_pair(&mut host, &client_beta, 1, 0xBB, 0x22).await;

        let second_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(second_host_ready.tick(), 1);

        let second_client_ready = wait_for_client_ready(&mut client_beta_events, EVENT_WAIT).await;
        assert_eq!(second_client_ready.tick(), 1);

        client_beta
            .shutdown()
            .await
            .expect("second client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_continues_ready_after_client_disconnect() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(
            listener,
            HostConfig {
                max_players: 4,
                ..Default::default()
            },
        )
        .await
        .expect("start host");

        let client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        activate_joined_client(&host, &mut host_events, client.client_id()).await;
        submit_control_pair(&mut host, &client, 0, 0xA0, 0xB0).await;
        let ready0 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready0.tick(), 0);
        assert_eq!(control_commands(&ready0), vec![0xA0, 0xB0]);

        let host_packet = legacy_packet(0, 1, 0xC0);
        host.submit_local_control(host_packet)
            .await
            .expect("host submit control");

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        let ready1 = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready1.tick(), 1);
        assert_eq!(control_commands(&ready1), vec![0xC0]);

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn running_disconnect_keeps_client_in_control_membership_until_sync_executes() {
        // OnClientDisconnect removes the peer from the status wait set, but
        // CtrlRemove changes C4GameControlNetwork's active-client copy only
        // when the host-authored CDT_Sync ClientRemove executes. Until then,
        // PackCompleteCtrl still includes that client's already-received
        // contribution (src/C4Network2.cpp:1786-1807;
        // src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220,260-297,329-345,741-783).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut client_events = client.take_event_receiver();
        let client_id = client.client_id();
        activate_joined_client(&host, &mut host_events, client_id).await;

        let running = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 0,
        };
        host.change_status(running).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::Status(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("client event stream ended before Go"),
            }
        }
        client.submit_status_ack(running).await.unwrap();
        host.status_reached().await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::StatusCommitted(status)) if status == running => break,
                Some(_) => continue,
                None => panic!("host event stream ended before Go committed"),
            }
        }

        client
            .submit_control(legacy_packet(client_id, 0, 0xB0))
            .await
            .unwrap();
        client.graceful_part().await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ClientLeft { client_id: left }) if left == client_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before client departure"),
            }
        }

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0xA0))
            .await
            .unwrap();
        let boundary = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(boundary.tick(), 0);
        assert_eq!(control_commands(&boundary), vec![0xA0, 0xB0]);

        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 1, 0xA1))
            .await
            .unwrap();
        let premature = timeout(Duration::from_millis(50), async {
            loop {
                match host_events.recv().await {
                    Some(HostEvent::Ready { packet }) => break packet,
                    Some(_) => continue,
                    None => panic!("host event stream ended before sync execution"),
                }
            }
        })
        .await;
        assert!(
            premature.is_err(),
            "disconnect released a host-only tick before ClientRemove executed"
        );

        host.status_reached().await.unwrap();
        let mut released = None;
        let mut synchronized_remove = None;
        let mut committed = false;
        while released.is_none() || synchronized_remove.is_none() || !committed {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::Ready { packet }) => {
                    assert!(released.replace(packet).is_none(), "tick released twice");
                }
                Some(HostEvent::SyncScheduled { controls, .. }) => {
                    assert!(
                        synchronized_remove.replace(controls).is_none(),
                        "ClientRemove synchronized twice"
                    );
                }
                Some(HostEvent::StatusCommitted(status)) if status.state == NETWORK_STATE_GO => {
                    committed = true;
                }
                Some(_) => continue,
                None => panic!("host event stream ended during sync execution"),
            }
        }

        let released = released.unwrap();
        assert_eq!(released.tick(), 1);
        assert_eq!(control_commands(&released), vec![0xA1]);
        let controls = synchronized_remove.unwrap();
        let [EngineControlPacket::ClientRemove(remove)] = controls.as_slice() else {
            panic!("expected one synchronized ClientRemove, got {controls:?}");
        };
        assert_eq!(remove.client_id, i32::try_from(client_id).unwrap());
        assert_eq!(remove.by_client, i32::try_from(HOST_CLIENT_ID).unwrap());

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn disconnect_broadcasts_host_authored_synchronized_client_remove() {
        // OnClientDisconnect calls C4ClientList::CtrlRemove, which broadcasts
        // a host-authored CDT_Sync ClientRemove and executes it at the frozen
        // synchronization boundary (src/C4Network2.cpp:1786-1802;
        // src/C4Client.cpp:293-303;
        // src/C4GameControlNetwork.cpp:181-220).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let alpha_id = alpha.client_id();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();

        alpha.shutdown().await.unwrap();
        let remove = loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::SyncScheduled { controls, .. }) => {
                    let Some(EngineControlPacket::ClientRemove(remove)) =
                        controls.into_iter().next()
                    else {
                        continue;
                    };
                    break remove;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected unexpectedly: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended unexpectedly"),
            }
        };
        assert_eq!(remove.client_id, i32::try_from(alpha_id).unwrap());
        assert_eq!(remove.by_client, 0);
        // LoadResStr(IDS_MSG_DISCONNECTED) supplies the synchronized reason
        // verbatim (planet/System.c4g/LanguageUS.txt:831).
        assert_eq!(remove.reason.as_bytes(), b"disconnected");

        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_half_accepted_join_is_removed_from_existing_clients() {
        // Join creates/broadcasts ClientJoin before mutual ConnRe. If the
        // socket then fails, OnConnectFail routes the provisional client
        // through the same synchronized CtrlRemove path
        // (src/C4Network2.cpp:1395-1445,1745-1755;
        // src/C4Client.cpp:293-303).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut witness = connect_client(
            addr,
            ClientConfig::new("Witness", ParticipantKind::Observer),
        )
        .await
        .unwrap();
        let mut witness_events = witness.take_event_receiver();

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut failed = crate::ControlTransport::new(stream);
        let _ = failed.read_message().await.unwrap();
        let name = lc_engine::LegacyCString::from_bytes(b"HalfJoin".to_vec()).unwrap();
        failed
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: lc_engine::ClientCoreControlData {
                        client_id: -1,
                        name: name.clone(),
                        nick: name,
                        ..Default::default()
                    },
                    build: CURRENT_GAME_BUILD,
                    password: lc_engine::LegacyCString::default(),
                    connection_id: 77,
                },
            ))
            .await
            .unwrap();
        loop {
            match failed.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    failed
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
        drop(failed);

        let mut provisional_id = None;
        loop {
            match timeout(EVENT_WAIT, witness_events.recv()).await.unwrap() {
                Some(ClientEvent::Direct { data, .. }) => {
                    if let Ok(EngineControlPacket::ClientJoin(join)) =
                        decode_control_entry_payload(&data)
                    {
                        provisional_id = Some(join.core.client_id);
                    }
                }
                Some(ClientEvent::SyncScheduled { controls, .. }) => {
                    if let Some(EngineControlPacket::ClientRemove(remove)) =
                        controls.into_iter().next()
                    {
                        assert_eq!(Some(remove.client_id), provisional_id);
                        assert_eq!(remove.by_client, 0);
                        break;
                    }
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("witness disconnected unexpectedly: {reason:?}")
                }
                Some(_) => {}
                None => panic!("witness event stream ended unexpectedly"),
            }
        }

        witness.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_secondary_known_connection_keeps_the_canonical_client() {
        // OnConnectFail removes a half-accepted client only when that client has
        // no other connection. Losing a secondary route therefore leaves the
        // already-connected canonical client registered
        // (src/C4Network2.cpp:1366-1380,1745-1765).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut canonical =
            connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
                .await
                .unwrap();
        let canonical_id = canonical.client_id();
        let mut canonical_events = canonical.take_event_receiver();

        let stream = TcpStream::connect(addr).await.unwrap();
        let mut secondary = crate::ControlTransport::new(stream);
        assert!(matches!(
            secondary.read_message().await.unwrap(),
            ControlMessage::ConnectionRequest(_)
        ));
        let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        secondary
            .send_message(ControlMessage::ConnectionRequest(
                crate::ConnectionRequest {
                    core: lc_engine::ClientCoreControlData {
                        client_id: i32::try_from(canonical_id).unwrap(),
                        activated: true,
                        observer: false,
                        name: name.clone(),
                        nick: name,
                        lobby_ready: true,
                    },
                    build: CURRENT_GAME_BUILD,
                    password: lc_engine::LegacyCString::default(),
                    connection_id: 29,
                },
            ))
            .await
            .unwrap();
        loop {
            match secondary.read_message().await.unwrap() {
                ControlMessage::ConnectionReply(reply) if reply.ok => break,
                ControlMessage::Ping(ping) => {
                    secondary
                        .send_message(ControlMessage::Pong(ping))
                        .await
                        .unwrap();
                }
                _ => continue,
            }
        }
        drop(secondary);

        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::SyncScheduled { controls, .. }) => assert!(
                    !controls.iter().any(|control| matches!(
                        control,
                        EngineControlPacket::ClientRemove(remove)
                            if remove.client_id == i32::try_from(canonical_id).unwrap()
                    )),
                    "secondary route failure queued ClientRemove for the canonical client"
                ),
                Some(HostEvent::ClientLeft { client_id }) if client_id == canonical_id => {
                    panic!("secondary route failure removed the canonical client")
                }
                Some(HostEvent::TransportError { client_id, error })
                    if error.contains("connection admission from") =>
                {
                    assert_eq!(client_id, None);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before secondary admission failed"),
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_millis(50);
        while let Ok(Some(event)) = timeout_at(deadline, canonical_events.recv()).await {
            match event {
                ClientEvent::SyncScheduled { controls, .. }
                    if controls.iter().any(|control| matches!(
                        control,
                        EngineControlPacket::ClientRemove(remove)
                            if remove.client_id == i32::try_from(canonical_id).unwrap()
                    )) => panic!("canonical client executed a secondary-route ClientRemove"),
                ClientEvent::Disconnected { reason } => {
                    panic!("canonical client disconnected unexpectedly: {reason:?}")
                }
                _ => {}
            }
        }

        canonical.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn new_client_starts_at_fresh_dynamic_tick_without_old_backlog() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let config = HostConfig {
            max_players: 4,
            ..Default::default()
        };
        let mut host = start_host(listener, config.clone())
            .await
            .expect("start host");

        let mut host_events = host.take_event_receiver();
        let client_alpha =
            connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
                .await
                .expect("connect alpha client");
        activate_joined_client(&host, &mut host_events, client_alpha.client_id()).await;

        submit_control_pair(&mut host, &client_alpha, 0, 0xA1, 0xB2).await;
        let ready_packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready_packet.tick(), 0);

        // A runtime join receives a dynamic snapshot for the next control tick.
        // C++ sends no eager backlog after JoinData; Init requests exactly the
        // snapshot tick, so controls already represented by the dynamic must
        // not replay (src/C4Network2.cpp:1820-1850;
        // src/C4GameControlNetwork.cpp:46-62,531-555).
        client_alpha.shutdown().await.expect("alpha shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;
        let mut fresh_snapshot = config.initial_join_snapshot.unwrap();
        fresh_snapshot.dynamic_tick = 1;
        host.publish_join_snapshot(fresh_snapshot)
            .await
            .expect("publish fresh dynamic");

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect beta client");
        let mut beta_events = client_beta.take_event_receiver();
        assert!(timeout(Duration::from_millis(50), beta_events.recv())
            .await
            .is_err());
        activate_joined_client(&host, &mut host_events, client_beta.client_id()).await;

        submit_control_pair(&mut host, &client_beta, 1, 0xC3, 0xD4).await;
        let ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready.tick(), 1);
        assert_eq!(control_commands(&ready), vec![0xC3, 0xD4]);
        assert_eq!(
            wait_for_client_ready(&mut beta_events, EVENT_WAIT).await,
            ready
        );

        client_beta.shutdown().await.expect("beta shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        host.shutdown().await.expect("host shutdown");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_resends_backlog_when_requested() {
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);

        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));

        let packet = ControlPacket::builder(7, 42)
            .timestamp_ms(1234)
            .payload(vec![0xDE, 0xAD, 0xBE, 0xFF]);
        command_tx
            .send(ClientCommand::SubmitControl(packet.clone()))
            .await
            .expect("submit control");

        match host_transport
            .read_message()
            .await
            .expect("receive control")
        {
            ControlMessage::Control(received) => {
                assert_eq!(received.client_id(), packet.client_id());
                assert_eq!(received.tick(), packet.tick());
                assert_eq!(received.payload(), packet.payload());
            }
            other => panic!("expected control packet, got {other:?}"),
        }

        // Ensure the client loop processed the send before issuing the request.
        while let Ok(Some(event)) = timeout(Duration::from_millis(20), event_rx.recv()).await {
            match event {
                ClientEvent::Ready { .. }
                | ClientEvent::Direct { .. }
                | ClientEvent::ExecSync { .. }
                | ClientEvent::Status(_)
                | ClientEvent::StatusAck(_)
                | ClientEvent::LobbyCountdown { .. }
                | ClientEvent::ReadyCheck { .. }
                | ClientEvent::ResourceAction(_)
                | ClientEvent::ResourceComplete { .. }
                | ClientEvent::ResourceLoadFailed { .. }
                | ClientEvent::ResourceDeriveUnsupported { .. }
                | ClientEvent::UnhandledPacket { .. }
                | ClientEvent::SyncScheduled { .. } => continue,
                ClientEvent::Disconnected { reason } => {
                    panic!("client disconnected unexpectedly: {reason:?}");
                }
            }
        }

        host_transport
            .send_message(ControlMessage::Request { from_tick: 42 })
            .await
            .expect("send request");

        match host_transport.read_message().await.expect("receive resend") {
            ControlMessage::Control(resend) => {
                assert_eq!(resend.client_id(), packet.client_id());
                assert_eq!(resend.tick(), packet.tick());
                assert_eq!(resend.payload(), packet.payload());
            }
            other => panic!("expected resend control packet, got {other:?}"),
        }

        shutdown_tx.send(()).ok();
        client_handle.await.expect("client loop exited");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_graceful_part_sends_exact_cpp_removal_frame_before_close() {
        // C4Network2ClientList::DeleteClient asks CloseConns to send a negative
        // PID_ConnRe with "removing client" before closing the connection
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (client_stream, mut host_stream) = duplex(128);
        let (command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let handle = ClientHandle {
            command_tx,
            event_rx: Some(event_rx),
            shutdown_tx: Some(shutdown_tx),
            join_handle: tokio::spawn(run_client_loop(
                crate::ControlTransport::new(client_stream),
                command_rx,
                event_tx,
                shutdown_rx,
            )),
            client_id: 1,
            join_data: None,
        };

        handle.graceful_part().await.expect("graceful client part");

        let mut bytes = Vec::new();
        host_stream.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(
            bytes,
            [
                0xff, 0x13, 0x00, 0x00, 0x00, 0x03, 0x00, b'r', b'e', b'm', b'o', b'v', b'i', b'n',
                b'g', b' ', b'c', b'l', b'i', b'e', b'n', b't', 0x00, 0x00,
            ]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_treats_negative_post_admission_connre_as_host_removal() {
        // CloseConns sends the same negative PID_ConnRe on an already accepted
        // connection so the peer can report the removal reason before EOF
        // (src/C4Network2Client.cpp:104-119).
        let (client_stream, host_stream) = duplex(128);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: false,
                message: lc_engine::LegacyCString::from_bytes(b"removing client".to_vec()).unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "removing client"
        ));
        timeout(EVENT_WAIT, client_loop)
            .await
            .expect("client loop did not close after host removal")
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_still_rejects_positive_post_admission_connre_as_duplicate() {
        // A positive ConnRe only completes connection admission. Receiving a
        // second positive reply after admission is not the CloseConns removal
        // signal (src/C4Network2.cpp:1448-1474).
        let (client_stream, host_stream) = duplex(128);
        let (_command_tx, command_rx) = mpsc::channel(1);
        let (event_tx, mut event_rx) = mpsc::channel(1);
        let (_shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            crate::ControlTransport::new(client_stream),
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let mut host_transport = crate::ControlTransport::new(host_stream);

        host_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: true,
                message: lc_engine::LegacyCString::from_bytes(b"duplicate".to_vec()).unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Disconnected { reason: Some(reason) })
                if reason == "host sent a duplicate connection reply"
        ));
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_reports_negative_post_admission_connre_once_before_eof() {
        // CloseConns writes the negative PID_ConnRe and immediately closes the
        // socket; the accepted connection must therefore report one removal,
        // not another disconnect when that close becomes EOF
        // (src/C4Network2Client.cpp:104-119,457-492).
        let (host_stream, client_stream) = duplex(128);
        let (_outbound_tx, outbound_rx) = HostOutboundSender::channel(1);
        let (host_tx, mut host_rx) = mpsc::channel(2);
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 7,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        let mut client_transport = crate::ControlTransport::new(client_stream);

        client_transport
            .send_message(ControlMessage::ConnectionReply(crate::ConnectionReply {
                ok: false,
                message: lc_engine::LegacyCString::from_bytes(b"removing client".to_vec()).unwrap(),
                wrong_password: false,
            }))
            .await
            .unwrap();
        drop(client_transport);
        task.await.unwrap();

        let mut messages = Vec::new();
        while let Some(message) = host_rx.recv().await {
            messages.push(message);
        }
        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages.pop(),
            Some(HostLoopMessage::ClientDisconnected {
                connection_id: 3,
                client_id: 7,
                next_inbound_packet: 0,
                post_mortem: None,
                reason: Some(reason),
            }) if reason == "removing client"
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn disconnected_connection_emits_cpp_post_mortem_backlog_for_peer_id() {
        // OnDisconn retains the closed connection; C4Network2 then builds one
        // recovery packet from its logged sends, identifying the dead socket
        // with iRemoteID so the peer can find its own local connection record
        // (src/C4Network2IO.cpp:520-570,1379-1396;
        // src/C4Network2.cpp:883-905).
        let (host_stream, client_stream) = duplex(256);
        let (outbound_tx, outbound_rx) = HostOutboundSender::channel(1);
        let (host_tx, mut host_rx) = mpsc::channel(1);
        let task = tokio::spawn(
            ClientTask {
                local_connection_id: 3,
                remote_connection_id: 5,
                client_id: 7,
                transport: crate::ControlTransport::new(host_stream),
                outbound_rx,
                host_tx,
                liveness: ConnectionLivenessState::new_accepted_system(),
            }
            .run(),
        );
        let mut client_transport = crate::ControlTransport::new(client_stream);
        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: -1,
        };

        outbound_tx
            .send(ControlMessage::Status(status))
            .await
            .unwrap();
        assert_eq!(
            client_transport.read_message().await.unwrap(),
            ControlMessage::Status(status)
        );
        drop(client_transport);
        task.await.unwrap();

        let Some(HostLoopMessage::ClientDisconnected {
            connection_id: 3,
            client_id: 7,
            next_inbound_packet: 0,
            post_mortem: Some(post_mortem),
            reason: None,
        }) = host_rx.recv().await
        else {
            panic!("expected recovery backlog for the disconnected route");
        };
        assert_eq!(post_mortem.connection_id, 5);
        assert_eq!(post_mortem.packet_counter, 1);
        assert_eq!(post_mortem.packets.len(), 1);
        assert_eq!(
            crate::transport::parse_complete_packet(&post_mortem.packets[0]).unwrap(),
            Some(ControlMessage::Status(status))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn graceful_client_part_emits_one_host_departure_with_cpp_reason() {
        // DeleteClient closes the accepted peer with "removing client"; the
        // receiving network owns one disconnect notification even though EOF
        // follows the ConnRe frame (src/C4Network2Client.cpp:104-119,457-492).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let client_id = client.client_id();

        client.graceful_part().await.unwrap();

        let mut departures = 0;
        let mut saw_reason = false;
        while departures == 0 || !saw_reason {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ClientLeft { client_id: left }) if left == client_id => {
                    departures += 1;
                }
                Some(HostEvent::TransportError {
                    client_id: Some(source),
                    error,
                }) if source == client_id && error == "removing client" => {
                    saw_reason = true;
                }
                Some(_) => {}
                None => panic!("host event stream ended before graceful departure"),
            }
        }
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), host_events.recv()).await {
            if matches!(event, HostEvent::ClientLeft { client_id: left } if left == client_id) {
                departures += 1;
            }
        }
        assert_eq!(departures, 1);

        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_lobby_countdown_without_disconnecting() {
        // MainDlg receives every PID_LobbyCountdown and updates its local
        // countdown state; the packet does not close the connection
        // (src/C4GameLobby.cpp:392-418,695-701).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let packet = crate::LobbyCountdownPacket::new(5);

        host_transport
            .send_message(ControlMessage::LobbyCountdown(packet))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::LobbyCountdown { packet: received }) if received == packet
        ));

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_surfaces_and_broadcasts_its_lobby_countdown() {
        // Countdown construction broadcasts the packet to clients while the
        // host applies the same packet directly to its local MainDlg
        // (src/C4GameLobby.cpp:1111-1131).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut client =
            connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
                .await
                .unwrap();
        let mut client_events = client.take_event_receiver();
        let packet = crate::LobbyCountdownPacket::new(5);

        host.submit_lobby_countdown(packet).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::LobbyCountdown { packet: received }) => {
                    assert_eq!(received, packet);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before lobby countdown"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, client_events.recv()).await.unwrap() {
                Some(ClientEvent::LobbyCountdown { packet: received }) => {
                    assert_eq!(received, packet);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("client disconnected during lobby countdown: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("client event stream ended before lobby countdown"),
            }
        }

        client.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_surfaces_ready_check_without_disconnecting() {
        // Accepted PID_ReadyCheck packets are dispatched through
        // C4Network2::HandlePacket/HandleReadyCheck and do not close the
        // connection (src/C4Network2.cpp:949-953,1625-1707).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let packet = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };

        host_transport
            .send_message(ControlMessage::ReadyCheck(packet))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet: received }) if received == packet
        ));

        let status = NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 0,
        };
        host_transport
            .send_message(ControlMessage::Status(status))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Status(received)) if received == status
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ignores_nonhost_ready_request_without_disconnecting() {
        // HandleReadyCheck accepts a Request only when packet.Client resolves
        // to the host; a rejected request returns without closing the network
        // connection (src/C4Network2.cpp:1625-1646).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_loop = tokio::spawn(run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let rejected = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Request,
        };

        host_transport
            .send_message(ControlMessage::ReadyCheck(rejected))
            .await
            .unwrap();
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        let accepted = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Ready,
        };
        host_transport
            .send_message(ControlMessage::ReadyCheck(accepted))
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_filters_nonhost_ready_request_buffered_during_join() {
        // Packets buffered until JoinData must still pass through the same
        // HandleReadyCheck host-request validation as live packets
        // (src/C4Network2.cpp:949-953,1625-1646).
        let (client_stream, _host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let rejected = ReadyCheckPacket {
            client_id: 1,
            data: crate::ReadyCheckData::Request,
        };
        let accepted = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };
        let mut resource_state = ClientResourceState::empty();
        resource_state.initial_ready_checks = vec![rejected, accepted];
        let client_loop = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::new(),
            resource_state,
        ));

        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::ReadyCheck { packet }) if packet == accepted
        ));
        assert!(timeout(Duration::from_millis(50), event_rx.recv())
            .await
            .is_err());

        shutdown_tx.send(()).ok();
        client_loop.await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_ignores_network_ready_request_but_relays_its_opaque_fanout_leg() {
        // HandleReadyCheck rejects every Request while this process is the
        // host. HandleFwdReq still relays the opaque packet to selected peers,
        // where a claimed host author is accepted; Ready/NotReady likewise
        // select packet.Client without checking the transport origin
        // (src/C4Network2IO.cpp:1077-1129;
        // src/C4Network2.cpp:1625-1654,1700-1703).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();
        let request = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Request,
        };

        alpha.submit_ready_check(request).await.unwrap();
        while let Ok(Some(event)) = timeout(Duration::from_millis(50), host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == request),
                "host surfaced a network-origin ready request"
            );
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, request);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during opaque request relay: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before opaque request relay"),
            }
        }

        let spoofed_ready = ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: crate::ReadyCheckData::Ready,
        };
        alpha.submit_ready_check(spoofed_ready).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, spoofed_ready);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before spoofed ready"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, spoofed_ready);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during spoofed ready: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before spoofed ready"),
            }
        }

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn host_relays_ready_check_unchanged_and_broadcasts_local_submission() {
        // Ready-check packets carry their claimed Client field through
        // BroadcastMsgToClients; HandleReadyCheck looks that client up without
        // comparing it to the transport origin (src/C4GameLobby.cpp:329-343,
        // 1072-1088; src/C4Network2.cpp:1625-1635).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let mut alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        let mut alpha_events = alpha.take_event_receiver();
        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let mut beta_events = beta.take_event_receiver();
        let relayed = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Ready,
        };

        alpha.submit_ready_check(relayed).await.unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, relayed);
                    break;
                }
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check relay"),
            }
        }
        loop {
            match timeout(EVENT_WAIT, beta_events.recv()).await.unwrap() {
                Some(ClientEvent::ReadyCheck { packet }) => {
                    assert_eq!(packet, relayed);
                    break;
                }
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected during ready-check relay: {reason:?}")
                }
                Some(_) => continue,
                None => panic!("beta event stream ended before ready-check relay"),
            }
        }
        let host_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(host_duplicate_deadline, host_events.recv()).await {
            assert!(
                !matches!(event, HostEvent::ReadyCheck { packet } if packet == relayed),
                "host handled one ready-check toggle twice"
            );
            assert!(
                !matches!(
                    event,
                    HostEvent::TransportError {
                        client_id: Some(client_id),
                        ..
                    } if client_id == alpha.client_id()
                ),
                "host rejected the ready-check forwarding leg"
            );
        }
        let beta_duplicate_deadline = tokio::time::Instant::now() + Duration::from_millis(100);
        while let Ok(Some(event)) = timeout_at(beta_duplicate_deadline, beta_events.recv()).await {
            assert!(
                !matches!(event, ClientEvent::ReadyCheck { packet } if packet == relayed),
                "beta received one ready-check toggle twice"
            );
        }

        let local = ReadyCheckPacket {
            client_id: 0,
            data: crate::ReadyCheckData::Request,
        };
        host.submit_ready_check(local).await.unwrap();
        for events in [&mut alpha_events, &mut beta_events] {
            loop {
                match timeout(EVENT_WAIT, events.recv()).await.unwrap() {
                    Some(ClientEvent::ReadyCheck { packet }) => {
                        assert_eq!(packet, local);
                        break;
                    }
                    Some(ClientEvent::Disconnected { reason }) => {
                        panic!("client disconnected during host ready-check: {reason:?}")
                    }
                    Some(_) => continue,
                    None => panic!("client event stream ended before host ready-check"),
                }
            }
        }

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn ready_check_updates_the_claimed_client_in_later_join_data() {
        // HandleReadyCheck mutates the C4Client selected by packet.Client;
        // later JoinData serializes that same Game.Clients registry
        // (src/C4Network2.cpp:1625-1635,1721-1729,1810-1850).
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let mut host = start_host(listener, HostConfig::default()).await.unwrap();
        let mut host_events = host.take_event_receiver();
        let alpha = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .unwrap();
        alpha
            .submit_ready_check(ReadyCheckPacket {
                client_id: 0,
                data: crate::ReadyCheckData::Ready,
            })
            .await
            .unwrap();
        loop {
            match timeout(EVENT_WAIT, host_events.recv()).await.unwrap() {
                Some(HostEvent::ReadyCheck { .. }) => break,
                Some(_) => continue,
                None => panic!("host event stream ended before ready-check"),
            }
        }

        let mut beta = connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
            .await
            .unwrap();
        let join_data = beta.take_join_data().expect("beta receives JoinData");
        assert!(
            join_data
                .parameters
                .clients
                .clients
                .iter()
                .find(|client| client.client_id == 0)
                .expect("host remains in client registry")
                .lobby_ready
        );

        alpha.shutdown().await.unwrap();
        beta.shutdown().await.unwrap();
        host.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn direct_client_join_extends_the_address_owner_registry() {
        // CID_ClientJoin executes as direct control before later PID_Addr
        // propagation for that owner. The receiver must therefore admit the
        // new owner before handling its address packets
        // (src/C4Network2.cpp:1395-1445;
        // src/C4Network2Client.cpp:581-621).
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::from([(0, Vec::new()), (1, Vec::new())]),
            ClientResourceState::empty(),
        ));
        let name = lc_engine::LegacyCString::from_bytes(b"Beta".to_vec()).unwrap();
        let direct = encode_control_entry_payload(&EngineControlPacket::ClientJoin(
            lc_engine::ClientJoinControlData {
                core: lc_engine::ClientCoreControlData {
                    client_id: 2,
                    name: name.clone(),
                    nick: name,
                    ..Default::default()
                },
                by_client: 0,
            },
        ))
        .unwrap();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: direct,
            })
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::Direct { .. })
        ));

        let address = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().unwrap(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(address))
            .await
            .unwrap();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Address(address)
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn synchronized_client_remove_changes_address_membership_at_exec_sync() {
        // CtrlRemove is delivered as CDT_Sync, so the client remains present
        // until PID_ExecSyncCtrl executes the queued removal
        // (src/C4Client.cpp:293-304;
        // src/C4GameControlNetwork.cpp:181-220,558-588).
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(run_client_loop_with_addresses(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
            None,
            BTreeMap::from([(0, Vec::new()), (1, Vec::new()), (2, Vec::new())]),
            ClientResourceState::empty(),
        ));
        let remove = encode_control_entry_payload(&EngineControlPacket::ClientRemove(
            lc_engine::ClientRemoveControlData {
                client_id: 2,
                reason: lc_engine::LegacyCString::from_bytes(b"left".to_vec()).unwrap(),
                by_client: 0,
            },
        ))
        .unwrap();
        host_transport
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Sync,
                data: remove,
            })
            .await
            .unwrap();

        let before_execution = crate::AddressPacket {
            client_id: 2,
            address: crate::NetworkAddress::new(
                crate::NetworkProtocol::Tcp,
                "198.51.100.22:11112".parse().unwrap(),
            ),
        };
        host_transport
            .send_message(ControlMessage::Address(before_execution))
            .await
            .unwrap();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Address(before_execution)
        );

        host_transport
            .send_message(ControlMessage::ExecSync { control_tick: 7 })
            .await
            .unwrap();
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await.unwrap(),
            Some(ClientEvent::SyncScheduled {
                control_tick: 7,
                ..
            })
        ));

        host_transport
            .send_message(ControlMessage::Address(crate::AddressPacket {
                client_id: 2,
                address: crate::NetworkAddress::new(
                    crate::NetworkProtocol::Tcp,
                    "198.51.100.23:11112".parse().unwrap(),
                ),
            }))
            .await
            .unwrap();
        assert!(
            timeout(Duration::from_millis(50), host_transport.read_message())
                .await
                .is_err()
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn decentral_client_waits_for_every_active_contribution_before_ready() {
        // In CNM_Decentral every client broadcasts and stores its own
        // contribution, but CheckCompleteCtrl exposes only the locally packed
        // C4ClientIDAll packet after all active clients contributed. Packing is
        // in client-ID order (pristine C++ src/C4GameControlNetwork.cpp:156-179,
        // 679-718,741-783).
        let (client_stream, host_stream) = duplex(2048);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let decentral = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 0,
            target_tick: 0,
        };

        host_transport
            .send_message(ControlMessage::StatusAck(decentral))
            .await
            .expect("send decentralized status");
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == decentral
        ));

        for (client_id, name) in [(0, b"Host".as_slice()), (1, b"Local".as_slice())] {
            let name = lc_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
            let join = EngineControlPacket::ClientJoin(lc_engine::ClientJoinControlData {
                core: lc_engine::ClientCoreControlData {
                    client_id,
                    activated: true,
                    observer: false,
                    name: name.clone(),
                    nick: name,
                    lobby_ready: false,
                },
                by_client: 0,
            });
            host_transport
                .send_message(ControlMessage::Packet {
                    delivery: ControlDelivery::Direct,
                    data: encode_control_entry_payload(&join).expect("encode client join"),
                })
                .await
                .expect("send active client join");
            assert!(matches!(
                timeout(EVENT_WAIT, event_rx.recv()).await,
                Ok(Some(ClientEvent::Direct {
                    delivery: ControlDelivery::Direct,
                    ..
                }))
            ));
        }

        let host = legacy_packet(0, 0, 0x11);
        let local = legacy_packet(1, 0, 0x22);
        host_transport
            .send_message(ControlMessage::Control(host.clone()))
            .await
            .expect("send host contribution");
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "one decentralized contribution must not execute"
        );

        command_tx
            .send(ClientCommand::SubmitControl(local.clone()))
            .await
            .expect("submit local contribution");
        let nested_packet = crate::transport::encode_complete_control_packet(&local).unwrap();
        assert_eq!(
            timeout(EVENT_WAIT, host_transport.read_message())
                .await
                .expect("local contribution send wait")
                .expect("read local contribution"),
            ControlMessage::ForwardRequest(crate::ForwardPacket {
                negative_list: true,
                clients: Vec::new(),
                nested_packet,
            })
        );
        let aggregate = match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("aggregate wait")
        {
            Some(ClientEvent::Ready { packet }) => packet,
            other => panic!("expected one aggregate ready event, got {other:?}"),
        };
        assert_eq!(aggregate.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&aggregate), vec![0x11, 0x22]);
        assert_eq!(
            aggregate
                .payload()
                .iter()
                .filter(|byte| **byte == 0xff)
                .count(),
            1,
            "the aggregate carries one C4Control list terminator"
        );

        for duplicate in [local, host] {
            host_transport
                .send_message(ControlMessage::Control(duplicate))
                .await
                .expect("echo duplicate contribution");
        }
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "local echo and host retransmit must not execute the completed tick again"
        );

        shutdown_tx.send(()).ok();
        drop(command_tx);
        client_handle.await.expect("client loop exited");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_emits_a_complete_tick_only_once_when_host_retransmits_it() {
        // A non-host in CNM_Central cannot pack per-client contributions and
        // waits for the host's C4ClientIDAll packet instead (pristine C++
        // src/C4GameControlNetwork.cpp:679-718,775-777).
        let (client_stream, host_stream) = duplex(512);
        let transport = crate::ControlTransport::new(client_stream);
        let mut host_transport = crate::ControlTransport::new(host_stream);
        let (_command_tx, command_rx) = mpsc::channel(8);
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let client_handle = tokio::spawn(super::run_client_loop(
            transport,
            command_rx,
            event_tx,
            shutdown_rx,
        ));
        let central = NetworkStatus {
            state: NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 5,
        };
        let complete = legacy_packet(BROADCAST_CLIENT_ID, 5, 0x44);

        host_transport
            .send_message(ControlMessage::StatusAck(central))
            .await
            .expect("send central status");
        assert!(matches!(
            timeout(EVENT_WAIT, event_rx.recv()).await,
            Ok(Some(ClientEvent::StatusAck(status))) if status == central
        ));
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .expect("send complete tick");
        host_transport
            .send_message(ControlMessage::Control(complete.clone()))
            .await
            .expect("retransmit complete tick");

        match timeout(EVENT_WAIT, event_rx.recv())
            .await
            .expect("ready wait")
        {
            Some(ClientEvent::Ready { packet }) => assert_eq!(packet, complete),
            other => panic!("expected one ready event, got {other:?}"),
        }
        assert!(
            timeout(Duration::from_millis(50), event_rx.recv())
                .await
                .is_err(),
            "a retransmitted complete packet must not execute twice"
        );

        shutdown_tx.send(()).ok();
        client_handle.await.expect("client loop exited");
    }

    async fn submit_control_pair(
        host: &mut HostHandle,
        client: &ClientHandle,
        tick: Tick,
        host_command: i32,
        client_command: i32,
    ) {
        let host_packet = legacy_packet(0, tick, host_command);
        host.submit_local_control(host_packet)
            .await
            .expect("host submit control");

        let client_packet = legacy_packet(client.client_id(), tick, client_command);
        client
            .submit_control(client_packet)
            .await
            .expect("client submit control");
    }

    async fn activate_joined_client(
        host: &HostHandle,
        events: &mut mpsc::Receiver<HostEvent>,
        client_id: ClientId,
    ) {
        // Join assigns a deactivated client ID. C4Network2::ActivateClient
        // queues a host-authored CUT_Activate, and only execution of that
        // synchronized control changes active control-list membership
        // (src/C4Network2.cpp:1395-1406,1553-1571;
        // src/C4Control.cpp:578-606).
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::ClientJoined {
                    client_id: joined_id,
                    ..
                })) if joined_id == client_id => break,
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(source),
                    ..
                })) if source != client_id => continue,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("transport error before client activation: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client join"),
                Err(_) => panic!("timed out waiting for client join"),
            }
        }

        let update = ClientUpdateControlData {
            update_type: CLIENT_UPDATE_ACTIVATE,
            client_id: i32::try_from(client_id).expect("test client ID fits i32"),
            data: 1,
            by_client: i32::try_from(HOST_CLIENT_ID).expect("host client ID fits i32"),
        };
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&EngineControlPacket::ClientUpdate(update.clone()))
                .expect("encode activation control"),
        )
        .await
        .expect("submit activation control");

        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::SyncScheduled { controls, .. }))
                    if controls == vec![EngineControlPacket::ClientUpdate(update.clone())] =>
                {
                    break;
                }
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(source),
                    ..
                })) if source != client_id => continue,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("transport error while activating client: {error}")
                }
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before client activation"),
                Err(_) => panic!("timed out waiting for client activation"),
            }
        }
    }

    fn legacy_packet(client_id: ClientId, tick: Tick, command: i32) -> ControlPacket {
        encode_control_packet(&LegacyControlFrame {
            client_id,
            tick,
            timestamp_ms: 0,
            controls: vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: i32::try_from(client_id).unwrap_or(i32::MAX),
                command,
                data: command,
                by_client: i32::try_from(client_id).unwrap_or(i32::MAX),
            })],
        })
        .expect("test legacy control encodes")
    }

    async fn raw_client_transport(
        address: SocketAddr,
        name: &[u8],
    ) -> (crate::ControlTransport<TcpStream>, ClientId) {
        let stream = TcpStream::connect(address).await.unwrap();
        let mut transport = crate::ControlTransport::new(stream);
        let name = lc_engine::LegacyCString::from_bytes(name.to_vec()).unwrap();
        let request = crate::ConnectionRequest {
            core: lc_engine::ClientCoreControlData {
                client_id: -1,
                activated: true,
                observer: false,
                name: name.clone(),
                nick: name,
                lobby_ready: false,
            },
            build: CURRENT_GAME_BUILD,
            password: lc_engine::LegacyCString::default(),
            connection_id: 0,
        };
        let handshake = run_client_connection_handshake(&mut transport, request)
            .await
            .unwrap();
        let client_id = ClientId::try_from(handshake.join_data.client_id).unwrap();
        (transport, client_id)
    }

    async fn drain_raw_client(transport: &mut crate::ControlTransport<TcpStream>) {
        while matches!(
            timeout(Duration::from_millis(20), transport.read_message()).await,
            Ok(Ok(_))
        ) {}
    }

    async fn raw_client_ping_barrier(transport: &mut crate::ControlTransport<TcpStream>) {
        let ping = crate::PingPacket {
            sent_at: 0x1020_3040,
            packet_counter: 0,
        };
        transport
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        let deadline = tokio::time::Instant::now() + EVENT_WAIT;
        loop {
            match timeout_at(deadline, transport.read_message()).await {
                Ok(Ok(ControlMessage::Pong(received))) if received == ping => return,
                Ok(Ok(_)) => continue,
                Ok(Err(error)) => panic!("ping barrier failed: {error}"),
                Err(_) => panic!("timed out waiting for ping barrier"),
            }
        }
    }

    async fn raw_client_received_message(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &ControlMessage,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if &message == expected {
                return true;
            }
        }
        false
    }

    async fn raw_tcp_received_frame(
        stream: &mut TcpStream,
        expected: &[u8],
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let mut header = [0_u8; 5];
            if !matches!(
                timeout_at(deadline, stream.read_exact(&mut header)).await,
                Ok(Ok(_))
            ) {
                return false;
            }
            assert_eq!(header[0], 0xff, "invalid TCP packet frame prefix");
            let size = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut body = vec![0; size];
            if !matches!(
                timeout_at(deadline, stream.read_exact(&mut body)).await,
                Ok(Ok(_))
            ) {
                return false;
            }
            if body == expected {
                return true;
            }
        }
    }

    async fn raw_client_received_control(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &ControlPacket,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if message == ControlMessage::Control(expected.clone()) {
                return true;
            }
        }
        false
    }

    async fn raw_client_received_forward(
        transport: &mut crate::ControlTransport<TcpStream>,
        expected: &crate::ForwardPacket,
        duration: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + duration;
        while let Ok(Ok(message)) = timeout_at(deadline, transport.read_message()).await {
            if message == ControlMessage::Forward(expected.clone()) {
                return true;
            }
        }
        false
    }

    async fn wait_for_host_error(
        events: &mut mpsc::Receiver<HostEvent>,
        source: ClientId,
    ) -> String {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error,
                })) if client_id == source => return error,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended before forwarding error"),
                Err(_) => panic!("timed out waiting for forwarding error"),
            }
        }
    }

    fn control_commands(packet: &ControlPacket) -> Vec<i32> {
        decode_control_packet(packet)
            .expect("complete control decodes")
            .controls
            .into_iter()
            .map(|control| match control {
                EngineControlPacket::PlayerControl(control) => control.command,
                other => panic!("expected player control, got {other:?}"),
            })
            .collect()
    }

    async fn wait_for_host_ready(
        events: &mut mpsc::Receiver<HostEvent>,
        duration: Duration,
    ) -> ControlPacket {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(HostEvent::Ready { packet })) => break packet,
                Ok(Some(HostEvent::ClientJoined { .. })) => continue,
                // A departing client's closing socket can surface a transient
                // transport error; tolerate it like ClientLeft. A real failure
                // still trips the timeout because Ready never arrives.
                Ok(Some(HostEvent::ClientLeft { .. }))
                | Ok(Some(HostEvent::UnhandledPacket { .. }))
                | Ok(Some(HostEvent::TransportError { .. })) => continue,
                Ok(Some(HostEvent::Direct { .. }))
                | Ok(Some(HostEvent::JoinDataNeeded { .. }))
                | Ok(Some(HostEvent::ExecSync { .. }))
                | Ok(Some(HostEvent::ActivationRequest { .. }))
                | Ok(Some(HostEvent::PlayerInfoUpdate { .. }))
                | Ok(Some(HostEvent::LobbyCountdown { .. }))
                | Ok(Some(HostEvent::ReadyCheck { .. }))
                | Ok(Some(HostEvent::ResourceAction(_)))
                | Ok(Some(HostEvent::ResourceComplete { .. }))
                | Ok(Some(HostEvent::ResourceLoadFailed { .. }))
                | Ok(Some(HostEvent::ResourceDeriveUnsupported { .. }))
                | Ok(Some(HostEvent::StatusAck { .. }))
                | Ok(Some(HostEvent::SyncScheduled { .. }))
                | Ok(Some(HostEvent::StatusCommitted(_))) => continue,
                Ok(None) => panic!("host event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for host ready event"),
            }
        }
    }

    async fn wait_for_client_ready(
        events: &mut mpsc::Receiver<ClientEvent>,
        duration: Duration,
    ) -> ControlPacket {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(ClientEvent::Ready { packet })) => break packet,
                Ok(Some(ClientEvent::ExecSync { .. })) => continue,
                Ok(Some(ClientEvent::Direct { .. })) => continue,
                Ok(Some(ClientEvent::Status(_))) | Ok(Some(ClientEvent::StatusAck(_))) => continue,
                Ok(Some(ClientEvent::LobbyCountdown { .. })) => continue,
                Ok(Some(ClientEvent::ReadyCheck { .. })) => continue,
                Ok(Some(ClientEvent::ResourceAction(_))) => continue,
                Ok(Some(ClientEvent::ResourceComplete { .. }))
                | Ok(Some(ClientEvent::ResourceLoadFailed { .. }))
                | Ok(Some(ClientEvent::ResourceDeriveUnsupported { .. })) => continue,
                Ok(Some(ClientEvent::UnhandledPacket { .. })) => continue,
                Ok(Some(ClientEvent::SyncScheduled { .. })) => continue,
                Ok(Some(ClientEvent::Disconnected { reason })) => {
                    panic!("client disconnected during test: {:?}", reason);
                }
                Ok(None) => panic!("client event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for client ready event"),
            }
        }
    }

    async fn wait_for_client_departure(events: &mut mpsc::Receiver<HostEvent>, duration: Duration) {
        loop {
            match timeout(duration, events.recv()).await {
                Ok(Some(HostEvent::ClientLeft { .. })) => break,
                Ok(Some(HostEvent::TransportError { .. })) => break,
                Ok(Some(_)) => continue,
                Ok(None) => panic!("host event stream ended unexpectedly"),
                Err(_) => panic!("timed out waiting for client departure"),
            }
        }
    }
}
