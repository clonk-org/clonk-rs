use std::net::SocketAddr;
use std::path::PathBuf;
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
    LegacyControlFrame, NetworkStatus, ParticipantKind, Tick,
};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::mpsc as tokio_mpsc;

use crate::prepared_host_bootstrap::PreparedHostBootstrap;

#[derive(Debug, Clone)]
pub enum NetworkMode {
    Host(HostSettings),
    Client(ClientSettings),
}

#[derive(Debug, Clone)]
pub struct HostSettings {
    pub bind_addr: SocketAddr,
    pub player_name: String,
    /// Canonical scenario/resources/dynamic materialized before binding.
    /// `None` is retained only for direct CLI compatibility while that path is
    /// migrated to the same scenario-first startup.
    pub prepared: Option<PreparedHostBootstrap>,
}

#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub server_addr: SocketAddr,
    pub player_name: String,
}

const HOST_CLIENT_ID: ClientId = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkRole {
    Host,
    Client,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NetworkStatusCommandError {
    #[error("only the network host may {operation}")]
    HostRoleRequired { operation: &'static str },
    #[error("only a network client may {operation}")]
    ClientRoleRequired { operation: &'static str },
    #[error("network worker is not accepting {operation}")]
    WorkerUnavailable { operation: &'static str },
    #[error("no host game status is waiting for local acknowledgement")]
    NoRequestedStatus,
}

#[derive(Debug, Default)]
struct ClientStatusState {
    requested: Option<NetworkStatus>,
    awaiting_commit: Option<NetworkStatus>,
}

impl ClientStatusState {
    fn receive_request(&mut self, status: NetworkStatus) {
        self.requested = Some(status);
        self.awaiting_commit = None;
    }

    fn acknowledge_requested(&mut self, expected: NetworkStatus) -> Option<NetworkStatus> {
        if self.requested != Some(expected) {
            return None;
        }
        self.requested = None;
        self.awaiting_commit = Some(expected);
        Some(expected)
    }

    fn restore_request(&mut self, status: NetworkStatus) {
        if self.awaiting_commit == Some(status) {
            self.awaiting_commit = None;
            self.requested = Some(status);
        }
    }

    fn commit(&mut self, status: NetworkStatus) -> bool {
        if self.awaiting_commit != Some(status) {
            return false;
        }
        self.awaiting_commit = None;
        true
    }
}

#[derive(Debug)]
pub struct NetworkManager {
    command_tx: tokio_mpsc::Sender<NetworkCommand>,
    event_rx: Receiver<NetworkEvent>,
    worker: Option<thread::JoinHandle<()>>,
    local_client_id: ClientId,
    role: NetworkRole,
    client_status: ClientStatusState,
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

    pub(crate) fn receive_join_allowed(
        &mut self,
    ) -> (bool, Sender<std::result::Result<(), String>>) {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::SetJoinAllowed {
                allowed,
                completion,
            }) => (allowed, completion),
            Some(command) => panic!("expected join-admission command, got {command:?}"),
            None => panic!("network command channel ended before join-admission command"),
        }
    }

    pub(crate) fn take_status_changes(&mut self) -> Vec<NetworkStatus> {
        let mut changes = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::ChangeStatus(status) = command {
                changes.push(status);
            }
        }
        changes
    }

    pub(crate) fn take_status_reached(&mut self) -> usize {
        let mut count = 0;
        while let Ok(command) = self.command_rx.try_recv() {
            if matches!(command, NetworkCommand::StatusReached) {
                count += 1;
            }
        }
        count
    }

    pub(crate) fn take_status_acknowledgements(&mut self) -> Vec<NetworkStatus> {
        let mut acknowledgements = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::AcknowledgeRequestedStatus(status) = command {
                acknowledgements.push(status);
            }
        }
        acknowledgements
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetworkEvent {
    JoinData(lc_network::JoinDataEnvelope),
    StatusRequested(NetworkStatus),
    StatusCommitted(NetworkStatus),
    PlayerInfoUpdateRequest {
        origin: ClientId,
        request: lc_network::PlayerInfoUpdateRequest,
        by_host: bool,
    },
    ReadyTick {
        tick: Tick,
        controls: Vec<NetworkControl>,
    },
    ScheduledSync {
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
    ResourceAction(lc_network::ResourceCatalogAction),
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
    Error(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkControl {
    ClientJoin(lc_engine::ClientJoinControlData),
    ClientUpdate(lc_engine::ClientUpdateControlData),
    ClientRemove(lc_engine::ClientRemoveControlData),
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
    ChangeStatus(NetworkStatus),
    StatusReached,
    AcknowledgeRequestedStatus(NetworkStatus),
    SetJoinAllowed {
        allowed: bool,
        completion: Sender<std::result::Result<(), String>>,
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
        let role = match &mode {
            WorkerMode::Host { .. } => NetworkRole::Host,
            WorkerMode::Client { .. } => NetworkRole::Client,
        };
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
            role,
            client_status: ClientStatusState::default(),
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

    pub fn set_join_allowed(&self, allowed: bool) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may change join admission"));
        }
        let (completion, applied) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::SetJoinAllowed {
                allowed,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting join-admission changes"))?;
        applied
            .recv()
            .map_err(|_| anyhow!("network worker ended before confirming join admission"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn change_status(
        &self,
        status: NetworkStatus,
    ) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Host {
            return Err(NetworkStatusCommandError::HostRoleRequired {
                operation: "change game status",
            });
        }
        self.command_tx
            .blocking_send(NetworkCommand::ChangeStatus(status))
            .map_err(|_| NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status changes",
            })
    }

    pub fn status_reached(&self) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Host {
            return Err(NetworkStatusCommandError::HostRoleRequired {
                operation: "mark game status reached",
            });
        }
        self.command_tx
            .blocking_send(NetworkCommand::StatusReached)
            .map_err(|_| NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status arrival",
            })
    }

    pub fn acknowledge_requested_status(
        &mut self,
    ) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Client {
            return Err(NetworkStatusCommandError::ClientRoleRequired {
                operation: "acknowledge a host game status",
            });
        }
        let status = self
            .client_status
            .requested
            .ok_or(NetworkStatusCommandError::NoRequestedStatus)?;
        if self
            .client_status
            .acknowledge_requested(status)
            .is_none()
        {
            return Err(NetworkStatusCommandError::NoRequestedStatus);
        }
        if self
            .command_tx
            .blocking_send(NetworkCommand::AcknowledgeRequestedStatus(status))
            .is_err()
        {
            self.client_status.restore_request(status);
            return Err(NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status acknowledgements",
            });
        }
        Ok(())
    }

    pub fn poll_events(&mut self) -> Vec<NetworkEvent> {
        let mut events = Vec::new();
        loop {
            match self.event_rx.try_recv() {
                Ok(event) => {
                    if self.role == NetworkRole::Client {
                        match &event {
                            NetworkEvent::JoinData(join_data) => {
                                self.client_status
                                    .receive_request(initial_client_status(join_data));
                            }
                            NetworkEvent::StatusRequested(status) => {
                                self.client_status.receive_request(*status);
                            }
                            NetworkEvent::StatusCommitted(status) => {
                                if !self.client_status.commit(*status) {
                                    continue;
                                }
                            }
                            _ => {}
                        }
                    }
                    events.push(event);
                }
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
                role: NetworkRole::Host,
                client_status: ClientStatusState::default(),
            },
            event_tx,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_stub_for_client_id(
        local_client_id: ClientId,
    ) -> (Self, Sender<NetworkEvent>) {
        let (command_tx, _command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        (
            Self {
                command_tx,
                event_rx,
                worker: None,
                local_client_id,
                role: NetworkRole::Client,
                client_status: ClientStatusState::default(),
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
                role: NetworkRole::Host,
                client_status: ClientStatusState::default(),
            },
            event_tx,
            TestNetworkCommands { command_rx },
        )
    }

    #[cfg(test)]
    pub(crate) fn test_stub_with_commands_for_client_id(
        local_client_id: ClientId,
    ) -> (Self, Sender<NetworkEvent>, TestNetworkCommands) {
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        (
            Self {
                command_tx,
                event_rx,
                worker: None,
                local_client_id,
                role: NetworkRole::Client,
                client_status: ClientStatusState::default(),
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
    let host_name = lc_engine::LegacyCString::from_bytes(settings.player_name.as_bytes().to_vec())
        .ok_or_else(|| anyhow!("host player name contains an interior NUL"))?;
    let host_config = match settings.prepared.as_ref() {
        Some(prepared) => match prepared.claim_host_config() {
            Ok(config) => config,
            Err(error) => {
                let message = format!("prepared host launch rejected: {error}");
                let _ = local_id_tx.send(Err(message.clone()));
                return Err(anyhow!(message));
            }
        },
        None => HostConfig {
            backlog_limit: 256,
            max_players: 8,
            resync_interval: Duration::from_millis(200),
            resync_cooldown: Duration::from_secs(2),
            start_tick: 0,
            local_core: lc_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: host_name.clone(),
                nick: host_name,
                lobby_ready: false,
            },
            // A C++ client must not be admitted until the selected scenario,
            // game resources, and synchronized dynamic are represented by real
            // C4Network2ResCore values and can be served by the resource layer.
            allow_join: false,
            initial_join_snapshot: None,
            resource_directory: Some(PathBuf::from("Network")),
            ..HostConfig::default()
        },
    };
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
                    NetworkCommand::SetJoinAllowed {
                        allowed,
                        completion,
                    } => {
                        match host.set_join_allowed(allowed).await {
                            Ok(()) => {
                                let _ = completion.send(Ok(()));
                            }
                            Err(error) => {
                                let message = format!(
                                    "host join-admission change failed: {error}"
                                );
                                let _ = completion.send(Err(message.clone()));
                                return Err(anyhow!(message));
                            }
                        }
                    }
                    NetworkCommand::ChangeStatus(status) => {
                        host.change_status(status)
                            .await
                            .map_err(|err| anyhow!("host status change failed: {err}"))?;
                    }
                    NetworkCommand::StatusReached => {
                        host.status_reached()
                            .await
                            .map_err(|err| anyhow!("host status arrival failed: {err}"))?;
                    }
                    NetworkCommand::AcknowledgeRequestedStatus(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "host attempted to send a client status acknowledgement".to_string(),
                        ));
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
        HostEvent::StatusCommitted(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
        }
        HostEvent::StatusAck { .. } => {
            // lc-network's status barrier consumes this before app-level
            // status transitions are enabled.
        }
        HostEvent::ActivationRequest { .. } => {
            // C++ eligibility needs synchronized client state, barrier
            // readiness, ping and host-frame inputs. The transport event is
            // intentionally not converted into an activation control here.
        }
        HostEvent::PlayerInfoUpdate { client_id, request } => {
            let _ = event_tx.send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: client_id,
                request,
                by_host: false,
            });
        }
        HostEvent::ResourceAction(action) => {
            let _ = event_tx.send(NetworkEvent::ResourceAction(action));
        }
        HostEvent::ResourceComplete {
            resource_id,
            core,
            path,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path,
            });
        }
        HostEvent::ResourceLoadFailed { resource_id } => {
            let _ = event_tx.send(NetworkEvent::ResourceLoadFailed { resource_id });
        }
        HostEvent::ResourceDeriveUnsupported { core } => {
            let _ = event_tx.send(NetworkEvent::ResourceDeriveUnsupported { core });
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
        HostEvent::JoinDataNeeded { .. } => {
            // The app publishes a fresh synchronized dynamic through the host
            // command path; the joining socket remains accepted meanwhile.
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
        HostEvent::SyncScheduled {
            control_tick,
            controls,
        } => {
            emit_scheduled_sync_controls(control_tick, controls, event_tx)?;
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
        ClientConfig::new(player_name.clone(), ParticipantKind::Player)
            .with_resource_directory(PathBuf::from("Network")),
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
    let (client_id, initial_status) = announce_connected_client(
        &mut client,
        player_name,
        &event_tx,
        &local_id_tx,
    )?;
    let mut client_events = client.take_event_receiver();
    let mut frame_builder = ControlFrameAccumulator::new(client_id);
    let mut client_status = ClientStatusState::default();
    client_status.receive_request(initial_status);

    loop {
        tokio::select! {
            maybe_event = client_events.recv() => {
                match maybe_event {
                    Some(ClientEvent::Status(status)) => {
                        client_status.receive_request(status);
                        handle_client_event(
                            ClientEvent::Status(status),
                            local_owner,
                            client_id,
                            &event_tx,
                        ).await?;
                    }
                    Some(ClientEvent::StatusAck(status)) => {
                        if client_status.commit(status) {
                            handle_client_event(
                                ClientEvent::StatusAck(status),
                                local_owner,
                                client_id,
                                &event_tx,
                            ).await?;
                        }
                    }
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
                    NetworkCommand::SetJoinAllowed { completion, .. } => {
                        let message =
                            "client attempted to change host join admission".to_string();
                        let _ = completion.send(Err(message.clone()));
                        let _ = event_tx.send(NetworkEvent::Error(
                            message,
                        ));
                    }
                    NetworkCommand::ChangeStatus(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to change authoritative game status".to_string(),
                        ));
                    }
                    NetworkCommand::StatusReached => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to mark authoritative game status reached".to_string(),
                        ));
                    }
                    NetworkCommand::AcknowledgeRequestedStatus(expected) => {
                        let Some(status) = client_status.acknowledge_requested(expected) else {
                            let _ = event_tx.send(NetworkEvent::Error(
                                "requested game status changed before client acknowledgement"
                                    .to_string(),
                            ));
                            continue;
                        };
                        if let Err(err) = client.submit_status_ack(status).await {
                            client_status.restore_request(status);
                            return Err(anyhow!("client status acknowledgement failed: {err}"));
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

fn announce_connected_client(
    client: &mut ClientHandle,
    player_name: String,
    event_tx: &Sender<NetworkEvent>,
    local_id_tx: &mpsc::Sender<Result<ClientId, String>>,
) -> Result<(ClientId, NetworkStatus)> {
    let join_data = match client.take_join_data() {
        Some(join_data) => join_data,
        None => {
            let message = "connected client did not retain JoinData".to_string();
            let _ = local_id_tx.send(Err(message.clone()));
            return Err(anyhow!(message));
        }
    };
    let initial_status = initial_client_status(&join_data);
    let client_id = client.client_id();
    let _ = event_tx.send(NetworkEvent::JoinData(join_data));
    let _ = local_id_tx.send(Ok(client_id));
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        client_id,
        name: player_name,
        kind: ParticipantKind::Player,
    });
    Ok((client_id, initial_status))
}

fn initial_client_status(join_data: &lc_network::JoinDataEnvelope) -> NetworkStatus {
    NetworkStatus {
        target_tick: join_data.start_control_tick,
        ..join_data.status
    }
}

async fn handle_client_event(
    event: ClientEvent,
    local_owner: i32,
    client_id: ClientId,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    match event {
        ClientEvent::Status(status) => {
            let _ = event_tx.send(NetworkEvent::StatusRequested(status));
        }
        ClientEvent::StatusAck(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
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
        ClientEvent::SyncScheduled {
            control_tick,
            controls,
        } => {
            emit_scheduled_sync_controls(control_tick, controls, event_tx)?;
        }
        ClientEvent::ResourceAction(action) => {
            let _ = event_tx.send(NetworkEvent::ResourceAction(action));
        }
        ClientEvent::ResourceComplete {
            resource_id,
            core,
            path,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path,
            });
        }
        ClientEvent::ResourceLoadFailed { resource_id } => {
            let _ = event_tx.send(NetworkEvent::ResourceLoadFailed { resource_id });
        }
        ClientEvent::ResourceDeriveUnsupported { core } => {
            let _ = event_tx.send(NetworkEvent::ResourceDeriveUnsupported { core });
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
    let controls = frame
        .controls
        .into_iter()
        .filter_map(network_control_for_packet)
        .collect();
    // C4GameControl::Execute obtains one complete C4Control for ControlTick
    // and executes it before simulation (src/C4GameControl.cpp:289-316).
    // Retain the decoded order (including SyncCheck positions) and even an
    // empty tick so "ready with no input" differs from "not ready".
    let _ = event_tx.send(NetworkEvent::ReadyTick { tick, controls });
    Ok(())
}

fn emit_scheduled_sync_controls(
    tick: Tick,
    controls: Vec<lc_engine::ControlPacket>,
    event_tx: &Sender<NetworkEvent>,
) -> Result<()> {
    let controls = controls
        .into_iter()
        .filter_map(network_control_for_packet)
        .collect();
    let _ = event_tx.send(NetworkEvent::ScheduledSync { tick, controls });
    Ok(())
}

fn network_control_for_packet(control: lc_engine::ControlPacket) -> Option<NetworkControl> {
    match control {
        lc_engine::ControlPacket::ClientJoin(join) => Some(NetworkControl::ClientJoin(join)),
        lc_engine::ControlPacket::ClientUpdate(update) => {
            Some(NetworkControl::ClientUpdate(update))
        }
        lc_engine::ControlPacket::ClientRemove(remove) => {
            Some(NetworkControl::ClientRemove(remove))
        }
        lc_engine::ControlPacket::PlayerControl(data) => {
            control_event_for_player_control(&data).map(|event| NetworkControl::Player {
                owner: data.player,
                event,
            })
        }
        lc_engine::ControlPacket::SyncCheck(packet) => Some(NetworkControl::SyncCheck(packet)),
        lc_engine::ControlPacket::PlayerInfo(info) => Some(NetworkControl::PlayerInfo(info)),
        lc_engine::ControlPacket::JoinPlayer(join) => Some(NetworkControl::JoinPlayer(join)),
        lc_engine::ControlPacket::Unknown { .. } => None,
    }
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

    #[tokio::test(flavor = "multi_thread")]
    async fn connected_client_emits_exact_join_data_before_peer_events() {
        // HandleJoinData applies the complete packet during client bootstrap,
        // before the client announces addresses or processes ordinary traffic
        // (src/C4Network2.cpp:1574-1623).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let address = listener.local_addr().expect("host address");
        let host_config = HostConfig::default();
        let host_core = host_config.local_core.clone();
        let host_status = host_config.initial_status;
        let snapshot = host_config
            .initial_join_snapshot
            .clone()
            .expect("default host publishes JoinData");
        let host = start_host(listener, host_config)
            .await
            .expect("start host");
        let mut client = connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let client_id = client.client_id();
        let wire_client_id = i32::try_from(client_id).expect("client ID fits wire field");
        let name = lc_engine::LegacyCString::from_bytes(b"Alice".to_vec())
            .expect("static client name is NUL-free");
        let mut parameters = snapshot.parameters;
        parameters.clients = lc_network::JoinClientRegistrySnapshot {
            clients: vec![
                host_core,
                lc_engine::ClientCoreControlData {
                    client_id: wire_client_id,
                    // The host deactivates a newly assigned core until the
                    // client requests synchronized activation
                    // (src/C4Network2.cpp:1395-1406).
                    activated: false,
                    observer: false,
                    name: name.clone(),
                    nick: name,
                    lobby_ready: false,
                },
            ],
            local_client_id: Some(wire_client_id),
        };
        let expected = lc_network::JoinDataEnvelope {
            client_id: wire_client_id,
            start_control_tick: snapshot.dynamic_tick,
            status: host_status,
            dynamic: snapshot.dynamic,
            parameters,
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (local_id_tx, local_id_rx) = mpsc::channel();

        let announced = announce_connected_client(
            &mut client,
            "Alice".to_string(),
            &event_tx,
            &local_id_tx,
        )
        .expect("announce connected client");

        assert_eq!(
            announced,
            (
                client_id,
                NetworkStatus {
                    target_tick: expected.start_control_tick,
                    ..host_status
                },
            )
        );
        assert_eq!(local_id_rx.recv().expect("local ID result"), Ok(client_id));
        assert_eq!(
            event_rx.recv().expect("JoinData event"),
            NetworkEvent::JoinData(expected)
        );
        assert_eq!(
            event_rx.recv().expect("peer event"),
            NetworkEvent::PeerConnected {
                client_id,
                name: "Alice".to_string(),
                kind: ParticipantKind::Player,
            }
        );

        client.shutdown().await.expect("client shutdown");
        host.shutdown().await.expect("host shutdown");
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
    fn host_manager_confirms_cpp_join_gate_changes_before_returning() {
        // C4Network2::AllowJoin mutates fAllowJoin synchronously, and
        // C4Game::InitNetworkHost does not enter DoLobby until it has returned
        // (src/C4Network2.cpp:835-843; src/C4Game.cpp:3869-3880).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let worker = thread::spawn(move || {
            manager
                .set_join_allowed(true)
                .expect("host confirms the live join gate change");
            manager
                .set_join_allowed(false)
                .expect("host confirms the live join gate change");
        });

        let (allowed, completion) = commands.receive_join_allowed();
        assert!(allowed);
        assert!(
            !worker.is_finished(),
            "the live gate has not acknowledged yet"
        );
        completion.send(Ok(())).expect("acknowledge open gate");

        let (allowed, completion) = commands.receive_join_allowed();
        assert!(!allowed);
        assert!(
            !worker.is_finished(),
            "the live gate has not acknowledged yet"
        );
        completion.send(Ok(())).expect("acknowledge closed gate");
        worker.join().expect("gate caller exits after both acks");
    }

    #[test]
    fn host_manager_queues_cpp_status_change_and_local_reach() {
        // ChangeGameStatus is host-only and CheckStatusReached records the
        // host's local arrival independently of remote acknowledgements
        // (src/C4Network2.cpp:2017-2051,2053-2086).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };

        manager
            .change_status(status)
            .expect("host queues status change");
        assert_eq!(commands.take_status_changes(), vec![status]);

        manager
            .status_reached()
            .expect("host queues local status arrival");
        assert_eq!(commands.take_status_reached(), 1);
    }

    #[test]
    fn client_manager_cannot_change_the_host_join_gate() {
        // C4Network2::AllowJoin is a host-only operation
        // (src/C4Network2.cpp:835-843).
        let (manager, _events) = NetworkManager::test_stub_for_client_id(7);

        assert_eq!(
            manager
                .set_join_allowed(true)
                .expect_err("client must not control host admission")
                .to_string(),
            "only the network host may change join admission"
        );
    }

    #[test]
    fn client_manager_acks_the_exact_prepared_status() {
        // A client echoes the status it actually reached in PID_StatusAck;
        // it does not synthesize a new state or control mode
        // (src/C4Network2.cpp:2074-2084).
        let (mut manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        event_tx
            .send(NetworkEvent::StatusRequested(status))
            .expect("queue exact host status");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusRequested(status)]
        );

        manager
            .acknowledge_requested_status()
            .expect("client queues the reached status acknowledgement");

        assert_eq!(commands.take_status_acknowledgements(), vec![status]);
        assert_eq!(manager.client_status.awaiting_commit, Some(status));
    }

    #[test]
    fn initial_join_status_ack_uses_the_initialized_control_tick() {
        // HandleJoinData installs the reference status first, then control
        // initialization and DoLobby retarget it to the client's current
        // control tick before CheckStatusReached echoes PID_StatusAck
        // (src/C4Network2.cpp:1574-1623,445-453,2073-2084).
        let (mut manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let host_config = HostConfig::default();
        let mut reference_status = host_config.initial_status;
        reference_status.target_tick = -1;
        let snapshot = host_config
            .initial_join_snapshot
            .expect("default host publishes JoinData");
        let join_data = lc_network::JoinDataEnvelope {
            client_id: 7,
            start_control_tick: 23,
            status: reference_status,
            dynamic: snapshot.dynamic,
            parameters: snapshot.parameters,
        };
        event_tx
            .send(NetworkEvent::JoinData(join_data.clone()))
            .expect("queue initial JoinData");
        assert_eq!(manager.poll_events(), vec![NetworkEvent::JoinData(join_data)]);

        manager
            .acknowledge_requested_status()
            .expect("acknowledge initialized JoinData status");

        assert_eq!(
            commands.take_status_acknowledgements(),
            vec![NetworkStatus {
                target_tick: 23,
                ..reference_status
            }]
        );
    }

    #[test]
    fn client_manager_rejects_status_ack_without_a_pending_request() {
        // CheckStatusReached can only echo the C4Network2::Status currently
        // installed by HandleStatus; there is no forgeable packet parameter
        // (src/C4Network2.cpp:1501-1511,2053-2084).
        let (mut manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);

        assert_eq!(
            manager
                .acknowledge_requested_status()
                .expect_err("client has no host status to acknowledge"),
            NetworkStatusCommandError::NoRequestedStatus
        );
        assert!(commands.take_status_acknowledgements().is_empty());
    }

    #[test]
    fn client_manager_commits_only_the_exact_acknowledged_status() {
        // The client ignores PID_StatusAck unless it matches the stored state
        // and exact target tick, and only a locally reached barrier can commit
        // (src/C4Network2.cpp:1513-1543).
        let (mut manager, event_tx, _commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        event_tx
            .send(NetworkEvent::StatusRequested(status))
            .expect("queue host status");
        assert_eq!(manager.poll_events().len(), 1);
        manager
            .acknowledge_requested_status()
            .expect("acknowledge exact host status");

        let stale = NetworkStatus {
            target_tick: 40,
            ..status
        };
        event_tx
            .send(NetworkEvent::StatusCommitted(stale))
            .expect("queue stale host acknowledgement");
        assert!(manager.poll_events().is_empty());
        assert_eq!(manager.client_status.awaiting_commit, Some(status));

        event_tx
            .send(NetworkEvent::StatusCommitted(status))
            .expect("queue exact host acknowledgement");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusCommitted(status)]
        );
        assert_eq!(manager.client_status.awaiting_commit, None);
    }

    #[test]
    fn status_commands_reject_the_wrong_runtime_role_with_typed_errors() {
        // ChangeGameStatus is host-only, while the non-host branch of
        // CheckStatusReached sends PID_StatusAck back to the host
        // (src/C4Network2.cpp:2017-2021,2073-2084).
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (client, _events) = NetworkManager::test_stub_for_client_id(7);
        assert_eq!(
            client
                .change_status(status)
                .expect_err("client cannot author status"),
            NetworkStatusCommandError::HostRoleRequired {
                operation: "change game status",
            }
        );
        assert_eq!(
            client
                .status_reached()
                .expect_err("client cannot mark the host barrier reached"),
            NetworkStatusCommandError::HostRoleRequired {
                operation: "mark game status reached",
            }
        );

        let (mut host, _events) = NetworkManager::test_stub();
        assert_eq!(
            host.acknowledge_requested_status()
                .expect_err("host cannot send a client acknowledgement"),
            NetworkStatusCommandError::ClientRoleRequired {
                operation: "acknowledge a host game status",
            }
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
    fn scheduled_sync_retains_client_lifecycle_controls_in_order() {
        // C4GameControlNetwork drains one FIFO SyncControl list at the tagged
        // control tick; ClientUpdate must remain ahead of ClientRemove
        // (src/C4GameControlNetwork.cpp:260-297,786-830).
        let update = lc_engine::ClientUpdateControlData {
            update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        };
        let remove = lc_engine::ClientRemoveControlData {
            client_id: 4,
            reason: lc_engine::LegacyCString::from_bytes(b"bye".to_vec())
                .expect("valid reason"),
            by_client: 0,
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_scheduled_sync_controls(
            23,
            vec![
                lc_engine::ControlPacket::ClientUpdate(update.clone()),
                lc_engine::ControlPacket::ClientRemove(remove.clone()),
            ],
            &event_tx,
        )
        .expect("emit scheduled sync controls");

        assert_eq!(
            event_rx.recv().expect("scheduled sync event"),
            NetworkEvent::ScheduledSync {
                tick: 23,
                controls: vec![
                    NetworkControl::ClientUpdate(update),
                    NetworkControl::ClientRemove(remove),
                ],
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_status_commit_is_forwarded_to_the_app() {
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = mpsc::channel();

        handle_host_event(HostEvent::StatusCommitted(status), 0, &event_tx)
            .await
            .expect("forward committed status");

        assert_eq!(
            event_rx.recv().expect("status event"),
            NetworkEvent::StatusCommitted(status)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_status_request_waits_for_app_preparation() {
        // HandleStatus stores the host-authored status, but the client sends
        // PID_StatusAck only after CheckStatusReached observes local arrival
        // (src/C4Network2.cpp:2017-2051,2053-2086).
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = mpsc::channel();

        handle_client_event(ClientEvent::Status(status), 0, 7, &event_tx)
            .await
            .expect("forward requested status");

        assert_eq!(
            event_rx.recv().expect("status request event"),
            NetworkEvent::StatusRequested(status)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_status_ack_commits_the_exact_host_status() {
        // The host broadcasts PID_StatusAck only when its local state and all
        // waited-for clients have reached the barrier; that packet releases
        // the client's status wait (src/C4Network2.cpp:2088-2113).
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        let (event_tx, event_rx) = mpsc::channel();

        handle_client_event(ClientEvent::StatusAck(status), 0, 7, &event_tx)
            .await
            .expect("forward committed status");

        assert_eq!(
            event_rx.try_recv(),
            Ok(NetworkEvent::StatusCommitted(status))
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
