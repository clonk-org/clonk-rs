use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use lc_engine::{
    interpret_player_control_command, CommandKind, ControlButton, ControlCommand, ControlEvent,
    PlayerControlData, SyncCheckPacket, COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT,
    COM_CURSOR_TOGGLE, COM_DIG, COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN,
    COM_MENU_ENTER, COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT,
    COM_MENU_SHOW_TEXT, COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE,
    COM_SPECIAL, COM_SPECIAL2, COM_THROW, COM_UP,
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

#[derive(Debug)]
pub enum NetworkEvent {
    Control {
        owner: i32,
        event: ControlEvent,
    },
    SyncCheck {
        packet: SyncCheckPacket,
    },
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

#[derive(Debug)]
enum NetworkCommand {
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

fn emit_frame_controls(
    frame: LegacyControlFrame,
    local_owner: i32,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    for control in frame.controls {
        match control {
            lc_engine::ControlPacket::PlayerControl(data) => {
                if let Some(event) = control_event_for_player_control(&data) {
                    if data.player == local_owner {
                        continue;
                    }
                    let _ = event_tx.send(NetworkEvent::Control {
                        owner: data.player,
                        event,
                    });
                }
            }
            lc_engine::ControlPacket::SyncCheck(packet) => {
                let _ = event_tx.send(NetworkEvent::SyncCheck { packet });
            }
            // CID_JoinPlr/CID_PlrInfo (remote player joins): the engine's
            // join pipeline consumes these in the shadow-diff runtime;
            // lc-app's network session has no remote-join event yet —
            // forwarding them (a join NetworkEvent driving
            // Engine::join_player like ffi.rs handle_join_player) is the
            // network-multiplayer join feature, not a control input.
            lc_engine::ControlPacket::JoinPlayer(_)
            | lc_engine::ControlPacket::PlayerInfo(_) => {}
            lc_engine::ControlPacket::Unknown { .. } => {}
        }
    }
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
