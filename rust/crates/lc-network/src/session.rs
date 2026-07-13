use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

use crate::legacy::{aggregate_control_packets_for_tick, validate_control_envelope};
use crate::{
    aggregate_ready_batch, ClientId, ControlBacklog, ControlCoordinator, ControlDelivery,
    ControlMessage, ControlOutcome, ControlPacket, MissingRange, ParticipantKind, ReadyBatch,
    ResyncScheduler, Tick, TransportError,
};

const PROTOCOL_VERSION: u32 = 1;
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
}

impl Default for HostConfig {
    fn default() -> Self {
        Self {
            backlog_limit: 256,
            resync_interval: Duration::from_millis(200),
            resync_cooldown: Duration::from_secs(2),
            max_players: 8,
            start_tick: 0,
        }
    }
}

/// Client metadata supplied during handshake.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub name: String,
    pub kind: ParticipantKind,
}

impl ClientConfig {
    pub fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        Self {
            name: name.into(),
            kind,
        }
    }
}

/// Events emitted by the host loop.
#[derive(Debug)]
pub enum HostEvent {
    PlayerInfoUpdate {
        client_id: ClientId,
        request: crate::PlayerInfoUpdateRequest,
    },
    ClientJoined {
        client_id: ClientId,
        name: String,
        kind: ParticipantKind,
    },
    ClientLeft {
        client_id: ClientId,
    },
    Ready {
        packet: ControlPacket,
    },
    Direct {
        client_id: ClientId,
        delivery: ControlDelivery,
        data: Vec<u8>,
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
    SubmitLocal(ControlPacket),
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
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
    let mut stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connection)
        .await
        .map_err(|_| {
            ClientError::Connect(io::Error::new(
                io::ErrorKind::TimedOut,
                "connection attempt timed out",
            ))
        })?
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();

    let ClientConfig { name, kind } = config;
    let request = HandshakeRequest {
        version: PROTOCOL_VERSION,
        name,
        kind,
    };

    let response = tokio::time::timeout(HANDSHAKE_TIMEOUT, async {
        write_handshake_request(&mut stream, &request)
            .await
            .map_err(|error| {
                ClientError::Handshake(format!("failed to send handshake: {error}"))
            })?;
        read_handshake_response(&mut stream)
            .await
            .map_err(|error| ClientError::Handshake(format!("failed to read handshake: {error}")))
    })
    .await
    .map_err(|_| ClientError::Handshake("host handshake timed out".to_string()))??;

    if response.version != PROTOCOL_VERSION {
        return Err(ClientError::Handshake(format!(
            "host protocol {} incompatible with client {PROTOCOL_VERSION}",
            response.version
        )));
    }

    let client_id = response.client_id;

    let (command_tx, command_rx) = mpsc::channel::<ClientCommand>(64);
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>(64);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let join_handle = tokio::spawn(run_client(stream, command_rx, event_tx, shutdown_rx));

    Ok(ClientHandle {
        command_tx,
        event_rx: Some(event_rx),
        shutdown_tx: Some(shutdown_tx),
        join_handle,
        client_id,
    })
}

/// Events observed by a connected client.
#[derive(Debug)]
pub enum ClientEvent {
    Ready {
        packet: ControlPacket,
    },
    Direct {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
    },
    Disconnected {
        reason: Option<String>,
    },
}

/// Commands available to a connected client.
#[derive(Debug)]
pub enum ClientCommand {
    SubmitPlayerInfoUpdate(crate::PlayerInfoUpdateRequest),
    SubmitControl(ControlPacket),
    SubmitPacket {
        delivery: ControlDelivery,
        data: Vec<u8>,
    },
    ExecSync {
        control_tick: Tick,
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

    pub async fn submit_player_info_update(
        &self,
        request: crate::PlayerInfoUpdateRequest,
    ) -> Result<(), ClientError> {
        self.command_tx
            .send(ClientCommand::SubmitPlayerInfoUpdate(request))
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
    _name: String,
    _kind: ParticipantKind,
}

#[derive(Debug)]
enum HostLoopMessage {
    ClientMessage {
        client_id: ClientId,
        message: ControlMessage,
    },
    ClientDisconnected {
        client_id: ClientId,
        reason: Option<String>,
    },
}

#[derive(Debug)]
struct HostState {
    config: HostConfig,
    coordinator: ControlCoordinator,
    backlog: ControlBacklog,
    scheduler: ResyncScheduler,
    clients: BTreeMap<ClientId, ClientConnection>,
    next_client_id: ClientId,
    event_tx: mpsc::Sender<HostEvent>,
}

async fn run_host(
    listener: TcpListener,
    config: HostConfig,
    mut commands: mpsc::Receiver<HostCommand>,
    event_tx: mpsc::Sender<HostEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
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
    let mut state = HostState {
        coordinator,
        backlog: ControlBacklog::new(backlog_limit),
        scheduler: ResyncScheduler::new(config.resync_cooldown),
        clients: BTreeMap::new(),
        next_client_id: 1,
        event_tx: event_tx.clone(),
        config,
    };

    let (client_tx, mut client_rx) = mpsc::channel::<HostLoopMessage>(128);
    let mut resync_timer = interval(state.config.resync_interval);

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                break;
            }
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((stream, addr)) => {
                        if let Err(error) = handle_accept(&mut state, stream, addr, client_tx.clone()).await {
                            let _ = state.event_tx
                                .send(HostEvent::TransportError {
                                    client_id: None,
                                    error,
                                })
                                .await;
                        }
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
            Some(message) = client_rx.recv() => {
                match message {
                    HostLoopMessage::ClientMessage { client_id, message } => {
                        handle_client_message(client_id, message, &mut state).await;
                    }
                    HostLoopMessage::ClientDisconnected { client_id, reason } => {
                        handle_client_disconnected(client_id, reason, &mut state).await;
                    }
                }
            }
            Some(command) = commands.recv() => {
                match command {
                    HostCommand::SubmitLocal(packet) => ingest_control(packet, &mut state).await,
                    HostCommand::SubmitPacket { delivery, data } => broadcast_packet(delivery, data, None, &mut state).await,
                    HostCommand::ExecSync { control_tick } => broadcast_exec_sync(control_tick, &mut state).await,
                    HostCommand::Shutdown => break,
                }
            }
            _ = resync_timer.tick() => {
                request_missing_controls(&mut state).await;
            }
        }
    }

    for (client_id, client) in state.clients.iter() {
        let _ = client
            .outbound
            .send(ControlMessage::ExecSync {
                control_tick: state.coordinator.current_tick(),
            })
            .await;
        let _ = state
            .event_tx
            .send(HostEvent::ClientLeft {
                client_id: *client_id,
            })
            .await;
    }
}

async fn handle_accept(
    state: &mut HostState,
    mut stream: TcpStream,
    addr: SocketAddr,
    host_tx: mpsc::Sender<HostLoopMessage>,
) -> Result<(), String> {
    stream
        .set_nodelay(true)
        .map_err(|error| format!("failed to configure connection {addr}: {error}"))?;

    let handshake = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_handshake_request(&mut stream))
        .await
        .map_err(|_| "client handshake timed out".to_string())?
        .map_err(|error| format!("failed to read handshake: {error}"))?;

    if handshake.version != PROTOCOL_VERSION {
        return Err(format!(
            "client protocol {} incompatible with host {PROTOCOL_VERSION}",
            handshake.version
        ));
    }

    let client_id = state.next_client_id;
    state.next_client_id = state.next_client_id.saturating_add(1);

    let response = HandshakeResponse {
        version: PROTOCOL_VERSION,
        client_id,
        start_tick: state.coordinator.current_tick(),
    };

    write_handshake_response(&mut stream, &response)
        .await
        .map_err(|error| format!("failed to write handshake: {error}"))?;

    let (outbound_tx, outbound_rx) = mpsc::channel::<ControlMessage>(64);
    let transport = crate::ControlTransport::new(stream);

    let client_task = ClientTask {
        client_id,
        transport,
        outbound_rx,
        host_tx: host_tx.clone(),
    };
    tokio::spawn(client_task.run());

    state
        .coordination_register(client_id)
        .map_err(|error| error.to_string())?;

    replay_backlog_to_client(&state.backlog, state.config.start_tick, &outbound_tx)
        .await
        .map_err(|error| format!("failed to replay backlog: {error}"))?;

    state.clients.insert(
        client_id,
        ClientConnection {
            outbound: outbound_tx.clone(),
            _name: handshake.name.clone(),
            _kind: handshake.kind,
        },
    );

    let _ = state
        .event_tx
        .send(HostEvent::ClientJoined {
            client_id,
            name: handshake.name,
            kind: handshake.kind,
        })
        .await;

    let _ = outbound_tx
        .send(ControlMessage::ExecSync {
            control_tick: state.coordinator.current_tick(),
        })
        .await;

    Ok(())
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
        ControlMessage::Status(_) | ControlMessage::StatusAck(_) => {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: "status handling is not initialized for this session".to_string(),
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
            broadcast_exec_sync(control_tick, state).await;
        }
    }
}

async fn handle_client_disconnected(
    client_id: ClientId,
    reason: Option<String>,
    state: &mut HostState,
) {
    state.clients.remove(&client_id);
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
        ControlDelivery::Queue | ControlDelivery::Sync | ControlDelivery::Decide => {
            let tick = state.coordinator.current_tick();
            let client_id = origin.unwrap_or(BROADCAST_CLIENT_ID);
            let packet = ControlPacket::builder(client_id, tick)
                .timestamp_ms(0)
                .payload(data.clone());
            ingest_control(packet, state).await;
        }
        ControlDelivery::Direct | ControlDelivery::Private => {
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

async fn broadcast_exec_sync(control_tick: Tick, state: &mut HostState) {
    for client in state.clients.values() {
        let _ = client
            .outbound
            .send(ControlMessage::ExecSync { control_tick })
            .await;
    }
    let _ = state
        .event_tx
        .send(HostEvent::ExecSync { control_tick })
        .await;
}

async fn replay_backlog_to_client(
    backlog: &ControlBacklog,
    from_tick: Tick,
    outbound: &mpsc::Sender<ControlMessage>,
) -> Result<(), String> {
    let replay = backlog.packets_from(from_tick);
    for (tick, packets) in replay {
        // Runtime join reuses the same complete-packet construction as live
        // readiness; C++ retrieves/sends its stored C4ClientIDAll packet rather
        // than concatenating serialized per-client frames
        // (src/C4GameControlNetwork.cpp:531-544,741-777).
        let aggregated = aggregate_control_packets_for_tick(tick, &packets)
            .map_err(|error| format!("failed to aggregate backlog tick {tick}: {error}"))?;
        outbound
            .send(ControlMessage::Control(aggregated))
            .await
            .map_err(|error| format!("client disconnected during backlog replay: {error}"))?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
struct HandshakeRequest {
    version: u32,
    name: String,
    kind: ParticipantKind,
}

#[derive(Debug, Serialize, Deserialize)]
struct HandshakeResponse {
    version: u32,
    client_id: ClientId,
    start_tick: Tick,
}

async fn read_handshake_request(stream: &mut TcpStream) -> Result<HandshakeRequest, io::Error> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

async fn write_handshake_response(
    stream: &mut TcpStream,
    response: &HandshakeResponse,
) -> Result<(), io::Error> {
    let payload = serde_json::to_vec(response)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let len = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&payload).await?;
    Ok(())
}

async fn write_handshake_request(
    stream: &mut TcpStream,
    request: &HandshakeRequest,
) -> Result<(), io::Error> {
    let payload = serde_json::to_vec(request)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let len = (payload.len() as u32).to_be_bytes();
    stream.write_all(&len).await?;
    stream.write_all(&payload).await?;
    Ok(())
}

async fn read_handshake_response(stream: &mut TcpStream) -> Result<HandshakeResponse, io::Error> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await?;
    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

struct ClientTask {
    client_id: ClientId,
    transport: crate::ControlTransport<TcpStream>,
    outbound_rx: mpsc::Receiver<ControlMessage>,
    host_tx: mpsc::Sender<HostLoopMessage>,
}

impl ClientTask {
    async fn run(mut self) {
        loop {
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
                    match result {
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
            }
        }
    }
}

async fn run_client(
    stream: TcpStream,
    commands: mpsc::Receiver<ClientCommand>,
    event_tx: mpsc::Sender<ClientEvent>,
    shutdown_rx: oneshot::Receiver<()>,
) {
    stream.set_nodelay(true).ok();
    let transport = crate::ControlTransport::new(stream);
    run_client_loop(transport, commands, event_tx, shutdown_rx).await;
}

async fn run_client_loop<S>(
    mut transport: crate::ControlTransport<S>,
    mut commands: mpsc::Receiver<ClientCommand>,
    event_tx: mpsc::Sender<ClientEvent>,
    mut shutdown_rx: oneshot::Receiver<()>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut backlog = ControlBacklog::new(CLIENT_BACKLOG_LIMIT);
    let mut received_controls = BTreeSet::<(ClientId, Tick)>::new();
    let mut highest_received_tick = None::<Tick>;

    'outer: loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break,
            Some(command) = commands.recv() => {
                match command {
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
                    ClientCommand::Shutdown => break,
                }
            }
            result = transport.read_message() => {
                match result {
                    Ok(ControlMessage::Status(_)) | Ok(ControlMessage::StatusAck(_)) => {
                        // Session-level status transitions are wired after the
                        // exact transport codec is established.
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
                        let _ = event_tx.send(ClientEvent::Direct { delivery, data }).await;
                    }
                    Ok(ControlMessage::ExecSync { control_tick }) => {
                        let _ = event_tx.send(ClientEvent::ExecSync { control_tick }).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        decode_control_packet, encode_control_packet, LegacyControlFrame, ParticipantKind,
    };
    use lc_engine::{ControlPacket as EngineControlPacket, PlayerControlData};
    use std::future::{pending, ready};
    use std::time::Duration;
    use tokio::io::duplex;
    use tokio::time::timeout;

    /// Upper bound for a single event wait. Generous so loaded parallel test
    /// runs do not trip it; a genuine failure still fails fast because the
    /// expected event never arrives at all.
    const EVENT_WAIT: Duration = Duration::from_secs(5);

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
            connect_client_from(
                ready(Ok(client_stream)),
                ClientConfig::new("Alice", ParticipantKind::Player),
            ),
        )
        .await;

        match result {
            Ok(Err(ClientError::Handshake(message))) => {
                assert_eq!(message, "host handshake timed out");
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

        let mut client = connect_client(addr, ClientConfig::new("Alice", ParticipantKind::Player))
            .await
            .expect("connect client");

        // Drain the initial exec sync event sent to the client.
        let mut client_events = client.take_event_receiver();
        drain_initial_exec_sync(&mut client_events).await;

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
    async fn host_rejects_a_client_control_that_claims_another_client_slot() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind listener");
        let addr = listener.local_addr().expect("local addr");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let mut client = connect_client(
            addr,
            ClientConfig::new("spoof-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let mut client_events = client.take_event_receiver();
        drain_initial_exec_sync(&mut client_events).await;

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
        let mut client = connect_client(
            addr,
            ClientConfig::new("validation-check", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let mut client_events = client.take_event_receiver();
        drain_initial_exec_sync(&mut client_events).await;

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
        let mut host = start_host(
            listener,
            HostConfig {
                max_players: 4,
                ..Default::default()
            },
        )
        .await
        .expect("start host");

        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        let mut client_events = client.take_event_receiver();
        drain_initial_exec_sync(&mut client_events).await;

        submit_control_pair(&mut host, &client, 0, 0xAA, 0x11).await;

        let first_host_ready = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(first_host_ready.tick(), 0);

        let first_client_ready = wait_for_client_ready(&mut client_events, EVENT_WAIT).await;
        assert_eq!(first_client_ready.tick(), 0);

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect second client");
        let mut client_beta_events = client_beta.take_event_receiver();
        drain_initial_exec_sync(&mut client_beta_events).await;

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

        let mut client = connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
            .await
            .expect("connect client");

        let mut host_events = host.take_event_receiver();
        let mut client_events = client.take_event_receiver();
        drain_initial_exec_sync(&mut client_events).await;

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
    async fn new_client_replays_backlog_on_join() {
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

        let mut host_events = host.take_event_receiver();
        let mut client_alpha =
            connect_client(addr, ClientConfig::new("Alpha", ParticipantKind::Player))
                .await
                .expect("connect alpha client");
        let mut alpha_events = client_alpha.take_event_receiver();
        drain_initial_exec_sync(&mut alpha_events).await;

        submit_control_pair(&mut host, &client_alpha, 0, 0xA1, 0xB2).await;
        let ready_packet = wait_for_host_ready(&mut host_events, EVENT_WAIT).await;
        assert_eq!(ready_packet.tick(), 0);
        let expected_payload = ready_packet.payload().to_vec();

        // Completed controls stay available after their originating client
        // departs. C++ stores the complete C4ClientIDAll packet and serves it
        // from HandleControlReq without rewriting history when a client is
        // removed (src/C4GameControlNetwork.cpp:531-544,615-629).
        client_alpha.shutdown().await.expect("alpha shutdown");
        wait_for_client_departure(&mut host_events, EVENT_WAIT).await;

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect beta client");
        let mut beta_events = client_beta.take_event_receiver();

        let mut backlog_packets = Vec::new();
        let mut saw_exec_sync = false;
        for _ in 0..4 {
            match timeout(EVENT_WAIT, beta_events.recv())
                .await
                .expect("beta event wait")
            {
                Some(ClientEvent::Ready { packet }) => backlog_packets.push(packet),
                Some(ClientEvent::ExecSync { control_tick }) => {
                    assert_eq!(control_tick, 1);
                    saw_exec_sync = true;
                    break;
                }
                Some(ClientEvent::Direct { .. }) => continue,
                Some(ClientEvent::Disconnected { reason }) => {
                    panic!("beta disconnected unexpectedly: {reason:?}");
                }
                None => panic!("beta event stream ended unexpectedly"),
            }
        }

        assert!(saw_exec_sync, "beta client never received exec sync");
        assert_eq!(
            backlog_packets.len(),
            1,
            "beta client did not receive backlog packet"
        );
        assert_eq!(backlog_packets[0].tick(), ready_packet.tick());
        assert_eq!(backlog_packets[0].payload(), expected_payload);
        assert_eq!(backlog_packets[0].client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(control_commands(&backlog_packets[0]), vec![0xA1, 0xB2]);

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
                | ClientEvent::ExecSync { .. } => continue,
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

    /// Consumes client events up to and including the initial exec sync that
    /// the host sends on join (see `handle_accept`). Backlog replays may
    /// legitimately precede it, so loop instead of guessing at timings.
    async fn drain_initial_exec_sync(events: &mut mpsc::Receiver<ClientEvent>) {
        loop {
            match timeout(EVENT_WAIT, events.recv()).await {
                Ok(Some(ClientEvent::ExecSync { .. })) => break,
                Ok(Some(ClientEvent::Ready { .. })) | Ok(Some(ClientEvent::Direct { .. })) => {
                    continue
                }
                Ok(Some(ClientEvent::Disconnected { reason })) => {
                    panic!("client disconnected before initial exec sync: {reason:?}")
                }
                Ok(None) => panic!("client event stream ended before initial exec sync"),
                Err(_) => panic!("timed out waiting for initial exec sync"),
            }
        }
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
                | Ok(Some(HostEvent::ExecSync { .. }))
                | Ok(Some(HostEvent::PlayerInfoUpdate { .. })) => continue,
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
