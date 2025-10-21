use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use lc_engine::{
    interpret_player_control_command, ControlButton, ControlEvent, PlayerControlData,
    COM_CLEAR_PRESSED_COMS, COM_DOWN, COM_LEFT, COM_RELEASE_OFFSET, COM_RIGHT, COM_UP,
};
use lc_network::{
    connect_client, decode_control_packet, encode_control_packet, start_host, ClientConfig,
    ClientEvent, ClientHandle, ClientId, ControlPacket, HostConfig, HostEvent, HostHandle,
    LegacyControlFrame, ParticipantKind, Tick,
};
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::mpsc as tokio_mpsc;

#[derive(Debug, Clone)]
pub enum NetworkMode {
    Host(HostSettings),
    Client(ClientSettings),
}

#[derive(Debug, Clone)]
pub struct HostSettings {
    pub bind_addr: SocketAddr,
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub server_addr: SocketAddr,
    pub player_name: String,
}

#[derive(Debug)]
pub struct NetworkManager {
    command_tx: tokio_mpsc::Sender<NetworkCommand>,
    event_rx: Receiver<NetworkEvent>,
    worker: Option<thread::JoinHandle<()>>,
}

#[derive(Debug)]
pub enum NetworkEvent {
    Control {
        owner: i32,
        event: ControlEvent,
    },
    PeerConnected {
        client_id: ClientId,
    },
    PeerDisconnected {
        client_id: ClientId,
        reason: Option<String>,
    },
    Error(String),
}

#[derive(Debug)]
enum NetworkCommand {
    SubmitLocal {
        owner: i32,
        event: ControlEvent,
        tick: Tick,
    },
    Shutdown,
}

enum WorkerMode {
    Host {
        settings: HostSettings,
        local_owner: i32,
    },
    Client {
        settings: ClientSettings,
        local_owner: i32,
    },
}

impl NetworkManager {
    pub fn for_mode(mode: NetworkMode, local_owner: i32) -> Result<Self> {
        let worker_mode = match mode {
            NetworkMode::Host(settings) => WorkerMode::Host {
                settings,
                local_owner,
            },
            NetworkMode::Client(settings) => WorkerMode::Client {
                settings,
                local_owner,
            },
        };
        Self::spawn(worker_mode)
    }

    fn spawn(mode: WorkerMode) -> Result<Self> {
        let (command_tx, command_rx) = tokio_mpsc::channel(128);
        let (event_tx, event_rx) = mpsc::channel();
        let thread_name = match mode {
            WorkerMode::Host { .. } => "lc-network-host",
            WorkerMode::Client { .. } => "lc-network-client",
        };
        let worker = thread::Builder::new()
            .name(thread_name.to_string())
            .spawn(move || {
                let runtime = RuntimeBuilder::new_multi_thread()
                    .enable_all()
                    .build()
                    .expect("failed to initialise tokio runtime");
                if let Err(err) = runtime.block_on(run_worker(mode, command_rx, event_tx.clone())) {
                    let _ = event_tx.send(NetworkEvent::Error(format!("{err:?}")));
                }
            })
            .context("failed to spawn network worker thread")?;

        Ok(Self {
            command_tx,
            event_rx,
            worker: Some(worker),
        })
    }

    pub fn submit_local_control(&self, owner: i32, event: ControlEvent, tick: Tick) {
        let command = NetworkCommand::SubmitLocal { owner, event, tick };
        let _ = self.command_tx.blocking_send(command);
    }

    pub fn poll_events(&mut self) -> Vec<NetworkEvent> {
        let mut events = Vec::new();
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        events
    }
}

impl Drop for NetworkManager {
    fn drop(&mut self) {
        let _ = self.command_tx.blocking_send(NetworkCommand::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

async fn run_worker(
    mode: WorkerMode,
    mut command_rx: tokio_mpsc::Receiver<NetworkCommand>,
    event_tx: Sender<NetworkEvent>,
) -> Result<()> {
    match mode {
        WorkerMode::Host {
            settings,
            local_owner,
        } => run_host_worker(settings, local_owner, &mut command_rx, event_tx).await,
        WorkerMode::Client {
            settings,
            local_owner,
        } => run_client_worker(settings, local_owner, &mut command_rx, event_tx).await,
    }
}

async fn run_host_worker(
    settings: HostSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    event_tx: Sender<NetworkEvent>,
) -> Result<()> {
    let listener = TcpListener::bind(settings.bind_addr)
        .await
        .with_context(|| format!("failed to bind host socket at {}", settings.bind_addr))?;
    let host_config = HostConfig {
        backlog_limit: 256,
        max_players: 8,
        resync_interval: Duration::from_millis(200),
        resync_cooldown: Duration::from_secs(2),
        start_tick: 0,
    };
    let mut host = start_host(listener, host_config)
        .await
        .context("failed to start host session")?;
    let mut host_events = host.take_event_receiver();

    loop {
        tokio::select! {
            maybe_event = host_events.recv() => {
                match maybe_event {
                    Some(event) => handle_host_event(event, local_owner, &event_tx).await?,
                    None => {
                        return Err(anyhow!("host event stream ended"));
                    }
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        submit_local_control(&mut host, owner, event, tick, 0, &event_tx).await?;
                    }
                    NetworkCommand::Shutdown => break,
                }
            }
            else => break,
        }
    }

    host.shutdown().await.ok();
    Ok(())
}

async fn handle_host_event(
    event: HostEvent,
    local_owner: i32,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    match event {
        HostEvent::Ready { packet } => {
            handle_ready_packet(packet, local_owner, event_tx)?;
        }
        HostEvent::ClientJoined { client_id, .. } => {
            let _ = event_tx.send(NetworkEvent::PeerConnected { client_id });
        }
        HostEvent::ClientLeft { client_id } => {
            let _ = event_tx.send(NetworkEvent::PeerDisconnected {
                client_id,
                reason: None,
            });
        }
        HostEvent::TransportError { client_id, error } => {
            let prefix = client_id
                .map(|id| format!("client {id}: "))
                .unwrap_or_default();
            let _ = event_tx.send(NetworkEvent::Error(format!("{prefix}{error}")));
        }
        HostEvent::Direct { .. } | HostEvent::ExecSync { .. } => {
            // Ignored for now; these can be surfaced later if needed.
        }
    }
    Ok(())
}

async fn run_client_worker(
    settings: ClientSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    event_tx: Sender<NetworkEvent>,
) -> Result<()> {
    let mut client = connect_client(
        settings.server_addr,
        ClientConfig::new(settings.player_name, ParticipantKind::Player),
    )
    .await
    .context("failed to connect to host")?;
    let client_id = client.client_id();
    let mut client_events = client.take_event_receiver();

    loop {
        tokio::select! {
            maybe_event = client_events.recv() => {
                match maybe_event {
                    Some(event) => handle_client_event(event, local_owner, client_id, &event_tx).await?,
                    None => {
                        return Err(anyhow!("client event stream ended"));
                    }
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        submit_client_control(&client, owner, event, tick, client_id, &event_tx).await?;
                    }
                    NetworkCommand::Shutdown => break,
                }
            }
            else => break,
        }
    }

    client.shutdown().await.ok();
    Ok(())
}

async fn handle_client_event(
    event: ClientEvent,
    local_owner: i32,
    client_id: ClientId,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    match event {
        ClientEvent::Ready { packet } => {
            handle_ready_packet(packet, local_owner, event_tx)?;
        }
        ClientEvent::Direct { .. } | ClientEvent::ExecSync { .. } => {
            // TODO: surface sync information if necessary.
        }
        ClientEvent::Disconnected { reason } => {
            let _ = event_tx.send(NetworkEvent::PeerDisconnected { client_id, reason });
        }
    }
    Ok(())
}

async fn submit_local_control(
    host: &mut HostHandle,
    owner: i32,
    event: ControlEvent,
    tick: Tick,
    client_id: ClientId,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    if let Some(frame) = control_frame(owner, event, tick, client_id) {
        match encode_control_packet(&frame) {
            Ok(packet) => {
                host.submit_local_control(packet)
                    .await
                    .map_err(|err| anyhow!("host submit failed: {err}"))?;
            }
            Err(err) => {
                let _ = event_tx.send(NetworkEvent::Error(format!(
                    "failed to encode control packet: {err:?}"
                )));
            }
        }
    }
    Ok(())
}

async fn submit_client_control(
    client: &ClientHandle,
    owner: i32,
    event: ControlEvent,
    tick: Tick,
    client_id: ClientId,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    if let Some(frame) = control_frame(owner, event, tick, client_id) {
        match encode_control_packet(&frame) {
            Ok(packet) => {
                client
                    .submit_control(packet)
                    .await
                    .map_err(|err| anyhow!("client submit failed: {err}"))?;
            }
            Err(err) => {
                let _ = event_tx.send(NetworkEvent::Error(format!(
                    "failed to encode control packet: {err:?}"
                )));
            }
        }
    }
    Ok(())
}

fn handle_ready_packet(
    packet: ControlPacket,
    local_owner: i32,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    match decode_control_packet(&packet) {
        Ok(frame) => emit_frame_controls(frame, local_owner, event_tx),
        Err(err) => {
            let _ = event_tx.send(NetworkEvent::Error(format!(
                "failed to decode control packet: {err:?}"
            )));
            Ok(())
        }
    }
}

fn emit_frame_controls(
    frame: LegacyControlFrame,
    local_owner: i32,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    for control in frame.controls {
        if let Some(event) = control_event_from_packet(&control) {
            if let Some(owner) = control_owner(&control) {
                if owner == local_owner {
                    continue;
                }
                let _ = event_tx.send(NetworkEvent::Control { owner, event });
            }
        }
    }
    Ok(())
}

fn control_event_from_packet(packet: &lc_engine::ControlPacket) -> Option<ControlEvent> {
    match packet {
        lc_engine::ControlPacket::PlayerControl(data) => {
            interpret_player_control_command(data.command)
        }
        lc_engine::ControlPacket::Unknown { .. } => None,
    }
}

fn control_owner(packet: &lc_engine::ControlPacket) -> Option<i32> {
    match packet {
        lc_engine::ControlPacket::PlayerControl(data) => Some(data.player),
        lc_engine::ControlPacket::Unknown { .. } => None,
    }
}

fn control_frame(
    owner: i32,
    event: ControlEvent,
    tick: Tick,
    client_id: ClientId,
) -> Option<LegacyControlFrame> {
    let command = control_command_for_event(event)?;
    let by_client = i32::try_from(client_id).ok()?;
    let data = lc_engine::ControlPacket::PlayerControl(PlayerControlData {
        player: owner,
        command,
        data: 0,
        by_client,
    });
    Some(LegacyControlFrame {
        client_id,
        tick,
        timestamp_ms: current_millis(),
        controls: vec![data],
    })
}

fn control_command_for_event(event: ControlEvent) -> Option<i32> {
    match event {
        ControlEvent::Press(button) => Some(i32::from(command_for_button(button))),
        ControlEvent::Release(button) => {
            Some(i32::from(command_for_button(button) + COM_RELEASE_OFFSET))
        }
        ControlEvent::ClearPressed => Some(i32::from(COM_CLEAR_PRESSED_COMS)),
    }
}

fn command_for_button(button: ControlButton) -> u8 {
    match button {
        ControlButton::Left => COM_LEFT,
        ControlButton::Right => COM_RIGHT,
        ControlButton::Up => COM_UP,
        ControlButton::Down => COM_DOWN,
    }
}

fn current_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}
