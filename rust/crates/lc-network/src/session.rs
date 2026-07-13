use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
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
    JoinDataEnvelope, MissingRange, NetworkStatus, ParticipantKind, ReadyBatch, RemoteBarrierState,
    ResourcePacket, ResyncScheduler, StatusBarrier, Tick, TransportError, CURRENT_GAME_BUILD,
    NETWORK_STATE_GO, NETWORK_STATE_LOBBY, NETWORK_STATE_PAUSE,
};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_BACKLOG_LIMIT: usize = 256;
const HOST_CLIENT_ID: ClientId = 0;

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
        }
    }
}

/// Client metadata supplied during handshake.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub name: String,
    pub kind: ParticipantKind,
    pub password: lc_engine::LegacyCString,
}

impl ClientConfig {
    pub fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        Self {
            name: name.into(),
            kind,
            password: lc_engine::LegacyCString::default(),
        }
    }

    pub fn with_password(mut self, password: lc_engine::LegacyCString) -> Self {
        self.password = password;
        self
    }
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
            loadable: false,
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
            scenario: lc_engine::NetworkResourceCore::default(),
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
    },
    PlayerInfoUpdate {
        client_id: ClientId,
        request: crate::PlayerInfoUpdateRequest,
    },
    ResourceAction(crate::ResourceCatalogAction),
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
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
    PublishJoinSnapshot(Box<HostJoinSnapshot>),
    SetJoinAllowed(bool),
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

    pub async fn set_join_allowed(&self, allowed: bool) -> Result<(), HostError> {
        self.command_tx
            .send(HostCommand::SetJoinAllowed(allowed))
            .await
            .map_err(|_| HostError::HostLoopGone)
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
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("failed to connect to host: {0}")]
    Connect(#[from] io::Error),
    #[error("handshake rejected: {0}")]
    Handshake(String),
    #[error("client loop terminated unexpectedly")]
    ClientLoopGone,
}

/// Starts the multiplayer host loop.
pub async fn start_host(
    listener: TcpListener,
    config: HostConfig,
) -> Result<HostHandle, HostError> {
    let (command_tx, command_rx) = mpsc::channel::<HostCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<HostEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_handle = tokio::spawn(run_host(
        listener,
        config,
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
    send_client_post_join_packets(&mut transport, start_control_tick, address_announcements)
        .await
        .map_err(|error| {
            ClientError::Handshake(format!("failed to initialize after JoinData: {error}"))
        })?;

    let (command_tx, command_rx) = mpsc::channel::<ClientCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let resource_state = ClientResourceState::from_join_data(
        &join_data,
        bootstrap.peer_core.client_id,
        bootstrap.pending_resources,
        bootstrap.pending_controls,
        bootstrap.liveness,
    );
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

async fn send_client_post_join_packets<S>(
    transport: &mut crate::ControlTransport<S>,
    start_control_tick: Tick,
    address_announcements: Vec<crate::AddressPacket>,
) -> Result<(), TransportError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    // C4GameControlNetwork::Init asks connected peers for the first control
    // tick before HandleJoinData announces additional addresses
    // (src/C4GameControlNetwork.cpp:46-62; src/C4Network2.cpp:1603-1623).
    transport
        .send_message(ControlMessage::Request {
            from_tick: start_control_tick,
        })
        .await?;
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
    Disconnected {
        reason: Option<String>,
    },
}

/// Commands available to a connected client.
#[derive(Debug)]
pub enum ClientCommand {
    SubmitStatusAck(NetworkStatus),
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
struct ClientConnection {
    outbound: mpsc::Sender<ControlMessage>,
    core: lc_engine::ClientCoreControlData,
    peer_addr: SocketAddr,
    join_data_sent: bool,
    join_data_needed_emitted: bool,
}

#[derive(Debug)]
struct ClientResourceState {
    catalog: crate::ResourceCatalog,
    host_peer_id: i32,
    initial_packets: Vec<ResourcePacket>,
    initial_controls: Vec<ControlPacket>,
    liveness: ConnectionLivenessState,
}

impl ClientResourceState {
    #[cfg(test)]
    fn empty() -> Self {
        Self {
            catalog: crate::ResourceCatalog::new(-1),
            host_peer_id: 0,
            initial_packets: Vec::new(),
            initial_controls: Vec::new(),
            liveness: ConnectionLivenessState::new_accepted_system(),
        }
    }

    fn from_join_data(
        join_data: &JoinDataEnvelope,
        host_peer_id: i32,
        initial_packets: Vec<ResourcePacket>,
        initial_controls: Vec<ControlPacket>,
        liveness: ConnectionLivenessState,
    ) -> Self {
        let mut catalog = crate::ResourceCatalog::new(join_data.client_id);
        let mut register_loadable = |core: &lc_engine::NetworkResourceCore| {
            if core.loadable {
                catalog.register(crate::ResourceRegistration::from_core(core, true, true));
            }
        };
        // HandleJoinData registers game resources, then dynamic, then player
        // resources. C4Network2ResList::Add prepends each registration
        // (src/C4Network2.cpp:1612-1620;
        // src/C4Network2Res.cpp:1431-1441,1473-1516).
        join_data
            .parameters
            .game_resources
            .iter()
            .for_each(&mut register_loadable);
        register_loadable(&join_data.dynamic);
        join_data
            .parameters
            .player_infos
            .clients
            .iter()
            .flat_map(|client| client.players.iter())
            .filter_map(|player| player.resource.as_ref())
            .for_each(&mut register_loadable);
        register_loadable(&join_data.parameters.scenario);
        Self {
            catalog,
            host_peer_id,
            initial_packets,
            initial_controls,
            liveness,
        }
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
        core: lc_engine::ClientCoreControlData,
        peer_addr: SocketAddr,
        outbound: mpsc::Sender<ControlMessage>,
        setup_tx: oneshot::Sender<Result<Option<ClientSetup>, String>>,
    },
    ClientMessage {
        client_id: ClientId,
        message: ControlMessage,
    },
    ClientDisconnected {
        client_id: ClientId,
        reason: Option<String>,
    },
    AdmissionFailed {
        connection_id: u32,
        error: String,
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
    pending_sync: Vec<lc_engine::ControlPacket>,
    status_barrier: StatusBarrier,
    admission: HostAdmission,
    client_cores: BTreeMap<i32, lc_engine::ClientCoreControlData>,
    client_addresses: BTreeMap<i32, Vec<crate::NetworkAddress>>,
    pending_kinds: BTreeMap<i32, ParticipantKind>,
    join_snapshot: Option<HostJoinSnapshot>,
    resource_catalog: crate::ResourceCatalog,
    next_connection_id: u32,
    pending_admissions: BTreeMap<u32, i32>,
    event_tx: mpsc::Sender<HostEvent>,
}

async fn run_host(
    listener: TcpListener,
    config: HostConfig,
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
    let mut state = HostState {
        coordinator,
        backlog: ControlBacklog::new(backlog_limit),
        scheduler: ResyncScheduler::new(config.resync_cooldown),
        clients: BTreeMap::new(),
        pending_sync: Vec::new(),
        status_barrier: StatusBarrier::stable(config.initial_status),
        admission,
        client_cores,
        client_addresses,
        pending_kinds: BTreeMap::new(),
        join_snapshot: config.initial_join_snapshot.clone(),
        resource_catalog,
        next_connection_id: 0,
        pending_admissions: BTreeMap::new(),
        event_tx: event_tx.clone(),
        config,
    };

    let (client_tx, mut client_rx) = mpsc::channel::<HostLoopMessage>(128);
    let (admission_tx, mut admission_rx) = mpsc::channel::<HostAdmissionRequest>(32);
    let mut resync_timer = interval(state.config.resync_interval);
    let resource_epoch = tokio::time::Instant::now();
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
                    HostLoopMessage::ClientAccepted { connection_id, core, peer_addr, outbound, setup_tx } => {
                        handle_client_accepted(connection_id, core, peer_addr, outbound, setup_tx, &mut state).await;
                    }
                    HostLoopMessage::ClientMessage { client_id, message } => {
                        handle_client_message(client_id, message, &mut state).await;
                    }
                    HostLoopMessage::ClientDisconnected { client_id, reason } => {
                        handle_client_disconnected(client_id, reason, &mut state).await;
                    }
                    HostLoopMessage::AdmissionFailed { connection_id, error } => {
                        handle_admission_failed(connection_id, error, &mut state).await;
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
                    HostCommand::SubmitLocal(packet) => ingest_control(packet, &mut state).await,
                    HostCommand::SubmitPacket { delivery, data } => broadcast_packet(delivery, data, None, &mut state).await,
                    HostCommand::ExecSync { control_tick } => broadcast_exec_sync(control_tick, &mut state).await,
                    HostCommand::PublishJoinSnapshot(snapshot) => {
                        state.join_snapshot = Some(*snapshot);
                        publish_pending_join_data(&mut state).await;
                    }
                    HostCommand::SetJoinAllowed(allowed) => {
                        state.admission.set_allow_join(allowed);
                    }
                    HostCommand::Shutdown => break,
                }
            }
            _ = resync_timer.tick() => {
                request_missing_controls(&mut state).await;
            }
            _ = resource_timer.tick() => {
                let actions = state
                    .resource_catalog
                    .on_timer(resource_epoch.elapsed().as_secs());
                dispatch_host_resource_actions(actions, &mut state).await;
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
            peer_core,
            liveness,
        } = handshake;
        let Ok(client_id) = ClientId::try_from(peer_core.client_id) else {
            let _ = host_tx
                .send(HostLoopMessage::AdmissionFailed {
                    connection_id,
                    error: "accepted peer has a negative client id".to_string(),
                })
                .await;
            return;
        };
        let (outbound, outbound_rx) = mpsc::channel(64);
        let (setup_tx, setup_rx) = oneshot::channel();
        if host_tx
            .send(HostLoopMessage::ClientAccepted {
                connection_id,
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
                        client_id,
                        reason: Some(error),
                    })
                    .await;
                return;
            }
            Err(_) => {
                let _ = host_tx
                    .send(HostLoopMessage::ClientDisconnected {
                        client_id,
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
                        client_id,
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
                            client_id,
                            reason: Some(format!("address send failed: {error}")),
                        })
                        .await;
                    return;
                }
            }
        }

        ClientTask {
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
    let mut decision = state.admission.admit_new_peer(&request.request);
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
        state
            .pending_admissions
            .insert(request.connection_id, peer_core.client_id);
    }
    let _ = request.decision_tx.send(decision);
}

async fn handle_client_accepted(
    connection_id: u32,
    core: lc_engine::ClientCoreControlData,
    peer_addr: SocketAddr,
    outbound: mpsc::Sender<ControlMessage>,
    setup_tx: oneshot::Sender<Result<Option<ClientSetup>, String>>,
    state: &mut HostState,
) {
    state.pending_admissions.remove(&connection_id);
    let Ok(client_id) = ClientId::try_from(core.client_id) else {
        let _ = setup_tx.send(Err("accepted peer has a negative client id".to_string()));
        return;
    };
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
            name: core.name.to_string_lossy().into_owned(),
            kind,
        })
        .await;

    let setup_result = match build_client_setup(client_id, state) {
        Ok(Some(setup)) => mark_join_data_sent(client_id, state).map(|()| Some(setup)),
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
            client_id,
            setup_error.or_else(|| Some("accepted connection setup was dropped".to_string())),
            state,
        )
        .await;
        return;
    }
    let actions = state.resource_catalog.on_peer_connected(core.client_id);
    dispatch_host_resource_actions(actions, state).await;
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

fn mark_join_data_sent(client_id: ClientId, state: &mut HostState) -> Result<(), String> {
    state
        .coordination_register(client_id)
        .map_err(|error| error.to_string())?;
    if let Some(client) = state.clients.get_mut(&client_id) {
        client.join_data_sent = true;
    }
    state
        .status_barrier
        .set_remote_state(client_id, RemoteBarrierState::Chasing);
    Ok(())
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
        if !failed {
            if let Err(error) = mark_join_data_sent(client_id, state) {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(client_id),
                        error,
                    })
                    .await;
            }
        }
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
    client_id: ClientId,
    message: ControlMessage,
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
        // PID_JoinData is host-to-client only; C++ silently ignores it on a
        // host (src/C4Network2.cpp:938-946).
        ControlMessage::JoinData(_) => {}
        ControlMessage::Address(packet) => {
            handle_received_host_address(client_id, packet, state).await;
        }
        ControlMessage::Resource(packet) => {
            let actions = state.resource_catalog.on_packet(client_id as i32, &packet);
            dispatch_host_resource_actions(actions, state).await;
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
        ControlMessage::ActivationRequest { tick } => {
            let _ = state
                .event_tx
                .send(HostEvent::ActivationRequest { client_id, tick })
                .await;
        }
        ControlMessage::PlayerInfoUpdate(request) => {
            let _ = state
                .event_tx
                .send(HostEvent::PlayerInfoUpdate { client_id, request })
                .await;
        }
        ControlMessage::Control(packet) => {
            if packet.client_id() != client_id {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: Some(client_id),
                        error: format!(
                            "control packet claimed client {}, but arrived on client {client_id}'s connection",
                            packet.client_id()
                        ),
                    })
                    .await;
                return;
            }
            ingest_control(packet, state).await;
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
    client_id: ClientId,
    reason: Option<String>,
    state: &mut HostState,
) {
    let disconnected = state.clients.remove(&client_id);
    if let Some(client) = &disconnected {
        state.pending_kinds.remove(&client.core.client_id);
    }
    let barrier_effects = state.status_barrier.remove_remote(client_id);
    let ready_batches = state
        .coordinator
        .remove_client(client_id)
        .unwrap_or_default();
    // Completed controls are immutable history. C++ removes the client only
    // from the future readiness set; its stored C4ClientIDAll packets remain
    // available to HandleControlReq and runtime joins.
    state.scheduler.remove_client(client_id);

    let _ = state
        .event_tx
        .send(HostEvent::ClientLeft { client_id })
        .await;

    for batch in ready_batches {
        publish_ready_batch(batch, state).await;
    }
    if let Some(client) = disconnected {
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
            reason: lc_engine::LegacyCString::from_bytes(b"Disconnected".to_vec())
                .unwrap_or_default(),
            by_client: 0,
        },
    )) else {
        return;
    };
    broadcast_packet(ControlDelivery::Sync, data, None, state).await;
}

async fn ingest_control(packet: ControlPacket, state: &mut HostState) {
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
    broadcast_control(&aggregated, state).await;
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
            for client in state.clients.values() {
                let _ = client
                    .outbound
                    .send(ControlMessage::Packet {
                        delivery,
                        data: data.clone(),
                    })
                    .await;
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
            if let Err(error) = authenticated_single_control(&data, expected_author) {
                let _ = state
                    .event_tx
                    .send(HostEvent::TransportError {
                        client_id: origin,
                        error,
                    })
                    .await;
                return;
            }
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
            let _ = state
                .event_tx
                .send(HostEvent::Direct {
                    client_id: origin.unwrap_or(BROADCAST_CLIENT_ID),
                    delivery,
                    data,
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
        lc_engine::ControlPacket::PlayerControl(data) => data.by_client,
        lc_engine::ControlPacket::SyncCheck(data) => data.by_client,
        lc_engine::ControlPacket::JoinPlayer(data) => data.by_client,
        lc_engine::ControlPacket::PlayerInfo(data) => data.by_client,
        lc_engine::ControlPacket::Unknown { .. } => {
            return Err("unsupported single control packet".to_string());
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
    apply_host_membership_controls(&controls, state);
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
    apply_host_membership_controls(&controls, state);
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

fn apply_host_membership_controls(controls: &[lc_engine::ControlPacket], state: &mut HostState) {
    for control in controls {
        if let lc_engine::ControlPacket::ClientRemove(remove) = control {
            if let Some(core) = state.client_cores.remove(&remove.client_id) {
                state.admission.remove_client_name(&core.name);
            }
            state.client_addresses.remove(&remove.client_id);
            state.resource_catalog.remove_at_client(remove.client_id);
            state.pending_kinds.remove(&remove.client_id);
        }
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

async fn apply_barrier_effects(effects: Vec<BarrierEffect>, state: &mut HostState) {
    let mut committed = false;
    for effect in effects {
        match effect {
            BarrierEffect::InvalidateReference
            | BarrierEffect::DriveControlTo(_)
            | BarrierEffect::StopControl
            | BarrierEffect::SetControlMode(_)
            | BarrierEffect::SweepUnjoinedPlayers
            | BarrierEffect::StartControl => {}
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
    client_id: ClientId,
    transport: crate::ControlTransport<S>,
    outbound_rx: mpsc::Receiver<ControlMessage>,
    host_tx: mpsc::Sender<HostLoopMessage>,
    liveness: ConnectionLivenessState,
}

impl<S> ClientTask<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    async fn run(mut self) {
        loop {
            let liveness_deadline = self.liveness.next_timer_at();
            tokio::select! {
                Some(message) = self.outbound_rx.recv() => {
                    if let Err(error) = self.transport.send_message(message).await {
                        let _ = self
                            .host_tx
                            .send(HostLoopMessage::ClientDisconnected {
                                client_id: self.client_id,
                                reason: Some(format!("send failed: {error}")),
                            })
                            .await;
                        break;
                    }
                }
                result = self.transport.read_message() => {
                    if let Ok(message) = &result {
                        self.liveness.record_inbound_message(message);
                    }
                    match result {
                        Ok(ControlMessage::Ping(packet)) => {
                            if let Err(error) = self
                                .transport
                                .send_message(ControlMessage::Pong(packet))
                                .await
                            {
                                let _ = self
                                    .host_tx
                                    .send(HostLoopMessage::ClientDisconnected {
                                        client_id: self.client_id,
                                        reason: Some(format!("pong send failed: {error}")),
                                    })
                                    .await;
                                break;
                            }
                        }
                        Ok(ControlMessage::Pong(packet)) => {
                            self.liveness.record_pong(packet);
                        }
                        Ok(message) => {
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ClientMessage {
                                    client_id: self.client_id,
                                    message,
                                })
                                .await;
                        }
                        Err(TransportError::Io(error)) if error.kind() == io::ErrorKind::UnexpectedEof => {
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ClientDisconnected {
                                    client_id: self.client_id,
                                    reason: None,
                                })
                                .await;
                            break;
                        }
                        Err(error) => {
                            let _ = self
                                .host_tx
                                .send(HostLoopMessage::ClientDisconnected {
                                    client_id: self.client_id,
                                    reason: Some(format!("read failed: {error}")),
                                })
                                .await;
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
                        let _ = self
                            .host_tx
                            .send(HostLoopMessage::ClientDisconnected {
                                client_id: self.client_id,
                                reason: Some(reason),
                            })
                            .await;
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
    let resource_epoch = tokio::time::Instant::now();
    let mut resource_timer = interval(Duration::from_millis(crate::NETWORK_TIMER_INTERVAL_MS));

    for packet in std::mem::take(&mut resource_state.initial_controls) {
        let key = (packet.client_id(), packet.tick());
        if received_controls.insert(key) {
            highest_received_tick =
                Some(highest_received_tick.map_or(packet.tick(), |tick| tick.max(packet.tick())));
            let _ = event_tx.send(ClientEvent::Ready { packet }).await;
        }
    }

    for packet in std::mem::take(&mut resource_state.initial_packets) {
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
                        match transport.send_message(ControlMessage::Control(packet)).await {
                            Ok(()) => backlog.record_packet(&clone),
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
                        if let Err(error) = transport
                            .send_message(ControlMessage::Packet { delivery, data })
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
                    ClientCommand::Shutdown => break,
                }
            }
            _ = resource_timer.tick() => {
                let actions = resource_state
                    .catalog
                    .on_timer(resource_epoch.elapsed().as_secs());
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
            result = transport.read_message() => {
                if let Ok(message) = &result {
                    resource_state.liveness.record_inbound_message(message);
                }
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
                    Ok(ControlMessage::ConnectionReply(_)) => {
                        let _ = event_tx
                            .send(ClientEvent::Disconnected {
                                reason: Some("host sent a duplicate connection reply".to_string()),
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
                    Ok(ControlMessage::Status(status)) => {
                        let _ = event_tx.send(ClientEvent::Status(status)).await;
                    }
                    Ok(ControlMessage::StatusAck(status)) => {
                        let _ = event_tx.send(ClientEvent::StatusAck(status)).await;
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
                        let _ = event_tx.send(ClientEvent::Ready { packet }).await;
                    }
                    Ok(ControlMessage::PlayerInfoUpdate(_)) => {
                        // PID_PlayerInfoUpdReq is accepted by the host only
                        // (src/C4Network2Players.cpp:405-411).
                    }
                    Ok(ControlMessage::Packet { delivery, data }) => {
                        match delivery {
                            ControlDelivery::Direct | ControlDelivery::Private => {
                                if let Ok(control) = decode_control_entry_payload(&data) {
                                    apply_client_membership(
                                        &mut client_addresses,
                                        &mut resource_state.catalog,
                                        &control,
                                    );
                                }
                                let _ = event_tx
                                    .send(ClientEvent::Direct { delivery, data })
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
                                apply_client_membership(
                                    &mut client_addresses,
                                    &mut resource_state.catalog,
                                    control,
                                );
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

fn apply_client_membership(
    client_addresses: &mut BTreeMap<i32, Vec<crate::NetworkAddress>>,
    resource_catalog: &mut crate::ResourceCatalog,
    control: &lc_engine::ControlPacket,
) {
    match control {
        lc_engine::ControlPacket::ClientJoin(join) => {
            client_addresses.entry(join.core.client_id).or_default();
        }
        lc_engine::ControlPacket::ClientRemove(remove) => {
            client_addresses.remove(&remove.client_id);
            resource_catalog.remove_at_client(remove.client_id);
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
    use lc_engine::{ControlPacket as EngineControlPacket, PlayerControlData};
    use std::future::{pending, ready};
    use std::time::Duration;
    use tokio::io::{duplex, AsyncReadExt};
    use tokio::time::timeout;

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
        let mut state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
        );

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
        let state = ClientResourceState::from_join_data(
            &join_data,
            0,
            Vec::new(),
            Vec::new(),
            ConnectionLivenessState::new_accepted_system(),
        );

        assert_eq!(state.catalog.discovery_packet().resource_ids, vec![8, 7]);
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

        client
            .submit_control(legacy_packet(1, 0, 0x12))
            .await
            .expect("submit client control");
        host.submit_local_control(legacy_packet(0, 0, 0x34))
            .await
            .expect("submit host control");

        let mut events = host.take_event_receiver();
        let mut saw_join = false;
        let mut saw_ready = false;

        for _ in 0..8 {
            if let Some(event) = tokio::time::timeout(EVENT_WAIT, events.recv())
                .await
                .expect("host event wait")
            {
                match event {
                    HostEvent::ClientJoined { .. } => saw_join = true,
                    HostEvent::Ready { packet } => {
                        saw_ready = true;
                        assert_eq!(packet.tick(), 0);
                        assert_eq!(packet.client_id(), BROADCAST_CLIENT_ID);
                        assert_eq!(control_commands(&packet), vec![0x34, 0x12]);
                    }
                    _ => {}
                }
                if saw_join && saw_ready {
                    break;
                }
            }
        }

        assert!(saw_join, "host did not report client join");
        assert!(saw_ready, "host did not emit ready packet");

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
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
        let (outbound_tx, outbound_rx) = mpsc::channel(4);
        let (host_tx, mut host_rx) = mpsc::channel(4);
        let task = tokio::spawn(
            ClientTask {
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
            let echoed = timeout(EVENT_WAIT, transport.read_message())
                .await
                .expect("client must re-announce a newly learned address")
                .unwrap();
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
    async fn host_rejects_a_client_control_that_claims_another_client_slot() {
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
            ClientConfig::new("spoof-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        client
            .submit_control(legacy_packet(HOST_CLIENT_ID, 0, 0x66))
            .await
            .expect("submit spoofed host control");
        host.submit_local_control(legacy_packet(HOST_CLIENT_ID, 0, 0x11))
            .await
            .expect("submit real host control");
        client
            .submit_control(legacy_packet(client.client_id(), 0, 0x22))
            .await
            .expect("submit real client control");

        let mut saw_rejection = false;
        let ready = loop {
            match timeout(EVENT_WAIT, host_events.recv())
                .await
                .expect("host event wait")
            {
                Some(HostEvent::TransportError {
                    client_id: Some(rejected_id),
                    error,
                }) => {
                    assert_eq!(rejected_id, client.client_id());
                    assert!(error.contains("claimed client 0"));
                    saw_rejection = true;
                }
                Some(HostEvent::Ready { packet }) => break packet,
                Some(_) => continue,
                None => panic!("host event stream ended before ready"),
            }
        };
        assert!(saw_rejection, "spoofed contribution was not rejected");
        assert_eq!(
            control_commands(&ready),
            vec![0x11, 0x22],
            "the authenticated host slot must not be replaced by client data"
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
            .payload(vec![0xDE, 0xAD, 0xBE, 0xEF]);
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
                | ClientEvent::ResourceAction(_)
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
    async fn client_emits_a_complete_tick_only_once_when_host_retransmits_it() {
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
        let complete = legacy_packet(BROADCAST_CLIENT_ID, 5, 0x44);

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
                | Ok(Some(HostEvent::TransportError { .. })) => continue,
                Ok(Some(HostEvent::Direct { .. }))
                | Ok(Some(HostEvent::JoinDataNeeded { .. }))
                | Ok(Some(HostEvent::ExecSync { .. }))
                | Ok(Some(HostEvent::ActivationRequest { .. }))
                | Ok(Some(HostEvent::PlayerInfoUpdate { .. }))
                | Ok(Some(HostEvent::ResourceAction(_)))
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
                Ok(Some(ClientEvent::ResourceAction(_))) => continue,
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
