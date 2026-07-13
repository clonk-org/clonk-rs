use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use lc_engine::{
    interpret_player_control_command, CommandKind, ControlButton, ControlCommand, ControlEvent,
    JoinPlayerControlData, PlayerControlData, PlayerInfoControlData, SyncCheckPacket,
    COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG,
    COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER,
    COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT, COM_MENU_SHOW_TEXT,
    COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL,
    COM_SPECIAL2, COM_THROW, COM_UP,
};
use lc_network::{
    connect_client, decode_control_entry_payload, decode_control_packet,
    encode_control_entry_payload, encode_control_packet, start_host, ClientConfig, ClientEvent,
    ClientHandle, ClientId, ControlDelivery, ControlPacket, HostConfig, HostEvent, HostHandle,
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
    pub player_name: String,
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub server_addr: SocketAddr,
    pub player_name: String,
}

const HOST_CLIENT_ID: ClientId = 0;

#[derive(Debug)]
pub struct NetworkManager {
    command_tx: tokio_mpsc::Sender<NetworkCommand>,
    event_rx: Receiver<NetworkEvent>,
    worker: Option<thread::JoinHandle<()>>,
    local_client_id: ClientId,
}

#[cfg(test)]
pub(crate) struct TestNetworkCommands {
    command_rx: tokio_mpsc::Receiver<NetworkCommand>,
}

#[cfg(test)]
impl TestNetworkCommands {
    pub(crate) fn take_submitted_local(&mut self) -> Vec<(i32, ControlEvent, Tick)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitLocal { owner, event, tick } = command {
                submitted.push((owner, event, tick));
            }
        }
        submitted
    }

    pub(crate) fn take_player_info_updates(&mut self) -> Vec<lc_network::PlayerInfoUpdateRequest> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerInfoUpdate(request) = command {
                submitted.push(request);
            }
        }
        submitted
    }

    pub(crate) fn take_broadcast_player_infos(&mut self) -> Vec<PlayerInfoControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::BroadcastPlayerInfo(info) = command {
                submitted.push(info);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_join_players(
        &mut self,
    ) -> Vec<(Tick, JoinPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitJoinPlayer { tick, join } = command {
                submitted.push((tick, join));
            }
        }
        submitted
    }
}

#[derive(Debug)]
pub enum NetworkEvent {
    PlayerInfoUpdateRequest {
        origin: ClientId,
        request: lc_network::PlayerInfoUpdateRequest,
        by_host: bool,
    },
    ReadyTick {
        tick: Tick,
        controls: Vec<NetworkControl>,
    },
    DirectControl(NetworkControl),
    PeerConnected {
        client_id: ClientId,
        name: String,
        kind: ParticipantKind,
    },
    PeerDisconnected {
        client_id: ClientId,
        reason: Option<String>,
    },
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkControl {
    PlayerInfo(PlayerInfoControlData),
    JoinPlayer(JoinPlayerControlData),
    Player {
        owner: i32,
        event: ControlEvent,
    },
    SyncCheck(SyncCheckPacket),
}

#[derive(Debug)]
enum NetworkCommand {
    SubmitPlayerInfoUpdate(lc_network::PlayerInfoUpdateRequest),
    BroadcastPlayerInfo(PlayerInfoControlData),
    SubmitJoinPlayer {
        tick: Tick,
        join: JoinPlayerControlData,
    },
    SubmitLocal {
        owner: i32,
        event: ControlEvent,
        tick: Tick,
    },
    SubmitSyncCheck {
        tick: Tick,
        check: SyncCheckPacket,
    },
    FinalizeTick {
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

#[derive(Debug, Default)]
struct ControlFrameAccumulator {
    client_id: ClientId,
    current_tick: Option<Tick>,
    controls: Vec<lc_engine::ControlPacket>,
    last_timestamp: Option<u64>,
    last_sent_tick: Option<Tick>,
}

impl ControlFrameAccumulator {
    fn new(client_id: ClientId) -> Self {
        Self {
            client_id,
            ..Default::default()
        }
    }

    fn record_control(&mut self, tick: Tick, control: lc_engine::ControlPacket, timestamp: u64) {
        if self.last_sent_tick.is_some_and(|last| tick <= last) {
            return;
        }
        if self.current_tick != Some(tick) {
            self.controls.clear();
            self.current_tick = Some(tick);
        }
        self.controls.push(control);
        self.last_timestamp = Some(timestamp);
    }

    fn finalize_tick(&mut self, tick: Tick) -> Option<LegacyControlFrame> {
        if self.last_sent_tick.is_some_and(|last| tick <= last) {
            return None;
        }

        let (controls, timestamp) = if self.current_tick == Some(tick) {
            let timestamp = self.last_timestamp.take().unwrap_or_else(current_millis);
            let controls = std::mem::take(&mut self.controls);
            self.current_tick = None;
            (controls, timestamp)
        } else {
            (Vec::new(), current_millis())
        };

        self.last_sent_tick = Some(tick);

        Some(LegacyControlFrame {
            client_id: self.client_id,
            tick,
            timestamp_ms: timestamp,
            controls,
        })
    }
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
        let (local_id_tx, local_id_rx) = mpsc::channel::<Result<ClientId, String>>();
        let thread_name = match mode {
            WorkerMode::Host { .. } => "lc-network-host",
            WorkerMode::Client { .. } => "lc-network-client",
        };
        let worker = thread::Builder::new()
            .name(thread_name.to_string())
            .spawn({
                let event_tx = event_tx.clone();
                move || {
                    let runtime = RuntimeBuilder::new_multi_thread()
                        .enable_all()
                        .build()
                        .expect("failed to initialise tokio runtime");
                    if let Err(err) = runtime.block_on(run_worker(
                        mode,
                        command_rx,
                        event_tx.clone(),
                        local_id_tx,
                    )) {
                        let _ = event_tx.send(NetworkEvent::Error(format!("{err:?}")));
                    }
                }
            })
            .context("failed to spawn network worker thread")?;
        let local_client_id = match local_id_rx
            .recv()
            .context("network worker did not report local client id")?
        {
            Ok(id) => id,
            Err(err) => return Err(anyhow!(err)),
        };

        Ok(Self {
            command_tx,
            event_rx,
            worker: Some(worker),
            local_client_id,
        })
    }

    pub fn submit_local_control(&self, owner: i32, event: ControlEvent, tick: Tick) {
        let command = NetworkCommand::SubmitLocal { owner, event, tick };
        let _ = self.command_tx.blocking_send(command);
    }

    pub fn submit_player_info_update(
        &self,
        request: lc_network::PlayerInfoUpdateRequest,
    ) -> Result<()> {
        self.command_tx
            .blocking_send(NetworkCommand::SubmitPlayerInfoUpdate(request))
            .map_err(|_| anyhow!("network worker is not accepting player-info updates"))
    }

    pub fn broadcast_player_info(&self, mut info: PlayerInfoControlData) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may broadcast PlayerInfo"));
        }
        info.by_client = i32::try_from(HOST_CLIENT_ID)
            .map_err(|_| anyhow!("host client id exceeds the PlayerInfo wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::BroadcastPlayerInfo(info))
            .map_err(|_| anyhow!("network worker is not accepting PlayerInfo broadcasts"))
    }

    pub fn submit_join_player(&self, tick: Tick, mut join: JoinPlayerControlData) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may submit JoinPlayer"));
        }
        join.by_client = i32::try_from(HOST_CLIENT_ID)
            .map_err(|_| anyhow!("host client id exceeds the JoinPlayer wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitJoinPlayer { tick, join })
            .map_err(|_| anyhow!("network worker is not accepting JoinPlayer controls"))
    }

    pub fn submit_sync_check(&self, tick: Tick, mut check: SyncCheckPacket) {
        if let Ok(id) = i32::try_from(self.local_client_id) {
            check.by_client = id;
        }
        let command = NetworkCommand::SubmitSyncCheck { tick, check };
        let _ = self.command_tx.blocking_send(command);
    }

    pub fn finalize_tick(&self, tick: Tick) {
        let command = NetworkCommand::FinalizeTick { tick };
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

    pub fn local_client_id(&self) -> ClientId {
        self.local_client_id
    }

    #[cfg(test)]
    pub(crate) fn test_stub() -> (Self, Sender<NetworkEvent>) {
        let (command_tx, _command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        (
            Self {
                command_tx,
                event_rx,
                worker: None,
                local_client_id: HOST_CLIENT_ID,
            },
            event_tx,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_stub_with_commands() -> (
        Self,
        Sender<NetworkEvent>,
        TestNetworkCommands,
    ) {
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        (
            Self {
                command_tx,
                event_rx,
                worker: None,
                local_client_id: HOST_CLIENT_ID,
            },
            event_tx,
            TestNetworkCommands { command_rx },
        )
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
    local_id_tx: mpsc::Sender<Result<ClientId, String>>,
) -> Result<()> {
    match mode {
        WorkerMode::Host {
            settings,
            local_owner,
        } => {
            run_host_worker(
                settings,
                local_owner,
                &mut command_rx,
                event_tx,
                local_id_tx,
            )
            .await
        }
        WorkerMode::Client {
            settings,
            local_owner,
        } => {
            run_client_worker(
                settings,
                local_owner,
                &mut command_rx,
                event_tx,
                local_id_tx,
            )
            .await
        }
    }
}

async fn run_host_worker(
    settings: HostSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    event_tx: Sender<NetworkEvent>,
    local_id_tx: mpsc::Sender<Result<ClientId, String>>,
) -> Result<()> {
    let listener = match TcpListener::bind(settings.bind_addr).await {
        Ok(listener) => listener,
        Err(err) => {
            let message = format!(
                "failed to bind host socket at {}: {err}",
                settings.bind_addr
            );
            let _ = local_id_tx.send(Err(message.clone()));
            return Err(anyhow!(message));
        }
    };
    let host_config = HostConfig {
        backlog_limit: 256,
        max_players: 8,
        resync_interval: Duration::from_millis(200),
        resync_cooldown: Duration::from_secs(2),
        start_tick: 0,
    };
    let mut host = match start_host(listener, host_config).await {
        Ok(host) => host,
        Err(err) => {
            let message = format!("failed to start host session: {err}");
            let _ = local_id_tx.send(Err(message.clone()));
            return Err(anyhow!(message));
        }
    };
    let _ = local_id_tx.send(Ok(HOST_CLIENT_ID));
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        client_id: HOST_CLIENT_ID,
        name: settings.player_name.clone(),
        kind: ParticipantKind::Player,
    });
    let mut host_events = host.take_event_receiver();
    let mut frame_builder = ControlFrameAccumulator::new(HOST_CLIENT_ID);

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
                    NetworkCommand::SubmitPlayerInfoUpdate(request) => {
                        let _ = event_tx.send(NetworkEvent::PlayerInfoUpdateRequest {
                            origin: HOST_CLIENT_ID,
                            request,
                            by_host: true,
                        });
                    }
                    NetworkCommand::BroadcastPlayerInfo(info) => {
                        send_player_info_from_host(&host, info, &event_tx).await?;
                    }
                    NetworkCommand::SubmitJoinPlayer { tick, join } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::JoinPlayer(join),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        if let Some(control) = control_packet_for_event(owner, event, HOST_CLIENT_ID) {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitSyncCheck { tick, check } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::SyncCheck(check),
                            current_millis(),
                        );
                    }
                    NetworkCommand::FinalizeTick { tick } => {
                        if let Some(frame) = frame_builder.finalize_tick(tick) {
                            send_frame_to_host(&host, frame, &event_tx).await?;
                        }
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
        HostEvent::StatusAck { .. } => {
            // lc-network's status barrier consumes this before app-level
            // status transitions are enabled.
        }
        HostEvent::PlayerInfoUpdate { client_id, request } => {
            let _ = event_tx.send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: client_id,
                request,
                by_host: false,
            });
        }
        HostEvent::Ready { packet } => {
            handle_ready_packet(packet, local_owner, event_tx)?;
        }
        HostEvent::ClientJoined {
            client_id,
            name,
            kind,
        } => {
            let _ = event_tx.send(NetworkEvent::PeerConnected {
                client_id,
                name,
                kind,
            });
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
        HostEvent::Direct { delivery, data, .. } => {
            handle_direct_packet(delivery, data, event_tx)?;
        }
        HostEvent::ExecSync { .. } => {
            // Synchronized-control execution is not surfaced yet.
        }
    }
    Ok(())
}

async fn run_client_worker(
    settings: ClientSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    event_tx: Sender<NetworkEvent>,
    local_id_tx: mpsc::Sender<Result<ClientId, String>>,
) -> Result<()> {
    let player_name = settings.player_name.clone();
    let mut client = match connect_client(
        settings.server_addr,
        ClientConfig::new(player_name.clone(), ParticipantKind::Player),
    )
    .await
    {
        Ok(client) => client,
        Err(err) => {
            let message = format!("failed to connect to host: {err}");
            let _ = local_id_tx.send(Err(message.clone()));
            return Err(anyhow!(message));
        }
    };
    let client_id = client.client_id();
    let _ = local_id_tx.send(Ok(client_id));
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        client_id,
        name: player_name,
        kind: ParticipantKind::Player,
    });
    let mut client_events = client.take_event_receiver();
    let mut frame_builder = ControlFrameAccumulator::new(client_id);

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
                    NetworkCommand::SubmitPlayerInfoUpdate(request) => {
                        client
                            .submit_player_info_update(request)
                            .await
                            .map_err(|err| anyhow!("client PlayerInfo update failed: {err}"))?;
                    }
                    NetworkCommand::BroadcastPlayerInfo(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to broadcast authoritative PlayerInfo".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitJoinPlayer { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit authoritative JoinPlayer".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        if let Some(control) = control_packet_for_event(owner, event, client_id) {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitSyncCheck { tick, check } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::SyncCheck(check),
                            current_millis(),
                        );
                    }
                    NetworkCommand::FinalizeTick { tick } => {
                        if let Some(frame) = frame_builder.finalize_tick(tick) {
                            send_frame_to_client(&client, frame, &event_tx).await?;
                        }
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
        ClientEvent::Status(_) | ClientEvent::StatusAck(_) => {
            // lc-network's status barrier consumes these before app-level
            // status transitions are enabled.
        }
        ClientEvent::Ready { packet } => {
            handle_ready_packet(packet, local_owner, event_tx)?;
        }
        ClientEvent::Direct { delivery, data } => {
            handle_direct_packet(delivery, data, event_tx)?;
        }
        ClientEvent::ExecSync { .. } => {
            // Synchronized-control execution is not surfaced yet.
        }
        ClientEvent::Disconnected { reason } => {
            let _ = event_tx.send(NetworkEvent::PeerDisconnected { client_id, reason });
        }
    }
    Ok(())
}

async fn send_frame_to_host(
    host: &HostHandle,
    frame: LegacyControlFrame,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
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
    Ok(())
}

async fn send_player_info_from_host(
    host: &HostHandle,
    info: PlayerInfoControlData,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    match encode_control_entry_payload(&lc_engine::ControlPacket::PlayerInfo(info)) {
        Ok(data) => host
            .submit_packet(ControlDelivery::Direct, data)
            .await
            .map_err(|err| anyhow!("host PlayerInfo broadcast failed: {err}")),
        Err(err) => {
            let _ = event_tx.send(NetworkEvent::Error(format!(
                "failed to encode direct PlayerInfo: {err:?}"
            )));
            Ok(())
        }
    }
}

async fn send_frame_to_client(
    client: &ClientHandle,
    frame: LegacyControlFrame,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
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

fn handle_direct_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    if !matches!(delivery, ControlDelivery::Direct | ControlDelivery::Private) {
        let _ = event_tx.send(NetworkEvent::Error(format!(
            "received non-direct control packet with delivery {delivery:?}"
        )));
        return Ok(());
    }

    match decode_control_entry_payload(&data) {
        Ok(lc_engine::ControlPacket::PlayerInfo(info)) => {
            let _ = event_tx.send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(info)));
        }
        Ok(control) => {
            let _ = event_tx.send(NetworkEvent::Error(format!(
                "unsupported immediate control packet: {control:?}"
            )));
        }
        Err(err) => {
            let _ = event_tx.send(NetworkEvent::Error(format!(
                "failed to decode direct control packet: {err:?}"
            )));
        }
    }
    Ok(())
}

fn emit_frame_controls(
    frame: LegacyControlFrame,
    _local_owner: i32,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    let tick = frame.tick;
    let mut controls = Vec::new();
    for control in frame.controls {
        match control {
            lc_engine::ControlPacket::PlayerControl(data) => {
                if let Some(event) = control_event_for_player_control(&data) {
                    controls.push(NetworkControl::Player {
                        owner: data.player,
                        event,
                    });
                }
            }
            lc_engine::ControlPacket::SyncCheck(packet) => {
                controls.push(NetworkControl::SyncCheck(packet));
            }
            lc_engine::ControlPacket::PlayerInfo(info) => {
                controls.push(NetworkControl::PlayerInfo(info));
            }
            lc_engine::ControlPacket::JoinPlayer(join) => {
                controls.push(NetworkControl::JoinPlayer(join));
            }
            lc_engine::ControlPacket::ClientUpdate(_)
            | lc_engine::ControlPacket::ClientRemove(_) => {}
            lc_engine::ControlPacket::Unknown { .. } => {}
        }
    }
    // C4GameControl::Execute obtains one complete C4Control for ControlTick
    // and executes it before simulation (src/C4GameControl.cpp:289-316).
    // Retain the decoded order (including SyncCheck positions) and even an
    // empty tick so "ready with no input" differs from "not ready".
    let _ = event_tx.send(NetworkEvent::ReadyTick { tick, controls });
    Ok(())
}

fn control_event_for_player_control(data: &PlayerControlData) -> Option<ControlEvent> {
    if data.data != 0 {
        let command = u8::try_from(data.command).ok()?;
        return Some(ControlEvent::RawPlayerControl {
            command,
            data: data.data,
        });
    }
    interpret_player_control_command(data.command)
}

fn control_packet_for_event(
    owner: i32,
    event: ControlEvent,
    client_id: ClientId,
) -> Option<lc_engine::ControlPacket> {
    let (command, data) = match event {
        ControlEvent::RawPlayerControl { command, data } => (i32::from(command), data),
        event => (control_command_for_event(event)?, 0),
    };
    let by_client = i32::try_from(client_id).ok()?;
    Some(lc_engine::ControlPacket::PlayerControl(PlayerControlData {
        player: owner,
        command,
        data,
        by_client,
    }))
}

fn control_command_for_event(event: ControlEvent) -> Option<i32> {
    match event {
        ControlEvent::Press(button) => Some(i32::from(command_for_button(button))),
        ControlEvent::Release(button) => {
            Some(i32::from(command_for_button(button) + COM_RELEASE_OFFSET))
        }
        ControlEvent::Command { command, kind } => command_code_for(command, kind).map(i32::from),
        ControlEvent::RawPlayerControl { .. } => None,
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

fn command_code_for(command: ControlCommand, kind: CommandKind) -> Option<u8> {
    let base = match command {
        ControlCommand::Throw => COM_THROW,
        ControlCommand::Dig => COM_DIG,
        ControlCommand::Special => COM_SPECIAL,
        ControlCommand::Special2 => COM_SPECIAL2,
        ControlCommand::CursorLeft => COM_CURSOR_LEFT,
        ControlCommand::CursorRight => COM_CURSOR_RIGHT,
        ControlCommand::CursorToggle => COM_CURSOR_TOGGLE,
        ControlCommand::PlayerMenu => COM_PLAYER_MENU,
        ControlCommand::MenuEnter => COM_MENU_ENTER,
        ControlCommand::MenuEnterAll => COM_MENU_ENTER_ALL,
        ControlCommand::MenuClose => COM_MENU_CLOSE,
        ControlCommand::MenuShowText => COM_MENU_SHOW_TEXT,
        ControlCommand::MenuLeft => COM_MENU_LEFT,
        ControlCommand::MenuRight => COM_MENU_RIGHT,
        ControlCommand::MenuUp => COM_MENU_UP,
        ControlCommand::MenuDown => COM_MENU_DOWN,
        ControlCommand::MenuSelect => COM_MENU_SELECT,
    };

    let value = match kind {
        CommandKind::Press => base,
        CommandKind::Single => base | COM_SINGLE,
        CommandKind::Double => base | COM_DOUBLE,
        CommandKind::Release => {
            if matches!(command, ControlCommand::PlayerMenu) {
                return None;
            }
            base + COM_RELEASE_OFFSET
        }
    };
    Some(value)
}

fn current_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0))
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sync_check(frame: i32) -> SyncCheckPacket {
        SyncCheckPacket {
            frame,
            control_tick: frame,
            random3: 0,
            random_count: 0,
            crew_positions_sum: 0,
            pxs_count: 0,
            mass_mover_index: 0,
            object_count: 0,
            object_enumeration_index: 0,
            sector_shape_sum: 0,
            by_client: 0,
        }
    }

    #[test]
    fn manager_queues_player_info_update_without_fabricating_an_author() {
        // C4PacketPlayerInfoUpdRequest carries C4ClientPlayerInfos unchanged
        // and has no C4ControlPacket ByClient field
        // (src/C4Network2Players.cpp:142-166;
        // src/C4PlayerInfo.cpp:1800-1803).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: 1,
            players: vec![lc_engine::ControlPlayerInfoEntry {
                id: 0,
                ..Default::default()
            }],
        };

        manager
            .submit_player_info_update(request.clone())
            .expect("queue PlayerInfo update request");

        assert_eq!(commands.take_player_info_updates(), vec![request]);
    }

    #[test]
    fn host_manager_stamps_authoritative_player_info_before_broadcast() {
        // The host constructs C4ControlPlayerInfo, whose base constructor sets
        // ByClient to the host control ID, then sends it with CDT_Direct
        // (src/C4Control.cpp:38-56;
        // src/C4Network2Players.cpp:232-239).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let info = PlayerInfoControlData {
            client_id: 3,
            by_client: 99,
            ..Default::default()
        };

        manager
            .broadcast_player_info(info.clone())
            .expect("host queues authoritative PlayerInfo");

        assert_eq!(
            commands.take_broadcast_player_infos(),
            vec![PlayerInfoControlData {
                by_client: 0,
                ..info
            }]
        );
    }

    #[test]
    fn host_manager_stamps_join_before_synchronized_submission() {
        // JoinUnjoinedPlayersInControlQueue constructs JoinPlayer on the host
        // and appends it to Game.Input for the next synchronized control tick
        // (src/C4Network2Players.cpp:353-388;
        // src/C4GameControl.cpp:234-265).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let join = JoinPlayerControlData {
            at_client: 3,
            info_id: 7,
            by_client: 99,
            ..Default::default()
        };

        manager
            .submit_join_player(23, join.clone())
            .expect("host queues synchronized JoinPlayer");

        assert_eq!(
            commands.take_submitted_join_players(),
            vec![(
                23,
                JoinPlayerControlData {
                    by_client: 0,
                    ..join
                },
            )]
        );
    }

    #[test]
    fn ready_frame_retains_tick_local_owner_and_decoded_order() {
        // Network control is retrieved as one control-tick batch before it is
        // executed (src/C4GameControl.cpp:289-316). The app event must not
        // flatten tick or move SyncCheck out of decoded packet order.
        let check = sync_check(17);
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 17,
            timestamp_ms: 99,
            controls: vec![
                control_packet_for_event(4, ControlEvent::Press(ControlButton::Right), 0)
                    .expect("local control packet"),
                lc_engine::ControlPacket::SyncCheck(check.clone()),
                control_packet_for_event(9, ControlEvent::Press(ControlButton::Left), 1)
                    .expect("remote control packet"),
            ],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 4, &event_tx).expect("emit ready frame");

        match event_rx.recv().expect("ready event") {
            NetworkEvent::ReadyTick { tick, controls } => {
                assert_eq!(tick, 17, "the aggregate control tick must be retained");
                assert_eq!(
                    controls,
                    vec![
                        NetworkControl::Player {
                            owner: 4,
                            event: ControlEvent::Press(ControlButton::Right),
                        },
                        NetworkControl::SyncCheck(check),
                        NetworkControl::Player {
                            owner: 9,
                            event: ControlEvent::Press(ControlButton::Left),
                        },
                    ],
                    "local, sync-check and remote controls stay in decoded order"
                );
            }
            other => panic!("expected one ready tick, got {other:?}"),
        }
        assert!(
            event_rx.try_recv().is_err(),
            "one aggregate must produce one scheduling event"
        );
    }

    #[test]
    fn ready_frame_retains_admission_controls_in_decoded_order() {
        // C4Control executes the same list order used by PreExecute, and the
        // complete network control preserves each client's packet order
        // (src/C4Control.cpp:73-109;
        // src/C4GameControlNetwork.cpp:741-769).
        let info = lc_engine::PlayerInfoControlData {
            client_id: 3,
            by_client: 4,
            ..Default::default()
        };
        let join = lc_engine::JoinPlayerControlData {
            at_client: 3,
            info_id: 7,
            by_client: 4,
            ..Default::default()
        };
        let player = control_packet_for_event(7, ControlEvent::Press(ControlButton::Right), 4)
            .expect("player control packet");
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![
                lc_engine::ControlPacket::PlayerInfo(info.clone()),
                player,
                lc_engine::ControlPacket::JoinPlayer(join.clone()),
            ],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 0, &event_tx).expect("emit ready frame");

        let NetworkEvent::ReadyTick { tick, controls } =
            event_rx.recv().expect("ready event")
        else {
            panic!("expected ready tick");
        };
        assert_eq!(tick, 23);
        assert_eq!(
            controls,
            vec![
                NetworkControl::PlayerInfo(info),
                NetworkControl::Player {
                    owner: 7,
                    event: ControlEvent::Press(ControlButton::Right),
                },
                NetworkControl::JoinPlayer(join),
            ]
        );
    }

    #[test]
    fn direct_player_info_emits_an_immediate_control_event() {
        // PID_ControlPkt with CDT_Direct executes immediately rather than
        // entering the synchronized control queue; network PlayerInfo is sent
        // through exactly that path (src/C4GameControlNetwork.cpp:558-566;
        // src/C4Network2Players.cpp:232-239).
        let info = PlayerInfoControlData {
            client_id: 3,
            by_client: 0,
            ..Default::default()
        };
        let payload = lc_network::encode_control_entry_payload(
            &lc_engine::ControlPacket::PlayerInfo(info.clone()),
        )
        .expect("encode direct PlayerInfo payload");
        let (event_tx, event_rx) = mpsc::channel();

        handle_direct_packet(lc_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct PlayerInfo");

        let NetworkEvent::DirectControl(NetworkControl::PlayerInfo(actual)) =
            event_rx.recv().expect("direct control event")
        else {
            panic!("expected one immediate PlayerInfo event");
        };
        assert_eq!(actual, info);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn pointer_menu_control_preserves_the_cpp_data_slot() {
        // C4ObjectMenu::OnUserSelectItem queues COM_MenuSelect with the
        // item index ORed with C4MN_AdjustPosition; the synchronized
        // C4ControlPlayerControl packet must retain that signed Data value
        // (C4ObjectMenu.cpp:461-465; C4Control.cpp:586-592).
        let data = lc_engine::C4MN_ADJUST_POSITION | 1;
        let packet = control_packet_for_event(
            7,
            ControlEvent::RawPlayerControl {
                command: COM_MENU_SELECT,
                data,
            },
            3,
        )
        .expect("pointer menu controls produce a network packet");
        let expected = PlayerControlData {
            player: 7,
            command: i32::from(COM_MENU_SELECT),
            data,
            by_client: 3,
        };
        assert_eq!(
            packet,
            lc_engine::ControlPacket::PlayerControl(expected.clone())
        );
        assert_eq!(
            control_event_for_player_control(&expected),
            Some(ControlEvent::RawPlayerControl {
                command: COM_MENU_SELECT,
                data,
            }),
            "remote peers must receive the same signed Data payload"
        );
    }

    #[test]
    fn accumulator_batches_controls_for_tick() {
        let mut acc = ControlFrameAccumulator::new(5);
        let first = control_packet_for_event(1, ControlEvent::Press(ControlButton::Left), 5)
            .expect("build first control packet");
        acc.record_control(3, first.clone(), 10);

        let second = control_packet_for_event(
            1,
            ControlEvent::Command {
                command: ControlCommand::Throw,
                kind: CommandKind::Press,
            },
            5,
        )
        .expect("build second control packet");
        acc.record_control(3, second.clone(), 20);

        let frame = acc
            .finalize_tick(3)
            .expect("finalizing tick with controls produces frame");
        assert_eq!(frame.client_id, 5);
        assert_eq!(frame.tick, 3);
        assert_eq!(frame.controls, vec![first, second]);
        assert!(
            acc.finalize_tick(3).is_none(),
            "second finalize for same tick yields no frame"
        );
    }

    #[test]
    fn accumulator_emits_empty_frame_without_controls() {
        let mut acc = ControlFrameAccumulator::new(2);
        let frame = acc
            .finalize_tick(10)
            .expect("empty finalize still yields frame");
        assert_eq!(frame.client_id, 2);
        assert_eq!(frame.tick, 10);
        assert!(frame.controls.is_empty());
    }

    #[test]
    fn accumulator_ignores_outdated_ticks() {
        let mut acc = ControlFrameAccumulator::new(1);
        let control = control_packet_for_event(1, ControlEvent::Press(ControlButton::Right), 1)
            .expect("build control packet");
        acc.record_control(2, control.clone(), 30);

        let frame = acc.finalize_tick(2).expect("first finalize produces frame");
        assert_eq!(frame.controls, vec![control.clone()]);

        // Attempt to record another control for an already-finalized tick.
        acc.record_control(2, control.clone(), 40);
        assert!(
            acc.finalize_tick(2).is_none(),
            "duplicate finalize does not emit new frame"
        );

        // Controls for older ticks are ignored.
        acc.record_control(1, control, 50);
        let frame = acc
            .finalize_tick(3)
            .expect("finalize advances to next tick");
        assert!(frame.controls.is_empty());
    }
}
