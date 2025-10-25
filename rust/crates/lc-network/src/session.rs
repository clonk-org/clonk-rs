use std::collections::BTreeMap;
use std::io;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::interval;

use crate::{
    ClientId, ControlBacklog, ControlCoordinator, ControlDelivery, ControlMessage, ControlOutcome,
    ControlPacket, MissingRange, ParticipantKind, ReadyBatch, ResyncScheduler, Tick,
    TransportError,
};

const PROTOCOL_VERSION: u32 = 1;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const CLIENT_BACKLOG_LIMIT: usize = 256;

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
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(ClientError::Connect)?;
    stream.set_nodelay(true).ok();

    let ClientConfig { name, kind } = config;
    let request = HandshakeRequest {
        version: PROTOCOL_VERSION,
        name,
        kind,
    };

    write_handshake_request(&mut stream, &request)
        .await
        .map_err(|error| ClientError::Handshake(format!("failed to send handshake: {error}")))?;

    let response = read_handshake_response(&mut stream)
        .await
        .map_err(|error| ClientError::Handshake(format!("failed to read handshake: {error}")))?;

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
    let mut state = HostState {
        coordinator: ControlCoordinator::with_start_tick(backlog_limit, config.start_tick),
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
        ControlMessage::Control(packet) => {
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
    state.coordinator.remove_client(client_id).ok();
    state.backlog.remove_client(client_id);
    state.scheduler.remove_client(client_id);

    let _ = state
        .event_tx
        .send(HostEvent::ClientLeft { client_id })
        .await;

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
    if client_id != BROADCAST_CLIENT_ID {
        if let Err(error) = state.coordination_register(client_id) {
            let _ = state
                .event_tx
                .send(HostEvent::TransportError {
                    client_id: Some(client_id),
                    error: error.to_string(),
                })
                .await;
            return;
        }
    }

    match state.coordinator.ingest(packet) {
        Ok(ControlOutcome { ready, missing, .. }) => {
            for batch in ready {
                state.backlog.record_ready_batch(&batch);
                let aggregated = aggregate_batch(&batch);
                broadcast_control(&aggregated, state).await;
                let _ = state
                    .event_tx
                    .send(HostEvent::Ready { packet: aggregated })
                    .await;
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
    if let Some(client) = state.clients.get(&client_id) {
        for packet in resend {
            let _ = client.outbound.send(ControlMessage::Control(packet)).await;
        }
    }
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

fn aggregate_packets_for_tick(packets: &[ControlPacket], tick: Tick) -> ControlPacket {
    let mut payload = Vec::new();
    let mut timestamp = 0;
    for packet in packets {
        payload.extend_from_slice(packet.payload());
        timestamp = timestamp.max(packet.timestamp_ms());
    }
    ControlPacket::builder(BROADCAST_CLIENT_ID, tick)
        .timestamp_ms(timestamp)
        .payload(payload)
}

fn aggregate_batch(batch: &ReadyBatch) -> ControlPacket {
    aggregate_packets_for_tick(batch.packets(), batch.tick())
}

async fn replay_backlog_to_client(
    backlog: &ControlBacklog,
    from_tick: Tick,
    outbound: &mpsc::Sender<ControlMessage>,
) -> Result<(), String> {
    let replay = backlog.packets_from(from_tick);
    for (tick, packets) in replay {
        let aggregated = aggregate_packets_for_tick(&packets, tick);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ParticipantKind;
    use std::time::Duration;
    use tokio::io::duplex;
    use tokio::time::timeout;

    #[tokio::test(flavor = "multi_thread")]
    async fn host_emits_ready_for_single_client() {
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
        if let Ok(Some(ClientEvent::ExecSync { .. })) =
            tokio::time::timeout(Duration::from_millis(200), client_events.recv()).await
        {
            // expected
        }

        let packet = ControlPacket::builder(1, 0)
            .timestamp_ms(0)
            .payload(vec![0x01, 0x02, 0x03]);
        client.submit_control(packet).await.expect("submit control");

        let mut events = host.take_event_receiver();
        let mut saw_join = false;
        let mut saw_ready = false;

        for _ in 0..8 {
            if let Some(event) = tokio::time::timeout(Duration::from_secs(1), events.recv())
                .await
                .expect("host event wait")
            {
                match event {
                    HostEvent::ClientJoined { .. } => saw_join = true,
                    HostEvent::Ready { packet } => {
                        saw_ready = true;
                        assert_eq!(packet.tick(), 0);
                        assert_eq!(packet.payload(), &[0x01, 0x02, 0x03]);
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

        submit_control_pair(&mut host, &client, 0, vec![0xAA], vec![0x11]).await;

        let first_host_ready = wait_for_host_ready(&mut host_events, Duration::from_secs(1)).await;
        assert_eq!(first_host_ready.tick(), 0);

        let first_client_ready =
            wait_for_client_ready(&mut client_events, Duration::from_secs(1)).await;
        assert_eq!(first_client_ready.tick(), 0);

        client.shutdown().await.expect("client shutdown");
        wait_for_client_departure(&mut host_events, Duration::from_secs(1)).await;

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect second client");
        let mut client_beta_events = client_beta.take_event_receiver();
        drain_initial_exec_sync(&mut client_beta_events).await;

        submit_control_pair(&mut host, &client_beta, 1, vec![0xBB], vec![0x22]).await;

        let second_host_ready = wait_for_host_ready(&mut host_events, Duration::from_secs(1)).await;
        assert_eq!(second_host_ready.tick(), 1);

        let second_client_ready =
            wait_for_client_ready(&mut client_beta_events, Duration::from_secs(1)).await;
        assert_eq!(second_client_ready.tick(), 1);

        client_beta
            .shutdown()
            .await
            .expect("second client shutdown");
        wait_for_client_departure(&mut host_events, Duration::from_secs(1)).await;

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

        submit_control_pair(&mut host, &client_alpha, 0, vec![0xA1], vec![0xB2]).await;
        let ready_packet = wait_for_host_ready(&mut host_events, Duration::from_secs(1)).await;
        assert_eq!(ready_packet.tick(), 0);
        let expected_payload = ready_packet.payload().to_vec();

        let mut client_beta =
            connect_client(addr, ClientConfig::new("Beta", ParticipantKind::Player))
                .await
                .expect("connect beta client");
        let mut beta_events = client_beta.take_event_receiver();

        let mut backlog_packets = Vec::new();
        let mut saw_exec_sync = false;
        for _ in 0..4 {
            match timeout(Duration::from_secs(1), beta_events.recv())
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

        client_beta.shutdown().await.expect("beta shutdown");
        wait_for_client_departure(&mut host_events, Duration::from_secs(1)).await;

        client_alpha.shutdown().await.expect("alpha shutdown");
        wait_for_client_departure(&mut host_events, Duration::from_secs(1)).await;

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

    async fn submit_control_pair(
        host: &mut HostHandle,
        client: &ClientHandle,
        tick: Tick,
        host_payload: Vec<u8>,
        client_payload: Vec<u8>,
    ) {
        let host_packet = ControlPacket::builder(0, tick)
            .timestamp_ms(0)
            .payload(host_payload);
        host.submit_local_control(host_packet)
            .await
            .expect("host submit control");

        let client_packet = ControlPacket::builder(client.client_id(), tick)
            .timestamp_ms(0)
            .payload(client_payload);
        client
            .submit_control(client_packet)
            .await
            .expect("client submit control");
    }

    async fn drain_initial_exec_sync(events: &mut mpsc::Receiver<ClientEvent>) {
        if let Ok(Some(ClientEvent::ExecSync { .. })) =
            timeout(Duration::from_millis(200), events.recv()).await
        {
            // expected initial sync packet
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
                Ok(Some(HostEvent::ClientLeft { .. })) => continue,
                Ok(Some(HostEvent::TransportError { error, .. })) => {
                    panic!("host reported transport error: {error}");
                }
                Ok(Some(HostEvent::Direct { .. })) | Ok(Some(HostEvent::ExecSync { .. })) => {
                    continue
                }
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

async fn run_client(
    mut stream: TcpStream,
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

    'outer: loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => break,
            Some(command) = commands.recv() => {
                match command {
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
                    Ok(ControlMessage::Control(packet)) => {
                        let _ = event_tx.send(ClientEvent::Ready { packet }).await;
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
