use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use lc_engine::{
    CommandKind, ControlButton, ControlCommand, ControlEvent,
    JoinPlayerControlData, PlayerCommandControlData, PlayerControlData, PlayerInfoControlData,
    MessageBoardAnswerControlData, MessageControlData, PlayerSelectControlData, ScriptControlData,
    SyncCheckPacket,
    COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG,
    COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER,
    COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT, COM_MENU_SHOW_TEXT,
    COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL,
    COM_SPECIAL2, COM_THROW, COM_UP,
};
use lc_network::{
    connect_client, decode_control_entry_payload, decode_control_packet,
    encode_control_entry_payload, encode_control_packet, start_host, ClientConfig, ClientEvent,
    ClientHandle, ClientId, ClientPlayerResourceRequest, ControlDelivery, ControlPacket,
    HostConfig, HostEvent, HostHandle, HostJoinSnapshot, LegacyControlFrame, LegacyControlSet,
    NetworkAddress, NetworkProtocol, NetworkStatus, ParticipantKind, Tick,
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
    pub resource_directory: PathBuf,
    pub local_system_path: Option<PathBuf>,
    pub local_resource_roots: Vec<PathBuf>,
}

impl ClientSettings {
    pub fn new(server_addr: SocketAddr, player_name: impl Into<String>) -> Self {
        Self {
            server_addr,
            player_name: player_name.into(),
            resource_directory: PathBuf::from("Network"),
            local_system_path: None,
            local_resource_roots: Vec::new(),
        }
    }
}

const HOST_CLIENT_ID: ClientId = 0;
const NETWORK_TELEMETRY_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkRole {
    Host,
    Client,
}

const MAX_CONTROL_RATE: i32 = 20;

/// C4GameControl's frame-to-ControlTick cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NetworkControlClock {
    control_tick: i32,
    control_rate: u64,
}

impl NetworkControlClock {
    pub(crate) fn new(start_tick: i32, control_rate: i32) -> Self {
        Self {
            control_tick: start_tick,
            control_rate: control_rate.clamp(1, MAX_CONTROL_RATE) as u64,
        }
    }

    /// Current control tick on frames where C4GameControl executes control.
    /// This is a non-consuming probe because a network stall retries the same
    /// frame and tick until `CtrlReady` succeeds.
    pub(crate) fn tick_for_frame(self, frame: u64) -> Option<i32> {
        if frame % self.control_rate != 0 {
            return None;
        }
        Some(self.control_tick)
    }

    /// Consume the tick whose control frame was admitted by `tick_for_frame`.
    /// Keep this independent of the current rate: a CID_Set in that frame may
    /// already have changed the cadence before execution completes.
    pub(crate) fn complete_control_frame(&mut self) {
        self.control_tick = self.control_tick.wrapping_add(1);
    }

    /// `C4CVT_ControlRate`: preserve the absolute FrameCounter phase while
    /// changing the divisor used by all subsequent frame probes.
    pub(crate) fn adjust_control_rate(&mut self, delta: i32) -> i32 {
        let control_rate = (self.control_rate as i32)
            .saturating_add(delta)
            .clamp(1, MAX_CONTROL_RATE);
        self.control_rate = control_rate as u64;
        control_rate
    }

    pub(crate) fn control_rate(self) -> i32 {
        self.control_rate as i32
    }

    pub(crate) fn current_tick(self) -> i32 {
        self.control_tick
    }

    pub(crate) fn engine_timing(
        self,
    ) -> Result<lc_engine::NetworkControlTiming, lc_engine::InvalidNetworkControlRate> {
        lc_engine::NetworkControlTiming::new(self.control_tick, self.control_rate as i32)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct NetworkWorkerReady {
    local_client_id: ClientId,
    local_addresses: Vec<NetworkAddress>,
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

fn same_client_status_barrier(left: NetworkStatus, right: NetworkStatus) -> bool {
    left.state == right.state && left.target_tick == right.target_tick
}

impl ClientStatusState {
    fn receive_request(&mut self, status: NetworkStatus) -> bool {
        // The host rebroadcasts the higher target supplied by this client's
        // acknowledgement before it can commit. The client has already
        // reached that exact barrier, so retain its awaiting-commit identity
        // instead of reopening an unreached request.
        if self
            .awaiting_commit
            .is_some_and(|awaiting| same_client_status_barrier(awaiting, status))
        {
            return false;
        }
        self.requested = Some(status);
        self.awaiting_commit = None;
        true
    }

    fn acknowledge_requested_at(
        &mut self,
        expected: NetworkStatus,
        current_control_tick: i32,
    ) -> Option<NetworkStatus> {
        let requested = self
            .requested
            .filter(|requested| same_client_status_barrier(*requested, expected))?;
        let acknowledgement = NetworkStatus {
            target_tick: current_control_tick,
            ..requested
        };
        self.requested = None;
        self.awaiting_commit = Some(acknowledgement);
        Some(acknowledgement)
    }

    fn restore_request(&mut self, requested: NetworkStatus, acknowledgement: NetworkStatus) {
        if self
            .awaiting_commit
            .is_some_and(|awaiting| same_client_status_barrier(awaiting, acknowledgement))
        {
            self.awaiting_commit = None;
            self.requested = Some(requested);
        }
    }

    fn commit(&mut self, status: NetworkStatus) -> bool {
        if !self
            .awaiting_commit
            .is_some_and(|awaiting| same_client_status_barrier(awaiting, status))
        {
            return false;
        }
        self.awaiting_commit = None;
        true
    }
}

const CLIENT_ACTIVATION_RETRY_INTERVAL: Duration = Duration::from_millis(5_000);

#[derive(Debug, Default)]
struct ClientActivationState {
    armed: bool,
    status_reached: bool,
    current_frame: i32,
    last_request_at: Option<tokio::time::Instant>,
}

impl ClientActivationState {
    fn arm_for_queued_player_info(&mut self, request: &lc_network::PlayerInfoUpdateRequest) {
        if request.flags & lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL != 0
            && !request.players.is_empty()
        {
            self.armed = true;
        }
    }

    fn status_reached(&mut self, current_frame: i32) {
        self.status_reached = true;
        self.current_frame = current_frame;
    }

    fn status_requested(&mut self) {
        self.status_reached = false;
    }

    fn refresh_frame(&mut self, current_frame: i32) {
        self.current_frame = current_frame;
    }

    fn mark_requested(&mut self, now: tokio::time::Instant) {
        self.last_request_at = Some(now);
    }

    fn apply_executed_client_update(
        &mut self,
        local_client_id: i32,
        update: &lc_engine::ClientUpdateControlData,
    ) {
        let activates = update.update_type == lc_engine::CLIENT_UPDATE_ACTIVATE && update.data != 0;
        let observes = update.update_type == lc_engine::CLIENT_UPDATE_SET_OBSERVER;
        if update.by_client == 0 && update.client_id == local_client_id && (activates || observes) {
            self.armed = false;
            self.last_request_at = None;
        }
    }

    fn request_tick_if_due(&self, now: tokio::time::Instant) -> Option<i32> {
        (self.armed
            && self.status_reached
            && self
                .last_request_at
                .is_none_or(|last| now >= last + CLIENT_ACTIVATION_RETRY_INTERVAL))
        .then_some(self.current_frame)
    }

    fn next_retry_at(&self) -> Option<tokio::time::Instant> {
        (self.armed && self.status_reached)
            .then(|| {
                self.last_request_at
                    .map(|last| last + CLIENT_ACTIVATION_RETRY_INTERVAL)
            })
            .flatten()
    }
}

#[derive(Debug)]
pub struct NetworkManager {
    command_tx: tokio_mpsc::Sender<NetworkCommand>,
    event_rx: Receiver<NetworkEvent>,
    telemetry_rx: Receiver<NetworkEvent>,
    worker: Option<thread::JoinHandle<()>>,
    local_client_id: ClientId,
    local_addresses: Vec<NetworkAddress>,
    role: NetworkRole,
    client_status: ClientStatusState,
}

#[cfg(test)]
pub(crate) struct TestNetworkCommands {
    command_rx: tokio_mpsc::Receiver<NetworkCommand>,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestLobbyStartCommand {
    Countdown(lc_network::LobbyCountdownPacket),
    Status(NetworkStatus),
}

#[cfg(test)]
type RuntimeHostJoinResult = (
    Vec<&'static str>,
    Vec<ClientPlayerResourceRequest>,
    Vec<PlayerInfoControlData>,
    Vec<(Tick, JoinPlayerControlData)>,
);

#[cfg(test)]
impl TestNetworkCommands {
    pub(crate) fn take_lobby_start_commands(&mut self) -> Vec<TestLobbyStartCommand> {
        let mut observed = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitLobbyCountdown(packet) => {
                    observed.push(TestLobbyStartCommand::Countdown(packet));
                }
                NetworkCommand::ChangeStatus(status) => {
                    observed.push(TestLobbyStartCommand::Status(status));
                }
                command => panic!("unexpected lobby-start command: {command:?}"),
            }
        }
        observed
    }

    pub(crate) fn complete_runtime_host_join(
        mut self,
        published_core: lc_engine::NetworkResourceCore,
        event_tx: Sender<NetworkEvent>,
        direct_ready: Sender<()>,
    ) -> RuntimeHostJoinResult {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut order = Vec::new();
        let mut publications = Vec::new();
        let mut player_infos = Vec::new();
        let mut joins = Vec::new();
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::PublishPlayerResource {
                    request,
                    completion,
                }) => {
                    order.push("publish");
                    publications.push(request);
                    let _ = completion.send(Ok(published_core.clone()));
                }
                Ok(NetworkCommand::BroadcastPlayerInfo(info)) => {
                    order.push("player-info");
                    player_infos.push(info.clone());
                    let _ = event_tx.send(NetworkEvent::DirectControl(NetworkControl::PlayerInfo(
                        info,
                    )));
                    let _ = direct_ready.send(());
                }
                Ok(NetworkCommand::SubmitJoinPlayer { tick, join }) => {
                    order.push("join-player");
                    joins.push((tick, join));
                    break;
                }
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected runtime-host command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        (order, publications, player_infos, joins)
    }

    pub(crate) fn complete_initial_client_join(
        mut self,
        published_cores: Vec<lc_engine::NetworkResourceCore>,
    ) -> (
        Vec<&'static str>,
        Vec<ClientPlayerResourceRequest>,
        Vec<lc_network::PlayerInfoUpdateRequest>,
        Vec<NetworkStatus>,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut order = Vec::new();
        let mut publications = Vec::new();
        let mut player_infos = Vec::new();
        let mut acknowledgements = Vec::new();
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::PublishPlayerResource {
                    request,
                    completion,
                }) => {
                    order.push("publish");
                    let result = published_cores
                        .get(publications.len())
                        .cloned()
                        .ok_or_else(|| "test did not provide a publication core".to_string());
                    publications.push(request);
                    let _ = completion.send(result);
                }
                Ok(NetworkCommand::SubmitPlayerInfoUpdate(request)) => {
                    order.push("player-info");
                    player_infos.push(request);
                }
                Ok(NetworkCommand::AcknowledgeRequestedStatus {
                    mut status,
                    current_control_tick,
                    ..
                }) => {
                    order.push("status-ack");
                    status.target_tick = current_control_tick;
                    acknowledgements.push(status);
                    break;
                }
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected initial-client command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        (order, publications, player_infos, acknowledgements)
    }

    pub(crate) fn take_submitted_local(&mut self) -> Vec<(i32, ControlEvent, Tick)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitLocal { owner, event, tick } = command {
                submitted.push((owner, event, tick));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_player_commands(
        &mut self,
    ) -> Vec<(Tick, PlayerCommandControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerCommand { tick, command } = command {
                submitted.push((tick, command));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_player_selects(
        &mut self,
    ) -> Vec<(Tick, PlayerSelectControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerSelect { tick, selection } = command {
                submitted.push((tick, selection));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_scripts(&mut self) -> Vec<(Tick, ScriptControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitScript { tick, script } = command {
                submitted.push((tick, script));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_message_board_answers(
        &mut self,
    ) -> Vec<(Tick, MessageBoardAnswerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitMessageBoardAnswer { tick, answer } = command {
                submitted.push((tick, answer));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_messages(&mut self) -> Vec<MessageControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitMessage(message) = command {
                submitted.push(message);
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

    pub(crate) fn take_published_join_snapshots(&mut self) -> Vec<HostJoinSnapshot> {
        let mut snapshots = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::PublishJoinSnapshot { snapshot } = command {
                snapshots.push(snapshot);
            }
        }
        snapshots
    }

    pub(crate) fn take_team_control_updates(
        &mut self,
    ) -> (Vec<PlayerInfoControlData>, Vec<HostJoinSnapshot>) {
        let mut player_infos = Vec::new();
        let mut snapshots = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::BroadcastPlayerInfo(info) => player_infos.push(info),
                NetworkCommand::PublishJoinSnapshot { snapshot } => snapshots.push(snapshot),
                _ => {}
            }
        }
        (player_infos, snapshots)
    }

    pub(crate) fn take_submitted_join_players(&mut self) -> Vec<(Tick, JoinPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitJoinPlayer { tick, join } = command {
                submitted.push((tick, join));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_remove_players(
        &mut self,
    ) -> Vec<(Tick, lc_engine::RemovePlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitRemovePlayer { tick, remove } = command {
                submitted.push((tick, remove));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_client_updates(
        &mut self,
    ) -> Vec<lc_engine::ClientUpdateControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitClientUpdate(update) = command {
                submitted.push(update);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_client_removes(
        &mut self,
    ) -> Vec<lc_engine::ClientRemoveControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitClientRemove(remove) = command {
                submitted.push(remove);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_init_scenario_players(
        &mut self,
    ) -> Vec<(Tick, lc_engine::InitScenarioPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitInitScenarioPlayer { tick, selection } = command {
                submitted.push((tick, selection));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_surrender_players(
        &mut self,
    ) -> Vec<(Tick, lc_engine::SurrenderPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitSurrenderPlayer { tick, surrender } = command {
                submitted.push((tick, surrender));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_internal_player_scripts(
        &mut self,
    ) -> Vec<(Tick, lc_engine::ControlPacket)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitInternalPlayerScript { tick, control } = command {
                submitted.push((tick, control));
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_votes(&mut self) -> Vec<lc_engine::VoteControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitVote(vote) = command {
                submitted.push(vote);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_vote_ends(&mut self) -> Vec<lc_engine::VoteControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitVoteEnd(result) = command {
                submitted.push(result);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_ready_checks(&mut self) -> Vec<lc_network::ReadyCheckPacket> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitReadyCheck(packet) = command {
                submitted.push(packet);
            }
        }
        submitted
    }

    pub(crate) fn take_submitted_lobby_countdowns(
        &mut self,
    ) -> Vec<lc_network::LobbyCountdownPacket> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitLobbyCountdown(packet) = command {
                submitted.push(packet);
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

    pub(crate) fn receive_resource_removal(
        &mut self,
    ) -> (i32, Sender<std::result::Result<(), String>>) {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::RemoveResource {
                resource_id,
                completion,
            }) => (resource_id, completion),
            Some(command) => panic!("expected resource-removal command, got {command:?}"),
            None => panic!("network command channel ended before resource-removal command"),
        }
    }

    pub(crate) fn receive_graceful_part(&mut self) -> Sender<std::result::Result<(), String>> {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::GracefulPart { completion }) => completion,
            Some(command) => panic!("expected graceful-part command, got {command:?}"),
            None => panic!("network command channel ended before graceful-part command"),
        }
    }

    pub(crate) fn complete_graceful_part(mut self) -> bool {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::GracefulPart { completion }) => {
                let _ = completion.send(Ok(()));
                true
            }
            Some(NetworkCommand::Shutdown) | None => false,
            Some(command) => panic!("unexpected teardown command: {command:?}"),
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
            if let NetworkCommand::AcknowledgeRequestedStatus {
                mut status,
                current_control_tick,
                ..
            } = command
            {
                status.target_tick = current_control_tick;
                acknowledgements.push(status);
            }
        }
        acknowledgements
    }

    pub(crate) fn take_framed_status_acknowledgements(&mut self) -> Vec<(NetworkStatus, i32)> {
        let mut acknowledgements = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::AcknowledgeRequestedStatus {
                mut status,
                current_control_tick,
                current_frame,
            } = command
            {
                status.target_tick = current_control_tick;
                acknowledgements.push((status, current_frame));
            }
        }
        acknowledgements
    }

    pub(crate) fn take_executed_client_updates(
        &mut self,
    ) -> Vec<lc_engine::ClientUpdateControlData> {
        let mut updates = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::ClientUpdateExecuted(update) = command {
                updates.push(update);
            }
        }
        updates
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum NetworkEvent {
    JoinData(lc_network::JoinDataEnvelope),
    LeagueRoundResults(lc_network::LeagueRoundResultsPacket),
    LobbyCountdown(lc_network::LobbyCountdownPacket),
    ReadyCheck(lc_network::ReadyCheckPacket),
    StatusRequested(NetworkStatus),
    StatusCommitted(NetworkStatus),
    ActivationRequest {
        client_id: ClientId,
        tick: i32,
        waited_for: bool,
        ping_ms: i32,
    },
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
    RemovePlayer(lc_engine::RemovePlayerControlData),
    SurrenderPlayer(lc_engine::SurrenderPlayerControlData),
    ActivateGameGoalMenu(lc_engine::ActivateGameGoalMenuControlData),
    ToggleHostility(lc_engine::ToggleHostilityControlData),
    ActivateGameGoalRule(lc_engine::ActivateGameGoalRuleControlData),
    SetPlayerTeam(lc_engine::SetPlayerTeamControlData),
    EliminatePlayer(lc_engine::EliminatePlayerControlData),
    Vote(lc_engine::VoteControlData),
    VoteEnd(lc_engine::VoteControlData),
    PlayerControl(PlayerControlData),
    PlayerCommand(PlayerCommandControlData),
    PlayerSelect(PlayerSelectControlData),
    Script(ScriptControlData),
    Message(MessageControlData),
    MessageBoardAnswer(MessageBoardAnswerControlData),
    CustomCommand(lc_engine::CustomCommandControlData),
    EmMoveObject(lc_engine::EmMoveObjectControlData),
    EmDrawTool(lc_engine::EmDrawToolControlData),
    EmDropDef(lc_engine::EmDropDefControlData),
    Player { owner: i32, event: ControlEvent },
    InitScenarioPlayer(lc_engine::InitScenarioPlayerControlData),
    Synchronize(lc_engine::SynchronizeControlData),
    SyncCheck(SyncCheckPacket),
    Set(LegacyControlSet),
}

#[derive(Debug)]
enum NetworkCommand {
    PublishPlayerResource {
        request: ClientPlayerResourceRequest,
        completion: Sender<std::result::Result<lc_engine::NetworkResourceCore, String>>,
    },
    RemoveResource {
        resource_id: i32,
        completion: Sender<std::result::Result<(), String>>,
    },
    SubmitPlayerInfoUpdate(lc_network::PlayerInfoUpdateRequest),
    BroadcastPlayerInfo(PlayerInfoControlData),
    SubmitJoinPlayer {
        tick: Tick,
        join: JoinPlayerControlData,
    },
    SubmitRemovePlayer {
        tick: Tick,
        remove: lc_engine::RemovePlayerControlData,
    },
    SubmitClientUpdate(lc_engine::ClientUpdateControlData),
    SubmitClientRemove(lc_engine::ClientRemoveControlData),
    SubmitInitScenarioPlayer {
        tick: Tick,
        selection: lc_engine::InitScenarioPlayerControlData,
    },
    SubmitSurrenderPlayer {
        tick: Tick,
        surrender: lc_engine::SurrenderPlayerControlData,
    },
    SubmitInternalPlayerScript {
        tick: Tick,
        control: lc_engine::ControlPacket,
    },
    SubmitMessage(MessageControlData),
    SubmitVote(lc_engine::VoteControlData),
    SubmitVoteEnd(lc_engine::VoteControlData),
    SubmitReadyCheck(lc_network::ReadyCheckPacket),
    SubmitLobbyCountdown(lc_network::LobbyCountdownPacket),
    SubmitLocal {
        owner: i32,
        event: ControlEvent,
        tick: Tick,
    },
    SubmitPlayerCommand {
        tick: Tick,
        command: PlayerCommandControlData,
    },
    SubmitPlayerSelect {
        tick: Tick,
        selection: PlayerSelectControlData,
    },
    SubmitScript {
        tick: Tick,
        script: ScriptControlData,
    },
    SubmitMessageBoardAnswer {
        tick: Tick,
        answer: MessageBoardAnswerControlData,
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
    AcknowledgeRequestedStatus {
        status: NetworkStatus,
        current_control_tick: i32,
        current_frame: i32,
    },
    ClientUpdateExecuted(lc_engine::ClientUpdateControlData),
    SetJoinAllowed {
        allowed: bool,
        completion: Sender<std::result::Result<(), String>>,
    },
    PublishJoinSnapshot {
        snapshot: HostJoinSnapshot,
    },
    GracefulPart {
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
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel::<Result<NetworkWorkerReady, String>>();
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
                        telemetry_tx,
                        local_id_tx,
                    )) {
                        let _ = event_tx.send(NetworkEvent::Error(format!("{err:?}")));
                    }
                }
            })
            .context("failed to spawn network worker thread")?;
        let ready = match local_id_rx
            .recv()
            .context("network worker did not report local client id")?
        {
            Ok(ready) => ready,
            Err(err) => return Err(anyhow!(err)),
        };

        Ok(Self {
            command_tx,
            event_rx,
            telemetry_rx,
            worker: Some(worker),
            local_client_id: ready.local_client_id,
            local_addresses: ready.local_addresses,
            role,
            client_status: ClientStatusState::default(),
        })
    }

    pub fn submit_local_control(&self, owner: i32, event: ControlEvent, tick: Tick) {
        let command = NetworkCommand::SubmitLocal { owner, event, tick };
        let _ = self.command_tx.blocking_send(command);
    }

    pub fn submit_player_command(
        &self,
        tick: Tick,
        mut command: PlayerCommandControlData,
    ) -> Result<()> {
        command.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the player-command wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitPlayerCommand { tick, command })
            .map_err(|_| anyhow!("network worker is not accepting player commands"))
    }

    pub fn submit_player_select(
        &self,
        tick: Tick,
        mut selection: PlayerSelectControlData,
    ) -> Result<()> {
        selection.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the player-select wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitPlayerSelect { tick, selection })
            .map_err(|_| anyhow!("network worker is not accepting player selections"))
    }

    pub fn submit_script_control(&self, tick: Tick, mut script: ScriptControlData) -> Result<()> {
        script.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the script-control wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitScript { tick, script })
            .map_err(|_| anyhow!("network worker is not accepting script controls"))
    }

    pub fn submit_message_board_answer(
        &self,
        tick: Tick,
        mut answer: MessageBoardAnswerControlData,
    ) -> Result<()> {
        answer.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the message-board answer wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitMessageBoardAnswer { tick, answer })
            .map_err(|_| anyhow!("network worker is not accepting message-board answers"))
    }

    /// Submit one non-synchronized `CID_Message`. C++ always sends these as
    /// `CDT_Private`, including non-private chat types; recipient visibility
    /// is decided when the app executes the message control.
    pub fn submit_message(&self, mut message: MessageControlData) -> Result<()> {
        message.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the message-control wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitMessage(message))
            .map_err(|_| anyhow!("network worker is not accepting message controls"))
    }

    pub fn broadcast_lobby_countdown(
        &self,
        countdown: lc_network::LobbyCountdownPacket,
    ) -> Result<()> {
        self.submit_lobby_countdown(countdown)
    }

    pub fn request_ready_check(&self) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may request a ready check"));
        }
        self.submit_ready_check(lc_network::ReadyCheckData::Request)
    }

    pub fn set_local_ready(&self, ready: bool) -> Result<()> {
        self.submit_ready_check(if ready {
            lc_network::ReadyCheckData::Ready
        } else {
            lc_network::ReadyCheckData::NotReady
        })
    }

    pub fn submit_player_info_update(
        &self,
        request: lc_network::PlayerInfoUpdateRequest,
    ) -> Result<()> {
        self.command_tx
            .blocking_send(NetworkCommand::SubmitPlayerInfoUpdate(request))
            .map_err(|_| anyhow!("network worker is not accepting player-info updates"))
    }

    pub fn submit_client_update(&self, update: lc_engine::ClientUpdateControlData) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a synchronized client update"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitClientUpdate(update))
            .map_err(|_| anyhow!("network worker is not accepting client updates"))
    }

    pub fn submit_client_remove(&self, remove: lc_engine::ClientRemoveControlData) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a synchronized client removal"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitClientRemove(remove))
            .map_err(|_| anyhow!("network worker is not accepting client removals"))
    }

    pub fn submit_init_scenario_player(&self, tick: Tick, player: i32, team: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the scenario-player wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitInitScenarioPlayer {
                tick,
                selection: lc_engine::InitScenarioPlayerControlData {
                    team,
                    player,
                    by_client,
                },
            })
            .map_err(|_| anyhow!("network worker is not accepting team selections"))
    }

    pub fn submit_surrender_player(&self, tick: Tick, player: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the surrender wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitSurrenderPlayer {
                tick,
                surrender: lc_engine::SurrenderPlayerControlData { player, by_client },
            })
            .map_err(|_| anyhow!("network worker is not accepting player surrender"))
    }

    fn submit_internal_player_script(
        &self,
        tick: Tick,
        control: lc_engine::ControlPacket,
    ) -> Result<()> {
        self.command_tx
            .blocking_send(NetworkCommand::SubmitInternalPlayerScript { tick, control })
            .map_err(|_| anyhow!("network worker is not accepting internal player controls"))
    }

    pub fn submit_activate_game_goal_menu(&self, tick: Tick, player: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the goal-menu wire field"))?;
        self.submit_internal_player_script(
            tick,
            lc_engine::ControlPacket::ActivateGameGoalMenu(
                lc_engine::ActivateGameGoalMenuControlData { player, by_client },
            ),
        )
    }

    pub fn submit_toggle_hostility(
        &self,
        tick: Tick,
        player: i32,
        opponent: i32,
    ) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the hostility wire field"))?;
        self.submit_internal_player_script(
            tick,
            lc_engine::ControlPacket::ToggleHostility(lc_engine::ToggleHostilityControlData {
                opponent,
                player,
                by_client,
            }),
        )
    }

    pub fn submit_activate_game_goal_rule(
        &self,
        tick: Tick,
        player: i32,
        object: i32,
    ) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the goal-rule wire field"))?;
        self.submit_internal_player_script(
            tick,
            lc_engine::ControlPacket::ActivateGameGoalRule(
                lc_engine::ActivateGameGoalRuleControlData {
                    object,
                    player,
                    by_client,
                },
            ),
        )
    }

    pub fn submit_set_player_team(&self, tick: Tick, player: i32, team: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the team-switch wire field"))?;
        self.submit_internal_player_script(
            tick,
            lc_engine::ControlPacket::SetPlayerTeam(lc_engine::SetPlayerTeamControlData {
                team,
                player,
                by_client,
            }),
        )
    }

    pub fn submit_eliminate_player(&self, tick: Tick, player: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the eliminate-player wire field"))?;
        self.submit_internal_player_script(
            tick,
            lc_engine::ControlPacket::EliminatePlayer(lc_engine::EliminatePlayerControlData {
                player,
                by_client,
            }),
        )
    }

    pub fn submit_vote(&self, vote_type: u8, approve: bool, data: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the vote wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitVote(lc_engine::VoteControlData {
                vote_type,
                approve,
                data,
                by_client,
            }))
            .map_err(|_| anyhow!("network worker is not accepting votes"))
    }

    pub fn submit_vote_end(&self, vote_type: u8, approve: bool, data: i32) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may end votes"));
        }
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the vote wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitVoteEnd(lc_engine::VoteControlData {
                vote_type,
                approve,
                data,
                by_client,
            }))
            .map_err(|_| anyhow!("network worker is not accepting vote results"))
    }

    pub fn submit_ready_check(&self, data: lc_network::ReadyCheckData) -> Result<()> {
        let client_id = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the ready-check wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitReadyCheck(
                lc_network::ReadyCheckPacket { client_id, data },
            ))
            .map_err(|_| anyhow!("network worker is not accepting ready checks"))
    }

    pub fn submit_lobby_countdown(&self, packet: lc_network::LobbyCountdownPacket) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a lobby countdown"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitLobbyCountdown(packet))
            .map_err(|_| anyhow!("network worker is not accepting lobby countdowns"))
    }

    pub fn publish_client_player_resource(
        &self,
        request: ClientPlayerResourceRequest,
    ) -> Result<lc_engine::NetworkResourceCore> {
        if self.role != NetworkRole::Client {
            return Err(anyhow!(
                "only a network client may publish a client player resource"
            ));
        }
        let (completion, published) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::PublishPlayerResource {
                request,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting player resources"))?;
        published
            .recv()
            .map_err(|_| anyhow!("network worker ended before publishing the player resource"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn publish_host_player_resource(
        &self,
        request: ClientPlayerResourceRequest,
    ) -> Result<lc_engine::NetworkResourceCore> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only a network host may publish a host player resource"
            ));
        }
        let (completion, published) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::PublishPlayerResource {
                request,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting player resources"))?;
        published
            .recv()
            .map_err(|_| anyhow!("network worker ended before publishing the player resource"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn remove_client_resource(&self, resource_id: i32) -> Result<()> {
        if self.role != NetworkRole::Client {
            return Err(anyhow!(
                "only a network client may remove a network resource"
            ));
        }
        let (completion, removed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::RemoveResource {
                resource_id,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting resource removals"))?;
        removed
            .recv()
            .map_err(|_| anyhow!("network worker ended before removing the resource"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn graceful_part(&self) -> Result<()> {
        if self.role != NetworkRole::Client {
            return Err(anyhow!("only a network client may part gracefully"));
        }
        let (completion, parted) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::GracefulPart { completion })
            .map_err(|_| anyhow!("network worker is not accepting graceful departure"))?;
        parted
            .recv()
            .map_err(|_| anyhow!("network worker ended before confirming graceful departure"))?
            .map_err(|message| anyhow!(message))
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

    pub fn submit_remove_player(
        &self,
        tick: Tick,
        player: i32,
        disconnected: bool,
    ) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may submit RemovePlr"));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitRemovePlayer {
                tick,
                remove: lc_engine::RemovePlayerControlData {
                    player,
                    disconnected,
                    by_client: i32::try_from(HOST_CLIENT_ID)
                        .map_err(|_| anyhow!("host client id exceeds the RemovePlr wire field"))?,
                },
            })
            .map_err(|_| anyhow!("network worker is not accepting RemovePlr controls"))
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

    pub fn publish_join_snapshot(&self, snapshot: HostJoinSnapshot) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may publish JoinData"));
        }
        self.command_tx
            .blocking_send(NetworkCommand::PublishJoinSnapshot { snapshot })
            .map_err(|_| anyhow!("network worker is not accepting JoinData updates"))
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

    pub fn acknowledge_requested_status_at_frame(
        &mut self,
        current_control_tick: i32,
        current_frame: i32,
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
        let Some(acknowledgement) = self
            .client_status
            .acknowledge_requested_at(status, current_control_tick)
        else {
            return Err(NetworkStatusCommandError::NoRequestedStatus);
        };
        if self
            .command_tx
            .blocking_send(NetworkCommand::AcknowledgeRequestedStatus {
                status,
                current_control_tick,
                current_frame,
            })
            .is_err()
        {
            self.client_status.restore_request(status, acknowledgement);
            return Err(NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status acknowledgements",
            });
        }
        Ok(())
    }

    pub fn notify_client_update_executed(
        &self,
        update: lc_engine::ClientUpdateControlData,
    ) -> Result<()> {
        if self.role != NetworkRole::Client {
            return Err(anyhow!(
                "only a network client may report an executed client update"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::ClientUpdateExecuted(update))
            .map_err(|_| anyhow!("network worker is not accepting executed client updates"))
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
        loop {
            match self.telemetry_rx.try_recv() {
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

    pub fn local_addresses(&self) -> &[NetworkAddress] {
        &self.local_addresses
    }

    #[cfg(test)]
    pub(crate) fn test_stub() -> (Self, Sender<NetworkEvent>) {
        let (command_tx, _command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                event_rx,
                telemetry_rx,
                worker: None,
                local_client_id: HOST_CLIENT_ID,
                local_addresses: Vec::new(),
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
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                event_rx,
                telemetry_rx,
                worker: None,
                local_client_id,
                local_addresses: Vec::new(),
                role: NetworkRole::Client,
                client_status: ClientStatusState::default(),
            },
            event_tx,
        )
    }

    #[cfg(test)]
    pub(crate) fn test_stub_with_commands() -> (Self, Sender<NetworkEvent>, TestNetworkCommands) {
        Self::test_stub_with_commands_for_client_id(HOST_CLIENT_ID)
    }

    #[cfg(test)]
    pub(crate) fn test_stub_with_commands_for_client_id(
        local_client_id: ClientId,
    ) -> (Self, Sender<NetworkEvent>, TestNetworkCommands) {
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = mpsc::channel();
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                event_rx,
                telemetry_rx,
                worker: None,
                local_client_id,
                local_addresses: Vec::new(),
                role: if local_client_id == HOST_CLIENT_ID {
                    NetworkRole::Host
                } else {
                    NetworkRole::Client
                },
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
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<Result<NetworkWorkerReady, String>>,
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
                telemetry_tx,
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
                telemetry_tx,
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
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<Result<NetworkWorkerReady, String>>,
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
    let bound_addr = match listener.local_addr() {
        Ok(address) => address,
        Err(error) => {
            let message = format!("failed to read bound host socket address: {error}");
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
    let _ = local_id_tx.send(Ok(NetworkWorkerReady {
        local_client_id: HOST_CLIENT_ID,
        // The current transport has one TCP listener. C++ appends UDP and
        // per-interface endpoints after its wildcard TCP address; those
        // require the corresponding live transports first
        // (src/C4Network2Client.cpp:281-317).
        local_addresses: vec![NetworkAddress::new(NetworkProtocol::Tcp, bound_addr)],
    }));
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
                    Some(event) => handle_host_event(
                        event,
                        local_owner,
                        &event_tx,
                        &telemetry_tx,
                    ).await?,
                    None => {
                        return Err(anyhow!("host event stream ended"));
                    }
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = host
                            .publish_player_resource(request)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::RemoveResource { completion, .. } => {
                        let _ = completion.send(Err(
                            "host attempted to remove a client network resource".to_string(),
                        ));
                    }
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
                    NetworkCommand::SubmitRemovePlayer { tick, remove } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::RemovePlayer(remove),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitClientUpdate(update) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::ClientUpdate(update),
                        )?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host client-update submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitClientRemove(remove) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::ClientRemove(remove),
                        )?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host client-remove submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitInitScenarioPlayer { tick, selection } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::InitScenarioPlayer(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitSurrenderPlayer { tick, surrender } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::SurrenderPlayer(surrender),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitInternalPlayerScript { tick, control } => {
                        frame_builder.record_control(tick, control, current_millis());
                    }
                    NetworkCommand::SubmitMessage(message) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::Message(message),
                        )?;
                        host.submit_packet(ControlDelivery::Private, data)
                            .await
                            .map_err(|error| anyhow!("host message submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitVote(vote) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::Vote(vote),
                        )?;
                        host.submit_packet(ControlDelivery::Direct, data)
                            .await
                            .map_err(|error| anyhow!("host vote submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitVoteEnd(result) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::VoteEnd(result),
                        )?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host vote-result submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitReadyCheck(packet) => {
                        host.submit_ready_check(packet)
                            .await
                            .map_err(|error| anyhow!("host ready-check submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitLobbyCountdown(packet) => {
                        host.submit_lobby_countdown(packet)
                            .await
                            .map_err(|error| anyhow!("host lobby-countdown submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        if let Some(control) = control_packet_for_event(owner, event, HOST_CLIENT_ID) {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitPlayerCommand { tick, command } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::PlayerCommand(command),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::PlayerSelect(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitScript { tick, script } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::Script(script),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitMessageBoardAnswer { tick, answer } => {
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::MessageBoardAnswer(answer),
                            current_millis(),
                        );
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
                    NetworkCommand::PublishJoinSnapshot { snapshot } => {
                        host.publish_join_snapshot(snapshot)
                            .await
                            .map_err(|error| anyhow!("host JoinData update failed: {error}"))?;
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
                    NetworkCommand::AcknowledgeRequestedStatus { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "host attempted to send a client status acknowledgement".to_string(),
                        ));
                    }
                    NetworkCommand::ClientUpdateExecuted(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "host attempted to report an executed client update".to_string(),
                        ));
                    }
                    NetworkCommand::GracefulPart { completion } => {
                        let _ = completion.send(Err(
                            "host attempted to issue a client graceful departure".to_string(),
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
    _telemetry_tx: &SyncSender<NetworkEvent>,
) -> Result<()> {
    match event {
        HostEvent::StatusCommitted(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
        }
        HostEvent::StatusAck { .. } => {}
        HostEvent::ActivationRequest {
            client_id,
            tick,
            waited_for,
            ping_ms,
        } => {
            let _ = event_tx.send(NetworkEvent::ActivationRequest {
                client_id,
                tick,
                waited_for,
                ping_ms,
            });
        }
        HostEvent::PlayerInfoUpdate { client_id, request } => {
            let _ = event_tx.send(NetworkEvent::PlayerInfoUpdateRequest {
                origin: client_id,
                request,
                by_host: false,
            });
        }
        HostEvent::LobbyCountdown { packet } => {
            let _ = event_tx.send(NetworkEvent::LobbyCountdown(packet));
        }
        HostEvent::ReadyCheck { packet } => {
            let _ = event_tx.send(NetworkEvent::ReadyCheck(packet));
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
        HostEvent::UnhandledPacket {
            client_id,
            packet_type,
        } => {
            let status = format!("{packet_type:02x}");
            tracing::error!(?client_id, %status, "Unhandled packet");
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
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<Result<NetworkWorkerReady, String>>,
) -> Result<()> {
    let player_name = settings.player_name.clone();
    let mut client_config = ClientConfig::new(player_name.clone(), ParticipantKind::Player)
        .with_resource_directory(settings.resource_directory)
        .with_local_resource_roots(settings.local_resource_roots);
    if let Some(system_path) = settings.local_system_path {
        client_config = client_config.with_local_system_path(system_path);
    }
    let mut client = match connect_client(settings.server_addr, client_config).await {
        Ok(client) => client,
        Err(err) => {
            let message = format!("failed to connect to host: {err}");
            let _ = local_id_tx.send(Err(message.clone()));
            return Err(anyhow!(message));
        }
    };
    let (client_id, initial_status) =
        announce_connected_client(&mut client, player_name, &event_tx, &local_id_tx)?;
    let mut client_events = client.take_event_receiver();
    let mut frame_builder = ControlFrameAccumulator::new(client_id);
    let mut client_status = ClientStatusState::default();
    client_status.receive_request(initial_status);
    let mut client_activation = ClientActivationState::default();

    loop {
        let activation_retry_at = client_activation.next_retry_at();
        tokio::select! {
            maybe_event = client_events.recv() => {
                match maybe_event {
                    Some(ClientEvent::Status(status)) => {
                        if client_status.receive_request(status) {
                            client_activation.status_requested();
                        }
                        handle_client_event(
                            ClientEvent::Status(status),
                            local_owner,
                            client_id,
                            &event_tx,
                            &telemetry_tx,
                        ).await?;
                    }
                    Some(ClientEvent::StatusAck(status)) => {
                        if client_status.commit(status) {
                            handle_client_event(
                                ClientEvent::StatusAck(status),
                                local_owner,
                                client_id,
                                &event_tx,
                                &telemetry_tx,
                            ).await?;
                        }
                    }
                    Some(event) => handle_client_event(
                        event,
                        local_owner,
                        client_id,
                        &event_tx,
                        &telemetry_tx,
                    ).await?,
                    None => {
                        return Err(anyhow!("client event stream ended"));
                    }
                }
            }
            Some(command) = command_rx.recv() => {
                match command {
                    NetworkCommand::PublishPlayerResource {
                        request,
                        completion,
                    } => {
                        let result = client
                            .publish_player_resource(request)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::RemoveResource {
                        resource_id,
                        completion,
                    } => {
                        let result = client
                            .remove_resource(resource_id)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::SubmitPlayerInfoUpdate(request) => {
                        let arms_activation = request.flags
                            & lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL
                            != 0
                            && !request.players.is_empty();
                        client
                            .submit_player_info_update(request.clone())
                            .await
                            .map_err(|err| anyhow!("client PlayerInfo update failed: {err}"))?;
                        if arms_activation {
                            client_activation.arm_for_queued_player_info(&request);
                            request_client_activation_if_due(
                                &client,
                                &mut client_activation,
                                tokio::time::Instant::now(),
                            )
                            .await?;
                        }
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
                    NetworkCommand::SubmitRemovePlayer { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit authoritative RemovePlr".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitClientUpdate(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit an authoritative client update".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitClientRemove(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit an authoritative client removal"
                                .to_string(),
                        ));
                    }
                    NetworkCommand::SubmitInitScenarioPlayer { tick, selection } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::InitScenarioPlayer(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitSurrenderPlayer { tick, surrender } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::SurrenderPlayer(surrender),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitInternalPlayerScript { tick, control } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(tick, control, current_millis());
                    }
                    NetworkCommand::SubmitMessage(message) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::Message(message.clone()),
                        )?;
                        client.submit_packet(ControlDelivery::Private, data)
                            .await
                            .map_err(|error| anyhow!("client message submission failed: {error}"))?;
                        // C4GameControlNetwork::DoInput executes a private
                        // packet immediately for its sender as well as
                        // broadcasting it to peers.
                        let _ = event_tx.send(NetworkEvent::DirectControl(
                            NetworkControl::Message(message),
                        ));
                    }
                    NetworkCommand::SubmitVote(vote) => {
                        let data = encode_control_entry_payload(
                            &lc_engine::ControlPacket::Vote(vote),
                        )?;
                        client.submit_packet(ControlDelivery::Direct, data)
                            .await
                            .map_err(|error| anyhow!("client vote submission failed: {error}"))?;
                        let _ = event_tx.send(NetworkEvent::DirectControl(
                            NetworkControl::Vote(vote),
                        ));
                    }
                    NetworkCommand::SubmitVoteEnd(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit an authoritative vote result".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitReadyCheck(packet) => {
                        client
                            .submit_ready_check(packet)
                            .await
                            .map_err(|error| anyhow!("client ready-check submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitLobbyCountdown(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to submit a host lobby countdown".to_string(),
                        ));
                    }
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        if let Some(control) = control_packet_for_event(owner, event, client_id) {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitPlayerCommand { tick, command } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::PlayerCommand(command),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::PlayerSelect(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitScript { tick, script } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::Script(script),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitMessageBoardAnswer { tick, answer } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::MessageBoardAnswer(answer),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitSyncCheck { tick, check } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
                        frame_builder.record_control(
                            tick,
                            lc_engine::ControlPacket::SyncCheck(check),
                            current_millis(),
                        );
                    }
                    NetworkCommand::FinalizeTick { tick } => {
                        client_activation.refresh_frame(frame_tick_to_i32(tick));
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
                    NetworkCommand::PublishJoinSnapshot { .. } => {
                        let message = "client attempted to publish host JoinData".to_string();
                        let _ = event_tx.send(NetworkEvent::Error(message));
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
                    NetworkCommand::AcknowledgeRequestedStatus {
                        status: expected,
                        current_control_tick,
                        current_frame,
                    } => {
                        let Some(status) = client_status
                            .acknowledge_requested_at(expected, current_control_tick)
                        else {
                            let _ = event_tx.send(NetworkEvent::Error(
                                "requested game status changed before client acknowledgement"
                                    .to_string(),
                            ));
                            continue;
                        };
                        if let Err(err) = client.submit_status_ack(status).await {
                            client_status.restore_request(expected, status);
                            return Err(anyhow!("client status acknowledgement failed: {err}"));
                        }
                        client_activation.status_reached(current_frame);
                        request_client_activation_if_due(
                            &client,
                            &mut client_activation,
                            tokio::time::Instant::now(),
                        )
                        .await?;
                    }
                    NetworkCommand::ClientUpdateExecuted(update) => {
                        if let Ok(local_client_id) = i32::try_from(client_id) {
                            client_activation
                                .apply_executed_client_update(local_client_id, &update);
                        }
                    }
                    NetworkCommand::GracefulPart { completion } => {
                        match client.graceful_part().await {
                            Ok(()) => {
                                let _ = completion.send(Ok(()));
                                return Ok(());
                            }
                            Err(error) => {
                                let message = error.to_string();
                                let _ = completion.send(Err(message.clone()));
                                return Err(anyhow!(message));
                            }
                        }
                    }
                    NetworkCommand::Shutdown => break,
                }
            }
            _ = wait_for_activation_retry(activation_retry_at) => {
                request_client_activation_if_due(
                    &client,
                    &mut client_activation,
                    tokio::time::Instant::now(),
                )
                .await?;
            }
            else => break,
        }
    }

    client.shutdown().await.ok();
    Ok(())
}

async fn wait_for_activation_retry(retry_at: Option<tokio::time::Instant>) {
    match retry_at {
        Some(retry_at) => tokio::time::sleep_until(retry_at).await,
        None => std::future::pending::<()>().await,
    }
}

async fn request_client_activation_if_due(
    client: &ClientHandle,
    activation: &mut ClientActivationState,
    now: tokio::time::Instant,
) -> Result<()> {
    let Some(tick) = activation.request_tick_if_due(now) else {
        return Ok(());
    };
    client
        .request_activation(tick)
        .await
        .map_err(|error| anyhow!("client activation request failed: {error}"))?;
    activation.mark_requested(now);
    Ok(())
}

fn frame_tick_to_i32(tick: Tick) -> i32 {
    i32::try_from(tick).unwrap_or(i32::MAX)
}

fn announce_connected_client(
    client: &mut ClientHandle,
    player_name: String,
    event_tx: &Sender<NetworkEvent>,
    local_id_tx: &mpsc::Sender<Result<NetworkWorkerReady, String>>,
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
    let _ = local_id_tx.send(Ok(NetworkWorkerReady {
        local_client_id: client_id,
        local_addresses: Vec::new(),
    }));
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
    _client_id: ClientId,
    event_tx: &Sender<NetworkEvent>,
    _telemetry_tx: &SyncSender<NetworkEvent>,
) -> Result<()> {
    match event {
        ClientEvent::Status(status) => {
            let _ = event_tx.send(NetworkEvent::StatusRequested(status));
        }
        ClientEvent::StatusAck(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
        }
        ClientEvent::LobbyCountdown { packet } => {
            let _ = event_tx.send(NetworkEvent::LobbyCountdown(packet));
        }
        ClientEvent::ReadyCheck { packet } => {
            let _ = event_tx.send(NetworkEvent::ReadyCheck(packet));
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
        ClientEvent::LeagueRoundResults { packet } => {
            let _ = event_tx.send(NetworkEvent::LeagueRoundResults(packet));
        }
        ClientEvent::UnhandledPacket { packet_type } => {
            let status = format!("{packet_type:02x}");
            tracing::error!(client_id = HOST_CLIENT_ID, %status, "Unhandled packet");
        }
        ClientEvent::Disconnected { reason } => {
            let _ = event_tx.send(NetworkEvent::PeerDisconnected {
                client_id: HOST_CLIENT_ID,
                reason,
            });
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
        Ok(control) => match network_control_for_packet(control) {
            Some(control) => {
                let _ = event_tx.send(NetworkEvent::DirectControl(control));
            }
            None => {
                let _ = event_tx.send(NetworkEvent::Error(
                    "unsupported immediate control packet".to_string(),
                ));
            }
        },
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
    if let Some(set) = LegacyControlSet::from_control_packet(&control) {
        return Some(NetworkControl::Set(set));
    }
    match control {
        lc_engine::ControlPacket::ClientJoin(join) => Some(NetworkControl::ClientJoin(join)),
        lc_engine::ControlPacket::ClientUpdate(update) => {
            Some(NetworkControl::ClientUpdate(update))
        }
        lc_engine::ControlPacket::ClientRemove(remove) => {
            Some(NetworkControl::ClientRemove(remove))
        }
        // Keep the original signed fields through ordered execution. The
        // C++ packet layer counts them before InCom narrows Command to a byte.
        lc_engine::ControlPacket::PlayerControl(data) => Some(NetworkControl::PlayerControl(data)),
        lc_engine::ControlPacket::PlayerCommand(data) => Some(NetworkControl::PlayerCommand(data)),
        lc_engine::ControlPacket::PlayerSelect(data) => Some(NetworkControl::PlayerSelect(data)),
        lc_engine::ControlPacket::Script(data) => Some(NetworkControl::Script(data)),
        lc_engine::ControlPacket::Message(data) => Some(NetworkControl::Message(data)),
        lc_engine::ControlPacket::MessageBoardAnswer(data) => {
            Some(NetworkControl::MessageBoardAnswer(data))
        }
        lc_engine::ControlPacket::CustomCommand(data) => {
            Some(NetworkControl::CustomCommand(data))
        }
        lc_engine::ControlPacket::EmMoveObject(data) => Some(NetworkControl::EmMoveObject(data)),
        lc_engine::ControlPacket::EmDrawTool(data) => Some(NetworkControl::EmDrawTool(data)),
        lc_engine::ControlPacket::EmDropDef(data) => Some(NetworkControl::EmDropDef(data)),
        lc_engine::ControlPacket::Synchronize(data) => Some(NetworkControl::Synchronize(data)),
        lc_engine::ControlPacket::SyncCheck(packet) => Some(NetworkControl::SyncCheck(packet)),
        lc_engine::ControlPacket::PlayerInfo(info) => Some(NetworkControl::PlayerInfo(info)),
        lc_engine::ControlPacket::JoinPlayer(join) => Some(NetworkControl::JoinPlayer(join)),
        lc_engine::ControlPacket::RemovePlayer(remove) => {
            Some(NetworkControl::RemovePlayer(remove))
        }
        lc_engine::ControlPacket::InitScenarioPlayer(selection) => {
            Some(NetworkControl::InitScenarioPlayer(selection))
        }
        lc_engine::ControlPacket::SurrenderPlayer(surrender) => {
            Some(NetworkControl::SurrenderPlayer(surrender))
        }
        lc_engine::ControlPacket::ActivateGameGoalMenu(control) => {
            Some(NetworkControl::ActivateGameGoalMenu(control))
        }
        lc_engine::ControlPacket::ToggleHostility(control) => {
            Some(NetworkControl::ToggleHostility(control))
        }
        lc_engine::ControlPacket::ActivateGameGoalRule(control) => {
            Some(NetworkControl::ActivateGameGoalRule(control))
        }
        lc_engine::ControlPacket::SetPlayerTeam(control) => {
            Some(NetworkControl::SetPlayerTeam(control))
        }
        lc_engine::ControlPacket::EliminatePlayer(control) => {
            Some(NetworkControl::EliminatePlayer(control))
        }
        lc_engine::ControlPacket::Vote(vote) => Some(NetworkControl::Vote(vote)),
        lc_engine::ControlPacket::VoteEnd(result) => Some(NetworkControl::VoteEnd(result)),
        lc_engine::ControlPacket::Unknown { .. } => None,
    }
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

    fn message_control(by_client: i32) -> MessageControlData {
        MessageControlData {
            message_type: lc_engine::MESSAGE_TYPE_PRIVATE,
            player: 4,
            to_player: 9,
            message: lc_engine::LegacyCString::from_bytes(b"private hello".to_vec())
                .expect("fixture is NUL-free"),
            by_client,
        }
    }

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
        let host = start_host(listener, host_config).await.expect("start host");
        let mut client =
            connect_client(address, ClientConfig::new("Alice", ParticipantKind::Player))
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

        let announced =
            announce_connected_client(&mut client, "Alice".to_string(), &event_tx, &local_id_tx)
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
        assert_eq!(
            local_id_rx.recv().expect("local ID result"),
            Ok(NetworkWorkerReady {
                local_client_id: client_id,
                local_addresses: Vec::new(),
            })
        );
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_worker_sends_status_ack_before_delayed_activation() {
        // The initial PlayerInfo request arms RequestActivate while the client
        // is still chasing. CheckStatusReached sends PID_StatusAck first and
        // PID_ClientActReq immediately afterwards with Game.FrameCounter
        // (src/C4Network2Players.cpp:124-136;
        // src/C4Network2.cpp:2041-2058,2116-2145).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let address = listener.local_addr().expect("host address");
        let host_config = HostConfig::default();
        let expected_status = NetworkStatus {
            target_tick: host_config
                .initial_join_snapshot
                .as_ref()
                .expect("default JoinData")
                .dynamic_tick,
            ..host_config.initial_status
        };
        let mut host = start_host(listener, host_config).await.expect("start host");
        let mut host_events = host.take_event_receiver();
        let temporary = tempfile::tempdir().expect("temporary client resource directory");
        let mut settings = ClientSettings::new(address, "Alice");
        settings.resource_directory = temporary.path().join("Network");
        let (command_tx, command_rx) = tokio_mpsc::channel(16);
        let (event_tx, _event_rx) = mpsc::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, _local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
            )
            .await
        });

        let client_id = loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("client join timeout")
            {
                Some(HostEvent::ClientJoined { client_id, .. }) => break client_id,
                Some(_) => continue,
                None => panic!("host event stream ended before client join"),
            }
        };
        let wire_client_id = i32::try_from(client_id).expect("client ID fits wire field");
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: wire_client_id,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        command_tx
            .send(NetworkCommand::SubmitPlayerInfoUpdate(request))
            .await
            .expect("queue initial PlayerInfo");
        command_tx
            .send(NetworkCommand::AcknowledgeRequestedStatus {
                status: expected_status,
                current_control_tick: expected_status.target_tick.saturating_add(3),
                current_frame: 41,
            })
            .await
            .expect("queue reached status");

        let mut protocol_order = Vec::new();
        while protocol_order.len() < 3 {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("initial client protocol timeout")
            {
                Some(HostEvent::PlayerInfoUpdate { .. }) => protocol_order.push(("player-info", 0)),
                Some(HostEvent::StatusAck { status, .. }) => {
                    protocol_order.push(("status-ack", status.target_tick));
                }
                Some(HostEvent::ActivationRequest { tick, .. }) => {
                    protocol_order.push(("activation", tick));
                }
                Some(_) => continue,
                None => panic!("host event stream ended during initial client protocol"),
            }
        }
        assert_eq!(
            protocol_order,
            vec![
                ("player-info", 0),
                ("status-ack", expected_status.target_tick.saturating_add(3)),
                ("activation", 41),
            ]
        );

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop client worker");
        worker
            .await
            .expect("join client worker")
            .expect("client worker exits cleanly");
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
    fn manager_queues_player_command_with_authenticated_local_author() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let command = PlayerCommandControlData {
            player: 3,
            command: 2,
            x: 100,
            y: 200,
            target: 0,
            target2: 91,
            data: 4,
            add_mode: 5,
            by_client: -1,
        };

        manager
            .submit_player_command(12, command)
            .expect("queue player command");

        assert_eq!(
            commands.take_submitted_player_commands(),
            vec![(
                12,
                PlayerCommandControlData {
                    by_client: 7,
                    ..command
                }
            )]
        );
    }

    #[test]
    fn manager_queues_script_control_with_authenticated_local_author() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let script = ScriptControlData {
            target_object: lc_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: lc_engine::ScriptStrictness::Strict3,
            script: lc_engine::LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                .expect("script is NUL-free"),
            by_client: -1,
        };

        manager
            .submit_script_control(12, script.clone())
            .expect("queue script control");

        assert_eq!(
            commands.take_submitted_scripts(),
            vec![(
                12,
                ScriptControlData {
                    by_client: 7,
                    ..script
                }
            )]
        );
    }

    #[test]
    fn manager_queues_message_board_answer_with_authenticated_local_author() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let answer = MessageBoardAnswerControlData {
            object: 42,
            answer: lc_engine::LegacyCString::from_bytes(b"q\"\\z".to_vec())
                .expect("answer is NUL-free"),
            player: 3,
            by_client: -1,
        };

        manager
            .submit_message_board_answer(12, answer.clone())
            .expect("queue message-board answer");

        assert_eq!(
            commands.take_submitted_message_board_answers(),
            vec![(
                12,
                MessageBoardAnswerControlData {
                    by_client: 7,
                    ..answer
                }
            )]
        );
    }

    #[test]
    fn client_manager_waits_for_selected_player_resource_publication() {
        // LoadFromLocalFile must finish AddByFile and retain the resulting
        // resource core before JoinLocalPlayer constructs PID_PlayerInfoUpdReq
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
        // src/C4Network2Players.cpp:78-136).
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let request = lc_network::ClientPlayerResourceRequest {
            source_path: PathBuf::from("Alice.c4p"),
            wire_name: lc_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec())
                .expect("fixture filename is NUL-free"),
            group_maker: lc_engine::LegacyCString::from_bytes(b"Alice".to_vec())
                .expect("fixture maker is NUL-free"),
        };
        let expected = request.clone();
        let caller = thread::spawn(move || manager.publish_client_player_resource(request));

        let NetworkCommand::PublishPlayerResource {
            request,
            completion,
        } = commands
            .command_rx
            .blocking_recv()
            .expect("publication command")
        else {
            panic!("expected selected-player publication command");
        };
        assert_eq!(request, expected);
        assert!(!caller.is_finished(), "publication has not completed yet");
        let core = lc_engine::NetworkResourceCore {
            resource_type: lc_network::HostResourceType::Player as u8,
            id: 7 << 16,
            loadable: true,
            ..Default::default()
        };
        completion
            .send(Ok(core.clone()))
            .expect("complete publication");

        assert_eq!(
            caller.join().expect("publication caller exits").unwrap(),
            core
        );
    }

    #[test]
    fn client_manager_waits_for_merged_dynamic_removal() {
        // RetrieveScenario calls Remove only after the scenario and dynamic
        // groups merge successfully, and the main thread observes that
        // removal before it continues with the packed combined scenario
        // (pristine 9ffa0a5d src/C4Network2.cpp:656-669;
        // src/C4Network2Res.cpp:825-829).
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let caller = thread::spawn(move || manager.remove_client_resource(23));

        let (resource_id, completion) = commands.receive_resource_removal();
        assert_eq!(resource_id, 23);
        assert!(
            !caller.is_finished(),
            "resource removal has not completed yet"
        );
        completion.send(Ok(())).expect("complete resource removal");

        caller
            .join()
            .expect("resource-removal caller exits")
            .expect("resource removal succeeds");
    }

    #[test]
    fn client_manager_waits_for_graceful_part_notification() {
        // C4Network2ClientList::DeleteClient sends the negative PID_ConnRe
        // before it closes the accepted connection. The app must not tear its
        // manager down until that write has completed (pristine 9ffa0a5d
        // src/C4Network2Client.cpp:104-119,457-492).
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let caller = thread::spawn(move || manager.graceful_part());

        let completion = commands.receive_graceful_part();
        assert!(!caller.is_finished(), "departure has not been written yet");
        completion
            .send(Ok(()))
            .expect("complete graceful departure");

        caller
            .join()
            .expect("departure caller exits")
            .expect("graceful departure succeeds");
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
    fn manager_stamps_message_author_for_non_ticked_private_submission() {
        // C4MessageInput submits CID_Message through CDT_Private rather than
        // appending it to Game.Input for a future synchronized control tick
        // (src/C4MessageInput.cpp:423-426; src/C4GameControl.cpp:385-410).
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let message = message_control(99);

        manager
            .submit_message(message.clone())
            .expect("client queues immediate message control");

        assert_eq!(
            commands.take_submitted_messages(),
            vec![MessageControlData {
                by_client: 7,
                ..message
            }]
        );
    }

    #[test]
    fn managers_stamp_team_choice_with_local_client_and_tick() {
        // C4ControlPacket captures the local control client in ByClient, and
        // DoTeamSelection queues the choice for the next complete control tick
        // on both host and client (src/C4Control.cpp:38-56;
        // src/C4Player.cpp:1775-1780; src/C4GameControl.cpp:394-400).
        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.submit_init_scenario_player(23, 4, 2)
            .expect("host queues team choice");
        assert_eq!(
            host_commands.take_submitted_init_scenario_players(),
            vec![(
                23,
                lc_engine::InitScenarioPlayerControlData {
                    team: 2,
                    player: 4,
                    by_client: 0,
                },
            )]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_init_scenario_player(41, 9, 3)
            .expect("client queues team choice");
        assert_eq!(
            client_commands.take_submitted_init_scenario_players(),
            vec![(
                41,
                lc_engine::InitScenarioPlayerControlData {
                    team: 3,
                    player: 9,
                    by_client: 7,
                },
            )]
        );
    }

    #[test]
    fn host_manager_queues_remove_player_with_host_authorship_and_tick() {
        let (host, _events, mut commands) = NetworkManager::test_stub_with_commands();
        host.submit_remove_player(23, 4, true)
            .expect("host queues RemovePlr");
        assert_eq!(
            commands.take_submitted_remove_players(),
            vec![(
                23,
                lc_engine::RemovePlayerControlData {
                    player: 4,
                    disconnected: true,
                    by_client: 0,
                },
            )]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        assert!(client.submit_remove_player(41, 9, false).is_err());
        assert!(client_commands.take_submitted_remove_players().is_empty());
    }

    #[test]
    fn managers_queue_surrender_with_local_authorship_and_tick() {
        // The Surrender menu command uses CDT_Queue, so C4GameControl retains
        // the local ByClient and sends the control in that client's requested
        // control tick (pristine 9ffa0a5d src/C4MainMenu.cpp:790-795;
        // src/C4GameControl.cpp:380-406;
        // src/C4GameControlNetwork.cpp:214-256).
        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.submit_surrender_player(23, 4)
            .expect("host queues surrender");
        assert_eq!(
            host_commands.take_submitted_surrender_players(),
            vec![(
                23,
                lc_engine::SurrenderPlayerControlData {
                    player: 4,
                    by_client: 0,
                },
            )]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_surrender_player(41, 9)
            .expect("client queues surrender");
        assert_eq!(
            client_commands.take_submitted_surrender_players(),
            vec![(
                41,
                lc_engine::SurrenderPlayerControlData {
                    player: 9,
                    by_client: 7,
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
    fn client_manager_acks_the_exact_prepared_status_at_the_same_tick() {
        // CheckStatusReached replaces TargetCtrlTick with the client's current
        // control tick. When that is still the requested tick, PID_StatusAck
        // remains byte-for-byte identical (src/C4Network2.cpp:2074-2084).
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
            .acknowledge_requested_status_at_frame(41, 0)
            .expect("client queues the reached status acknowledgement");

        assert_eq!(commands.take_status_acknowledgements(), vec![status]);
        assert_eq!(manager.client_status.awaiting_commit, Some(status));
    }

    #[test]
    fn client_manager_retargets_status_ack_to_current_control_tick() {
        // A client may pass the requested barrier before it gets a chance to
        // send PID_StatusAck. C4Network2::CheckStatusReached acknowledges the
        // current ControlTick, while FrameCounter remains a separate value for
        // a delayed activation request (src/C4Network2.cpp:2041-2058,2073-2084).
        let (mut manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let requested = NetworkStatus {
            state: lc_network::NETWORK_STATE_PAUSE,
            control_mode: 2,
            target_tick: 41,
        };
        let acknowledgement = NetworkStatus {
            target_tick: 44,
            ..requested
        };
        event_tx
            .send(NetworkEvent::StatusRequested(requested))
            .expect("queue host status");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusRequested(requested)]
        );

        manager
            .acknowledge_requested_status_at_frame(44, 123)
            .expect("client queues retargeted status acknowledgement");

        assert_eq!(
            commands.take_framed_status_acknowledgements(),
            vec![(acknowledgement, 123)]
        );
        assert_eq!(manager.client_status.awaiting_commit, Some(acknowledgement));

        event_tx
            .send(NetworkEvent::StatusRequested(acknowledgement))
            .expect("queue host's retargeted status broadcast");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusRequested(acknowledgement)]
        );
        assert_eq!(manager.client_status.requested, None);
        assert_eq!(manager.client_status.awaiting_commit, Some(acknowledgement));

        event_tx
            .send(NetworkEvent::StatusCommitted(requested))
            .expect("queue stale host acknowledgement");
        assert!(manager.poll_events().is_empty());
        assert_eq!(manager.client_status.awaiting_commit, Some(acknowledgement));

        event_tx
            .send(NetworkEvent::StatusCommitted(acknowledgement))
            .expect("queue retargeted host acknowledgement");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusCommitted(acknowledgement)]
        );
        assert_eq!(manager.client_status.awaiting_commit, None);
    }

    #[test]
    fn client_activation_waits_for_status_ack_then_uses_current_frame() {
        // JoinLocalPlayer requests activation after its nonempty CIF_Initial
        // packet, but RequestActivate delays while the status is unreached.
        // CheckStatusReached sends PID_StatusAck first, then the delayed
        // PID_ClientActReq with the current Game.FrameCounter
        // (src/C4Network2Players.cpp:124-136;
        // src/C4Network2.cpp:2041-2058,2116-2145).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        let now = tokio::time::Instant::now();

        activation.arm_for_queued_player_info(&request);
        assert_eq!(activation.request_tick_if_due(now), None);

        activation.status_reached(41);
        assert_eq!(activation.request_tick_if_due(now), Some(41));
    }

    #[test]
    fn client_activation_retries_at_five_seconds_with_the_latest_frame() {
        // A non-host with an outstanding activation request calls
        // RequestActivate again from Execute. The strict interval check allows
        // the request at exactly 5,000 ms, and each packet carries the then
        // current Game.FrameCounter (src/C4Network2.cpp:739-743,2116-2145;
        // src/C4Network2.h:57-60).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached(41);
        activation.mark_requested(first_sent_at);
        activation.refresh_frame(52);

        assert_eq!(
            activation.request_tick_if_due(
                first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL - Duration::from_millis(1)
            ),
            None
        );
        assert_eq!(
            activation.request_tick_if_due(first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL),
            Some(52)
        );
    }

    #[test]
    fn client_activation_retry_waits_for_a_new_status_to_be_reached() {
        // RequestActivate sets fDelayedActivateReq instead of sending while
        // fStatusReached is false. CheckStatusReached releases that delayed
        // request only after sending the new PID_StatusAck
        // (src/C4Network2.cpp:2039-2058,2133-2145).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached(41);
        activation.mark_requested(first_sent_at);

        activation.status_requested();
        activation.refresh_frame(60);
        let overdue = first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL;
        assert_eq!(activation.request_tick_if_due(overdue), None);

        activation.status_reached(61);
        assert_eq!(activation.request_tick_if_due(overdue), Some(61));
    }

    #[test]
    fn client_activation_clears_only_for_executed_host_local_activation() {
        // CUT_Activate is trusted only from the host and changes the local
        // activation state when C4ControlClientUpdate executes. A false update,
        // another client's update, or a client-authored update cannot stop the
        // outstanding RequestActivate retries (src/C4Control.cpp:578-606;
        // src/C4Network2.cpp:2116-2145).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached(41);
        activation.mark_requested(first_sent_at);
        let retry_at = first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL;

        for update in [
            lc_engine::ClientUpdateControlData {
                update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 1,
                by_client: 3,
            },
            lc_engine::ClientUpdateControlData {
                update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 8,
                data: 1,
                by_client: 0,
            },
            lc_engine::ClientUpdateControlData {
                update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
        ] {
            activation.apply_executed_client_update(7, &update);
            assert_eq!(activation.request_tick_if_due(retry_at), Some(41));
        }

        activation.apply_executed_client_update(
            7,
            &lc_engine::ClientUpdateControlData {
                update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 1,
                by_client: 0,
            },
        );
        assert_eq!(activation.request_tick_if_due(retry_at), None);
    }

    #[test]
    fn empty_initial_player_info_never_arms_client_activation() {
        // The empty initial packet is still sent so the host can return all
        // player infos, but JoinLocalPlayer calls RequestActivate only when at
        // least one player was present (src/C4Network2Players.cpp:124-136).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
        };
        activation.arm_for_queued_player_info(&request);
        activation.status_reached(41);

        assert_eq!(
            activation.request_tick_if_due(tokio::time::Instant::now()),
            None
        );
    }

    #[test]
    fn executed_host_observer_update_clears_client_activation() {
        // CUT_SetObserver deactivates the local client and RequestActivate then
        // clears its outstanding retry state (src/C4Control.cpp:607-619;
        // src/C4Network2.cpp:2116-2122).
        let mut activation = ClientActivationState::default();
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached(41);
        activation.mark_requested(first_sent_at);

        activation.apply_executed_client_update(
            7,
            &lc_engine::ClientUpdateControlData {
                update_type: lc_engine::CLIENT_UPDATE_SET_OBSERVER,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
        );

        assert_eq!(
            activation.request_tick_if_due(first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL),
            None
        );
    }

    #[test]
    fn client_manager_orders_initial_player_info_before_framed_status_ack() {
        // JoinLocalPlayer sends PID_PlayerInfoUpdReq before DoLobby reaches the
        // status. CheckStatusReached then sends PID_StatusAck and releases the
        // activation request carrying Game.FrameCounter
        // (src/C4Network2Players.cpp:124-136;
        // src/C4Network2.cpp:2041-2058,2116-2145).
        let (mut manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let status = NetworkStatus {
            state: lc_network::NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 23,
        };
        let request = lc_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: lc_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![lc_engine::ControlPlayerInfoEntry::default()],
        };
        event_tx
            .send(NetworkEvent::StatusRequested(status))
            .expect("queue host status");
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusRequested(status)]
        );

        manager
            .submit_player_info_update(request.clone())
            .expect("queue initial PlayerInfo");
        manager
            .acknowledge_requested_status_at_frame(23, 41)
            .expect("queue framed status acknowledgement");

        assert!(matches!(
            commands.command_rx.blocking_recv(),
            Some(NetworkCommand::SubmitPlayerInfoUpdate(actual)) if actual == request
        ));
        assert!(matches!(
            commands.command_rx.blocking_recv(),
            Some(NetworkCommand::AcknowledgeRequestedStatus {
                status: actual,
                current_control_tick: 23,
                current_frame: 41,
            }) if actual == status
        ));
    }

    #[test]
    fn client_manager_reports_client_update_only_after_app_execution() {
        // C4ControlClientUpdate changes activation from Execute, after the
        // synchronized control is released. Packet receipt alone must not clear
        // RequestActivate retry state (src/C4GameControlNetwork.cpp:279-297,
        // 558-588; src/C4Control.cpp:578-606).
        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let update = lc_engine::ClientUpdateControlData {
            update_type: lc_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 7,
            data: 1,
            by_client: 0,
        };

        manager
            .notify_client_update_executed(update.clone())
            .expect("queue executed client update");

        assert!(matches!(
            commands.command_rx.blocking_recv(),
            Some(NetworkCommand::ClientUpdateExecuted(actual)) if actual == update
        ));
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
        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::JoinData(join_data)]
        );

        manager
            .acknowledge_requested_status_at_frame(23, 0)
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
                .acknowledge_requested_status_at_frame(0, 0)
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
            .acknowledge_requested_status_at_frame(41, 0)
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
    fn client_manager_commit_identity_ignores_control_mode() {
        // The non-host HandleStatusAck branch compares only State and the exact
        // TargetCtrlTick. CtrlMode is intentionally not part of commit identity
        // (pristine 9ffa0a5d src/C4Network2.cpp:1513-1548).
        let (mut manager, event_tx, _commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let requested = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        event_tx
            .send(NetworkEvent::StatusRequested(requested))
            .expect("queue host status");
        assert_eq!(manager.poll_events().len(), 1);
        manager
            .acknowledge_requested_status_at_frame(41, 0)
            .expect("acknowledge host status");

        let committed = NetworkStatus {
            control_mode: 9,
            ..requested
        };
        event_tx
            .send(NetworkEvent::StatusCommitted(committed))
            .expect("queue host acknowledgement");

        assert_eq!(
            manager.poll_events(),
            vec![NetworkEvent::StatusCommitted(committed)]
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
            host.acknowledge_requested_status_at_frame(0, 0)
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
        // flatten tick or move Synchronize/SyncCheck out of decoded packet
        // order (src/C4Control.cpp:73-109,537-543).
        let check = sync_check(17);
        let synchronize = lc_engine::SynchronizeControlData {
            save_player_files: false,
            sync_clearance: true,
            by_client: 0,
        };
        let right = PlayerControlData {
            player: 4,
            command: i32::from(COM_RIGHT),
            data: 0,
            by_client: 0,
        };
        let left = PlayerControlData {
            player: 9,
            command: i32::from(COM_LEFT),
            data: 0,
            by_client: 1,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 17,
            timestamp_ms: 99,
            controls: vec![
                lc_engine::ControlPacket::PlayerControl(right.clone()),
                lc_engine::ControlPacket::Synchronize(synchronize.clone()),
                lc_engine::ControlPacket::SyncCheck(check.clone()),
                lc_engine::ControlPacket::PlayerControl(left.clone()),
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
                        NetworkControl::PlayerControl(right),
                        NetworkControl::Synchronize(synchronize),
                        NetworkControl::SyncCheck(check),
                        NetworkControl::PlayerControl(left),
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
    fn ready_frame_retains_typed_control_set_data() {
        let set = LegacyControlSet {
            value_type: 5,
            data: 12_345,
            by_client: 0,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 19,
            timestamp_ms: 0,
            controls: vec![set.into_control_packet()],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 0, &event_tx).expect("emit ready frame");

        assert_eq!(
            event_rx.recv().expect("ready event"),
            NetworkEvent::ReadyTick {
                tick: 19,
                controls: vec![NetworkControl::Set(set)],
            }
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
        let player = PlayerControlData {
            player: 7,
            command: i32::from(COM_RIGHT),
            data: 0,
            by_client: 4,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![
                lc_engine::ControlPacket::PlayerInfo(info.clone()),
                lc_engine::ControlPacket::PlayerControl(player.clone()),
                lc_engine::ControlPacket::JoinPlayer(join.clone()),
            ],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 0, &event_tx).expect("emit ready frame");

        let NetworkEvent::ReadyTick { tick, controls } = event_rx.recv().expect("ready event")
        else {
            panic!("expected ready tick");
        };
        assert_eq!(tick, 23);
        assert_eq!(
            controls,
            vec![
                NetworkControl::PlayerInfo(info),
                NetworkControl::PlayerControl(player),
                NetworkControl::JoinPlayer(join),
            ]
        );
    }

    #[test]
    fn ready_frame_retains_player_surrender_and_its_authenticated_source() {
        // C4Control executes every queued packet in list order, and
        // C4ControlInternalPlayerScriptBase authorizes the player against the
        // packet's iByClient (src/C4Control.cpp:93-109,1572-1578).
        let surrender = lc_engine::SurrenderPlayerControlData {
            player: 7,
            by_client: 3,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![lc_engine::ControlPacket::SurrenderPlayer(surrender)],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 0, &event_tx).expect("emit ready frame");

        assert_eq!(
            event_rx.recv().expect("ready event"),
            NetworkEvent::ReadyTick {
                tick: 23,
                controls: vec![NetworkControl::SurrenderPlayer(surrender)],
            }
        );
    }

    #[test]
    fn ready_frame_exposes_scenario_player_initialization() {
        // C4Player::DoTeamSelection queues CID_InitScenarioPlayer, which is
        // released in the ordinary complete control tick before simulation
        // (src/C4Player.cpp:1775-1780; src/C4GameControl.cpp:273-316).
        let selection = lc_engine::InitScenarioPlayerControlData {
            team: 2,
            player: 4,
            by_client: 7,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![lc_engine::ControlPacket::InitScenarioPlayer(selection)],
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_frame_controls(frame, 0, &event_tx).expect("emit ready frame");

        assert_eq!(
            event_rx.recv().expect("ready event"),
            NetworkEvent::ReadyTick {
                tick: 23,
                controls: vec![NetworkControl::InitScenarioPlayer(selection)],
            }
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
            reason: lc_engine::LegacyCString::from_bytes(b"bye".to_vec()).expect("valid reason"),
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

    #[test]
    fn scheduled_sync_retains_host_authored_vote_end() {
        // The control host resolves a ballot by submitting CID_VoteEnd with
        // CDT_Sync; C4ControlVoteEnd::Execute then requires HostControl before
        // applying it (src/C4Control.cpp:1433-1442,1456-1461).
        let result = lc_engine::VoteControlData {
            vote_type: lc_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 0,
        };
        let (event_tx, event_rx) = mpsc::channel();

        emit_scheduled_sync_controls(
            23,
            vec![lc_engine::ControlPacket::VoteEnd(result)],
            &event_tx,
        )
        .expect("emit synchronized VoteEnd");

        assert_eq!(
            event_rx.recv(),
            Ok(NetworkEvent::ScheduledSync {
                tick: 23,
                controls: vec![NetworkControl::VoteEnd(result)],
            })
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
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_host_event(
            HostEvent::StatusCommitted(status),
            0,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward committed status");

        assert_eq!(
            event_rx.recv().expect("status event"),
            NetworkEvent::StatusCommitted(status)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lobby_countdown_is_forwarded_to_the_app_for_host_and_client() {
        // The host applies its locally constructed packet directly and each
        // client receives the same PID_LobbyCountdown through MainDlg
        // (src/C4GameLobby.cpp:392-418,1111-1131).
        let packet = lc_network::LobbyCountdownPacket::new(5);
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_host_event(
            HostEvent::LobbyCountdown { packet },
            0,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward host lobby countdown");
        handle_client_event(
            ClientEvent::LobbyCountdown { packet },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward client lobby countdown");

        assert_eq!(
            event_rx.recv().expect("host countdown event"),
            NetworkEvent::LobbyCountdown(packet)
        );
        assert_eq!(
            event_rx.recv().expect("client countdown event"),
            NetworkEvent::LobbyCountdown(packet)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_ready_check_is_forwarded_to_the_app_unchanged() {
        // C4Network2::HandlePacket passes the compiled packet, including its
        // claimed Client field, directly to HandleReadyCheck
        // (src/C4Network2.cpp:949-953,1625-1635).
        let packet = lc_network::ReadyCheckPacket {
            client_id: 7,
            data: lc_network::ReadyCheckData::Ready,
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_host_event(
            HostEvent::ReadyCheck { packet },
            0,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward ready check");

        assert_eq!(
            event_rx.recv().expect("ready-check event"),
            NetworkEvent::ReadyCheck(packet)
        );
    }

    #[test]
    fn managers_stamp_ready_check_with_the_local_client() {
        // MainDlg::OnReadyCheck always places Game.Clients.getLocalID() in
        // the packet before broadcasting it (src/C4GameLobby.cpp:329-343).
        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.submit_ready_check(lc_network::ReadyCheckData::Ready)
            .expect("host submits ready state");
        assert_eq!(
            host_commands.take_submitted_ready_checks(),
            vec![lc_network::ReadyCheckPacket {
                client_id: 0,
                data: lc_network::ReadyCheckData::Ready,
            }]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_ready_check(lc_network::ReadyCheckData::NotReady)
            .expect("client submits not-ready state");
        assert_eq!(
            client_commands.take_submitted_ready_checks(),
            vec![lc_network::ReadyCheckPacket {
                client_id: 7,
                data: lc_network::ReadyCheckData::NotReady,
            }]
        );
    }

    #[test]
    fn host_manager_queues_cpp_lobby_countdown_packet() {
        // Countdown's constructor broadcasts the initial timer verbatim as
        // PID_LobbyCountdown before installing its one-second callback
        // (src/C4GameLobby.cpp:1111-1130).
        let (host, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let packet = lc_network::LobbyCountdownPacket::new(5);

        host.submit_lobby_countdown(packet)
            .expect("host queues initial countdown");

        assert_eq!(commands.take_submitted_lobby_countdowns(), vec![packet]);
    }

    #[test]
    fn client_manager_rejects_host_lobby_countdown() {
        // Countdown exists only on the network host; the C++ constructor
        // asserts Game.Network.isHost() (src/C4GameLobby.cpp:1111-1115).
        let (client, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);

        assert!(client
            .submit_lobby_countdown(lc_network::LobbyCountdownPacket::new(5))
            .is_err());
        assert!(commands.take_submitted_lobby_countdowns().is_empty());
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
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(ClientEvent::Status(status), 0, 7, &event_tx, &telemetry_tx)
            .await
            .expect("forward requested status");

        assert_eq!(
            event_rx.recv().expect("status request event"),
            NetworkEvent::StatusRequested(status)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_ready_check_is_forwarded_to_the_app_unchanged() {
        // The client receives PID_ReadyCheck through the same packet handler
        // and preserves its compiled Client/Data fields
        // (src/C4Network2.cpp:949-953,1625-1635).
        let packet = lc_network::ReadyCheckPacket {
            client_id: 9,
            data: lc_network::ReadyCheckData::NotReady,
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(
            ClientEvent::ReadyCheck { packet },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward ready check");

        assert_eq!(
            event_rx.recv().expect("ready-check event"),
            NetworkEvent::ReadyCheck(packet)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_league_round_results_are_forwarded_to_the_app_unchanged() {
        let packet = lc_network::LeagueRoundResultsPacket {
            success: true,
            result_string: lc_engine::LegacyCString::from_bytes(b"Result:\xe4".to_vec()).unwrap(),
            players: vec![lc_network::LeagueRoundResultsPlayer {
                player_info_id: 17,
                total_playing_time: 1_234,
                settlement_score_old: -2,
                settlement_score_new: 300,
                league_score_new: 1_500,
                league_score_gain: 25,
                league_rank_new: 3,
                league_rank_symbol_new: 9,
                league_progress_data: lc_engine::LegacyCString::from_bytes(b"A=1\xff".to_vec())
                    .unwrap(),
                status: lc_network::LeagueRoundPlayerStatus::Won,
            }],
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(
            ClientEvent::LeagueRoundResults {
                packet: packet.clone(),
            },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward league round results");

        assert_eq!(
            event_rx.recv().expect("league round-results event"),
            NetworkEvent::LeagueRoundResults(packet)
        );
        assert!(matches!(
            telemetry_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_socket_loss_reports_the_host_client_id() {
        // A client has one server peer: C4Network2::OnClientDisconnect checks
        // pClient->isHost() before clearing the live network game. The local
        // client ID must never be substituted for that host peer
        // (pristine 9ffa0a5d src/C4Network2.cpp:1758-1765,1786-1817).
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(
            ClientEvent::Disconnected {
                reason: Some("socket closed".to_string()),
            },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward host disconnect");

        assert_eq!(
            event_rx.recv().expect("disconnect event"),
            NetworkEvent::PeerDisconnected {
                client_id: HOST_CLIENT_ID,
                reason: Some("socket closed".to_string()),
            }
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
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(
            ClientEvent::StatusAck(status),
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward committed status");

        assert_eq!(
            event_rx.try_recv(),
            Ok(NetworkEvent::StatusCommitted(status))
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lobby_messages_are_forwarded_as_typed_app_events() {
        let countdown = lc_network::LobbyCountdownPacket::new(10);
        let request = lc_network::ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: lc_network::ReadyCheckData::Request,
        };
        let ready = lc_network::ReadyCheckPacket {
            client_id: 7,
            data: lc_network::ReadyCheckData::Ready,
        };
        let host_ready = lc_network::ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: lc_network::ReadyCheckData::Ready,
        };
        let (event_tx, event_rx) = mpsc::channel();
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_host_event(
            HostEvent::LobbyCountdown { packet: countdown },
            0,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward host countdown");
        handle_client_event(
            ClientEvent::ReadyCheck { packet: request },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward client ready request");
        handle_client_event(
            ClientEvent::ReadyCheck { packet: ready },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward client ready reply");
        handle_host_event(
            HostEvent::ReadyCheck { packet: host_ready },
            0,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward host ready toggle");

        assert_eq!(
            event_rx.recv().expect("countdown"),
            NetworkEvent::LobbyCountdown(countdown)
        );
        assert_eq!(
            event_rx.recv().expect("ready request"),
            NetworkEvent::ReadyCheck(request)
        );
        assert_eq!(
            event_rx.recv().expect("ready reply"),
            NetworkEvent::ReadyCheck(ready)
        );
        assert_eq!(
            event_rx.recv().expect("host ready toggle"),
            NetworkEvent::ReadyCheck(host_ready)
        );
        assert!(matches!(
            telemetry_rx.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn lobby_methods_queue_only_role_appropriate_commands() {
        let countdown = lc_network::LobbyCountdownPacket::new(30);
        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.broadcast_lobby_countdown(countdown)
            .expect("host countdown");
        host.request_ready_check().expect("host ready request");
        host.set_local_ready(true).expect("host ready state");
        assert!(matches!(
            host_commands.command_rx.try_recv(),
            Ok(NetworkCommand::SubmitLobbyCountdown(value)) if value == countdown
        ));
        assert!(matches!(
            host_commands.command_rx.try_recv(),
            Ok(NetworkCommand::SubmitReadyCheck(
                lc_network::ReadyCheckPacket {
                    client_id: 0,
                    data: lc_network::ReadyCheckData::Request,
                }
            ))
        ));
        assert!(matches!(
            host_commands.command_rx.try_recv(),
            Ok(NetworkCommand::SubmitReadyCheck(
                lc_network::ReadyCheckPacket {
                    client_id: 0,
                    data: lc_network::ReadyCheckData::Ready,
                }
            ))
        ));

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client.set_local_ready(true).expect("client ready state");
        assert!(client.broadcast_lobby_countdown(countdown).is_err());
        assert!(client.request_ready_check().is_err());
        assert!(matches!(
            client_commands.command_rx.try_recv(),
            Ok(NetworkCommand::SubmitReadyCheck(
                lc_network::ReadyCheckPacket {
                    client_id: 7,
                    data: lc_network::ReadyCheckData::Ready,
                }
            ))
        ));
    }

    #[test]
    fn app_lobby_telemetry_is_bounded_without_blocking_critical_events() {
        let (critical_tx, critical_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        for index in 0..NETWORK_TELEMETRY_CAPACITY {
            event_tx
                .try_send(NetworkEvent::Error(format!("diagnostic {index}")))
                .expect("event bridge has advertised capacity");
        }
        assert!(matches!(
            event_tx.try_send(NetworkEvent::Error("overflow".to_string())),
            Err(mpsc::TrySendError::Full(_))
        ));
        let committed = NetworkStatus {
            state: lc_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        critical_tx
            .send(NetworkEvent::StatusCommitted(committed))
            .expect("critical channel remains independent of telemetry");
        assert_eq!(
            critical_rx.recv().expect("critical status commit"),
            NetworkEvent::StatusCommitted(committed)
        );
        assert_eq!(event_rx.try_iter().count(), NETWORK_TELEMETRY_CAPACITY);
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
    fn private_message_emits_an_immediate_control_event() {
        // Despite the per-message Private subtype, C4MessageInput uses
        // CDT_Private for every C4ControlMessage. HandleControlPkt executes
        // that delivery immediately instead of entering SyncControl
        // (src/C4MessageInput.cpp:423-426;
        // src/C4GameControlNetwork.cpp:558-566).
        let message = message_control(7);
        let payload = lc_network::encode_control_entry_payload(&lc_engine::ControlPacket::Message(
            message.clone(),
        ))
        .expect("encode private CID_Message payload");
        let (event_tx, event_rx) = mpsc::channel();

        handle_direct_packet(lc_network::ControlDelivery::Private, payload, &event_tx)
            .expect("handle private CID_Message");

        assert_eq!(
            event_rx.recv(),
            Ok(NetworkEvent::DirectControl(NetworkControl::Message(
                message
            )))
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn direct_client_join_emits_an_immediate_control_event() {
        // Every CDT_Direct control executes immediately through
        // C4GameControl::ExecControl; the host sends C4ControlClientJoin this
        // way before admission completes (pristine 9ffa0a5d
        // src/C4GameControlNetwork.cpp:558-566;
        // src/C4Network2.cpp:1417-1448).
        let join = lc_engine::ClientJoinControlData {
            core: lc_engine::ClientCoreControlData {
                client_id: 7,
                name: lc_engine::LegacyCString::from_bytes(b"Client".to_vec()).unwrap(),
                ..Default::default()
            },
            by_client: 0,
        };
        let payload = lc_network::encode_control_entry_payload(
            &lc_engine::ControlPacket::ClientJoin(join.clone()),
        )
        .expect("encode direct ClientJoin payload");
        let (event_tx, event_rx) = mpsc::channel();

        handle_direct_packet(lc_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct ClientJoin");

        let NetworkEvent::DirectControl(NetworkControl::ClientJoin(actual)) =
            event_rx.recv().expect("direct control event")
        else {
            panic!("expected one immediate ClientJoin event");
        };
        assert_eq!(actual, join);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn direct_vote_emits_an_immediate_authenticated_control_event() {
        // C4Network2::Vote submits CID_Vote through CDT_Direct, so the
        // authenticated ByClient ballot executes immediately instead of
        // entering the synchronized control queue
        // (src/C4Network2.cpp:2842-2868;
        // src/C4GameControlNetwork.cpp:449-490,558-566).
        let vote = lc_engine::VoteControlData {
            vote_type: lc_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 7,
        };
        let payload =
            lc_network::encode_control_entry_payload(&lc_engine::ControlPacket::Vote(vote))
                .expect("encode direct Vote payload");
        let (event_tx, event_rx) = mpsc::channel();

        handle_direct_packet(lc_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct Vote");

        assert_eq!(
            event_rx.recv(),
            Ok(NetworkEvent::DirectControl(NetworkControl::Vote(vote)))
        );
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn direct_control_set_emits_typed_data_instead_of_a_string_error() {
        let set = LegacyControlSet {
            value_type: 3,
            data: 2,
            by_client: 0,
        };
        let payload = lc_network::encode_control_entry_payload(&set.into_control_packet())
            .expect("encode direct CID_Set payload");
        let (event_tx, event_rx) = mpsc::channel();

        handle_direct_packet(lc_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct CID_Set");

        assert_eq!(
            event_rx.recv().expect("direct control event"),
            NetworkEvent::DirectControl(NetworkControl::Set(set))
        );
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
            network_control_for_packet(lc_engine::ControlPacket::PlayerControl(expected.clone())),
            Some(NetworkControl::PlayerControl(expected)),
            "decoded execution must receive the same signed Data payload"
        );

        let raw = PlayerControlData {
            player: 7,
            command: 273,
            data: 4,
            by_client: 3,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::PlayerControl(raw.clone())),
            Some(NetworkControl::PlayerControl(raw)),
            "decoded replay keeps the original signed command for CountControl"
        );
    }

    #[test]
    fn every_in_com_byte_survives_event_codec_and_network_dispatch() {
        let controls = (1..=u8::MAX)
            .map(|command| {
                let event = lc_engine::interpret_player_control_command(i32::from(command))
                    .unwrap_or_else(|| panic!("command {command} was dropped"));
                let packet = control_packet_for_event(7, event, 3)
                    .unwrap_or_else(|| panic!("command {command} was not encoded"));
                assert_eq!(
                    packet,
                    lc_engine::ControlPacket::PlayerControl(PlayerControlData {
                        player: 7,
                        command: i32::from(command),
                        data: 0,
                        by_client: 3,
                    }),
                    "command {command}"
                );
                packet
            })
            .collect::<Vec<_>>();
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 17,
            timestamp_ms: 99,
            controls,
        };

        let encoded = encode_control_packet(&frame).expect("encode every InCom byte");
        let decoded = decode_control_packet(&encoded).expect("decode every InCom byte");
        for (command, packet) in (1..=u8::MAX).zip(decoded.controls) {
            let expected = PlayerControlData {
                player: 7,
                command: i32::from(command),
                data: 0,
                by_client: 3,
            };
            assert_eq!(
                network_control_for_packet(packet),
                Some(NetworkControl::PlayerControl(expected)),
                "command {command}"
            );
        }
    }

    #[test]
    fn decoded_player_command_is_retained_for_scheduled_execution() {
        let command = PlayerCommandControlData {
            player: 7,
            command: 2,
            x: 120,
            y: 45,
            target: 0,
            target2: 91,
            data: 3,
            add_mode: 5,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::PlayerCommand(command)),
            Some(NetworkControl::PlayerCommand(command))
        );
    }

    #[test]
    fn decoded_player_select_is_retained_for_scheduled_execution() {
        let selection = PlayerSelectControlData {
            player: 7,
            objects: vec![12, -4, 91],
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::PlayerSelect(selection.clone())),
            Some(NetworkControl::PlayerSelect(selection))
        );
    }

    #[test]
    fn decoded_script_is_retained_for_scheduled_execution() {
        let script = ScriptControlData {
            target_object: lc_engine::SCRIPT_SCOPE_CONSOLE,
            strictness: lc_engine::ScriptStrictness::Strict2,
            script: lc_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::Script(script.clone())),
            Some(NetworkControl::Script(script))
        );
    }

    #[test]
    fn decoded_message_board_answer_is_retained_for_scheduled_execution() {
        let answer = MessageBoardAnswerControlData {
            object: 42,
            answer: lc_engine::LegacyCString::from_bytes(b"typed answer".to_vec())
                .expect("answer is NUL-free"),
            player: 3,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::MessageBoardAnswer(
                answer.clone(),
            )),
            Some(NetworkControl::MessageBoardAnswer(answer))
        );
    }

    #[test]
    fn decoded_custom_command_is_retained_for_scheduled_execution() {
        let command = lc_engine::CustomCommandControlData {
            command: lc_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("command is NUL-free"),
            argument: lc_engine::LegacyCString::from_bytes(b"arg".to_vec())
                .expect("argument is NUL-free"),
            player: 3,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::CustomCommand(
                command.clone(),
            )),
            Some(NetworkControl::CustomCommand(command))
        );
    }

    #[test]
    fn decoded_em_move_object_is_retained_for_ordered_execution() {
        let control = lc_engine::EmMoveObjectControlData {
            action: lc_engine::EMMO_SCRIPT,
            tx: -12,
            ty: 34,
            target_object: 42,
            objects: vec![7, 9],
            strictness: lc_engine::ScriptStrictness::Strict2,
            script: lc_engine::LegacyCString::from_bytes(b"SetXDir(0)".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::EmMoveObject(
                control.clone(),
            )),
            Some(NetworkControl::EmMoveObject(control))
        );
    }

    #[test]
    fn decoded_em_draw_tool_is_retained_for_ordered_execution() {
        let control = lc_engine::EmDrawToolControlData {
            action: lc_engine::EMDT_LINE,
            mode: lc_engine::LANDSCAPE_MODE_EXACT,
            x: -12,
            y: 34,
            x2: 56,
            y2: -78,
            grade: 9,
            ift: true,
            material: lc_engine::LegacyCString::from_bytes(b"Earth".to_vec())
                .expect("material is NUL-free"),
            texture: lc_engine::LegacyCString::from_bytes(b"Smooth".to_vec())
                .expect("texture is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::EmDrawTool(
                control.clone(),
            )),
            Some(NetworkControl::EmDrawTool(control))
        );
    }

    #[test]
    fn decoded_em_drop_def_is_retained_for_ordered_execution() {
        let control = lc_engine::EmDropDefControlData {
            id: *b"HUT2",
            x: -130,
            y: 130,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(lc_engine::ControlPacket::EmDropDef(control)),
            Some(NetworkControl::EmDropDef(control))
        );
    }

    #[test]
    fn decoded_internal_player_scripts_are_retained_for_ordered_execution() {
        let cases = vec![
            (
                lc_engine::ControlPacket::ActivateGameGoalMenu(
                    lc_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ActivateGameGoalMenu(
                    lc_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
            ),
            (
                lc_engine::ControlPacket::ToggleHostility(
                    lc_engine::ToggleHostilityControlData {
                        opponent: 5,
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ToggleHostility(lc_engine::ToggleHostilityControlData {
                    opponent: 5,
                    player: 3,
                    by_client: 4,
                }),
            ),
            (
                lc_engine::ControlPacket::ActivateGameGoalRule(
                    lc_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ActivateGameGoalRule(
                    lc_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client: 4,
                    },
                ),
            ),
            (
                lc_engine::ControlPacket::SetPlayerTeam(lc_engine::SetPlayerTeamControlData {
                    team: 6,
                    player: 3,
                    by_client: 4,
                }),
                NetworkControl::SetPlayerTeam(lc_engine::SetPlayerTeamControlData {
                    team: 6,
                    player: 3,
                    by_client: 4,
                }),
            ),
            (
                lc_engine::ControlPacket::EliminatePlayer(
                    lc_engine::EliminatePlayerControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::EliminatePlayer(lc_engine::EliminatePlayerControlData {
                    player: 3,
                    by_client: 4,
                }),
            ),
        ];

        for (packet, expected) in cases {
            assert_eq!(network_control_for_packet(packet), Some(expected));
        }
    }

    #[test]
    fn manager_queues_internal_player_scripts_with_authenticated_local_author() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        manager
            .submit_activate_game_goal_menu(20, 3)
            .expect("queue goal menu");
        manager
            .submit_toggle_hostility(21, 3, 4)
            .expect("queue hostility toggle");
        manager
            .submit_activate_game_goal_rule(22, 3, 42)
            .expect("queue goal/rule activation");
        manager
            .submit_set_player_team(23, 3, 5)
            .expect("queue team switch");
        manager
            .submit_eliminate_player(24, 3)
            .expect("queue elimination");

        assert_eq!(
            commands.take_submitted_internal_player_scripts(),
            vec![
                (
                    20,
                    lc_engine::ControlPacket::ActivateGameGoalMenu(
                        lc_engine::ActivateGameGoalMenuControlData {
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    21,
                    lc_engine::ControlPacket::ToggleHostility(
                        lc_engine::ToggleHostilityControlData {
                            opponent: 4,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    22,
                    lc_engine::ControlPacket::ActivateGameGoalRule(
                        lc_engine::ActivateGameGoalRuleControlData {
                            object: 42,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    23,
                    lc_engine::ControlPacket::SetPlayerTeam(
                        lc_engine::SetPlayerTeamControlData {
                            team: 5,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    24,
                    lc_engine::ControlPacket::EliminatePlayer(
                        lc_engine::EliminatePlayerControlData {
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
            ]
        );
    }

    #[test]
    fn custom_command_frame_roundtrips_through_the_tick_accumulator() {
        let command = lc_engine::CustomCommandControlData {
            command: lc_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("command is NUL-free"),
            argument: lc_engine::LegacyCString::from_bytes(b"arg".to_vec())
                .expect("argument is NUL-free"),
            player: 3,
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            lc_engine::ControlPacket::CustomCommand(command.clone()),
            100,
        );
        let frame = accumulator
            .finalize_tick(12)
            .expect("custom command produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(
            frame.controls,
            vec![lc_engine::ControlPacket::CustomCommand(command)]
        );
    }

    #[test]
    fn message_board_answer_frame_roundtrips_through_the_tick_accumulator() {
        let answer = MessageBoardAnswerControlData {
            object: 42,
            answer: lc_engine::LegacyCString::from_bytes(b"typed answer".to_vec())
                .expect("answer is NUL-free"),
            player: 3,
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            lc_engine::ControlPacket::MessageBoardAnswer(answer.clone()),
            100,
        );
        let frame = accumulator
            .finalize_tick(12)
            .expect("message-board answer produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(
            frame.controls,
            vec![lc_engine::ControlPacket::MessageBoardAnswer(answer)]
        );
    }

    #[test]
    fn script_frame_roundtrips_through_the_tick_accumulator() {
        let script = ScriptControlData {
            target_object: lc_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: lc_engine::ScriptStrictness::Strict3,
            script: lc_engine::LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(12, lc_engine::ControlPacket::Script(script.clone()), 100);
        let frame = accumulator
            .finalize_tick(12)
            .expect("script control produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(frame.controls, vec![lc_engine::ControlPacket::Script(script)]);
    }

    #[test]
    fn player_command_frame_roundtrips_through_the_tick_accumulator() {
        let command = PlayerCommandControlData {
            player: 3,
            command: 14,
            x: -25,
            y: 40,
            target: 91,
            target2: 0,
            data: 7,
            add_mode: 5,
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            lc_engine::ControlPacket::PlayerCommand(command),
            100,
        );
        let frame = accumulator
            .finalize_tick(12)
            .expect("player command produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(
            frame.controls,
            vec![lc_engine::ControlPacket::PlayerCommand(command)]
        );
    }

    #[test]
    fn player_select_frame_roundtrips_through_the_tick_accumulator() {
        let selection = PlayerSelectControlData {
            player: 3,
            objects: vec![91, 42],
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            lc_engine::ControlPacket::PlayerSelect(selection.clone()),
            100,
        );
        let frame = accumulator
            .finalize_tick(12)
            .expect("player selection produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(
            frame.controls,
            vec![lc_engine::ControlPacket::PlayerSelect(selection)]
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
    fn control_clock_advances_only_after_a_ready_cpp_control_frame() {
        // C4GameControl executes ControlTick only when
        // FrameCounter%ControlRate==0, then Ticks increments it. JoinData
        // installs the host's signed start tick first (pristine 9ffa0a5d
        // src/C4GameControl.cpp:245-329;
        // src/C4GameControlNetwork.cpp:48-60).
        let mut clock = NetworkControlClock::new(9, 2);

        assert_eq!(clock.tick_for_frame(0), Some(9));
        assert_eq!(clock.tick_for_frame(0), Some(9), "waiting keeps the tick");
        assert_eq!(clock.current_tick(), 9);
        clock.complete_control_frame();
        assert_eq!(clock.current_tick(), 10);

        assert_eq!(clock.tick_for_frame(1), None);
        assert_eq!(clock.current_tick(), 10, "non-control frames do not tick");

        assert_eq!(clock.tick_for_frame(2), Some(10));
        clock.complete_control_frame();
        assert_eq!(clock.tick_for_frame(3), None);
        assert_eq!(clock.tick_for_frame(4), Some(11));
    }

    #[test]
    fn control_clock_rate_change_consumes_the_admitted_tick_without_resetting_phase() {
        let mut clock = NetworkControlClock::new(9, 2);

        assert_eq!(clock.tick_for_frame(2), Some(9));
        assert_eq!(clock.adjust_control_rate(2), 4);
        clock.complete_control_frame();

        assert_eq!(clock.current_tick(), 10);
        assert_eq!(clock.tick_for_frame(3), None);
        assert_eq!(clock.tick_for_frame(4), Some(10));
        assert_eq!(clock.adjust_control_rate(i32::MAX), MAX_CONTROL_RATE);
        assert_eq!(clock.adjust_control_rate(i32::MIN), 1);
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
