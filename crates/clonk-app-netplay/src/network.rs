use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use clonk_engine::{
    CommandKind, ControlButton, ControlCommand, ControlEvent, JoinPlayerControlData,
    MessageBoardAnswerControlData, MessageControlData, PlayerCommandControlData, PlayerControlData,
    PlayerInfoControlData, PlayerSelectControlData, ScriptControlData, SyncCheckPacket,
    COM_CLEAR_PRESSED_COMS, COM_CURSOR_LEFT, COM_CURSOR_RIGHT, COM_CURSOR_TOGGLE, COM_DIG,
    COM_DOUBLE, COM_DOWN, COM_LEFT, COM_MENU_CLOSE, COM_MENU_DOWN, COM_MENU_ENTER,
    COM_MENU_ENTER_ALL, COM_MENU_LEFT, COM_MENU_RIGHT, COM_MENU_SELECT, COM_MENU_SHOW_TEXT,
    COM_MENU_UP, COM_PLAYER_MENU, COM_RELEASE_OFFSET, COM_RIGHT, COM_SINGLE, COM_SPECIAL,
    COM_SPECIAL2, COM_THROW, COM_UP,
};
#[cfg(test)]
use clonk_network::start_host;
use clonk_network::{
    connect_client_addresses, decode_control_entry_payload, decode_control_packet,
    encode_control_entry_payload, encode_control_packet, start_host_with_bindings, ClientConfig,
    ClientEvent, ClientHandle, ClientId, ClientMeshPuncherConfig, ClientPlayerResourceRequest,
    ControlDelivery, ControlLatencyEstimator, ControlPacket, HostConfig, HostEvent, HostHandle,
    HostJoinSnapshot, HostUdpBinding, LegacyControlFrame, LegacyControlSet, NetpuncherGameIds,
    NetworkAddress, NetworkJoinRoutePlan, NetworkProtocol, NetworkStatus, ParticipantKind, Tick,
};
pub use clonk_network::{
    RuntimeLobbyClientTelemetry, RuntimeNetworkClientState, RuntimeNetworkConnection,
};
use parking_lot::Mutex;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::mpsc as tokio_mpsc;

use crate::prepared_host_bootstrap::{
    league_checksum_start, PreparedHostBootstrap, PreparedLeagueHostConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkEventWake {
    ReadyTick(Tick),
    ResourceComplete(i32),
    ResourceLoadFailed(i32),
}

pub type NetworkEventWakeCallback = Arc<dyn Fn(NetworkEventWake) + Send + Sync>;

#[derive(Clone, Default)]
struct NetworkEventWakeHandle {
    callback: Arc<Mutex<Option<NetworkEventWakeCallback>>>,
}

impl std::fmt::Debug for NetworkEventWakeHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NetworkEventWakeHandle")
            .field("installed", &self.callback.lock().is_some())
            .finish()
    }
}

impl NetworkEventWakeHandle {
    fn install(&self, callback: NetworkEventWakeCallback) {
        *self.callback.lock() = Some(callback);
    }

    fn notify(&self, event: NetworkEventWake) {
        let callback = self.callback.lock().clone();
        if let Some(callback) = callback {
            callback(event);
        }
    }
}

/// Lossless app event sender with the native network-pipe wake side effect.
///
/// C++ first queues the packet event, then signals its application pipe. Keep
/// that ordering so a woken app always observes the corresponding event.
#[derive(Clone, Debug)]
pub struct NetworkEventSender {
    sender: Sender<NetworkEvent>,
    wake: NetworkEventWakeHandle,
}

impl NetworkEventSender {
    fn channel() -> (Self, Receiver<NetworkEvent>) {
        let (sender, receiver) = mpsc::channel();
        (
            Self {
                sender,
                wake: NetworkEventWakeHandle::default(),
            },
            receiver,
        )
    }

    // Preserve `std::sync::mpsc::Sender::send` semantics so callers recover the
    // exact unsent event when the application-side receiver has gone away.
    #[allow(clippy::result_large_err)]
    pub fn send(
        &self,
        event: NetworkEvent,
    ) -> std::result::Result<(), mpsc::SendError<NetworkEvent>> {
        let wake = match &event {
            NetworkEvent::ReadyTick { tick, .. } => Some(NetworkEventWake::ReadyTick(*tick)),
            NetworkEvent::ResourceComplete { resource_id, .. } => {
                Some(NetworkEventWake::ResourceComplete(*resource_id))
            }
            NetworkEvent::ResourceLoadFailed { resource_id } => {
                Some(NetworkEventWake::ResourceLoadFailed(*resource_id))
            }
            _ => None,
        };
        self.sender.send(event)?;
        if let Some(wake) = wake {
            self.wake.notify(wake);
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
// This public mode is destructured throughout the app; boxing just the host
// configuration would add pervasive indirection to a startup-only value.
#[allow(clippy::large_enum_variant)]
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
    /// Original prepared reference routes, retained once each for connection
    /// progress and diagnostics before local routes are expanded by interface.
    pub logical_server_addresses: Vec<NetworkAddress>,
    /// Concrete transport endpoints to dial. Local reference routes may occur
    /// once per local interface ID.
    pub server_addresses: Vec<NetworkAddress>,
    /// C++ `C4XVERBUILD` value this client presents for the selected game.
    /// Reference-backed joins use the host's advertised build so Rust release
    /// versioning does not prevent an otherwise compatible connection.
    pub compatibility_build: i32,
    pub player_name: String,
    pub observer: bool,
    pub group_maker: clonk_engine::LegacyCString,
    pub password: clonk_engine::LegacyCString,
    pub resource_directory: PathBuf,
    pub local_system_path: Option<PathBuf>,
    pub local_resource_roots: Vec<PathBuf>,
    /// `Config.Network.MaxResSearchRecursion` (C4Config.cpp:527-533), the depth
    /// `SearchLocal` walks candidate folders to (C4Network2Res.cpp:460-490).
    pub max_resource_search_recursion: usize,
    pub league_transport: clonk_network::LeagueHttpTransportConfig,
    pub league_auth: clonk_network::LeagueAuthRequestHead,
    /// League HTTP host retained from accepted JoinData for later local-player
    /// authentication dialogs after the join envelope is released.
    pub league_server_name: String,
    pub mesh_tcp_bind_address: Option<SocketAddr>,
    pub mesh_udp_bind_address: Option<SocketAddr>,
    pub netpuncher_address: Option<String>,
    pub netpuncher_game_ids: NetpuncherGameIds,
}

impl ClientSettings {
    pub fn new(server_addr: SocketAddr, player_name: impl Into<String>) -> Self {
        let player_name = player_name.into();
        let group_maker = clonk_engine::LegacyCString::from_bytes(player_name.as_bytes().to_vec())
            .unwrap_or_default();
        let wildcard = if server_addr.is_ipv4() {
            SocketAddr::from(([0, 0, 0, 0], 0))
        } else {
            SocketAddr::from(([0_u16; 8], 0))
        };
        let logical_server_addresses = vec![
            NetworkAddress::new(NetworkProtocol::Tcp, server_addr),
            NetworkAddress::new(NetworkProtocol::Udp, server_addr),
        ];
        Self {
            logical_server_addresses: logical_server_addresses.clone(),
            server_addresses: logical_server_addresses,
            compatibility_build: clonk_network::CURRENT_GAME_BUILD,
            player_name,
            observer: false,
            group_maker,
            password: clonk_engine::LegacyCString::default(),
            resource_directory: PathBuf::from("Network"),
            local_system_path: None,
            local_resource_roots: Vec::new(),
            max_resource_search_recursion: 1,
            league_transport: clonk_network::LeagueHttpTransportConfig::default(),
            league_auth: clonk_network::LeagueAuthRequestHead::default(),
            league_server_name: String::new(),
            mesh_tcp_bind_address: Some(wildcard),
            mesh_udp_bind_address: Some(wildcard),
            netpuncher_address: None,
            netpuncher_game_ids: NetpuncherGameIds { ipv4: 0, ipv6: 0 },
        }
    }

    /// Replaces only the concrete dial attempts. The logical routes remain
    /// available for progress presentation.
    pub fn with_join_attempts(
        mut self,
        addresses: impl IntoIterator<Item = NetworkAddress>,
    ) -> Self {
        self.server_addresses = addresses.into_iter().collect();
        self
    }

    pub fn with_join_route_plan(mut self, route_plan: NetworkJoinRoutePlan) -> Self {
        self.logical_server_addresses = route_plan.logical_addresses;
        self.server_addresses = route_plan.dial_attempts;
        self
    }

    pub fn join_protocol_enabled(&self, address: &NetworkAddress) -> bool {
        client_join_protocol_enabled(
            address,
            self.mesh_tcp_bind_address.is_some(),
            self.mesh_udp_bind_address.is_some(),
        )
    }

    pub fn with_compatibility_build(mut self, compatibility_build: i32) -> Self {
        self.compatibility_build = compatibility_build;
        self
    }

    pub fn with_password(mut self, password: clonk_engine::LegacyCString) -> Self {
        self.password = password;
        self
    }

    pub fn with_netpuncher(
        mut self,
        address: impl Into<String>,
        game_ids: NetpuncherGameIds,
    ) -> Self {
        let address = address.into();
        self.netpuncher_address = (!address.is_empty()).then_some(address);
        self.netpuncher_game_ids = game_ids;
        self
    }

    pub fn with_league_auth(mut self, auth: clonk_network::LeagueAuthRequestHead) -> Self {
        self.league_auth = auth;
        self
    }
}

async fn resolve_client_mesh_punchers(
    address: Option<&str>,
    game_ids: NetpuncherGameIds,
) -> Vec<ClientMeshPuncherConfig> {
    let Some(address) = address.map(str::trim).filter(|address| !address.is_empty()) else {
        return Vec::new();
    };
    resolve_netpuncher_addresses(address)
        .await
        .into_iter()
        .map(|address| {
            let game_id = if address.is_ipv4() {
                game_ids.ipv4
            } else {
                game_ids.ipv6
            };
            ClientMeshPuncherConfig { address, game_id }
        })
        .collect()
}

fn client_join_protocol_enabled(
    address: &NetworkAddress,
    tcp_enabled: bool,
    udp_enabled: bool,
) -> bool {
    match address.protocol {
        NetworkProtocol::Tcp => tcp_enabled,
        NetworkProtocol::Udp => udp_enabled,
        NetworkProtocol::Unknown(_) => false,
        _ => false,
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NetworkStartError {
    #[error("host rejected the client password: {message:?}")]
    WrongPassword {
        message: clonk_engine::LegacyCString,
    },
    #[error("network startup was cancelled")]
    Cancelled,
    #[error("{0}")]
    Other(String),
}

impl From<String> for NetworkStartError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

/// Renders a failed client join for the startup caption the app shows.
fn client_startup_error(error: clonk_network::ClientError) -> NetworkStartError {
    match error {
        clonk_network::ClientError::WrongPassword { message } => {
            NetworkStartError::WrongPassword { message }
        }
        other => NetworkStartError::Other(other.to_string()),
    }
}

#[derive(Clone, Debug)]
pub struct NetworkStartupCancellation {
    inner: Arc<NetworkStartupCancellationInner>,
}

#[derive(Debug)]
struct NetworkStartupCancellationInner {
    cancelled: AtomicBool,
    notification: tokio::sync::Notify,
}

impl NetworkStartupCancellation {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(NetworkStartupCancellationInner {
                cancelled: AtomicBool::new(false),
                notification: tokio::sync::Notify::new(),
            }),
        }
    }

    pub fn cancel(&self) -> bool {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            return false;
        }
        // `notify_one` retains a permit when cancellation races the first
        // poll of `cancelled`; `notify_waiters` would lose that wake-up.
        self.inner.notification.notify_one();
        true
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    async fn cancelled(&self) {
        loop {
            let notification = self.inner.notification.notified();
            if self.is_cancelled() {
                return;
            }
            notification.await;
        }
    }
}

impl Default for NetworkStartupCancellation {
    fn default() -> Self {
        Self::new()
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
const MAX_CONTROL_PRESEND: i32 = 15;
const DEFAULT_CONTROL_TARGET_FPS: i32 = 38;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPreSendChange {
    pub control_presend: i32,
    pub target_fps: i32,
}

/// C4GameControl's frame-to-ControlTick cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkControlClock {
    control_tick: i32,
    control_rate: u64,
    control_sent: i32,
    control_presend: i32,
    target_fps: i32,
    avg_control_send_time_us: i32,
    control_send_time_ms: Option<i32>,
    /// Measured lateness of the last consumed control tick; see
    /// `observe_control_lateness_ms`.
    control_lateness_ms: Option<i32>,
    target_tick: Option<i32>,
    local_activated: Option<bool>,
    /// Sizes PreSend from the delivery-time tail rather than its mean. See
    /// `ControlLatencyEstimator` and the PORT_STATUS divergence entry.
    latency: ControlLatencyEstimator,
}

impl NetworkControlClock {
    pub fn new(start_tick: i32, control_rate: i32) -> Self {
        Self {
            control_tick: start_tick,
            control_rate: control_rate.clamp(1, MAX_CONTROL_RATE) as u64,
            control_sent: start_tick.saturating_sub(1),
            control_presend: 1,
            target_fps: DEFAULT_CONTROL_TARGET_FPS,
            avg_control_send_time_us: 0,
            control_send_time_ms: None,
            control_lateness_ms: None,
            target_tick: None,
            local_activated: None,
            latency: ControlLatencyEstimator::new(),
        }
    }

    /// Return every local contribution whose predicted execution frame lies
    /// inside the current PreSend horizon. The cursor is advanced when the
    /// app queues `FinalizeTick`, matching `DoInput`'s synchronous increment
    /// of C++ `iControlSent`.
    pub fn take_due_ticks(&mut self, frame: u64, activated: bool) -> Vec<i32> {
        match self.local_activated.replace(activated) {
            Some(false) if activated => {
                // C4GameControlNetwork::SetActivated starts at the next
                // control tick instead of backfilling the inactive interval.
                self.control_sent = self.control_tick.saturating_sub(1);
            }
            _ => {}
        }
        if !activated {
            return Vec::new();
        }

        // `control_tick` is Rust's next tick to execute. Between cadence
        // frames that is one ahead of C++ ControlTick, so derive the horizon
        // from the distance to that next execution instead of applying
        // C++ getCtrlTick directly to this representation.
        let phase = frame % self.control_rate;
        let frames_until_control = if phase == 0 {
            0
        } else {
            self.control_rate - phase
        };
        let presend = self.control_presend as u64;
        let send_through = if presend < frames_until_control {
            self.control_tick.saturating_sub(1)
        } else {
            let additional = (presend - frames_until_control) / self.control_rate;
            self.control_tick
                .saturating_add(i32::try_from(additional).unwrap_or(i32::MAX))
        };

        let mut due = Vec::new();
        while self.control_sent < send_through
            && self
                .target_tick
                .is_none_or(|target_tick| self.control_sent < target_tick)
        {
            self.control_sent = self.control_sent.saturating_add(1);
            due.push(self.control_sent);
        }
        due
    }

    pub fn next_unsent_tick(self) -> i32 {
        self.control_sent.saturating_add(1)
    }

    pub fn set_target_tick(&mut self, target_tick: Option<i32>) {
        self.target_tick = target_tick;
    }

    /// Store C++ CalcPerformance's topology-aware message-route sample for the
    /// successfully consumed control tick.
    pub fn observe_control_send_time_ms(&mut self, control_send_time_ms: i32) {
        self.control_send_time_ms = Some(control_send_time_ms);
    }

    /// Store how late the consumed control tick actually was — the interval
    /// between reaching the control tick and the aggregate becoming available.
    ///
    /// This is the client-side counterpart of the host's
    /// `ClientPerformanceStats::wait_ms` and of C++'s
    /// `C4GameControlClient::AddPerf(pCtrl->getTime() - iWaitStart)`. Unlike
    /// ping it is measured against the cadence rather than against a send time,
    /// so it stays independent of the horizon it is used to size: a bigger
    /// PreSend makes control arrive earlier relative to its slot, which closes
    /// the loop instead of feeding it.
    pub fn observe_control_lateness_ms(&mut self, control_lateness_ms: i32) {
        self.control_lateness_ms = Some(control_lateness_ms.max(0));
    }

    pub fn control_presend(self) -> i32 {
        self.control_presend
    }

    /// Lateness of the last consumed control tick, or `None` before one has
    /// been measured. Read-only diagnostics; the horizon uses it through
    /// `update_control_presend`.
    pub fn control_lateness_ms(self) -> Option<i32> {
        self.control_lateness_ms
    }

    /// Delivery-time budget the horizon is currently sized to cover. Unlike
    /// [`avg_control_send_time`](Self::avg_control_send_time), which stays on
    /// C++'s ping-derived mean for the script- and dialog-visible ACT field,
    /// this is the tail-aware envelope PreSend is actually chosen from.
    pub fn control_latency_budget(self) -> Duration {
        self.latency.budget()
    }

    pub fn target_fps(self) -> i32 {
        self.target_fps
    }

    /// `C4GameControlNetwork::setTargetFPS` changes only the target. The
    /// rolling-average calculation updates PreSend after a later consumed
    /// control, so do not recalculate or reset the average here.
    pub fn set_target_fps(&mut self, target_fps: i32) {
        debug_assert!(target_fps > 0);
        self.target_fps = target_fps;
    }

    /// Smoothed control-send time in microseconds, matching
    /// `C4GameControlNetwork::getAvgControlSendTime()` and the `ACT` field in
    /// the runtime network client-list dialog.
    pub fn avg_control_send_time(self) -> i64 {
        i64::from(self.avg_control_send_time_us)
    }

    fn update_control_presend(&mut self) -> Option<ControlPreSendChange> {
        let ping_sample = self.control_send_time_ms.filter(|sample| *sample != 0);
        if let Some(control_send_time_ms) = ping_sample {
            // Both fields and both expressions are `int32_t` in C++. Preserve
            // the target's full script-visible range and the platform's two's-
            // complement arithmetic instead of silently widening extreme values.
            self.avg_control_send_time_us = self
                .avg_control_send_time_us
                .wrapping_mul(149)
                .wrapping_add(control_send_time_ms.wrapping_mul(1_000))
                / 150;
        }
        // The C++ average above stays exactly as C++ computes it because it is
        // the script- and dialog-visible ACT field. Only the PreSend decision
        // moves to the tail-aware budget: C++ sizes the horizon so that the
        // *mean* control arrives in time, which leaves every above-mean packet
        // stalling the whole session.
        //
        // The decision also takes the larger of the route ping and the control
        // lateness actually measured for the tick just consumed. Ping alone is
        // blind to a client that is slow rather than distant — a weak machine, a
        // saturated uplink queue, a host that waited on somebody else — and that
        // blindness is what silently drops a struggling player's input. Taking
        // the maximum rather than replacing keeps a punctual client on exactly
        // C++'s horizon, so the extra input latency is charged only where it
        // buys something.
        let sample = match (ping_sample, self.control_lateness_ms) {
            (Some(ping), Some(lateness)) => ping.max(lateness),
            (Some(ping), None) => ping,
            (None, Some(lateness)) => lateness,
            (None, None) => return None,
        };
        self.latency.observe(sample);
        let next = self
            .target_fps
            .wrapping_mul(self.latency.budget_us())
            .wrapping_div(1_000_000)
            .wrapping_add(1)
            .clamp(1, MAX_CONTROL_PRESEND);
        if next == self.control_presend {
            return None;
        }
        self.control_presend = next;
        Some(ControlPreSendChange {
            control_presend: next,
            target_fps: self.target_fps,
        })
    }

    /// Current control tick on frames where C4GameControl executes control.
    /// This is a non-consuming probe because a network stall retries the same
    /// frame and tick until `CtrlReady` succeeds.
    pub fn tick_for_frame(self, frame: u64) -> Option<i32> {
        if !frame.is_multiple_of(self.control_rate) {
            return None;
        }
        Some(self.control_tick)
    }

    /// C4GameControlNetwork::GetControl runs this before decoded controls.
    /// Keeping it separate from tick advancement ensures a SetPreSend in the
    /// current control cannot affect the sample that was already consumed.
    pub fn calculate_performance(&mut self) -> Option<ControlPreSendChange> {
        self.update_control_presend()
    }

    /// Consume the tick whose control frame was admitted by `tick_for_frame`.
    /// Keep this independent of the current rate: a CID_Set in that frame may
    /// already have changed the cadence before execution completes.
    pub fn complete_control_frame(&mut self) {
        self.control_tick = self.control_tick.wrapping_add(1);
    }

    /// `C4CVT_ControlRate`: preserve the absolute FrameCounter phase while
    /// changing the divisor used by all subsequent frame probes.
    pub fn adjust_control_rate(&mut self, delta: i32) -> i32 {
        let control_rate = (self.control_rate as i32)
            .saturating_add(delta)
            .clamp(1, MAX_CONTROL_RATE);
        self.control_rate = control_rate as u64;
        control_rate
    }

    pub fn set_control_rate(&mut self, control_rate: i32) -> i32 {
        let control_rate = control_rate.clamp(1, MAX_CONTROL_RATE);
        self.control_rate = control_rate as u64;
        control_rate
    }

    pub fn control_rate(self) -> i32 {
        self.control_rate as i32
    }

    pub fn current_tick(self) -> i32 {
        self.control_tick
    }

    /// `C4GameControl::ControlTick` for a presented frame. Rust advances its
    /// next-tick cursor as soon as the cadence frame completes; native keeps
    /// displaying the executed tick until the next cadence boundary.
    pub fn display_control_tick_for_frame(self, frame: u64) -> i32 {
        if frame.is_multiple_of(self.control_rate) {
            self.control_tick
        } else {
            self.control_tick.wrapping_sub(1)
        }
    }

    pub fn engine_timing(
        self,
    ) -> Result<clonk_engine::NetworkControlTiming, clonk_engine::InvalidNetworkControlRate> {
        clonk_engine::NetworkControlTiming::new(self.control_tick, self.control_rate as i32)
    }
}

#[derive(Debug)]
struct NetworkWorkerReady {
    local_client_id: ClientId,
    control_send_time: clonk_network::ControlSendTimeSnapshot,
    league_start_response: Option<clonk_network::LeagueStartResponse>,
    /// Why this host is running unregistered, when the league server refused
    /// the `Start` that `C4Network2::InitHost` survives
    /// (src/C4Network2.cpp:259-272).
    league_start_failure: Option<String>,
    league_runtime_available: bool,
    league_record_runtime: Option<LeagueRecordRuntimeHandle>,
    network_io_statistics: clonk_network::NetworkIoStatistics,
}

#[derive(Debug)]
enum LeagueRuntimeCommand {
    AuthenticatePlayer {
        auth: clonk_network::LeagueAuthRequestHead,
        player: clonk_engine::ControlPlayerInfoEntry,
        completion: Sender<std::result::Result<clonk_network::LeagueAuthResponse, String>>,
    },
    CheckPlayer {
        player: clonk_engine::ControlPlayerInfoEntry,
        completion: Sender<std::result::Result<clonk_network::LeagueJoinResponse, String>>,
    },
    Update {
        now: i64,
        reference: clonk_network::HostGameReference,
    },
    End {
        reference: clonk_network::HostGameReference,
        record: Option<clonk_network::LeagueEndRecord>,
        completion: tokio::sync::oneshot::Sender<LeagueEndAttempt>,
    },
    FinalizeEndFailure {
        packet: clonk_network::LeagueRoundResultsPacket,
        completion: tokio::sync::oneshot::Sender<Option<clonk_network::LeagueRoundResultsPacket>>,
    },
    ReportDisconnect {
        reason: clonk_network::LeagueDisconnectReason,
        players: clonk_network::ClientPlayerInfosSnapshot,
        fbids: clonk_network::LeagueFbidRegistry,
        completion: Sender<std::result::Result<(), String>>,
    },
    Invalidate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeagueEndFailurePhase {
    Start,
    Send,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeagueEndAttempt {
    Finished(Option<clonk_network::LeagueRoundResultsPacket>),
    Rejected(clonk_network::LeagueRoundResultsPacket),
    Retryable {
        phase: LeagueEndFailurePhase,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaguePlayerCheck {
    Accepted,
    Rejected(clonk_engine::LegacyCString),
    Unavailable,
}

#[derive(Debug, Default)]
struct LeagueRuntimeGate {
    busy: bool,
    priority_pending: usize,
}

struct LeagueRuntimeGateLease {
    gate: Arc<Mutex<LeagueRuntimeGate>>,
    priority: bool,
}

impl Drop for LeagueRuntimeGateLease {
    fn drop(&mut self) {
        let mut gate = self.gate.lock();
        if self.priority {
            gate.priority_pending = gate.priority_pending.saturating_sub(1);
        }
        gate.busy = gate.priority_pending != 0;
    }
}

#[derive(Debug)]
struct LeagueRuntimeHandle {
    command_tx: tokio_mpsc::Sender<LeagueRuntimeCommand>,
    gate: Arc<Mutex<LeagueRuntimeGate>>,
}

impl LeagueRuntimeHandle {
    fn try_update(&self, now: i64, reference: clonk_network::HostGameReference) {
        {
            let mut gate = self.gate.lock();
            if gate.busy {
                return;
            }
            gate.busy = true;
        }
        if self
            .command_tx
            .try_send(LeagueRuntimeCommand::Update { now, reference })
            .is_err()
        {
            let mut gate = self.gate.lock();
            gate.busy = gate.priority_pending != 0;
        }
    }

    async fn send_priority(&self, command: LeagueRuntimeCommand) -> std::result::Result<(), ()> {
        {
            let mut gate = self.gate.lock();
            gate.priority_pending = gate.priority_pending.saturating_add(1);
            gate.busy = true;
        }
        if self.command_tx.send(command).await.is_err() {
            let mut gate = self.gate.lock();
            gate.priority_pending = gate.priority_pending.saturating_sub(1);
            gate.busy = gate.priority_pending != 0;
            return Err(());
        }
        Ok(())
    }
}

struct LeagueRuntimeState {
    config: PreparedLeagueHostConfig,
    transport: clonk_network::LeagueHttpPostTransport,
    session: Option<clonk_network::LeagueHostSession>,
    heartbeat: Option<clonk_network::LeagueHeartbeat>,
    end_sent: bool,
    projected_gains: HashMap<i32, i32>,
    fbids: clonk_network::LeagueFbidRegistry,
}

#[derive(Debug)]
enum LeagueRecordRuntimeCommand {
    Start {
        now: i64,
        completion: Sender<std::result::Result<(), String>>,
    },
    Append(Vec<u8>),
    Pump {
        now: i64,
    },
    Finish {
        now: i64,
        completion: Sender<std::result::Result<(), String>>,
    },
    Shutdown {
        completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    },
}

#[derive(Debug, Clone)]
struct LeagueRecordRuntimeHandle {
    command_tx: tokio_mpsc::UnboundedSender<LeagueRecordRuntimeCommand>,
    status: Arc<Mutex<LeagueRecordStreamStatus>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LeagueRecordStreamStatus {
    is_streaming: bool,
    /// Bytes still waiting in C4Record's uncompressed StreamingBuf.
    waiting_raw_bytes: usize,
    /// Uncompressed position consumed by the persistent zlib stream.
    input_position: u64,
    /// C++'s `getPendingStreamData`: compressed bytes retained for upload,
    /// including an in-flight prefix until its successful acknowledgement.
    pending_compressed_bytes: usize,
    /// Successfully acknowledged compressed-byte position.
    sent_position: u32,
}

impl LeagueRecordStreamStatus {
    pub fn is_streaming(self) -> bool {
        self.is_streaming
    }

    pub fn pending_compressed_bytes(self) -> usize {
        self.pending_compressed_bytes
    }

    pub fn waiting_raw_bytes(self) -> usize {
        self.waiting_raw_bytes
    }

    pub fn input_position(self) -> u64 {
        self.input_position
    }

    pub fn sent_position(self) -> u32 {
        self.sent_position
    }
}

impl LeagueRecordRuntimeHandle {
    fn status(&self) -> LeagueRecordStreamStatus {
        *self.status.lock()
    }
}

#[derive(Debug)]
struct LeagueRecordUploadResult {
    success: bool,
    error: Option<String>,
}

fn spawn_league_record_runtime(
    endpoint: String,
    config: clonk_network::LeagueHttpTransportConfig,
) -> std::result::Result<LeagueRecordRuntimeHandle, clonk_network::LeagueHttpTransportError> {
    let transport = clonk_network::LeagueHttpPostTransport::for_backend(config.http_backend)?;
    let (command_tx, command_rx) = tokio_mpsc::unbounded_channel();
    let status = Arc::new(Mutex::new(LeagueRecordStreamStatus::default()));
    tokio::spawn(run_league_record_runtime(
        endpoint,
        transport,
        config,
        Arc::clone(&status),
        command_rx,
    ));
    Ok(LeagueRecordRuntimeHandle { command_tx, status })
}

async fn run_league_record_runtime(
    endpoint: String,
    transport: clonk_network::LeagueHttpPostTransport,
    config: clonk_network::LeagueHttpTransportConfig,
    status: Arc<Mutex<LeagueRecordStreamStatus>>,
    mut command_rx: tokio_mpsc::UnboundedReceiver<LeagueRecordRuntimeCommand>,
) {
    let (upload_result_tx, mut upload_result_rx) =
        tokio_mpsc::channel::<LeagueRecordUploadResult>(1);
    let mut stream: Option<clonk_network::LeagueRecordStream> = None;
    let mut finish_requested = false;
    let mut shutdown_completion: Option<
        tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    > = None;
    let mut last_now = 0;

    loop {
        tokio::select! {
            biased;
            result = upload_result_rx.recv() => {
                let Some(result) = result else { break };
                let success = result.success;
                let upload_error = result.error;
                if let Some(error) = upload_error.as_deref() {
                    tracing::warn!(%error, "league record upload failed; retaining bytes for retry");
                }
                let Some(stream) = stream.as_mut() else {
                    publish_league_record_stream_status(&status, None);
                    continue;
                };
                if let Err(error) = stream.acknowledge_upload(success) {
                    tracing::error!(%error, "league record upload acknowledgement failed");
                }
                let shutdown_result = if shutdown_completion.is_some() {
                    if !success {
                        Some(Err(upload_error.unwrap_or_else(|| {
                            "league record upload failed during shutdown".to_string()
                        })))
                    } else if !stream.is_streaming() {
                        Some(Ok(()))
                    } else {
                        dispatch_league_record_upload(
                            stream,
                            last_now,
                            &transport,
                            &config,
                            &upload_result_tx,
                        )
                        .err()
                        .map(|error| Err(error.to_string()))
                    }
                } else {
                    None
                };
                publish_league_record_stream_status(&status, Some(&*stream));
                if let Some(result) = shutdown_result {
                    if let Some(completion) = shutdown_completion.take() {
                        complete_league_record_runtime_shutdown(&status, completion, result);
                    }
                    break;
                }
            }
            command = command_rx.recv() => {
                let Some(command) = command else { break };
                match command {
                    LeagueRecordRuntimeCommand::Start { now, completion } => {
                        last_now = now;
                        let result = if stream.as_ref().is_some_and(clonk_network::LeagueRecordStream::is_streaming) {
                            Err("league record stream is already active".to_string())
                        } else {
                            stream = Some(clonk_network::LeagueRecordStream::new(endpoint.clone(), now));
                            finish_requested = false;
                            Ok(())
                        };
                        publish_league_record_stream_status(&status, stream.as_ref());
                        let _ = completion.send(result);
                    }
                    LeagueRecordRuntimeCommand::Append(bytes) => {
                        let result = stream
                            .as_mut()
                            .ok_or_else(|| "league record stream is not active".to_string())
                            .and_then(|stream| stream.append(&bytes).map_err(|error| error.to_string()));
                        if let Err(error) = result {
                            tracing::error!(%error, "failed to append league record bytes");
                        }
                        publish_league_record_stream_status(&status, stream.as_ref());
                    }
                    LeagueRecordRuntimeCommand::Pump { now } => {
                        last_now = now;
                        if let Some(stream) = stream.as_mut() {
                            if let Err(error) = dispatch_league_record_upload(
                                stream,
                                now,
                                &transport,
                                &config,
                                &upload_result_tx,
                            ) {
                                tracing::error!(%error, "failed to pump league record stream");
                            }
                        }
                        publish_league_record_stream_status(&status, stream.as_ref());
                    }
                    LeagueRecordRuntimeCommand::Finish { now, completion } => {
                        last_now = now;
                        let result = match stream.as_mut() {
                            Some(stream) => stream
                                .finish()
                                .map_err(|error| error.to_string())
                                .and_then(|()| {
                                    dispatch_league_record_upload(
                                        stream,
                                        now,
                                        &transport,
                                        &config,
                                        &upload_result_tx,
                                    )
                                    .map_err(|error| error.to_string())
                                }),
                            None => Err("league record stream is not active".to_string()),
                        };
                        if result.is_ok() {
                            finish_requested = true;
                        }
                        publish_league_record_stream_status(&status, stream.as_ref());
                        let _ = completion.send(result);
                    }
                    LeagueRecordRuntimeCommand::Shutdown { completion } => {
                        let needs_drain = finish_requested
                            && stream
                                .as_ref()
                                .is_some_and(clonk_network::LeagueRecordStream::is_streaming);
                        if !needs_drain {
                            complete_league_record_runtime_shutdown(
                                &status,
                                completion,
                                Ok(()),
                            );
                            break;
                        }
                        shutdown_completion = Some(completion);
                        let dispatch_result = dispatch_league_record_upload(
                            stream.as_mut().expect("active finishing stream checked above"),
                            last_now,
                            &transport,
                            &config,
                            &upload_result_tx,
                        );
                        publish_league_record_stream_status(&status, stream.as_ref());
                        match dispatch_result {
                            Err(error) => {
                                if let Some(completion) = shutdown_completion.take() {
                                    complete_league_record_runtime_shutdown(
                                        &status,
                                        completion,
                                        Err(error.to_string()),
                                    );
                                }
                                break;
                            }
                            Ok(())
                                if stream
                                    .as_ref()
                                    .is_some_and(|stream| !stream.is_streaming()) =>
                            {
                                if let Some(completion) = shutdown_completion.take() {
                                    complete_league_record_runtime_shutdown(
                                        &status,
                                        completion,
                                        Ok(()),
                                    );
                                }
                                break;
                            }
                            Ok(()) => {}
                        }
                    }
                }
            }
        }
    }

    publish_league_record_stream_status(&status, None);
}

fn complete_league_record_runtime_shutdown(
    status: &Mutex<LeagueRecordStreamStatus>,
    completion: tokio::sync::oneshot::Sender<std::result::Result<(), String>>,
    result: std::result::Result<(), String>,
) {
    publish_league_record_stream_status(status, None);
    let _ = completion.send(result);
}

fn publish_league_record_stream_status(
    status: &Mutex<LeagueRecordStreamStatus>,
    stream: Option<&clonk_network::LeagueRecordStream>,
) {
    *status.lock() = stream.map_or_else(LeagueRecordStreamStatus::default, |stream| {
        LeagueRecordStreamStatus {
            is_streaming: stream.is_streaming(),
            waiting_raw_bytes: stream.pending_raw_len(),
            input_position: stream.input_position(),
            pending_compressed_bytes: stream.pending_compressed_len(),
            sent_position: stream.position(),
        }
    });
}

fn dispatch_league_record_upload(
    stream: &mut clonk_network::LeagueRecordStream,
    now: i64,
    transport: &clonk_network::LeagueHttpPostTransport,
    config: &clonk_network::LeagueHttpTransportConfig,
    upload_result_tx: &tokio_mpsc::Sender<LeagueRecordUploadResult>,
) -> std::result::Result<(), clonk_network::LeagueRecordStreamError> {
    let Some(upload) = stream.pump(now)? else {
        return Ok(());
    };
    let transport = transport.clone();
    let config = config.clone();
    let upload_result_tx = upload_result_tx.clone();
    tokio::spawn(async move {
        let result = transport
            .post(&upload.endpoint, &upload.body, &config)
            .await;
        let message = LeagueRecordUploadResult {
            success: result.is_ok(),
            error: result.err().map(|error| error.to_string()),
        };
        let _ = upload_result_tx.send(message).await;
    });
    Ok(())
}

async fn shutdown_league_record_runtime(runtime: &LeagueRecordRuntimeHandle) {
    let (completion, completed) = tokio::sync::oneshot::channel();
    if runtime
        .command_tx
        .send(LeagueRecordRuntimeCommand::Shutdown { completion })
        .is_err()
    {
        return;
    }
    match tokio::time::timeout(
        clonk_network::LEAGUE_HTTP_TIMEOUT + Duration::from_secs(1),
        completed,
    )
    .await
    {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => {
            tracing::warn!(%error, "league record stream did not drain during shutdown");
        }
        Ok(Err(_)) => {
            tracing::warn!("league record runtime ended before shutdown completed");
        }
        Err(_) => {
            tracing::warn!("timed out draining the league record stream during shutdown");
        }
    }
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum LocalClientActivation {
    #[default]
    Deactivated,
    Activated,
    Observer,
}

#[derive(Debug, Default)]
struct ClientActivationState {
    armed: bool,
    status_reached: bool,
    last_request_at: Option<tokio::time::Instant>,
    local: LocalClientActivation,
}

impl ClientActivationState {
    fn arm_for_queued_player_info(&mut self, request: &clonk_network::PlayerInfoUpdateRequest) {
        if request.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL != 0
            && !request.players.is_empty()
            && self.local == LocalClientActivation::Deactivated
        {
            self.armed = true;
        }
    }

    fn arm_for_queued_control(&mut self) {
        if self.local == LocalClientActivation::Deactivated {
            self.armed = true;
        }
    }

    fn can_finalize(&self) -> bool {
        self.local == LocalClientActivation::Activated
    }

    fn status_reached(&mut self) {
        self.status_reached = true;
    }

    fn status_requested(&mut self) {
        self.status_reached = false;
    }

    fn mark_requested(&mut self, now: tokio::time::Instant) {
        self.last_request_at = Some(now);
    }

    fn apply_executed_client_update(
        &mut self,
        local_client_id: i32,
        update: &clonk_engine::ClientUpdateControlData,
    ) -> bool {
        if update.by_client != 0 || update.client_id != local_client_id {
            return false;
        }
        let became_activated = match update.update_type {
            clonk_engine::CLIENT_UPDATE_ACTIVATE if update.data != 0 => {
                let changed = self.local != LocalClientActivation::Activated;
                self.local = LocalClientActivation::Activated;
                changed
            }
            clonk_engine::CLIENT_UPDATE_ACTIVATE => {
                self.local = LocalClientActivation::Deactivated;
                return false;
            }
            clonk_engine::CLIENT_UPDATE_SET_OBSERVER => {
                self.local = LocalClientActivation::Observer;
                false
            }
            _ => return false,
        };
        self.armed = false;
        self.last_request_at = None;
        became_activated
    }

    fn request_tick_if_due(&self, now: tokio::time::Instant, current_frame: i32) -> Option<i32> {
        (self.local == LocalClientActivation::Deactivated
            && self.armed
            && self.status_reached
            && self
                .last_request_at
                .is_none_or(|last| now >= last + CLIENT_ACTIVATION_RETRY_INTERVAL))
        .then_some(current_frame)
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
    control_tick_tx: tokio_mpsc::UnboundedSender<ControlTickProbe>,
    control_performance_tx: tokio_mpsc::UnboundedSender<ControlPerformanceEvent>,
    control_tick_probe: Mutex<Option<ControlTickProbe>>,
    current_frame: Arc<AtomicI32>,
    event_rx: Receiver<NetworkEvent>,
    telemetry_rx: Receiver<NetworkEvent>,
    event_wake: NetworkEventWakeHandle,
    worker: Option<thread::JoinHandle<()>>,
    local_client_id: ClientId,
    netpuncher_state: Arc<Mutex<NetworkNetpuncherState>>,
    role: NetworkRole,
    client_status: ClientStatusState,
    league_start_response: Option<clonk_network::LeagueStartResponse>,
    league_start_failure: Option<String>,
    league_runtime_available: AtomicBool,
    league_record_runtime: Option<LeagueRecordRuntimeHandle>,
    network_io_statistics: clonk_network::NetworkIoStatistics,
    control_send_time: clonk_network::ControlSendTimeSnapshot,
    #[cfg(any(test, feature = "test-hooks"))]
    test_runtime_client_states: Arc<Mutex<Vec<RuntimeNetworkClientState>>>,
    #[cfg(any(test, feature = "test-hooks"))]
    test_lobby_client_telemetry: Arc<Mutex<Option<RuntimeLobbyClientTelemetry>>>,
}

const MASTERSERVER_SIGNUP_PENDING: u8 = 0;
const MASTERSERVER_SIGNUP_CANCELLED: u8 = 1;
const MASTERSERVER_SIGNUP_FINISHED: u8 = 2;

#[derive(Debug)]
pub struct PendingMasterserverSignup {
    enabled: bool,
    previous_enabled: bool,
    completion: Receiver<std::result::Result<Option<clonk_network::LeagueStartResponse>, String>>,
    cancellation: Option<tokio::sync::oneshot::Sender<()>>,
    transition: Arc<std::sync::atomic::AtomicU8>,
    cancel_on_drop: bool,
}

#[derive(Debug)]
pub struct PendingLeaguePlayerAuth {
    completion: Receiver<std::result::Result<clonk_network::LeagueAuthResponse, String>>,
}

impl PendingLeaguePlayerAuth {
    pub fn try_complete(&self) -> Option<Result<clonk_network::LeagueAuthResponse>> {
        match self.completion.try_recv() {
            Ok(result) => Some(result.map_err(|message| anyhow!(message))),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(anyhow!(
                "league runtime ended before authenticating the player"
            ))),
        }
    }

    fn wait(self) -> Result<clonk_network::LeagueAuthResponse> {
        self.completion
            .recv_timeout(clonk_network::LEAGUE_HTTP_TIMEOUT + Duration::from_secs(1))
            .map_err(|_| anyhow!("league runtime did not finish authenticating the player"))?
            .map_err(|message| anyhow!(message))
    }
}

impl PendingMasterserverSignup {
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn previous_enabled(&self) -> bool {
        self.previous_enabled
    }

    /// Keep an already-committed registration cleanup alive while the owning
    /// lobby and network manager are torn down.
    ///
    /// C++ sends `End` before `DeinitLeague`. A pending disable which follows
    /// a successful `Start` has the same obligation: dropping the UI-side
    /// handle must let the worker finish `End`, after which the manager's
    /// queued shutdown can deinitialise the league runtime.
    pub fn finish_committed_cleanup_on_worker_shutdown(&mut self) -> bool {
        if self.enabled || !self.previous_enabled {
            return false;
        }
        self.cancel_on_drop = false;
        true
    }

    /// Returns false when the worker already committed a terminal result. In
    /// that race the caller must consume completion instead of painting an
    /// off state over a live registration.
    pub fn cancel(&mut self) -> bool {
        if self
            .transition
            .compare_exchange(
                MASTERSERVER_SIGNUP_PENDING,
                MASTERSERVER_SIGNUP_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_err()
        {
            return false;
        }
        if let Some(cancellation) = self.cancellation.take() {
            let _ = cancellation.send(());
        }
        true
    }
}

impl Drop for PendingMasterserverSignup {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            let _ = self.cancel();
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ControlTickProbe {
    tick: Tick,
    control_rate: i32,
    target_fps: i32,
    reached_at: tokio::time::Instant,
    queued: bool,
}

#[derive(Debug)]
enum ControlPerformanceEvent {
    TickConsumed {
        tick: Tick,
        consumed_at: tokio::time::Instant,
        client_ids: Vec<ClientId>,
    },
    Reset,
}

#[derive(Debug, Clone, Default)]
struct NetworkNetpuncherState {
    local_addresses: Vec<NetworkAddress>,
    game_ids: clonk_network::NetpuncherGameIds,
}

#[cfg(any(test, feature = "test-hooks"))]
pub struct TestNetworkCommands {
    command_rx: tokio_mpsc::Receiver<NetworkCommand>,
    // The app's test-hooks feature constructs this probe without consuming
    // performance events; clonk-app-netplay's own regression does consume it.
    #[allow(dead_code)]
    control_performance_rx: tokio_mpsc::UnboundedReceiver<ControlPerformanceEvent>,
}

#[cfg(any(test, feature = "test-hooks"))]
pub struct TestMasterserverSignupCommand {
    pub enabled: bool,
    pub config: PreparedLeagueHostConfig,
    pub reference: clonk_network::HostGameReference,
    completion: Sender<std::result::Result<Option<clonk_network::LeagueStartResponse>, String>>,
    cancellation: tokio::sync::oneshot::Receiver<()>,
    transition: Arc<std::sync::atomic::AtomicU8>,
}

#[cfg(any(test, feature = "test-hooks"))]
pub struct TestLeaguePlayerAuthCommand {
    pub auth: clonk_network::LeagueAuthRequestHead,
    pub player: clonk_engine::ControlPlayerInfoEntry,
    completion: Sender<std::result::Result<clonk_network::LeagueAuthResponse, String>>,
}

#[cfg(any(test, feature = "test-hooks"))]
impl TestLeaguePlayerAuthCommand {
    pub fn complete(
        self,
        result: std::result::Result<clonk_network::LeagueAuthResponse, String>,
    ) -> bool {
        self.completion.send(result).is_ok()
    }
}

#[cfg(any(test, feature = "test-hooks"))]
impl TestMasterserverSignupCommand {
    pub fn complete(
        self,
        result: std::result::Result<Option<clonk_network::LeagueStartResponse>, String>,
    ) {
        self.transition
            .compare_exchange(
                MASTERSERVER_SIGNUP_PENDING,
                MASTERSERVER_SIGNUP_FINISHED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .expect("test signup completion must win the pending transaction");
        self.completion.send(result).expect("return signup result");
    }

    pub fn wait_for_cancellation(self) {
        self.cancellation
            .blocking_recv()
            .expect("pending signup must signal cancellation");
        assert_eq!(
            self.transition.load(Ordering::Acquire),
            MASTERSERVER_SIGNUP_CANCELLED
        );
    }

    pub fn wait_for_cleanup_preservation(self) {
        assert!(
            self.cancellation.blocking_recv().is_err(),
            "committed cleanup must release its UI handle without signalling cancellation"
        );
        assert_eq!(
            self.transition.load(Ordering::Acquire),
            MASTERSERVER_SIGNUP_PENDING,
            "the worker retains ownership of the committed cleanup transition"
        );
    }
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestLobbyStartCommand {
    Countdown(clonk_network::LobbyCountdownPacket),
    BeginGo {
        status: NetworkStatus,
        join_allowed: bool,
    },
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestLobbyPlayerUpdateCommand {
    Countdown(clonk_network::LobbyCountdownPacket),
    PlayerInfo(clonk_network::PlayerInfoUpdateRequest),
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestRuntimeStatusCommand {
    Change(NetworkStatus),
    Reached {
        status: NetworkStatus,
        actual_control_tick: i32,
    },
}

#[cfg(any(test, feature = "test-hooks"))]
#[derive(Debug, Default)]
pub struct TestLeagueEndFlowResult {
    pub attempts: usize,
    pub finalizations: Vec<Vec<u8>>,
    pub broadcasts: Vec<clonk_network::LeagueRoundResultsPacket>,
}

#[cfg(any(test, feature = "test-hooks"))]
type RuntimeHostJoinResult = (
    Vec<&'static str>,
    Vec<ClientPlayerResourceRequest>,
    Vec<PlayerInfoControlData>,
    Vec<(Tick, JoinPlayerControlData)>,
);

#[cfg(any(test, feature = "test-hooks"))]
type SubmittedPlayerInputs = (
    Vec<(i32, ControlEvent, Tick)>,
    Vec<(Tick, PlayerCommandControlData)>,
    Vec<(Tick, PlayerSelectControlData)>,
);

#[cfg(any(test, feature = "test-hooks"))]
impl TestNetworkCommands {
    pub fn receive_league_player_auth(&mut self) -> TestLeaguePlayerAuthCommand {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::LeagueAuthenticatePlayer {
                auth,
                player,
                completion,
            }) => TestLeaguePlayerAuthCommand {
                auth,
                player,
                completion,
            },
            Some(command) => panic!("unexpected league auth command: {command:?}"),
            None => panic!("network command channel ended before league auth command"),
        }
    }

    pub fn complete_league_end_flow(
        mut self,
        outcomes: Vec<LeagueEndAttempt>,
    ) -> TestLeagueEndFlowResult {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut observed = TestLeagueEndFlowResult::default();
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::LeagueEnd { completion, .. }) => {
                    let outcome = outcomes
                        .get(observed.attempts)
                        .cloned()
                        .expect("test did not provide a league End outcome");
                    observed.attempts += 1;
                    completion.send(Ok(outcome)).expect("complete league End");
                }
                Ok(NetworkCommand::LeagueFinalizeEndFailure { packet, completion }) => {
                    observed
                        .finalizations
                        .push(packet.result_string.as_bytes().to_vec());
                    completion
                        .send(Ok(Some(packet)))
                        .expect("complete league End failure finalization");
                }
                Ok(NetworkCommand::BroadcastLeagueRoundResults(packet)) => {
                    observed.broadcasts.push(packet);
                    break;
                }
                Ok(NetworkCommand::SubmitLocal { .. }) => {}
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected league End command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        observed
    }

    pub fn take_finalized_ticks(&mut self) -> Vec<Tick> {
        let mut ticks = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::FinalizeTick { tick } = command {
                ticks.push(tick);
            }
        }
        ticks
    }

    pub fn take_host_restart_broadcasts(&mut self) -> Vec<u16> {
        let mut observed = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::BroadcastHostRestarting { rejoin_seconds } = command {
                observed.push(rejoin_seconds);
            }
        }
        observed
    }

    pub fn take_lobby_start_commands(&mut self) -> Vec<TestLobbyStartCommand> {
        let mut observed = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitLobbyCountdown(packet) => {
                    observed.push(TestLobbyStartCommand::Countdown(packet));
                }
                command => panic!("unexpected lobby-start command: {command:?}"),
            }
        }
        observed
    }

    pub fn complete_lobby_start(
        &mut self,
        result: std::result::Result<(), String>,
    ) -> Vec<TestLobbyStartCommand> {
        let mut observed = Vec::new();
        loop {
            match self.command_rx.blocking_recv() {
                Some(NetworkCommand::SubmitLobbyCountdown(packet)) => {
                    observed.push(TestLobbyStartCommand::Countdown(packet));
                }
                Some(NetworkCommand::BeginGo {
                    status,
                    join_allowed,
                    completion,
                }) => {
                    observed.push(TestLobbyStartCommand::BeginGo {
                        status,
                        join_allowed,
                    });
                    completion
                        .send(result)
                        .expect("return atomic Go transition result");
                    return observed;
                }
                Some(command) => panic!("unexpected lobby-start command: {command:?}"),
                None => panic!("network command channel ended before atomic Go command"),
            }
        }
    }

    pub fn take_lobby_player_update_commands(&mut self) -> Vec<TestLobbyPlayerUpdateCommand> {
        let mut observed = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitLobbyCountdown(packet) => {
                    observed.push(TestLobbyPlayerUpdateCommand::Countdown(packet));
                }
                NetworkCommand::SubmitPlayerInfoUpdate(request) => {
                    observed.push(TestLobbyPlayerUpdateCommand::PlayerInfo(request));
                }
                command => panic!("unexpected lobby-player update command: {command:?}"),
            }
        }
        observed
    }

    pub fn complete_runtime_host_join(
        mut self,
        published_core: clonk_engine::NetworkResourceCore,
        event_tx: NetworkEventSender,
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
                Ok(NetworkCommand::BroadcastPreexecutedPlayerInfo {
                    info,
                    join_players_on_echo,
                }) => {
                    order.push("player-info");
                    player_infos.push(info.clone());
                    let _ = event_tx.send(NetworkEvent::PreexecutedPlayerInfoEcho {
                        original: info.clone(),
                        info,
                        join_players_on_echo,
                    });
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

    pub fn complete_initial_client_join(
        mut self,
        published_cores: Vec<clonk_engine::NetworkResourceCore>,
    ) -> (
        Vec<&'static str>,
        Vec<ClientPlayerResourceRequest>,
        Vec<clonk_network::PlayerInfoUpdateRequest>,
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

    pub fn complete_initial_league_client_join(
        mut self,
        published_cores: Vec<clonk_engine::NetworkResourceCore>,
        auth_responses: Vec<clonk_network::LeagueAuthResponse>,
    ) -> (
        Vec<&'static str>,
        Vec<clonk_network::LeagueAuthRequestHead>,
        Vec<clonk_engine::ControlPlayerInfoEntry>,
        Vec<clonk_network::PlayerInfoUpdateRequest>,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut order = Vec::new();
        let mut publications = 0;
        let mut auth_heads = Vec::new();
        let mut auth_players = Vec::new();
        let mut player_infos = Vec::new();
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::PublishPlayerResource { completion, .. }) => {
                    order.push("publish");
                    let result = published_cores
                        .get(publications)
                        .cloned()
                        .ok_or_else(|| "test did not provide a publication core".to_string());
                    publications += 1;
                    let _ = completion.send(result);
                }
                Ok(NetworkCommand::LeagueAuthenticatePlayer {
                    auth,
                    player,
                    completion,
                }) => {
                    order.push("auth");
                    let response = auth_responses
                        .get(auth_players.len())
                        .cloned()
                        .ok_or_else(|| "test did not provide an Auth response".to_string());
                    auth_heads.push(auth);
                    auth_players.push(player);
                    let _ = completion.send(response);
                }
                Ok(NetworkCommand::SubmitPlayerInfoUpdate(request)) => {
                    order.push("player-info");
                    player_infos.push(request);
                    break;
                }
                // Opening the native modal cancels any active synchronized
                // player menu before it submits league credentials.
                Ok(NetworkCommand::SubmitLocal {
                    event: ControlEvent::ClearPressed,
                    ..
                }) => {}
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected league-client command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        (order, auth_heads, auth_players, player_infos)
    }

    pub fn complete_league_player_auths(
        mut self,
        auth_responses: Vec<clonk_network::LeagueAuthResponse>,
    ) -> (
        Vec<clonk_network::LeagueAuthRequestHead>,
        Vec<clonk_engine::ControlPlayerInfoEntry>,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut auth_heads = Vec::new();
        let mut auth_players = Vec::new();
        while std::time::Instant::now() < deadline && auth_players.len() < auth_responses.len() {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::LeagueAuthenticatePlayer {
                    auth,
                    player,
                    completion,
                }) => {
                    let response = auth_responses
                        .get(auth_players.len())
                        .cloned()
                        .ok_or_else(|| "test did not provide an Auth response".to_string());
                    auth_heads.push(auth);
                    auth_players.push(player);
                    let _ = completion.send(response);
                }
                Ok(NetworkCommand::SubmitLocal {
                    event: ControlEvent::ClearPressed,
                    ..
                }) => {}
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected league-auth command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        (auth_heads, auth_players)
    }

    pub fn complete_host_league_player_checks(
        mut self,
        responses: Vec<clonk_network::LeagueJoinResponse>,
        expected_broadcasts: usize,
    ) -> (
        Vec<clonk_engine::ControlPlayerInfoEntry>,
        Vec<PlayerInfoControlData>,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let mut checked = Vec::new();
        let mut broadcasts = Vec::new();
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::LeagueCheckPlayer { player, completion }) => {
                    let response = responses
                        .get(checked.len())
                        .cloned()
                        .ok_or_else(|| "test did not provide a Join response".to_string());
                    checked.push(player);
                    let _ = completion.send(response);
                }
                Ok(NetworkCommand::BroadcastPlayerInfo(info))
                | Ok(NetworkCommand::BroadcastPreexecutedPlayerInfo { info, .. }) => {
                    broadcasts.push(info);
                }
                Ok(NetworkCommand::PublishJoinSnapshot { .. }) => {}
                Ok(NetworkCommand::Shutdown) => break,
                Ok(command) => panic!("unexpected league-host command: {command:?}"),
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    if checked.len() == responses.len() && broadcasts.len() >= expected_broadcasts {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        (checked, broadcasts)
    }

    pub fn take_submitted_local(&mut self) -> Vec<(i32, ControlEvent, Tick)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitLocal { owner, event, tick } = command {
                submitted.push((owner, event, tick));
            }
        }
        submitted
    }

    pub fn take_submitted_player_inputs(&mut self) -> SubmittedPlayerInputs {
        let mut controls = Vec::new();
        let mut commands = Vec::new();
        let mut selections = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitLocal { owner, event, tick } => {
                    controls.push((owner, event, tick));
                }
                NetworkCommand::SubmitPlayerCommand { tick, command } => {
                    commands.push((tick, command));
                }
                NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                    selections.push((tick, selection));
                }
                command => panic!("unexpected player-input command: {command:?}"),
            }
        }
        (controls, commands, selections)
    }

    pub fn take_submitted_player_commands(&mut self) -> Vec<(Tick, PlayerCommandControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerCommand { tick, command } = command {
                submitted.push((tick, command));
            }
        }
        submitted
    }

    pub fn take_submitted_player_selects(&mut self) -> Vec<(Tick, PlayerSelectControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerSelect { tick, selection } = command {
                submitted.push((tick, selection));
            }
        }
        submitted
    }

    pub fn take_submitted_mouse_controls(&mut self) -> SubmittedPlayerInputs {
        let mut local = Vec::new();
        let mut commands = Vec::new();
        let mut selections = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitLocal { owner, event, tick } => {
                    local.push((owner, event, tick));
                }
                NetworkCommand::SubmitPlayerCommand { tick, command } => {
                    commands.push((tick, command));
                }
                NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                    selections.push((tick, selection));
                }
                _ => {}
            }
        }
        (local, commands, selections)
    }

    pub fn take_submitted_scripts(&mut self) -> Vec<(Tick, ScriptControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitScript { tick, script } = command {
                submitted.push((tick, script));
            }
        }
        submitted
    }

    pub fn take_submitted_message_board_answers(
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

    pub fn take_submitted_messages(&mut self) -> Vec<MessageControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitMessage(message) = command {
                submitted.push(message);
            }
        }
        submitted
    }

    pub fn take_submitted_decided_controls_and_messages(
        &mut self,
    ) -> (
        Vec<(Tick, clonk_engine::ControlPacket, bool)>,
        Vec<MessageControlData>,
    ) {
        let mut controls = Vec::new();
        let mut messages = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::SubmitDecidedControl {
                    tick,
                    control,
                    sync,
                } => controls.push((tick, control, sync)),
                NetworkCommand::SubmitMessage(message) => messages.push(message),
                command => panic!("unexpected console-input command: {command:?}"),
            }
        }
        (controls, messages)
    }

    pub fn take_player_info_updates(&mut self) -> Vec<clonk_network::PlayerInfoUpdateRequest> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitPlayerInfoUpdate(request) = command {
                submitted.push(request);
            }
        }
        submitted
    }

    pub fn take_broadcast_player_infos(&mut self) -> Vec<PlayerInfoControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::BroadcastPlayerInfo(info)
                | NetworkCommand::BroadcastPreexecutedPlayerInfo { info, .. } => {
                    submitted.push(info)
                }
                _ => {}
            }
        }
        submitted
    }

    pub fn take_preexecuted_player_infos(
        &mut self,
    ) -> Vec<(
        PlayerInfoControlData,
        Vec<clonk_engine::ControlPlayerInfoEntry>,
    )> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::BroadcastPreexecutedPlayerInfo {
                info,
                join_players_on_echo,
            } = command
            {
                submitted.push((info, join_players_on_echo));
            }
        }
        submitted
    }

    pub fn take_league_update_effects(&mut self) -> (Vec<PlayerInfoControlData>, usize) {
        let mut player_infos = Vec::new();
        let mut invalidations = 0;
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::BroadcastPlayerInfo(info) => player_infos.push(info),
                NetworkCommand::LeagueInvalidate => invalidations += 1,
                _ => {}
            }
        }
        (player_infos, invalidations)
    }

    pub fn complete_league_disconnect_report(
        mut self,
    ) -> Option<(
        clonk_network::LeagueDisconnectReason,
        clonk_network::ClientPlayerInfosSnapshot,
    )> {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        while std::time::Instant::now() < deadline {
            match self.command_rx.try_recv() {
                Ok(NetworkCommand::LeagueReportDisconnect {
                    reason,
                    players,
                    completion,
                    ..
                }) => {
                    let _ = completion.send(Ok(()));
                    return Some((reason, players));
                }
                Ok(NetworkCommand::Shutdown) => return None,
                Ok(_) => {}
                Err(tokio_mpsc::error::TryRecvError::Empty) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(tokio_mpsc::error::TryRecvError::Disconnected) => return None,
            }
        }
        None
    }

    pub fn take_published_join_snapshots(&mut self) -> Vec<HostJoinSnapshot> {
        let mut snapshots = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::PublishJoinSnapshot { snapshot } = command {
                snapshots.push(snapshot);
            }
        }
        snapshots
    }

    pub fn take_team_control_updates(
        &mut self,
    ) -> (Vec<PlayerInfoControlData>, Vec<HostJoinSnapshot>) {
        let mut player_infos = Vec::new();
        let mut snapshots = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::BroadcastPlayerInfo(info)
                | NetworkCommand::BroadcastPreexecutedPlayerInfo { info, .. } => {
                    player_infos.push(info)
                }
                NetworkCommand::PublishJoinSnapshot { snapshot } => snapshots.push(snapshot),
                _ => {}
            }
        }
        (player_infos, snapshots)
    }

    pub fn take_submitted_join_players(&mut self) -> Vec<(Tick, JoinPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitJoinPlayer { tick, join } = command {
                submitted.push((tick, join));
            }
        }
        submitted
    }

    pub fn take_submitted_remove_players(
        &mut self,
    ) -> Vec<(Tick, clonk_engine::RemovePlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitRemovePlayer { tick, remove } = command {
                submitted.push((tick, remove));
            }
        }
        submitted
    }

    pub fn take_submitted_client_updates(&mut self) -> Vec<clonk_engine::ClientUpdateControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitClientUpdate(update) = command {
                submitted.push(update);
            }
        }
        submitted
    }

    pub fn take_submitted_control_sets(&mut self) -> Vec<LegacyControlSet> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitControlSet(set) = command {
                submitted.push(set);
            }
        }
        submitted
    }

    pub fn take_submitted_decided_controls(
        &mut self,
    ) -> Vec<(Tick, clonk_engine::ControlPacket, bool)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitDecidedControl {
                tick,
                control,
                sync,
            } = command
            {
                submitted.push((tick, control, sync));
            }
        }
        submitted
    }

    pub fn take_submitted_client_removes(&mut self) -> Vec<clonk_engine::ClientRemoveControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitClientRemove(remove) = command {
                submitted.push(remove);
            }
        }
        submitted
    }

    pub fn take_submitted_init_scenario_players(
        &mut self,
    ) -> Vec<(Tick, clonk_engine::InitScenarioPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitInitScenarioPlayer { tick, selection } = command {
                submitted.push((tick, selection));
            }
        }
        submitted
    }

    pub fn take_submitted_surrender_players(
        &mut self,
    ) -> Vec<(Tick, clonk_engine::SurrenderPlayerControlData)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitSurrenderPlayer { tick, surrender } = command {
                submitted.push((tick, surrender));
            }
        }
        submitted
    }

    pub fn take_submitted_internal_player_scripts(
        &mut self,
    ) -> Vec<(Tick, clonk_engine::ControlPacket)> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitInternalPlayerScript { tick, control } = command {
                submitted.push((tick, control));
            }
        }
        submitted
    }

    pub fn take_submitted_votes(&mut self) -> Vec<clonk_engine::VoteControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitVote(vote) = command {
                submitted.push(vote);
            }
        }
        submitted
    }

    pub fn take_submitted_vote_ends(&mut self) -> Vec<clonk_engine::VoteControlData> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitVoteEnd(result) = command {
                submitted.push(result);
            }
        }
        submitted
    }

    pub fn take_submitted_ready_checks(&mut self) -> Vec<clonk_network::ReadyCheckPacket> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitReadyCheck(packet) = command {
                submitted.push(packet);
            }
        }
        submitted
    }

    pub fn take_submitted_lobby_countdowns(&mut self) -> Vec<clonk_network::LobbyCountdownPacket> {
        let mut submitted = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::SubmitLobbyCountdown(packet) = command {
                submitted.push(packet);
            }
        }
        submitted
    }

    pub fn receive_join_allowed(&mut self) -> (bool, Sender<std::result::Result<(), String>>) {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::SetJoinAllowed {
                allowed,
                completion,
            }) => (allowed, completion),
            Some(command) => panic!("expected join-admission command, got {command:?}"),
            None => panic!("network command channel ended before join-admission command"),
        }
    }

    pub fn receive_host_password(
        &mut self,
    ) -> (
        clonk_engine::LegacyCString,
        Sender<std::result::Result<(), String>>,
    ) {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::SetHostPassword {
                password,
                completion,
            }) => (password, completion),
            Some(command) => panic!("expected host-password command, got {command:?}"),
            None => panic!("network command channel ended before host-password command"),
        }
    }

    pub fn receive_masterserver_signup(&mut self) -> TestMasterserverSignupCommand {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::SetMasterserverSignup {
                enabled,
                config,
                reference,
                completion,
                cancellation,
                transition,
            }) => TestMasterserverSignupCommand {
                enabled,
                config,
                reference,
                completion,
                cancellation,
                transition,
            },
            Some(command) => panic!("expected masterserver-signup command, got {command:?}"),
            None => panic!("network command channel ended before masterserver-signup command"),
        }
    }

    pub fn receive_resource_removal(&mut self) -> (i32, Sender<std::result::Result<(), String>>) {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::RemoveResource {
                resource_id,
                completion,
            }) => (resource_id, completion),
            Some(command) => panic!("expected resource-removal command, got {command:?}"),
            None => panic!("network command channel ended before resource-removal command"),
        }
    }

    pub fn receive_graceful_part(&mut self) -> Sender<std::result::Result<(), String>> {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::GracefulPart { completion }) => completion,
            Some(command) => panic!("expected graceful-part command, got {command:?}"),
            None => panic!("network command channel ended before graceful-part command"),
        }
    }

    pub fn complete_graceful_part(mut self) -> bool {
        match self.command_rx.blocking_recv() {
            Some(NetworkCommand::GracefulPart { completion }) => {
                let _ = completion.send(Ok(()));
                true
            }
            Some(NetworkCommand::Shutdown) | None => false,
            Some(command) => panic!("unexpected teardown command: {command:?}"),
        }
    }

    pub fn take_status_changes(&mut self) -> Vec<NetworkStatus> {
        let mut changes = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            if let NetworkCommand::ChangeStatus(status) = command {
                changes.push(status);
            }
        }
        changes
    }

    pub fn take_status_reached(&mut self) -> usize {
        let mut count = 0;
        while let Ok(command) = self.command_rx.try_recv() {
            if matches!(
                command,
                NetworkCommand::StatusReachedCurrent | NetworkCommand::StatusReached { .. }
            ) {
                count += 1;
            }
        }
        count
    }

    pub fn take_runtime_status_commands(&mut self) -> Vec<TestRuntimeStatusCommand> {
        let mut observed = Vec::new();
        while let Ok(command) = self.command_rx.try_recv() {
            match command {
                NetworkCommand::ChangeStatus(status) => {
                    observed.push(TestRuntimeStatusCommand::Change(status));
                }
                NetworkCommand::StatusReached {
                    status,
                    actual_control_tick,
                } => {
                    observed.push(TestRuntimeStatusCommand::Reached {
                        status,
                        actual_control_tick,
                    });
                }
                _ => {}
            }
        }
        observed
    }

    pub fn take_status_acknowledgements(&mut self) -> Vec<NetworkStatus> {
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

    pub fn take_framed_status_acknowledgements(&mut self) -> Vec<(NetworkStatus, i32)> {
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

    pub fn take_executed_client_updates(&mut self) -> Vec<clonk_engine::ClientUpdateControlData> {
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
// JoinData is a one-shot event moved through an unbounded channel. Keeping the
// envelope inline avoids a separate allocation on the compatibility boundary.
#[allow(clippy::large_enum_variant)]
pub enum NetworkEvent {
    HostPingMeasured {
        round_trip_ms: i32,
    },
    HostStatusChanged(NetworkStatus),
    HostStatusAck {
        client_id: ClientId,
        status: NetworkStatus,
    },
    JoinData(clonk_network::JoinDataEnvelope),
    LeagueRoundResults(clonk_network::LeagueRoundResultsPacket),
    LeagueUpdate(clonk_network::LeagueUpdateResponse),
    LobbyCountdown(clonk_network::LobbyCountdownPacket),
    ReadyCheck(clonk_network::ReadyCheckPacket),
    StatusRequested(NetworkStatus),
    StatusCommitted(NetworkStatus),
    ActivationRequest {
        client_id: ClientId,
        tick: i32,
        waited_for: bool,
        ping_ms: i32,
    },
    JoinDataNeeded {
        client_id: ClientId,
        current_control_tick: Tick,
    },
    PlayerInfoUpdateRequest {
        origin: ClientId,
        request: clonk_network::PlayerInfoUpdateRequest,
        by_host: bool,
    },
    PreexecutedPlayerInfoEcho {
        original: PlayerInfoControlData,
        info: PlayerInfoControlData,
        join_players_on_echo: Vec<clonk_engine::ControlPlayerInfoEntry>,
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
    PeerConnectionFailed {
        client_id: ClientId,
    },
    /// The host is closing this session to restart the round and expects to be
    /// reachable again at the same address. Always precedes the matching
    /// `PeerDisconnected`. See [`clonk_network::host_restart`].
    HostRestarting {
        rejoin_seconds: u16,
    },
    NetpuncherStateChanged {
        game_ids: clonk_network::NetpuncherGameIds,
        local_addresses: Vec<NetworkAddress>,
    },
    ResourceAction(clonk_network::ResourceCatalogAction),
    ResourceProgress {
        resource_id: i32,
        present_percent: u8,
    },
    ResourceComplete {
        resource_id: i32,
        core: clonk_engine::NetworkResourceCore,
        path: PathBuf,
        local: bool,
    },
    ResourceLoadFailed {
        resource_id: i32,
    },
    ResourceDeriveUnsupported {
        core: clonk_engine::NetworkResourceCore,
    },
    /// A connection attempt or secondary route failed while the authoritative
    /// logical session remained live.
    RecoverableRouteDiagnostic {
        client_id: Option<ClientId>,
        error: String,
    },
    /// A peer transport/protocol error that does not stop the network worker.
    TransportDiagnostic {
        client_id: Option<ClientId>,
        error: String,
    },
    /// An application-level network diagnostic while the worker remains live.
    Error(String),
    /// The network worker exited and can no longer service the session.
    FatalError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkControl {
    ClientJoin(clonk_engine::ClientJoinControlData),
    ClientUpdate(clonk_engine::ClientUpdateControlData),
    ClientRemove(clonk_engine::ClientRemoveControlData),
    PlayerInfo(PlayerInfoControlData),
    JoinPlayer(JoinPlayerControlData),
    RemovePlayer(clonk_engine::RemovePlayerControlData),
    SurrenderPlayer(clonk_engine::SurrenderPlayerControlData),
    ActivateGameGoalMenu(clonk_engine::ActivateGameGoalMenuControlData),
    ToggleHostility(clonk_engine::ToggleHostilityControlData),
    ActivateGameGoalRule(clonk_engine::ActivateGameGoalRuleControlData),
    SetPlayerTeam(clonk_engine::SetPlayerTeamControlData),
    EliminatePlayer(clonk_engine::EliminatePlayerControlData),
    Vote(clonk_engine::VoteControlData),
    VoteEnd(clonk_engine::VoteControlData),
    PlayerControl(PlayerControlData),
    PlayerCommand(PlayerCommandControlData),
    PlayerSelect(PlayerSelectControlData),
    Script(ScriptControlData),
    Message(MessageControlData),
    MessageBoardAnswer(MessageBoardAnswerControlData),
    CustomCommand(clonk_engine::CustomCommandControlData),
    EmMoveObject(clonk_engine::EmMoveObjectControlData),
    EmDrawTool(clonk_engine::EmDrawToolControlData),
    EmDropDef(clonk_engine::EmDropDefControlData),
    Player { owner: i32, event: ControlEvent },
    InitScenarioPlayer(clonk_engine::InitScenarioPlayerControlData),
    Synchronize(clonk_engine::SynchronizeControlData),
    SyncCheck(SyncCheckPacket),
    Set(LegacyControlSet),
    DebugRecord(clonk_engine::DebugRecordControlData),
}

impl NetworkControl {
    /// Recover the exact legacy control packet represented by this app-facing
    /// value. CtrlRec stores the packet/list compiler payload rather than the
    /// reduced execution enum, so recording must make this conversion before
    /// executing a batch.
    pub fn into_packet(self) -> Option<clonk_engine::ControlPacket> {
        Some(match self {
            Self::ClientJoin(value) => clonk_engine::ControlPacket::ClientJoin(value),
            Self::ClientUpdate(value) => clonk_engine::ControlPacket::ClientUpdate(value),
            Self::ClientRemove(value) => clonk_engine::ControlPacket::ClientRemove(value),
            Self::PlayerInfo(value) => clonk_engine::ControlPacket::PlayerInfo(value),
            Self::JoinPlayer(value) => clonk_engine::ControlPacket::JoinPlayer(value),
            Self::RemovePlayer(value) => clonk_engine::ControlPacket::RemovePlayer(value),
            Self::SurrenderPlayer(value) => clonk_engine::ControlPacket::SurrenderPlayer(value),
            Self::ActivateGameGoalMenu(value) => {
                clonk_engine::ControlPacket::ActivateGameGoalMenu(value)
            }
            Self::ToggleHostility(value) => clonk_engine::ControlPacket::ToggleHostility(value),
            Self::ActivateGameGoalRule(value) => {
                clonk_engine::ControlPacket::ActivateGameGoalRule(value)
            }
            Self::SetPlayerTeam(value) => clonk_engine::ControlPacket::SetPlayerTeam(value),
            Self::EliminatePlayer(value) => clonk_engine::ControlPacket::EliminatePlayer(value),
            Self::Vote(value) => clonk_engine::ControlPacket::Vote(value),
            Self::VoteEnd(value) => clonk_engine::ControlPacket::VoteEnd(value),
            Self::PlayerControl(value) => clonk_engine::ControlPacket::PlayerControl(value),
            Self::PlayerCommand(value) => clonk_engine::ControlPacket::PlayerCommand(value),
            Self::PlayerSelect(value) => clonk_engine::ControlPacket::PlayerSelect(value),
            Self::Script(value) => clonk_engine::ControlPacket::Script(value),
            Self::Message(value) => clonk_engine::ControlPacket::Message(value),
            Self::MessageBoardAnswer(value) => {
                clonk_engine::ControlPacket::MessageBoardAnswer(value)
            }
            Self::CustomCommand(value) => clonk_engine::ControlPacket::CustomCommand(value),
            Self::EmMoveObject(value) => clonk_engine::ControlPacket::EmMoveObject(value),
            Self::EmDrawTool(value) => clonk_engine::ControlPacket::EmDrawTool(value),
            Self::EmDropDef(value) => clonk_engine::ControlPacket::EmDropDef(value),
            Self::Player { owner, event } => {
                return control_packet_for_event(owner, event, HOST_CLIENT_ID);
            }
            Self::InitScenarioPlayer(value) => {
                clonk_engine::ControlPacket::InitScenarioPlayer(value)
            }
            Self::Synchronize(value) => clonk_engine::ControlPacket::Synchronize(value),
            Self::SyncCheck(value) => clonk_engine::ControlPacket::SyncCheck(value),
            Self::Set(value) => value.into_control_packet(),
            Self::DebugRecord(value) => clonk_engine::ControlPacket::DebugRecord(value),
        })
    }
}

#[derive(Debug)]
enum PlayerInfoEchoProvenance {
    Normal,
    Preexecuted {
        original: PlayerInfoControlData,
        join_players_on_echo: Vec<clonk_engine::ControlPlayerInfoEntry>,
    },
}

#[derive(Debug)]
enum NetworkCommand {
    InspectRuntimeConnections {
        completion: Sender<std::result::Result<Vec<RuntimeNetworkConnection>, String>>,
    },
    InspectLobbyClientTelemetry {
        client_ids: Vec<ClientId>,
        completion: Sender<std::result::Result<RuntimeLobbyClientTelemetry, String>>,
    },
    InspectRuntimeClientStates {
        tick: Tick,
        probe: Option<ControlTickProbe>,
        completion: Sender<std::result::Result<Vec<RuntimeNetworkClientState>, String>>,
    },
    DisconnectRuntimeConnection {
        connection_id: u32,
        completion: Sender<std::result::Result<(), String>>,
    },
    PublishRuntimeDynamic {
        dynamic: Box<clonk_network::LiveNetworkDynamic>,
        dynamic_tick: i32,
        parameters: Box<clonk_network::JoinGameParametersEnvelope>,
        completion: Sender<std::result::Result<clonk_engine::NetworkResourceCore, String>>,
    },
    RemoveRuntimeDynamic {
        completion: Sender<std::result::Result<bool, String>>,
    },
    FailPendingJoinData {
        reason: clonk_engine::LegacyCString,
        completion: Sender<std::result::Result<usize, String>>,
    },
    PublishPlayerResource {
        request: ClientPlayerResourceRequest,
        completion: Sender<std::result::Result<clonk_engine::NetworkResourceCore, String>>,
    },
    BeginResourceDerive {
        resource_id: i32,
        source_path: PathBuf,
        ownership: clonk_network::ResourceFileOwnership,
        completion: Sender<std::result::Result<clonk_network::ResourceDerivation, String>>,
    },
    FinishResourceDerive {
        derivation: clonk_network::ResourceDerivation,
        completion: Sender<std::result::Result<clonk_engine::NetworkResourceCore, String>>,
    },
    RemoveResource {
        resource_id: i32,
        completion: Sender<std::result::Result<(), String>>,
    },
    SubmitPlayerInfoUpdate(clonk_network::PlayerInfoUpdateRequest),
    BroadcastPlayerInfo(PlayerInfoControlData),
    BroadcastPreexecutedPlayerInfo {
        info: PlayerInfoControlData,
        join_players_on_echo: Vec<clonk_engine::ControlPlayerInfoEntry>,
    },
    BroadcastLeagueRoundResults(clonk_network::LeagueRoundResultsPacket),
    LeagueAuthenticatePlayer {
        auth: clonk_network::LeagueAuthRequestHead,
        player: clonk_engine::ControlPlayerInfoEntry,
        completion: Sender<std::result::Result<clonk_network::LeagueAuthResponse, String>>,
    },
    LeagueCheckPlayer {
        player: clonk_engine::ControlPlayerInfoEntry,
        completion: Sender<std::result::Result<clonk_network::LeagueJoinResponse, String>>,
    },
    LeagueUpdate {
        now: i64,
        reference: clonk_network::HostGameReference,
    },
    LeagueEnd {
        reference: clonk_network::HostGameReference,
        record: Option<clonk_network::LeagueEndRecord>,
        completion: Sender<std::result::Result<LeagueEndAttempt, String>>,
    },
    LeagueFinalizeEndFailure {
        packet: clonk_network::LeagueRoundResultsPacket,
        completion:
            Sender<std::result::Result<Option<clonk_network::LeagueRoundResultsPacket>, String>>,
    },
    LeagueReportDisconnect {
        reason: clonk_network::LeagueDisconnectReason,
        players: clonk_network::ClientPlayerInfosSnapshot,
        fbids: clonk_network::LeagueFbidRegistry,
        completion: Sender<std::result::Result<(), String>>,
    },
    LeagueInvalidate,
    SetMasterserverSignup {
        enabled: bool,
        config: PreparedLeagueHostConfig,
        reference: clonk_network::HostGameReference,
        completion: Sender<std::result::Result<Option<clonk_network::LeagueStartResponse>, String>>,
        cancellation: tokio::sync::oneshot::Receiver<()>,
        transition: Arc<std::sync::atomic::AtomicU8>,
    },
    SubmitJoinPlayer {
        tick: Tick,
        join: JoinPlayerControlData,
    },
    SubmitRemovePlayer {
        tick: Tick,
        remove: clonk_engine::RemovePlayerControlData,
    },
    SubmitClientUpdate(clonk_engine::ClientUpdateControlData),
    SubmitClientRemove(clonk_engine::ClientRemoveControlData),
    SubmitControlSet(LegacyControlSet),
    SubmitDecidedControl {
        tick: Tick,
        control: clonk_engine::ControlPacket,
        sync: bool,
    },
    SubmitInitScenarioPlayer {
        tick: Tick,
        selection: clonk_engine::InitScenarioPlayerControlData,
    },
    SubmitSurrenderPlayer {
        tick: Tick,
        surrender: clonk_engine::SurrenderPlayerControlData,
    },
    SubmitInternalPlayerScript {
        tick: Tick,
        control: clonk_engine::ControlPacket,
    },
    SubmitMessage(MessageControlData),
    SubmitVote(clonk_engine::VoteControlData),
    SubmitVoteEnd(clonk_engine::VoteControlData),
    SubmitReadyCheck(clonk_network::ReadyCheckPacket),
    SubmitLobbyCountdown(clonk_network::LobbyCountdownPacket),
    BroadcastHostRestarting {
        rejoin_seconds: u16,
    },
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
    BeginGo {
        status: NetworkStatus,
        join_allowed: bool,
        completion: Sender<std::result::Result<(), String>>,
    },
    StatusReachedCurrent,
    StatusReached {
        status: NetworkStatus,
        actual_control_tick: i32,
    },
    AcknowledgeRequestedStatus {
        status: NetworkStatus,
        current_control_tick: i32,
        current_frame: i32,
    },
    ClientUpdateExecuted(clonk_engine::ClientUpdateControlData),
    SetJoinAllowed {
        allowed: bool,
        completion: Sender<std::result::Result<(), String>>,
    },
    SetHostPassword {
        password: clonk_engine::LegacyCString,
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

// Worker modes are moved once into the network thread. Their settings remain
// inline so startup code can continue to destructure the public mode directly.
#[allow(clippy::large_enum_variant)]
enum WorkerMode {
    Host {
        settings: HostSettings,
        local_owner: i32,
    },
    Client {
        settings: ClientSettings,
        local_owner: i32,
        startup_cancellation: Option<NetworkStartupCancellation>,
    },
}

#[derive(Debug, Default)]
struct ControlFrameAccumulator {
    client_id: ClientId,
    current_tick: Option<Tick>,
    controls: Vec<clonk_engine::ControlPacket>,
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

    fn record_control(
        &mut self,
        tick: Tick,
        control: clonk_engine::ControlPacket,
        timestamp: u64,
    ) -> bool {
        if self.last_sent_tick.is_some_and(|last| tick <= last) {
            return false;
        }
        if self.current_tick != Some(tick) {
            self.controls.clear();
            self.current_tick = Some(tick);
        }
        self.controls.push(control);
        self.last_timestamp = Some(timestamp);
        true
    }

    fn rebase_pending_to_first_activated_tick(&mut self, tick: Tick) {
        if !self.controls.is_empty() && self.last_sent_tick.is_none_or(|last| last < tick) {
            self.current_tick = Some(tick);
        }
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

/// `Config.General.ThreadPoolThreadCount` defaults to 8 and sizes the global
/// asynchronous pool on every non-Windows target (C4Config.cpp:406-408;
/// C4Application.cpp:152-159). Windows builds the pool from the system default
/// instead, so the port keeps its own worker count there.
#[cfg(not(windows))]
pub const DEFAULT_NETWORK_RUNTIME_WORKER_THREADS: usize = 8;
#[cfg(windows)]
pub const DEFAULT_NETWORK_RUNTIME_WORKER_THREADS: usize = 4;

/// Set once at startup, before any worker thread builds its runtime. C++ keeps
/// the equivalent in `C4ThreadPool::Global`.
static NETWORK_RUNTIME_WORKER_THREADS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_NETWORK_RUNTIME_WORKER_THREADS);

/// Sizes the asynchronous worker runtime. A zero or absent configuration keeps
/// the native default rather than asking tokio for an invalid pool.
pub fn set_network_runtime_worker_threads(workers: usize) {
    let workers = if workers == 0 {
        DEFAULT_NETWORK_RUNTIME_WORKER_THREADS
    } else {
        workers
    };
    NETWORK_RUNTIME_WORKER_THREADS.store(workers, std::sync::atomic::Ordering::Release);
}

pub fn network_runtime_worker_threads() -> usize {
    NETWORK_RUNTIME_WORKER_THREADS.load(std::sync::atomic::Ordering::Acquire)
}

fn build_network_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    RuntimeBuilder::new_multi_thread()
        .worker_threads(network_runtime_worker_threads())
        .enable_all()
        .build()
}

/// What the just-consumed control tick cost, from the two independent signals
/// PreSend is sized from.
///
/// They are kept apart deliberately: a blended "lag" number cannot distinguish
/// "this player needs a better connection" from "this player needs a better
/// computer", and the two call for different responses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlTickCost {
    /// C++ `CalcPerformance`'s topology-aware route sample, derived from ping.
    pub send_time_ms: i32,
    /// Measured interval from reaching the control tick to consuming it, when
    /// the probe for this tick was still current.
    pub lateness_ms: Option<i32>,
}

impl NetworkManager {
    pub fn for_mode(
        mode: NetworkMode,
        local_owner: i32,
    ) -> std::result::Result<Self, NetworkStartError> {
        let worker_mode = match mode {
            NetworkMode::Host(settings) => WorkerMode::Host {
                settings,
                local_owner,
            },
            NetworkMode::Client(settings) => WorkerMode::Client {
                settings,
                local_owner,
                startup_cancellation: None,
            },
        };
        Self::spawn(worker_mode)
    }

    pub fn for_client_cancellable(
        settings: ClientSettings,
        local_owner: i32,
        startup_cancellation: NetworkStartupCancellation,
    ) -> std::result::Result<Self, NetworkStartError> {
        let worker_mode = WorkerMode::Client {
            settings,
            local_owner,
            startup_cancellation: Some(startup_cancellation),
        };
        Self::spawn(worker_mode)
    }

    fn spawn(mode: WorkerMode) -> std::result::Result<Self, NetworkStartError> {
        let role = match &mode {
            WorkerMode::Host { .. } => NetworkRole::Host,
            WorkerMode::Client { .. } => NetworkRole::Client,
        };
        let (command_tx, command_rx) = tokio_mpsc::channel(128);
        let (control_tick_tx, control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (control_performance_tx, control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) =
            mpsc::channel::<std::result::Result<NetworkWorkerReady, NetworkStartError>>();
        let netpuncher_state = Arc::new(Mutex::new(NetworkNetpuncherState::default()));
        let current_frame = Arc::new(AtomicI32::new(0));
        let thread_name = match mode {
            WorkerMode::Host { .. } => "clonk-network-host",
            WorkerMode::Client { .. } => "clonk-network-client",
        };
        let worker = thread::Builder::new()
            .name(thread_name.to_string())
            .spawn({
                let event_tx = event_tx.clone();
                let worker_netpuncher_state = netpuncher_state.clone();
                let worker_current_frame = current_frame.clone();
                move || {
                    let runtime =
                        build_network_runtime().expect("failed to initialise tokio runtime");
                    if let Err(err) = runtime.block_on(run_worker(
                        mode,
                        command_rx,
                        control_tick_rx,
                        control_performance_rx,
                        event_tx.clone(),
                        telemetry_tx,
                        local_id_tx,
                        worker_netpuncher_state,
                        worker_current_frame,
                    )) {
                        let _ = event_tx.send(NetworkEvent::FatalError(format!("{err:?}")));
                    }
                }
            })
            .map_err(|error| {
                NetworkStartError::Other(format!("failed to spawn network worker thread: {error}"))
            })?;
        let ready = match local_id_rx.recv() {
            Ok(ready) => ready,
            Err(error) => {
                let _ = worker.join();
                return Err(NetworkStartError::Other(format!(
                    "network worker did not report local client id: {error}"
                )));
            }
        };
        let ready = match ready {
            Ok(ready) => ready,
            Err(error) => {
                let _ = worker.join();
                return Err(error);
            }
        };

        let league_runtime_available = AtomicBool::new(ready.league_runtime_available);
        Ok(Self {
            command_tx,
            control_tick_tx,
            control_performance_tx,
            control_tick_probe: Mutex::new(None),
            current_frame,
            event_rx,
            telemetry_rx,
            event_wake: event_tx.wake.clone(),
            worker: Some(worker),
            local_client_id: ready.local_client_id,
            netpuncher_state,
            role,
            client_status: ClientStatusState::default(),
            league_start_response: ready.league_start_response,
            league_start_failure: ready.league_start_failure,
            league_runtime_available,
            league_record_runtime: ready.league_record_runtime,
            network_io_statistics: ready.network_io_statistics,
            control_send_time: ready.control_send_time,
            #[cfg(any(test, feature = "test-hooks"))]
            test_runtime_client_states: Arc::new(Mutex::new(Vec::new())),
            #[cfg(any(test, feature = "test-hooks"))]
            test_lobby_client_telemetry: Arc::new(Mutex::new(None)),
        })
    }

    /// Publish the live game frame for background network timers. C++ reads
    /// `Game.FrameCounter` when each activation retry is built, so the worker
    /// must not reuse the frame from the input that armed the first request.
    pub fn refresh_current_frame(&self, current_frame: i32) {
        self.current_frame.store(current_frame, Ordering::Relaxed);
    }

    pub fn runtime_connections(&self) -> Result<Vec<RuntimeNetworkConnection>> {
        if self.worker.is_none() {
            return Err(anyhow!(
                "runtime connection inspection is unavailable without a network worker"
            ));
        }
        let (completion, inspected) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::InspectRuntimeConnections { completion })
            .map_err(|_| anyhow!("network worker is not accepting connection inspection"))?;
        inspected
            .recv()
            .map_err(|_| anyhow!("network worker ended before returning live connections"))?
            .map_err(|message| anyhow!(message))
    }

    /// Returns the most recently completed C++-cadence input/output samples,
    /// generating a due one-second edge from the live socket accounting first.
    /// `C4Network2IO::getProtIRate/getProtORate/getProtBCRate` read the
    /// cached interval values; only `GenerateStatistics` recomputes them. The
    /// status overlay must therefore *read* the accumulator rather than
    /// regenerate it, which would steal the interval the per-second chart
    /// sampling owns (src/C4Network2.cpp:1171-1178).
    pub fn protocol_rate_statistics(
        &self,
        protocol: clonk_network::NetworkProtocol,
    ) -> clonk_network::ProtocolRateStatistics {
        self.network_io_statistics.protocol_statistics(protocol)
    }

    pub fn protocol_rate_samples(
        &self,
    ) -> (
        clonk_network::ProtocolRateSample,
        clonk_network::ProtocolRateSample,
    ) {
        self.network_io_statistics
            .generate_statistics(current_millis());
        let snapshot = self.network_io_statistics.snapshot();
        let rate = |value: u64| i32::try_from(value).unwrap_or(i32::MAX);
        (
            clonk_network::ProtocolRateSample::new(
                rate(snapshot.tcp.input_rate),
                rate(snapshot.udp.input_rate),
            ),
            clonk_network::ProtocolRateSample::new(
                rate(snapshot.tcp.output_rate),
                rate(snapshot.udp.output_rate),
            ),
        )
    }

    pub fn lobby_client_telemetry(
        &self,
        client_ids: Vec<ClientId>,
    ) -> Result<RuntimeLobbyClientTelemetry> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.worker.is_none() {
            if let Some(mut telemetry) = self.test_lobby_client_telemetry.lock().clone() {
                let requested = client_ids
                    .into_iter()
                    .collect::<std::collections::HashSet<_>>();
                telemetry
                    .connections
                    .retain(|connection| requested.contains(&connection.client_id));
                telemetry
                    .resource_progress
                    .retain(|(client_id, _)| requested.contains(client_id));
                return Ok(telemetry);
            }
        }
        if self.worker.is_none() {
            return Err(anyhow!(
                "lobby client telemetry is unavailable without a network worker"
            ));
        }
        let (completion, inspected) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::InspectLobbyClientTelemetry {
                client_ids,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting lobby telemetry inspection"))?;
        inspected
            .recv()
            .map_err(|_| anyhow!("network worker ended before returning lobby telemetry"))?
            .map_err(|message| anyhow!(message))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_test_lobby_client_telemetry(&self, telemetry: RuntimeLobbyClientTelemetry) {
        *self.test_lobby_client_telemetry.lock() = Some(telemetry);
    }

    pub fn runtime_client_states(&self, tick: Tick) -> Result<Vec<RuntimeNetworkClientState>> {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.worker.is_none() {
            return Ok(self.test_runtime_client_states.lock().clone());
        }
        if self.worker.is_none() {
            return Err(anyhow!(
                "runtime client-state inspection is unavailable without a network worker"
            ));
        }
        let probe = self
            .control_tick_probe
            .lock()
            .as_ref()
            .copied()
            .filter(|probe| probe.tick == tick);
        let (completion, inspected) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::InspectRuntimeClientStates {
                tick,
                probe,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting client-state inspection"))?;
        inspected
            .recv()
            .map_err(|_| anyhow!("network worker ended before returning live client states"))?
            .map_err(|message| anyhow!(message))
    }

    #[cfg(any(test, feature = "test-hooks"))]
    /// Seed the shared per-protocol accumulator the way live traffic would,
    /// so the status overlay and the chart can be exercised against the same
    /// sample without a real transport.
    pub fn record_test_protocol_traffic(
        &self,
        connection_id: u32,
        protocol: clonk_network::NetworkProtocol,
        input_bytes: usize,
        output_bytes: usize,
        broadcast_bytes: usize,
    ) {
        let recorder = self
            .network_io_statistics
            .open_connection(connection_id, protocol);
        recorder.record_input(input_bytes);
        recorder.record_output(output_bytes);
        self.network_io_statistics
            .record_broadcast_datagram(protocol, broadcast_bytes);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    /// Seed the bound local addresses the status overlay reads to decide which
    /// NetIO backs the message and data protocols.
    pub fn set_test_local_addresses(&self, addresses: impl IntoIterator<Item = NetworkAddress>) {
        self.netpuncher_state.lock().local_addresses = addresses.into_iter().collect();
    }

    #[cfg(any(test, feature = "test-hooks"))]
    /// `C4Network2IO::GenerateStatistics`'s interval boundary, exposed so a
    /// test can close one sample deterministically.
    pub fn generate_test_statistics(&self, now_ms: u64) {
        self.network_io_statistics.generate_statistics(now_ms);
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_test_runtime_client_states(
        &self,
        states: impl IntoIterator<Item = RuntimeNetworkClientState>,
    ) {
        *self.test_runtime_client_states.lock() = states.into_iter().collect();
    }

    pub fn disconnect_runtime_connection(&self, connection_id: u32) -> Result<()> {
        if self.worker.is_none() {
            return Err(anyhow!(
                "runtime connection disconnect is unavailable without a network worker"
            ));
        }
        let (completion, disconnected) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::DisconnectRuntimeConnection {
                connection_id,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting connection disconnects"))?;
        disconnected
            .recv()
            .map_err(|_| anyhow!("network worker ended before disconnecting the connection"))?
            .map_err(|message| anyhow!(message))
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

    fn submit_decided_control(
        &self,
        tick: Tick,
        control: clonk_engine::ControlPacket,
        sync: bool,
    ) -> Result<()> {
        self.command_tx
            .blocking_send(NetworkCommand::SubmitDecidedControl {
                tick,
                control,
                sync: sync && self.role == NetworkRole::Host,
            })
            .map_err(|_| anyhow!("network worker is not accepting decided controls"))
    }

    pub fn submit_decided_control_set(
        &self,
        tick: Tick,
        mut set: LegacyControlSet,
        sync: bool,
    ) -> Result<()> {
        set.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the control-set wire field"))?;
        self.submit_decided_control(tick, set.into_control_packet(), sync)
    }

    /// Queue one host-authored `CID_Synchronize` through `CDT_Sync`.
    ///
    /// The live host scheduler chooses the synchronized execution boundary;
    /// `tick` is retained for deterministic command stubs and diagnostics.
    pub fn submit_synchronize(
        &self,
        tick: Tick,
        save_player_files: bool,
        sync_clearance: bool,
    ) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a synchronization control"
            ));
        }
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the synchronize-control wire field"))?;
        self.submit_decided_control(
            tick,
            clonk_engine::ControlPacket::Synchronize(clonk_engine::SynchronizeControlData {
                save_player_files,
                sync_clearance,
                by_client,
            }),
            true,
        )
    }

    /// Append `CID_Synchronize` to the caller's ordinary control input. This
    /// is `C4GameControl::DoInput(..., CDT_Queue)`, used by
    /// RequestRuntimeRecord on hosts and clients alike; it is deliberately
    /// distinct from the host-only CDT_Sync runtime-join seam above.
    pub fn submit_queued_synchronize(
        &self,
        tick: Tick,
        save_player_files: bool,
        sync_clearance: bool,
    ) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the synchronize-control wire field"))?;
        self.submit_decided_control(
            tick,
            clonk_engine::ControlPacket::Synchronize(clonk_engine::SynchronizeControlData {
                save_player_files,
                sync_clearance,
                by_client,
            }),
            false,
        )
    }

    pub fn submit_decided_script_control(
        &self,
        tick: Tick,
        mut script: ScriptControlData,
        sync: bool,
    ) -> Result<()> {
        script.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the script-control wire field"))?;
        self.submit_decided_control(tick, clonk_engine::ControlPacket::Script(script), sync)
    }

    pub fn submit_decided_em_move_object_control(
        &self,
        tick: Tick,
        mut control: clonk_engine::EmMoveObjectControlData,
        sync: bool,
    ) -> Result<()> {
        control.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the editor-control wire field"))?;
        self.submit_decided_control(
            tick,
            clonk_engine::ControlPacket::EmMoveObject(control),
            sync,
        )
    }

    pub fn submit_decided_em_draw_tool_control(
        &self,
        tick: Tick,
        mut control: clonk_engine::EmDrawToolControlData,
        sync: bool,
    ) -> Result<()> {
        control.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the editor-control wire field"))?;
        self.submit_decided_control(tick, clonk_engine::ControlPacket::EmDrawTool(control), sync)
    }

    pub fn submit_custom_command(
        &self,
        tick: Tick,
        mut command: clonk_engine::CustomCommandControlData,
        sync: bool,
    ) -> Result<()> {
        command.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the custom-command wire field"))?;
        self.submit_decided_control(
            tick,
            clonk_engine::ControlPacket::CustomCommand(command),
            sync,
        )
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
        countdown: clonk_network::LobbyCountdownPacket,
    ) -> Result<()> {
        self.submit_lobby_countdown(countdown)
    }

    pub fn request_ready_check(&self) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may request a ready check"));
        }
        self.submit_ready_check(clonk_network::ReadyCheckData::Request)
    }

    pub fn set_local_ready(&self, ready: bool) -> Result<()> {
        self.submit_ready_check(if ready {
            clonk_network::ReadyCheckData::Ready
        } else {
            clonk_network::ReadyCheckData::NotReady
        })
    }

    pub fn submit_player_info_update(
        &self,
        request: clonk_network::PlayerInfoUpdateRequest,
    ) -> Result<()> {
        self.command_tx
            .blocking_send(NetworkCommand::SubmitPlayerInfoUpdate(request))
            .map_err(|_| anyhow!("network worker is not accepting player-info updates"))
    }

    pub fn submit_client_update(
        &self,
        update: clonk_engine::ClientUpdateControlData,
    ) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a synchronized client update"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitClientUpdate(update))
            .map_err(|_| anyhow!("network worker is not accepting client updates"))
    }

    pub fn submit_client_remove(
        &self,
        remove: clonk_engine::ClientRemoveControlData,
    ) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a synchronized client removal"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitClientRemove(remove))
            .map_err(|_| anyhow!("network worker is not accepting client removals"))
    }

    /// Submit one `CID_Set` with the synchronized delivery selected by
    /// `CDT_Decide` while the lobby is frozen. The host remains responsible
    /// for enforcing each setting's `HostControl` rule when it executes the
    /// echoed control.
    pub fn submit_control_set(&self, mut set: LegacyControlSet) -> Result<()> {
        set.by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the control-set wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitControlSet(set))
            .map_err(|_| anyhow!("network worker is not accepting control-set updates"))
    }

    pub fn submit_init_scenario_player(&self, tick: Tick, player: i32, team: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the scenario-player wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitInitScenarioPlayer {
                tick,
                selection: clonk_engine::InitScenarioPlayerControlData {
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
                surrender: clonk_engine::SurrenderPlayerControlData { player, by_client },
            })
            .map_err(|_| anyhow!("network worker is not accepting player surrender"))
    }

    fn submit_internal_player_script(
        &self,
        tick: Tick,
        control: clonk_engine::ControlPacket,
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
            clonk_engine::ControlPacket::ActivateGameGoalMenu(
                clonk_engine::ActivateGameGoalMenuControlData { player, by_client },
            ),
        )
    }

    pub fn submit_toggle_hostility(&self, tick: Tick, player: i32, opponent: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the hostility wire field"))?;
        self.submit_internal_player_script(
            tick,
            clonk_engine::ControlPacket::ToggleHostility(
                clonk_engine::ToggleHostilityControlData {
                    opponent,
                    player,
                    by_client,
                },
            ),
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
            clonk_engine::ControlPacket::ActivateGameGoalRule(
                clonk_engine::ActivateGameGoalRuleControlData {
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
            clonk_engine::ControlPacket::SetPlayerTeam(clonk_engine::SetPlayerTeamControlData {
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
            clonk_engine::ControlPacket::EliminatePlayer(
                clonk_engine::EliminatePlayerControlData { player, by_client },
            ),
        )
    }

    pub fn submit_vote(&self, vote_type: u8, approve: bool, data: i32) -> Result<()> {
        let by_client = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the vote wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitVote(clonk_engine::VoteControlData {
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
            .blocking_send(NetworkCommand::SubmitVoteEnd(
                clonk_engine::VoteControlData {
                    vote_type,
                    approve,
                    data,
                    by_client,
                },
            ))
            .map_err(|_| anyhow!("network worker is not accepting vote results"))
    }

    pub fn submit_ready_check(&self, data: clonk_network::ReadyCheckData) -> Result<()> {
        let client_id = i32::try_from(self.local_client_id)
            .map_err(|_| anyhow!("local client id exceeds the ready-check wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::SubmitReadyCheck(
                clonk_network::ReadyCheckPacket { client_id, data },
            ))
            .map_err(|_| anyhow!("network worker is not accepting ready checks"))
    }

    pub fn submit_lobby_countdown(
        &self,
        packet: clonk_network::LobbyCountdownPacket,
    ) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may submit a lobby countdown"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitLobbyCountdown(packet))
            .map_err(|_| anyhow!("network worker is not accepting lobby countdowns"))
    }

    /// Announces that this session is closing to restart the round, so clients
    /// follow the host into the new lobby instead of reading the imminent
    /// disconnect as a dead host.
    ///
    /// Deliberately does not wait for the notice to reach the wire: the app
    /// tears the manager down immediately afterwards, and `Drop` queues
    /// `Shutdown` behind this on the same channel. The worker's own await of
    /// the session broadcast is what orders the bytes ahead of the teardown.
    pub fn broadcast_host_restarting(&self, rejoin_seconds: u16) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may announce a round restart"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::BroadcastHostRestarting { rejoin_seconds })
            .map_err(|_| anyhow!("network worker is not accepting a restart notice"))
    }

    pub fn publish_runtime_dynamic(
        &self,
        dynamic: clonk_network::LiveNetworkDynamic,
        dynamic_tick: i32,
        parameters: clonk_network::JoinGameParametersEnvelope,
    ) -> Result<clonk_engine::NetworkResourceCore> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may publish a runtime dynamic"
            ));
        }
        let (completion, published) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::PublishRuntimeDynamic {
                dynamic: Box::new(dynamic),
                dynamic_tick,
                parameters: Box::new(parameters),
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting runtime dynamics"))?;
        published
            .recv()
            .map_err(|_| anyhow!("network worker ended before publishing the runtime dynamic"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn remove_runtime_dynamic(&self) -> Result<bool> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may remove a runtime dynamic"
            ));
        }
        let (completion, removed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::RemoveRuntimeDynamic { completion })
            .map_err(|_| anyhow!("network worker is not accepting runtime-dynamic removal"))?;
        removed
            .recv()
            .map_err(|_| anyhow!("network worker ended before removing the runtime dynamic"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn fail_pending_join_data(&self, reason: clonk_engine::LegacyCString) -> Result<usize> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may fail pending JoinData"));
        }
        let (completion, failed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::FailPendingJoinData { reason, completion })
            .map_err(|_| anyhow!("network worker is not accepting pending JoinData failure"))?;
        failed
            .recv()
            .map_err(|_| anyhow!("network worker ended before failing pending JoinData"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn publish_client_player_resource(
        &self,
        request: ClientPlayerResourceRequest,
    ) -> Result<clonk_engine::NetworkResourceCore> {
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
    ) -> Result<clonk_engine::NetworkResourceCore> {
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

    /// Protect a complete resource before its mutable source is replaced.
    /// Every peer performs this half of C4Network2Res::Derive.
    pub fn begin_resource_derive(
        &self,
        resource_id: i32,
        source_path: impl Into<PathBuf>,
        ownership: clonk_network::ResourceFileOwnership,
    ) -> Result<clonk_network::ResourceDerivation> {
        let (completion, derived) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::BeginResourceDerive {
                resource_id,
                source_path: source_path.into(),
                ownership,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting resource derivations"))?;
        derived
            .recv()
            .map_err(|_| anyhow!("network worker ended before protecting the resource"))?
            .map_err(|message| anyhow!(message))
    }

    /// Give a protected replacement its official core and broadcast the
    /// derive announcement. C++ permits this only on the control host.
    pub fn finish_resource_derive(
        &self,
        derivation: clonk_network::ResourceDerivation,
    ) -> Result<clonk_engine::NetworkResourceCore> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may finish a resource derivation"
            ));
        }
        let (completion, finished) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::FinishResourceDerive {
                derivation,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting resource derivations"))?;
        finished
            .recv()
            .map_err(|_| anyhow!("network worker ended before publishing the derived resource"))?
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

    /// Queue a client resource removal without waiting for either command
    /// capacity or the network worker's completion notification.
    pub fn remove_client_resource_async(
        &self,
        resource_id: i32,
    ) -> Result<Receiver<std::result::Result<(), String>>> {
        if self.role != NetworkRole::Client {
            return Err(anyhow!(
                "only a network client may remove a network resource"
            ));
        }
        let (completion, removed) = mpsc::channel();
        self.command_tx
            .try_send(NetworkCommand::RemoveResource {
                resource_id,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting resource removals"))?;
        Ok(removed)
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

    pub fn broadcast_preexecuted_player_info(
        &self,
        mut info: PlayerInfoControlData,
        join_players_on_echo: Vec<clonk_engine::ControlPlayerInfoEntry>,
    ) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may broadcast PlayerInfo"));
        }
        info.by_client = i32::try_from(HOST_CLIENT_ID)
            .map_err(|_| anyhow!("host client id exceeds the PlayerInfo wire field"))?;
        self.command_tx
            .blocking_send(NetworkCommand::BroadcastPreexecutedPlayerInfo {
                info,
                join_players_on_echo,
            })
            .map_err(|_| anyhow!("network worker is not accepting PlayerInfo broadcasts"))
    }

    pub fn broadcast_league_round_results(
        &self,
        packet: clonk_network::LeagueRoundResultsPacket,
    ) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may broadcast league round results"
            ));
        }
        self.command_tx
            .blocking_send(NetworkCommand::BroadcastLeagueRoundResults(packet))
            .map_err(|_| anyhow!("network worker is not accepting league round results"))
    }

    /// Runs the local-player `Action=Auth` exchange and returns the raw reply.
    /// A missing league runtime is the native `!pLeagueClient` failure.
    pub fn league_player_auth_response(
        &self,
        auth: clonk_network::LeagueAuthRequestHead,
        player: &clonk_engine::ControlPlayerInfoEntry,
    ) -> Result<clonk_network::LeagueAuthResponse> {
        let Some(pending) = self.begin_authenticate_league_player(auth, player)? else {
            return Err(anyhow!("league runtime is unavailable"));
        };
        pending.wait()
    }

    pub fn begin_authenticate_league_player(
        &self,
        auth: clonk_network::LeagueAuthRequestHead,
        player: &clonk_engine::ControlPlayerInfoEntry,
    ) -> Result<Option<PendingLeaguePlayerAuth>> {
        if !self.league_runtime_available.load(Ordering::Acquire) {
            return Ok(None);
        }
        let (completion, completed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::LeagueAuthenticatePlayer {
                auth,
                player: player.clone(),
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting league authentication"))?;
        Ok(Some(PendingLeaguePlayerAuth {
            completion: completed,
        }))
    }

    pub fn authenticate_league_player(
        &self,
        auth: clonk_network::LeagueAuthRequestHead,
        player: &mut clonk_engine::ControlPlayerInfoEntry,
    ) -> Result<bool> {
        if !self.league_runtime_available.load(Ordering::Acquire) {
            return Ok(false);
        }
        let response = self.league_player_auth_response(auth, player)?;
        Ok(response.apply_player_auth(player))
    }

    /// Runs the host's `Action=Join` check and applies the synchronized league
    /// fields to an accepted new player. The response consumes its AUID.
    pub fn check_league_player(
        &self,
        league: &clonk_engine::LegacyCString,
        player: &mut clonk_engine::ControlPlayerInfoEntry,
    ) -> Result<LeaguePlayerCheck> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may check league players"));
        }
        if !self.league_runtime_available.load(Ordering::Acquire) {
            return Ok(LeaguePlayerCheck::Unavailable);
        }
        let (completion, completed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::LeagueCheckPlayer {
                player: player.clone(),
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting league player checks"))?;
        let response = completed
            .recv_timeout(clonk_network::LEAGUE_HTTP_TIMEOUT + Duration::from_secs(1))
            .map_err(|_| anyhow!("league runtime did not finish checking the player"))?
            .map_err(|message| anyhow!(message))?;
        if response.apply_auth_check(league, player) {
            Ok(LeaguePlayerCheck::Accepted)
        } else {
            Ok(LeaguePlayerCheck::Rejected(response.head.message))
        }
    }

    pub fn update_league_reference(
        &self,
        now: i64,
        reference: clonk_network::HostGameReference,
    ) -> Result<()> {
        if self.role != NetworkRole::Host || !self.league_runtime_available.load(Ordering::Acquire)
        {
            return Ok(());
        }
        self.command_tx
            .blocking_send(NetworkCommand::LeagueUpdate { now, reference })
            .map_err(|_| anyhow!("network worker is not accepting league updates"))
    }

    pub fn end_league(
        &self,
        reference: clonk_network::HostGameReference,
        record: Option<clonk_network::LeagueEndRecord>,
    ) -> Result<LeagueEndAttempt> {
        if self.role != NetworkRole::Host || !self.league_runtime_available.load(Ordering::Acquire)
        {
            return Ok(LeagueEndAttempt::Finished(None));
        }
        let (completion, completed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::LeagueEnd {
                reference,
                record,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting league end requests"))?;
        completed
            .recv()
            .map_err(|_| anyhow!("league runtime ended before finishing the game"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn finalize_league_end_failure(
        &self,
        packet: clonk_network::LeagueRoundResultsPacket,
    ) -> Result<Option<clonk_network::LeagueRoundResultsPacket>> {
        if self.role != NetworkRole::Host || !self.league_runtime_available.load(Ordering::Acquire)
        {
            return Ok(None);
        }
        let (completion, completed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::LeagueFinalizeEndFailure { packet, completion })
            .map_err(|_| anyhow!("network worker is not accepting league end finalization"))?;
        completed
            .recv()
            .map_err(|_| anyhow!("league runtime ended before finalizing the game"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn report_league_disconnect(
        &self,
        reason: clonk_network::LeagueDisconnectReason,
        players: clonk_network::ClientPlayerInfosSnapshot,
        fbids: clonk_network::LeagueFbidRegistry,
    ) -> Result<()> {
        if !self.league_runtime_available.load(Ordering::Acquire) {
            return Err(anyhow!("league runtime is unavailable"));
        }
        let (completion, completed) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::LeagueReportDisconnect {
                reason,
                players,
                fbids,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting league disconnect reports"))?;
        completed
            .recv()
            .map_err(|_| anyhow!("league runtime ended before reporting the disconnect"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn take_league_start_response(&mut self) -> Option<clonk_network::LeagueStartResponse> {
        self.league_start_response.take()
    }

    /// Why this host is running unregistered, when the league server refused
    /// its `Start`. `C4Network2::InitHost` survives that refusal and leaves the
    /// decision to the user's modal answer (src/C4Network2.cpp:259-272).
    pub fn take_league_start_failure(&mut self) -> Option<String> {
        self.league_start_failure.take()
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn set_test_league_start_failure(&mut self, message: impl Into<String>) {
        self.league_start_failure = Some(message.into());
    }

    pub fn invalidate_league_reference(&self) -> Result<()> {
        if self.role != NetworkRole::Host || !self.league_runtime_available.load(Ordering::Acquire)
        {
            return Ok(());
        }
        self.command_tx
            .blocking_send(NetworkCommand::LeagueInvalidate)
            .map_err(|_| anyhow!("network worker is not accepting league invalidations"))
    }

    pub fn start_league_record_stream(&self, now: i64) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may stream a league record"));
        }
        let runtime = self
            .league_record_runtime
            .as_ref()
            .ok_or_else(|| anyhow!("league record streaming is unavailable"))?;
        let (completion, completed) = mpsc::channel();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Start { now, completion })
            .map_err(|_| anyhow!("network worker is not accepting league record streams"))?;
        completed
            .recv()
            .map_err(|_| anyhow!("league record runtime ended before starting"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn league_record_stream_available(&self) -> bool {
        self.role == NetworkRole::Host && self.league_record_runtime.is_some()
    }

    pub fn league_record_stream_status(&self) -> Option<LeagueRecordStreamStatus> {
        self.league_record_runtime
            .as_ref()
            .map(LeagueRecordRuntimeHandle::status)
    }

    pub fn append_league_record_bytes(&self, bytes: &[u8]) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may stream a league record"));
        }
        if bytes.is_empty() {
            return Ok(());
        }
        let runtime = self
            .league_record_runtime
            .as_ref()
            .ok_or_else(|| anyhow!("league record streaming is unavailable"))?;
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Append(bytes.to_vec()))
            .map_err(|_| anyhow!("network worker is not accepting league record bytes"))
    }

    pub fn pump_league_record_stream(&self, now: i64) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Ok(());
        }
        let Some(runtime) = self.league_record_runtime.as_ref() else {
            return Ok(());
        };
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Pump { now })
            .map_err(|_| anyhow!("network worker is not accepting league record pumps"))
    }

    pub fn finish_league_record_stream(&self, now: i64) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may stream a league record"));
        }
        let runtime = self
            .league_record_runtime
            .as_ref()
            .ok_or_else(|| anyhow!("league record streaming is unavailable"))?;
        let (completion, completed) = mpsc::channel();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Finish { now, completion })
            .map_err(|_| anyhow!("network worker is not accepting league record finish"))?;
        completed
            .recv()
            .map_err(|_| anyhow!("league record runtime ended before finishing"))?
            .map_err(|message| anyhow!(message))
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

    pub fn submit_remove_player(&self, tick: Tick, player: i32, disconnected: bool) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!("only the network host may submit RemovePlr"));
        }
        self.command_tx
            .blocking_send(NetworkCommand::SubmitRemovePlayer {
                tick,
                remove: clonk_engine::RemovePlayerControlData {
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

    pub fn control_tick_consumed(
        &self,
        tick: Tick,
        client_ids: Vec<ClientId>,
    ) -> Option<ControlTickCost> {
        self.worker.as_ref()?;
        let consumed_at = tokio::time::Instant::now();
        let send_time_ms = self.control_send_time.sample(&client_ids);
        // How long the game loop actually waited for this tick: from reaching
        // the control tick to the aggregate becoming available. The probe is
        // stamped by `control_tick_reached` on the frame the cadence came round,
        // so this is arrival measured against the cadence — the same quantity
        // the host records as `ClientPerformanceStats::wait_ms`, and the one
        // signal that notices a client which is slow rather than distant.
        let lateness_ms = self
            .control_tick_probe
            .lock()
            .as_ref()
            .filter(|probe| probe.tick == tick)
            .map(|probe| {
                consumed_at
                    .saturating_duration_since(probe.reached_at)
                    .as_millis()
                    .min(i32::MAX as u128) as i32
            });
        self.control_performance_tx
            .send(ControlPerformanceEvent::TickConsumed {
                tick,
                consumed_at,
                client_ids,
            })
            .ok()?;
        Some(ControlTickCost {
            send_time_ms,
            lateness_ms,
        })
    }

    pub fn reset_client_performance(&self) {
        #[cfg(any(test, feature = "test-hooks"))]
        if self.worker.is_none() {
            return;
        }
        if self.worker.is_none() {
            return;
        }
        let _ = self
            .control_performance_tx
            .send(ControlPerformanceEvent::Reset);
    }

    pub fn control_tick_reached(
        &self,
        tick: Tick,
        control_rate: i32,
        target_fps: i32,
        reached_at: tokio::time::Instant,
    ) {
        let mut probe = self.control_tick_probe.lock();
        if probe.as_ref().is_none_or(|probe| probe.tick != tick) {
            *probe = Some(ControlTickProbe {
                tick,
                control_rate,
                target_fps,
                reached_at,
                queued: false,
            });
        } else if let Some(probe) = probe.as_mut() {
            if probe.control_rate != control_rate || probe.target_fps != target_fps {
                probe.control_rate = control_rate;
                probe.target_fps = target_fps;
                // Queue a same-tick refresh with the original timestamp.
                probe.queued = false;
            }
        }
        let probe = probe.as_mut().expect("control-tick probe was initialized");
        if probe.queued {
            return;
        }
        if self.control_tick_tx.send(*probe).is_ok() {
            probe.queued = true;
        }
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

    /// Leaves lobby mode by applying the authoritative Go barrier and the
    /// runtime join policy in one acknowledged host-loop operation.
    pub fn begin_go(&self, status: NetworkStatus, join_allowed: bool) -> Result<()> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!("only the network host may begin the game"));
        }
        if status.state != clonk_network::NETWORK_STATE_GO {
            return Err(anyhow!("beginning the game requires a Go network status"));
        }
        let (completion, applied) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::BeginGo {
                status,
                join_allowed,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting the Go transition"))?;
        applied
            .recv()
            .map_err(|_| anyhow!("network worker ended before confirming the Go transition"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn set_host_password(&self, password: clonk_engine::LegacyCString) -> Result<()> {
        if self.local_client_id != HOST_CLIENT_ID {
            return Err(anyhow!(
                "only the network host may change the host password"
            ));
        }
        let (completion, applied) = mpsc::channel();
        self.command_tx
            .blocking_send(NetworkCommand::SetHostPassword {
                password,
                completion,
            })
            .map_err(|_| anyhow!("network worker is not accepting host-password changes"))?;
        applied
            .recv()
            .map_err(|_| anyhow!("network worker ended before confirming the host password"))?
            .map_err(|message| anyhow!(message))
    }

    pub fn begin_masterserver_signup(
        &self,
        enabled: bool,
        config: PreparedLeagueHostConfig,
        reference: clonk_network::HostGameReference,
    ) -> Result<PendingMasterserverSignup> {
        if self.role != NetworkRole::Host {
            return Err(anyhow!(
                "only the network host may change masterserver signup"
            ));
        }
        let previous_enabled = self.league_runtime_available.load(Ordering::Acquire);
        let (completion, completed) = mpsc::channel();
        let (cancellation, cancelled) = tokio::sync::oneshot::channel();
        let transition = Arc::new(std::sync::atomic::AtomicU8::new(
            MASTERSERVER_SIGNUP_PENDING,
        ));
        if self
            .command_tx
            .try_send(NetworkCommand::SetMasterserverSignup {
                enabled,
                config,
                reference,
                completion,
                cancellation: cancelled,
                transition: Arc::clone(&transition),
            })
            .is_err()
        {
            return Err(anyhow!(
                "network worker cannot accept a masterserver-signup change right now"
            ));
        }
        Ok(PendingMasterserverSignup {
            enabled,
            previous_enabled,
            completion: completed,
            cancellation: Some(cancellation),
            transition,
            cancel_on_drop: true,
        })
    }

    pub fn poll_masterserver_signup(
        &self,
        pending: &mut PendingMasterserverSignup,
    ) -> Option<Result<Option<clonk_network::LeagueStartResponse>>> {
        let result = match pending.completion.try_recv() {
            Ok(result) => result.map_err(|message| anyhow!(message)),
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(anyhow!(
                "network worker ended before finishing the masterserver-signup change"
            )),
        };
        pending.cancellation.take();
        self.league_runtime_available.store(
            if result.is_ok() {
                pending.enabled
            } else if pending.enabled {
                pending.previous_enabled
            } else {
                false
            },
            Ordering::Release,
        );
        Some(result)
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

    pub fn status_reached(
        &self,
        status: NetworkStatus,
        actual_control_tick: i32,
    ) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Host {
            return Err(NetworkStatusCommandError::HostRoleRequired {
                operation: "mark game status reached",
            });
        }
        self.command_tx
            .blocking_send(NetworkCommand::StatusReached {
                status,
                actual_control_tick,
            })
            .map_err(|_| NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status arrival",
            })
    }

    pub fn status_reached_current(&self) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Host {
            return Err(NetworkStatusCommandError::HostRoleRequired {
                operation: "mark game status reached",
            });
        }
        self.command_tx
            .blocking_send(NetworkCommand::StatusReachedCurrent)
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
        self.acknowledge_expected_status_at_frame(status, current_control_tick, current_frame)
    }

    pub fn acknowledge_expected_status_at_frame(
        &mut self,
        expected: NetworkStatus,
        current_control_tick: i32,
        current_frame: i32,
    ) -> std::result::Result<(), NetworkStatusCommandError> {
        if self.role != NetworkRole::Client {
            return Err(NetworkStatusCommandError::ClientRoleRequired {
                operation: "acknowledge a host game status",
            });
        }
        let Some(acknowledgement) = self
            .client_status
            .acknowledge_requested_at(expected, current_control_tick)
        else {
            return Err(NetworkStatusCommandError::NoRequestedStatus);
        };
        if self
            .command_tx
            .blocking_send(NetworkCommand::AcknowledgeRequestedStatus {
                status: expected,
                current_control_tick,
                current_frame,
            })
            .is_err()
        {
            self.client_status
                .restore_request(expected, acknowledgement);
            return Err(NetworkStatusCommandError::WorkerUnavailable {
                operation: "game-status acknowledgements",
            });
        }
        Ok(())
    }

    pub fn notify_client_update_executed(
        &self,
        update: clonk_engine::ClientUpdateControlData,
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
                            NetworkEvent::StatusCommitted(status)
                                if !self.client_status.commit(*status) =>
                            {
                                continue;
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

    pub fn install_event_waker(&self, callback: NetworkEventWakeCallback) {
        self.event_wake.install(callback);
    }

    pub fn local_client_id(&self) -> ClientId {
        self.local_client_id
    }

    pub fn local_addresses(&self) -> Vec<NetworkAddress> {
        self.netpuncher_state.lock().local_addresses.clone()
    }

    pub fn netpuncher_state(&self) -> (clonk_network::NetpuncherGameIds, Vec<NetworkAddress>) {
        let state = self.netpuncher_state.lock();
        (state.game_ids, state.local_addresses.clone())
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub() -> (Self, NetworkEventSender) {
        let (command_tx, _command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                control_tick_tx: tokio_mpsc::unbounded_channel().0,
                control_performance_tx: tokio_mpsc::unbounded_channel().0,
                control_tick_probe: Mutex::new(None),
                current_frame: Arc::new(AtomicI32::new(0)),
                event_rx,
                telemetry_rx,
                event_wake: event_tx.wake.clone(),
                worker: None,
                local_client_id: HOST_CLIENT_ID,
                netpuncher_state: Arc::new(Mutex::new(NetworkNetpuncherState::default())),
                role: NetworkRole::Host,
                client_status: ClientStatusState::default(),
                league_start_response: None,
                league_start_failure: None,
                league_runtime_available: AtomicBool::new(false),
                league_record_runtime: None,
                network_io_statistics: clonk_network::NetworkIoStatistics::new(0),
                control_send_time: clonk_network::ControlSendTimeSnapshot::default(),
                test_runtime_client_states: Arc::new(Mutex::new(Vec::new())),
                test_lobby_client_telemetry: Arc::new(Mutex::new(None)),
            },
            event_tx,
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub_for_client_id(local_client_id: ClientId) -> (Self, NetworkEventSender) {
        let (command_tx, _command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                control_tick_tx: tokio_mpsc::unbounded_channel().0,
                control_performance_tx: tokio_mpsc::unbounded_channel().0,
                control_tick_probe: Mutex::new(None),
                current_frame: Arc::new(AtomicI32::new(0)),
                event_rx,
                telemetry_rx,
                event_wake: event_tx.wake.clone(),
                worker: None,
                local_client_id,
                netpuncher_state: Arc::new(Mutex::new(NetworkNetpuncherState::default())),
                role: NetworkRole::Client,
                client_status: ClientStatusState::default(),
                league_start_response: None,
                league_start_failure: None,
                league_runtime_available: AtomicBool::new(false),
                league_record_runtime: None,
                network_io_statistics: clonk_network::NetworkIoStatistics::new(0),
                control_send_time: clonk_network::ControlSendTimeSnapshot::default(),
                test_runtime_client_states: Arc::new(Mutex::new(Vec::new())),
                test_lobby_client_telemetry: Arc::new(Mutex::new(None)),
            },
            event_tx,
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub_with_league_record_stream(endpoint: String) -> (Self, NetworkEventSender) {
        let (mut manager, event_tx) = Self::test_stub();
        manager.league_record_runtime = Some(
            spawn_league_record_runtime(
                endpoint,
                clonk_network::LeagueHttpTransportConfig::default(),
            )
            .expect("build test league record runtime"),
        );
        (manager, event_tx)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub_with_commands() -> (Self, NetworkEventSender, TestNetworkCommands) {
        Self::test_stub_with_commands_for_client_id(HOST_CLIENT_ID)
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub_with_commands_for_client_id(
        local_client_id: ClientId,
    ) -> (Self, NetworkEventSender, TestNetworkCommands) {
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (control_performance_tx, control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (_telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        (
            Self {
                command_tx,
                control_tick_tx: tokio_mpsc::unbounded_channel().0,
                control_performance_tx,
                control_tick_probe: Mutex::new(None),
                current_frame: Arc::new(AtomicI32::new(0)),
                event_rx,
                telemetry_rx,
                event_wake: event_tx.wake.clone(),
                worker: None,
                local_client_id,
                netpuncher_state: Arc::new(Mutex::new(NetworkNetpuncherState::default())),
                role: if local_client_id == HOST_CLIENT_ID {
                    NetworkRole::Host
                } else {
                    NetworkRole::Client
                },
                client_status: ClientStatusState::default(),
                league_start_response: None,
                league_start_failure: None,
                league_runtime_available: AtomicBool::new(false),
                league_record_runtime: None,
                network_io_statistics: clonk_network::NetworkIoStatistics::new(0),
                control_send_time: clonk_network::ControlSendTimeSnapshot::default(),
                test_runtime_client_states: Arc::new(Mutex::new(Vec::new())),
                test_lobby_client_telemetry: Arc::new(Mutex::new(None)),
            },
            event_tx,
            TestNetworkCommands {
                command_rx,
                control_performance_rx,
            },
        )
    }

    #[cfg(any(test, feature = "test-hooks"))]
    pub fn test_stub_with_league_commands_for_client_id(
        local_client_id: ClientId,
    ) -> (Self, NetworkEventSender, TestNetworkCommands) {
        let (manager, events, commands) =
            Self::test_stub_with_commands_for_client_id(local_client_id);
        manager
            .league_runtime_available
            .store(true, Ordering::Release);
        (manager, events, commands)
    }
}

impl Drop for NetworkManager {
    fn drop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        let _ = self.command_tx.blocking_send(NetworkCommand::Shutdown);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

pub fn apply_league_start_response_to_parameters(
    parameters: &mut clonk_network::JoinGameParametersEnvelope,
    response: &clonk_network::LeagueStartResponse,
) -> std::result::Result<Option<usize>, String> {
    let max_players = (response.max_players != 0)
        .then(|| usize::try_from(response.max_players))
        .transpose()
        .map_err(|_| {
            format!(
                "league Start MaxPlayers {} is outside the supported range",
                response.max_players
            )
        })?;
    parameters.league = response.league.clone();
    if let Some(seed) = response.seed {
        parameters.random_seed = seed;
    }
    if max_players.is_some() {
        parameters.max_players = response.max_players;
    }
    if response.league.is_empty() {
        parameters.league_address = clonk_engine::LegacyCString::default();
    }
    Ok(max_players)
}

fn apply_league_start_response_to_reference(
    reference: &clonk_network::HostGameReference,
    response: &clonk_network::LeagueStartResponse,
) -> std::result::Result<clonk_network::HostGameReference, String> {
    let mut parameters = reference.parameters().clone();
    apply_league_start_response_to_parameters(&mut parameters, response)?;
    reference
        .replacing_parameters(parameters)
        .map_err(|error| format!("league Start settings are invalid: {error}"))
}

/// Build the identity which a compensating `End` must send after the server
/// has committed `Start`, even when the local admission layer rejects one of
/// the returned values.
///
/// In particular, admission converts `MaxPlayers` to `usize` and generated
/// landscapes may reject an assigned seed. Neither local failure rewinds the
/// server session. C++ updates the synchronized parameters before ending that
/// session, so cleanup must carry every raw Start field that fits the wire
/// reference, including the server-assigned `RandomSeed`.
fn league_cleanup_reference_after_start(
    reference: &clonk_network::HostGameReference,
    response: &clonk_network::LeagueStartResponse,
) -> std::result::Result<clonk_network::HostGameReference, String> {
    let mut parameters = reference.parameters().clone();
    parameters.league = response.league.clone();
    if let Some(seed) = response.seed {
        parameters.random_seed = seed;
    }
    if response.max_players != 0 {
        parameters.max_players = response.max_players;
    }
    if response.league.is_empty() {
        parameters.league_address = clonk_engine::LegacyCString::default();
    }
    reference
        .replacing_parameters(parameters)
        .map_err(|error| format!("cannot build league cleanup reference: {error}"))
}

async fn register_league_host(
    config: PreparedLeagueHostConfig,
    reference: &clonk_network::HostGameReference,
    event_tx: NetworkEventSender,
) -> Result<(clonk_network::LeagueStartResponse, LeagueRuntimeHandle)> {
    let transport = clonk_network::LeagueHttpPostTransport::cpp_default()
        .map_err(|error| anyhow!("cannot initialise league HTTP transport: {error}"))?;
    let request = clonk_network::encode_league_start_request(reference, league_checksum_start())
        .map_err(|error| anyhow!("cannot encode league Start request: {error}"))?;
    let reply = transport
        .post(&config.endpoint, &request, &config.transport)
        .await
        .map_err(|error| anyhow!("league Start request failed: {error}"))?;
    let mut session = clonk_network::LeagueHostSession::new();
    let response = session
        .accept_start_response(&reply)
        .map_err(|error| anyhow!("league Start reply was rejected: {error}"))?;
    let (runtime, command_rx, gate) = league_runtime_channels();
    tokio::spawn(run_league_runtime(
        LeagueRuntimeState {
            heartbeat: Some(clonk_network::LeagueHeartbeat::new(
                config.update_period_secs,
            )),
            config,
            transport,
            session: Some(session),
            end_sent: false,
            projected_gains: HashMap::new(),
            fbids: clonk_network::LeagueFbidRegistry::new(),
        },
        command_rx,
        gate,
        event_tx,
    ));
    Ok((response, runtime))
}

fn spawn_league_client(
    endpoint: String,
    transport_config: clonk_network::LeagueHttpTransportConfig,
    event_tx: NetworkEventSender,
) -> Result<LeagueRuntimeHandle> {
    let transport =
        clonk_network::LeagueHttpPostTransport::for_backend(transport_config.http_backend)
            .map_err(|error| anyhow!("cannot initialise league HTTP transport: {error}"))?;
    let (runtime, command_rx, gate) = league_runtime_channels();
    tokio::spawn(run_league_runtime(
        LeagueRuntimeState {
            config: PreparedLeagueHostConfig {
                endpoint,
                transport: transport_config,
                update_period_secs: 0,
                league_server_signup: false,
            },
            transport,
            session: None,
            heartbeat: None,
            end_sent: false,
            projected_gains: HashMap::new(),
            fbids: clonk_network::LeagueFbidRegistry::new(),
        },
        command_rx,
        gate,
        event_tx,
    ));
    Ok(runtime)
}

fn league_runtime_channels() -> (
    LeagueRuntimeHandle,
    tokio_mpsc::Receiver<LeagueRuntimeCommand>,
    Arc<Mutex<LeagueRuntimeGate>>,
) {
    let (command_tx, command_rx) = tokio_mpsc::channel(8);
    let gate = Arc::new(Mutex::new(LeagueRuntimeGate::default()));
    (
        LeagueRuntimeHandle {
            command_tx,
            gate: Arc::clone(&gate),
        },
        command_rx,
        gate,
    )
}

async fn finish_league_runtime_attempt(
    runtime: &LeagueRuntimeHandle,
    reference: clonk_network::HostGameReference,
    record: Option<clonk_network::LeagueEndRecord>,
) -> std::result::Result<LeagueEndAttempt, String> {
    let (completion, completed) = tokio::sync::oneshot::channel();
    runtime
        .send_priority(LeagueRuntimeCommand::End {
            reference,
            record,
            completion,
        })
        .await
        .map_err(|_| "league runtime is unavailable".to_string())?;
    completed
        .await
        .map_err(|_| "league runtime ended before finishing the game".to_string())
}

async fn finish_league_runtime(
    runtime: &LeagueRuntimeHandle,
    reference: clonk_network::HostGameReference,
    record: Option<clonk_network::LeagueEndRecord>,
) -> std::result::Result<Option<clonk_network::LeagueRoundResultsPacket>, String> {
    let mut last_rejected_players = Vec::new();
    for attempt in 0..10 {
        match finish_league_runtime_attempt(runtime, reference.clone(), record.clone()).await? {
            LeagueEndAttempt::Finished(packet) => return Ok(packet),
            LeagueEndAttempt::Rejected(packet) if attempt < 9 => {
                last_rejected_players = packet.players;
                continue;
            }
            LeagueEndAttempt::Rejected(mut packet) => {
                let error =
                    clonk_resources::decode_legacy_script_text(packet.result_string.as_bytes());
                packet.result_string =
                    league_end_failure_message(LeagueEndFailurePhase::Send, &error);
                return finalize_league_end_failure_runtime(runtime, packet).await;
            }
            LeagueEndAttempt::Retryable { phase, error: _ }
                if phase == LeagueEndFailurePhase::Send && attempt < 9 =>
            {
                continue;
            }
            LeagueEndAttempt::Retryable { phase, error } => {
                return finalize_league_end_failure_runtime(
                    runtime,
                    clonk_network::LeagueRoundResultsPacket {
                        success: false,
                        result_string: league_end_failure_message(phase, &error),
                        players: last_rejected_players,
                    },
                )
                .await;
            }
        }
    }
    unreachable!("the fixed league End retry loop always returns")
}

async fn finalize_league_end_failure_runtime(
    runtime: &LeagueRuntimeHandle,
    packet: clonk_network::LeagueRoundResultsPacket,
) -> std::result::Result<Option<clonk_network::LeagueRoundResultsPacket>, String> {
    let (completion, completed) = tokio::sync::oneshot::channel();
    runtime
        .send_priority(LeagueRuntimeCommand::FinalizeEndFailure { packet, completion })
        .await
        .map_err(|_| "league runtime is unavailable".to_string())?;
    completed
        .await
        .map_err(|_| "league runtime ended before finalizing the game".to_string())
}

async fn authenticate_league_runtime_player(
    state: &LeagueRuntimeState,
    auth: &clonk_network::LeagueAuthRequestHead,
    player: &clonk_engine::ControlPlayerInfoEntry,
) -> std::result::Result<clonk_network::LeagueAuthResponse, String> {
    let player_info = clonk_network::encode_league_player_info_section(player)
        .map_err(|error| error.to_string())?;
    let request =
        clonk_network::encode_league_auth_request(auth, &player_info, league_checksum_start())
            .map_err(|error| error.to_string())?;
    let reply = state
        .transport
        .post(&state.config.endpoint, &request, &state.config.transport)
        .await
        .map_err(|error| error.to_string())?;
    Ok(clonk_network::decode_league_auth_response(&reply))
}

async fn check_league_runtime_player(
    state: &LeagueRuntimeState,
    player: &clonk_engine::ControlPlayerInfoEntry,
) -> std::result::Result<clonk_network::LeagueJoinResponse, String> {
    let csid = state
        .session
        .as_ref()
        .and_then(clonk_network::LeagueHostSession::csid)
        .cloned()
        .ok_or_else(|| "league host session has no CSID".to_string())?;
    let player_info = clonk_network::encode_league_player_info_section(player)
        .map_err(|error| error.to_string())?;
    let request = clonk_network::encode_league_join_request(
        &clonk_network::LeagueJoinRequestHead {
            csid,
            auid: player.auth_id.clone(),
        },
        &player_info,
        league_checksum_start(),
    )
    .map_err(|error| error.to_string())?;
    let reply = state
        .transport
        .post(&state.config.endpoint, &request, &state.config.transport)
        .await
        .map_err(|error| error.to_string())?;
    Ok(clonk_network::decode_league_join_response(&reply))
}

async fn run_league_runtime(
    mut state: LeagueRuntimeState,
    mut command_rx: tokio_mpsc::Receiver<LeagueRuntimeCommand>,
    gate: Arc<Mutex<LeagueRuntimeGate>>,
    event_tx: NetworkEventSender,
) {
    while let Some(command) = command_rx.recv().await {
        let priority = !matches!(&command, LeagueRuntimeCommand::Update { .. });
        let _gate_lease = LeagueRuntimeGateLease {
            gate: Arc::clone(&gate),
            priority,
        };
        match command {
            LeagueRuntimeCommand::AuthenticatePlayer {
                auth,
                player,
                completion,
            } => {
                let result = authenticate_league_runtime_player(&state, &auth, &player).await;
                if let Ok(response) = &result {
                    if response.is_success() && !response.auid.is_empty() {
                        state
                            .fbids
                            .insert(response.account.clone(), response.fbid.clone());
                    }
                }
                let _ = completion.send(result);
            }
            LeagueRuntimeCommand::CheckPlayer { player, completion } => {
                let result = check_league_runtime_player(&state, &player).await;
                let _ = completion.send(result);
            }
            LeagueRuntimeCommand::Update { now, reference } => {
                if state.end_sent {
                    continue;
                }
                let (Some(session), Some(heartbeat)) =
                    (state.session.as_ref(), state.heartbeat.as_mut())
                else {
                    continue;
                };
                if !heartbeat.is_due(now) {
                    continue;
                }
                let request =
                    match session.encode_update_request(&reference, league_checksum_start()) {
                        Ok(request) => request,
                        Err(error) => {
                            tracing::error!(%error, "failed to encode league Update request");
                            continue;
                        }
                    };
                heartbeat.update_dispatched(now);
                match state
                    .transport
                    .post(&state.config.endpoint, &request, &state.config.transport)
                    .await
                    .map_err(|error| error.to_string())
                    .and_then(|reply| {
                        clonk_network::decode_league_update_response(&reply)
                            .map_err(|error| error.to_string())
                    }) {
                    Ok(response) => {
                        let mut response_gains = HashMap::new();
                        for player in &response.player_infos.players {
                            response_gains
                                .entry(player.id)
                                .or_insert(player.league_projected_gain);
                        }
                        state.projected_gains.extend(response_gains);
                        let _ = event_tx.send(NetworkEvent::LeagueUpdate(response));
                    }
                    Err(error) => tracing::error!(%error, "league Update failed"),
                }
            }
            LeagueRuntimeCommand::End {
                reference,
                record,
                completion,
            } => {
                if state.end_sent {
                    let _ = completion.send(LeagueEndAttempt::Finished(None));
                    continue;
                }
                let Some(session) = state.session.as_ref() else {
                    let _ = completion.send(LeagueEndAttempt::Finished(None));
                    continue;
                };
                let reference =
                    match reference_with_projected_gains(&reference, &state.projected_gains) {
                        Ok(reference) => reference,
                        Err(error) => {
                            tracing::error!(%error, "failed to refresh league End reference");
                            reference
                        }
                    };
                let request = match session.encode_end_request(
                    &reference,
                    record.as_ref(),
                    league_checksum_start(),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        let _ = completion.send(LeagueEndAttempt::Retryable {
                            phase: LeagueEndFailurePhase::Start,
                            error: error.to_string(),
                        });
                        continue;
                    }
                };
                let reply = match state
                    .transport
                    .post(&state.config.endpoint, &request, &state.config.transport)
                    .await
                {
                    Ok(reply) => reply,
                    Err(error) => {
                        let _ = completion.send(LeagueEndAttempt::Retryable {
                            phase: LeagueEndFailurePhase::Send,
                            error: error.to_string(),
                        });
                        continue;
                    }
                };
                match clonk_network::decode_league_end_response(&reply) {
                    Ok(response) => {
                        state.end_sent = true;
                        let result_message = if state.config.league_server_signup {
                            "League: evaluation successful."
                        } else {
                            "Internet game evaluated."
                        };
                        let _ = completion.send(LeagueEndAttempt::Finished(Some(
                            clonk_network::LeagueRoundResultsPacket {
                                success: true,
                                result_string: legacy_runtime_message(result_message),
                                players: response.players,
                            },
                        )));
                    }
                    Err(clonk_network::LeagueResponseDecodeError::EndRejected(response)) => {
                        let _ = completion.send(LeagueEndAttempt::Rejected(
                            clonk_network::LeagueRoundResultsPacket {
                                success: false,
                                result_string: response.head.message,
                                players: response.players,
                            },
                        ));
                    }
                    Err(error) => {
                        let _ = completion.send(LeagueEndAttempt::Rejected(
                            clonk_network::LeagueRoundResultsPacket {
                                success: false,
                                result_string: legacy_runtime_message(&error.to_string()),
                                players: Vec::new(),
                            },
                        ));
                    }
                }
            }
            LeagueRuntimeCommand::FinalizeEndFailure { packet, completion } => {
                if state.end_sent {
                    let _ = completion.send(None);
                    continue;
                }
                state.end_sent = true;
                let _ = completion.send(Some(packet));
            }
            LeagueRuntimeCommand::ReportDisconnect {
                reason,
                players,
                fbids,
                completion,
            } => {
                let mut fbids_with_authenticated_players = state.fbids.clone();
                fbids_with_authenticated_players.extend_from(&fbids);
                let csid = state
                    .session
                    .as_ref()
                    .and_then(clonk_network::LeagueHostSession::csid)
                    .cloned()
                    .unwrap_or_default();
                let request = match clonk_network::encode_league_report_disconnect_request(
                    &csid,
                    reason,
                    &players,
                    &fbids_with_authenticated_players,
                    league_checksum_start(),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        tracing::error!(%error, "failed to encode league disconnect report");
                        let _ = completion.send(Err(error.to_string()));
                        continue;
                    }
                };
                match state
                    .transport
                    .post(&state.config.endpoint, &request, &state.config.transport)
                    .await
                {
                    Ok(reply) => {
                        let response = clonk_network::decode_league_auth_response(&reply);
                        if !response.is_success() {
                            let message = response.message.to_string_lossy().into_owned();
                            tracing::error!(
                                message = %message,
                                "league disconnect report was rejected"
                            );
                            let _ = completion.send(Err(message));
                        } else {
                            let _ = completion.send(Ok(()));
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "league disconnect report failed");
                        let _ = completion.send(Err(error.to_string()));
                    }
                }
            }
            LeagueRuntimeCommand::Invalidate => {
                if let Some(heartbeat) = state.heartbeat.as_mut() {
                    heartbeat.invalidate_reference();
                }
            }
        }
    }
}

fn reference_with_projected_gains(
    reference: &clonk_network::HostGameReference,
    gains: &HashMap<i32, i32>,
) -> std::result::Result<clonk_network::HostGameReference, clonk_network::HostGameReferenceError> {
    if gains.is_empty() {
        return Ok(reference.clone());
    }
    let mut parameters = reference.parameters().clone();
    for client in &mut parameters.player_infos.clients {
        for player in &mut client.players {
            if let Some(&gain) = gains.get(&player.id) {
                player.league_projected_gain = gain;
            }
        }
    }
    reference.replacing_parameters(parameters)
}

fn legacy_runtime_message(message: &str) -> clonk_engine::LegacyCString {
    let mut bytes = clonk_resources::encode_legacy_script_text(message)
        .unwrap_or_else(|| message.as_bytes().to_vec());
    bytes.retain(|byte| *byte != 0);
    clonk_engine::LegacyCString::from_bytes(bytes)
        .expect("filtered runtime league message contains no interior NUL")
}

fn league_end_failure_message(
    phase: LeagueEndFailurePhase,
    error: &str,
) -> clonk_engine::LegacyCString {
    let error = if error.is_empty() {
        "Empty reply"
    } else {
        error
    };
    let prefix = match phase {
        LeagueEndFailurePhase::Start => "Could not finish game",
        LeagueEndFailurePhase::Send => "Could not send game result",
    };
    legacy_runtime_message(&format!("{prefix}: {error}"))
}

const DEFAULT_NETPUNCHER_PORT: u16 = 11_115;

async fn resolve_netpuncher_addresses(address: &str) -> Vec<SocketAddr> {
    if address.is_empty() {
        return Vec::new();
    }
    let Some((host, port)) = netpuncher_lookup_target(address) else {
        tracing::warn!(%address, "invalid netpuncher address");
        return Vec::new();
    };
    let resolved = match tokio::net::lookup_host((host.as_str(), port)).await {
        Ok(resolved) => resolved.collect::<Vec<_>>(),
        Err(error) => {
            tracing::warn!(%error, %address, "cannot resolve netpuncher address");
            return Vec::new();
        }
    };
    normalize_resolved_netpuncher_addresses(resolved)
}

fn netpuncher_lookup_target(address: &str) -> Option<(String, u16)> {
    if let Ok(mut address) = address.parse::<SocketAddr>() {
        if address.port() == 0 {
            address.set_port(DEFAULT_NETPUNCHER_PORT);
        }
        return Some((address.ip().to_string(), address.port()));
    }
    if let Ok(ip) = address.parse::<std::net::IpAddr>() {
        return Some((ip.to_string(), DEFAULT_NETPUNCHER_PORT));
    }
    if let Some(bracketed) = address.strip_prefix('[') {
        if let Some((host, suffix)) = bracketed.split_once(']') {
            let port = match suffix {
                "" => DEFAULT_NETPUNCHER_PORT,
                suffix => {
                    let port = suffix.strip_prefix(':')?.parse::<u16>().ok()?;
                    if port == 0 {
                        DEFAULT_NETPUNCHER_PORT
                    } else {
                        port
                    }
                }
            };
            return Some((host.to_string(), port));
        }
        return None;
    }
    if let Some((host, port)) = address.rsplit_once(':') {
        if !host.contains(':') {
            return port.parse::<u16>().ok().map(|port| {
                (
                    host.to_string(),
                    if port == 0 {
                        DEFAULT_NETPUNCHER_PORT
                    } else {
                        port
                    },
                )
            });
        }
    }
    Some((address.to_string(), DEFAULT_NETPUNCHER_PORT))
}

fn normalize_resolved_netpuncher_addresses(
    resolved: impl IntoIterator<Item = SocketAddr>,
) -> Vec<SocketAddr> {
    let mut ipv4 = None;
    let mut ipv6 = None;
    for mut address in resolved {
        if address.port() == 0 {
            address.set_port(DEFAULT_NETPUNCHER_PORT);
        }
        let family = if address.is_ipv4() {
            &mut ipv4
        } else {
            &mut ipv6
        };
        if family.is_none() {
            *family = Some(address);
        }
    }
    ipv4.into_iter().chain(ipv6).collect()
}

fn host_registration_addresses(
    tcp_address: Option<SocketAddr>,
    configured_tcp_port: Option<u16>,
    udp_address: Option<SocketAddr>,
    configured_udp_port: Option<u16>,
) -> Vec<NetworkAddress> {
    let mut addresses = Vec::with_capacity(2);
    if let Some(tcp_address) = tcp_address {
        let port = configured_tcp_port.unwrap_or(tcp_address.port());
        if port != 0 {
            addresses.push(NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::new(tcp_address.ip(), port),
            ));
        }
    }
    if let Some(udp_address) = udp_address {
        let port = configured_udp_port.unwrap_or(udp_address.port());
        if port != 0 {
            addresses.push(NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::new(udp_address.ip(), port),
            ));
        }
    }
    addresses
}

// These independently owned channels form the network-thread boundary; a
// wrapper would only move the same ownership list into an opaque aggregate.
#[allow(clippy::too_many_arguments)]
async fn run_worker(
    mode: WorkerMode,
    mut command_rx: tokio_mpsc::Receiver<NetworkCommand>,
    mut control_tick_rx: tokio_mpsc::UnboundedReceiver<ControlTickProbe>,
    mut control_performance_rx: tokio_mpsc::UnboundedReceiver<ControlPerformanceEvent>,
    event_tx: NetworkEventSender,
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<std::result::Result<NetworkWorkerReady, NetworkStartError>>,
    netpuncher_state: Arc<Mutex<NetworkNetpuncherState>>,
    current_frame: Arc<AtomicI32>,
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
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                netpuncher_state,
            )
            .await
        }
        WorkerMode::Client {
            settings,
            local_owner,
            startup_cancellation,
        } => {
            run_client_worker(
                settings,
                local_owner,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                netpuncher_state,
                current_frame,
                startup_cancellation,
            )
            .await
        }
    }
}

// Keep host worker channels explicit so their ownership and shutdown behavior
// remain visible at every production and test entry point.
#[allow(clippy::too_many_arguments)]
async fn run_host_worker(
    settings: HostSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    control_tick_rx: &mut tokio_mpsc::UnboundedReceiver<ControlTickProbe>,
    control_performance_rx: &mut tokio_mpsc::UnboundedReceiver<ControlPerformanceEvent>,
    event_tx: NetworkEventSender,
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<std::result::Result<NetworkWorkerReady, NetworkStartError>>,
    netpuncher_state: Arc<Mutex<NetworkNetpuncherState>>,
) -> Result<()> {
    let host_name =
        clonk_engine::LegacyCString::from_bytes(settings.player_name.as_bytes().to_vec())
            .ok_or_else(|| anyhow!("host player name contains an interior NUL"))?;
    let mut prepared = settings.prepared.clone();
    let mut host_config = match prepared.as_ref() {
        Some(prepared) => match prepared.claim_host_config() {
            Ok(config) => config,
            Err(error) => {
                let message = format!("prepared host launch rejected: {error}");
                let _ = local_id_tx.send(Err(message.clone().into()));
                return Err(anyhow!(message));
            }
        },
        None => HostConfig {
            backlog_limit: 256,
            max_players: 8,
            resync_interval: Duration::from_millis(200),
            resync_cooldown: Duration::from_secs(2),
            start_tick: 0,
            local_core: clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: host_name.clone(),
                nick: host_name.clone(),
                lobby_ready: false,
            },
            group_maker: host_name,
            // A C++ client must not be admitted until the selected scenario,
            // game resources, and synchronized dynamic are represented by real
            // C4Network2ResCore values and can be served by the resource layer.
            allow_join: false,
            initial_join_snapshot: None,
            resource_directory: Some(PathBuf::from("Network")),
            ..HostConfig::default()
        },
    };
    let is_prepared = settings.prepared.is_some();
    let tcp_bind_address =
        (!is_prepared || host_config.configured_tcp_port != Some(0)).then_some(settings.bind_addr);
    let (listener, bound_addr, tcp_bind_error) = match tcp_bind_address {
        Some(bind_address) => match TcpListener::bind(bind_address).await {
            Ok(listener) => match listener.local_addr() {
                Ok(address) => (Some(listener), Some(address), None),
                Err(error) => (
                    None,
                    None,
                    Some(format!("failed to read bound host socket address: {error}")),
                ),
            },
            Err(error) => (
                None,
                None,
                Some(format!(
                    "failed to bind host socket at {bind_address}: {error}"
                )),
            ),
        },
        None => (None, None, None),
    };
    if is_prepared {
        // Production supplies the configured TCP port. A nonzero prepared
        // port may still be deliberately overridden with an ephemeral
        // HostSettings address by tests and embedders.
        host_config.configured_tcp_port = Some(bound_addr.map_or(0, |address| address.port()));
    }
    let udp_port = if is_prepared {
        host_config.configured_udp_port.unwrap_or_else(|| {
            bound_addr.map_or(settings.bind_addr.port(), |address| address.port())
        })
    } else {
        bound_addr.map_or(settings.bind_addr.port(), |address| address.port())
    };
    let udp_bind_address = (udp_port != 0).then_some(SocketAddr::new(
        bound_addr.map_or(settings.bind_addr.ip(), |address| address.ip()),
        udp_port,
    ));
    host_config.udp_bind_address = udp_bind_address;
    let udp_binding = HostUdpBinding::bind(&host_config);
    if listener.is_none() && udp_binding.local_addr().is_none() {
        let udp_bind_error = udp_binding.bind_error();
        let message = match (tcp_bind_error.as_deref(), udp_bind_error) {
            (Some(tcp), Some(udp)) => format!("{tcp}; {udp}"),
            (Some(tcp), None) => tcp.to_string(),
            (None, Some(udp)) => udp.to_string(),
            (None, None) => "no configured host transport is available".to_string(),
        };
        let _ = local_id_tx.send(Err(message.clone().into()));
        return Err(anyhow!(message));
    }
    let mut league_runtime = None;
    let mut league_record_runtime = None;
    let mut league_start_response = None;
    let mut league_start_failure = None;
    let mut latest_league_reference = None;
    if let Some(prepared_host) = prepared.as_mut() {
        if let Some(league_config) = prepared_host.league_config().cloned() {
            let record_transport_config = league_config.transport.clone();
            let registration_addresses = host_registration_addresses(
                bound_addr,
                host_config.configured_tcp_port,
                udp_binding.local_addr(),
                host_config.configured_udp_port,
            );
            let reference =
                match prepared_host.initial_host_game_reference(false, &registration_addresses) {
                    Ok(reference) => reference,
                    Err(error) => {
                        let message = format!("cannot build league Start reference: {error}");
                        let _ = local_id_tx.send(Err(message.clone().into()));
                        return Err(anyhow!(message));
                    }
                };
            // A refused registration is not a refused game: C4Network2::InitHost
            // answers a failed LeagueStart with DeinitLeague and keeps hosting,
            // and returns false only for the modal's Abort — which is all
            // `pCancel` reports — or a console build
            // (src/C4Network2.cpp:259-272,2292-2400). The caller owns that
            // choice, so the worker hosts unregistered and reports the refusal.
            let registration =
                match register_league_host(league_config, &reference, event_tx.clone()).await {
                    Ok(result) => Some(result),
                    Err(error) => {
                        let message = format!("league registration failed: {error}");
                        tracing::error!(error = %message, "hosting without a league registration");
                        league_start_failure = Some(message);
                        None
                    }
                };
            if let Some((response, runtime)) = registration {
                let cleanup_reference =
                    match league_cleanup_reference_after_start(&reference, &response) {
                        Ok(reference) => reference,
                        Err(error) => {
                            tracing::error!(
                                %error,
                                "falling back to the pre-Start league cleanup reference"
                            );
                            reference.clone()
                        }
                    };
                if let Err(error) = prepared_host.apply_league_start_response(&response) {
                    if let Err(cleanup_error) =
                        finish_league_runtime(&runtime, cleanup_reference, None).await
                    {
                        tracing::error!(%cleanup_error, "failed to end rejected league registration");
                    }
                    let message = format!("league Start settings are invalid: {error}");
                    let _ = local_id_tx.send(Err(message.clone().into()));
                    return Err(anyhow!(message));
                }
                let reference = match prepared_host
                    .initial_host_game_reference(false, &registration_addresses)
                {
                    Ok(reference) => reference,
                    Err(error) => {
                        if let Err(cleanup_error) =
                            finish_league_runtime(&runtime, cleanup_reference, None).await
                        {
                            tracing::error!(%cleanup_error, "failed to end unusable league registration");
                        }
                        let message = format!("cannot rebuild league Start reference: {error}");
                        let _ = local_id_tx.send(Err(message.clone().into()));
                        return Err(anyhow!(message));
                    }
                };
                if !prepared_host.stream_address().is_empty() {
                    let stream_address = clonk_resources::decode_legacy_script_text(
                        prepared_host.stream_address().as_bytes(),
                    );
                    league_record_runtime = match spawn_league_record_runtime(
                        stream_address,
                        record_transport_config,
                    ) {
                        Ok(runtime) => Some(runtime),
                        Err(error) => {
                            if let Err(cleanup_error) =
                                finish_league_runtime(&runtime, reference.clone(), None).await
                            {
                                tracing::error!(%cleanup_error, "failed to end league registration after stream setup failure");
                            }
                            let message =
                                format!("cannot initialise league record HTTP transport: {error}");
                            let _ = local_id_tx.send(Err(message.clone().into()));
                            return Err(anyhow!(message));
                        }
                    };
                }
                league_start_response = Some(response);
                latest_league_reference = Some(reference);
                league_runtime = Some(runtime);
            } else if let Err(error) = prepared_host.clear_live_league_registration() {
                let message = format!("cannot clear the refused league registration: {error}");
                let _ = local_id_tx.send(Err(message.clone().into()));
                return Err(anyhow!(message));
            }
            host_config = prepared_host.host_config().clone();
            host_config.configured_tcp_port = Some(bound_addr.map_or(0, |address| address.port()));
            host_config.udp_bind_address = udp_bind_address;
        }
    }
    if league_runtime.is_some() {
        if let Some(prepared_host) = prepared.as_ref() {
            let puncher_address = clonk_resources::decode_legacy_script_text(
                prepared_host.netpuncher_address().as_bytes(),
            );
            host_config.netpuncher_addresses = resolve_netpuncher_addresses(&puncher_address).await;
        }
    }
    let configured_tcp_port = host_config.configured_tcp_port;
    let configured_udp_port = host_config.configured_udp_port;
    let mut host = match start_host_with_bindings(listener, host_config, udp_binding).await {
        Ok(host) => host,
        Err(err) => {
            if let (Some(runtime), Some(reference)) =
                (league_runtime.as_ref(), latest_league_reference.take())
            {
                if let Err(error) = finish_league_runtime(runtime, reference, None).await {
                    tracing::error!(%error, "failed to end league registration after host startup failure");
                }
            }
            let message = format!("failed to start host session: {err}");
            let _ = local_id_tx.send(Err(message.clone().into()));
            return Err(anyhow!(message));
        }
    };
    let mut local_addresses = Vec::new();
    if let Some(bound_addr) = bound_addr {
        let advertised_tcp_port = configured_tcp_port.unwrap_or(bound_addr.port());
        if advertised_tcp_port != 0 {
            local_addresses.push(NetworkAddress::new(
                NetworkProtocol::Tcp,
                SocketAddr::new(bound_addr.ip(), advertised_tcp_port),
            ));
        }
    }
    if let Some(udp_addr) = host.udp_local_addr() {
        let advertised_udp_port = configured_udp_port.unwrap_or(udp_addr.port());
        if advertised_udp_port != 0 {
            local_addresses.push(NetworkAddress::new(
                NetworkProtocol::Udp,
                SocketAddr::new(udp_addr.ip(), advertised_udp_port),
            ));
        }
    }
    netpuncher_state.lock().local_addresses = local_addresses.clone();
    let _ = local_id_tx.send(Ok(NetworkWorkerReady {
        local_client_id: HOST_CLIENT_ID,
        control_send_time: host.control_send_time_snapshot(),
        league_start_response,
        league_start_failure,
        league_runtime_available: league_runtime.is_some(),
        league_record_runtime: league_record_runtime.clone(),
        network_io_statistics: host.io_statistics(),
    }));
    if let Some(error) = tcp_bind_error {
        let _ = event_tx.send(NetworkEvent::Error(error));
    }
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        client_id: HOST_CLIENT_ID,
        name: settings.player_name.clone(),
        kind: ParticipantKind::Player,
    });
    let mut host_events = host.take_event_receiver();
    let mut frame_builder = ControlFrameAccumulator::new(HOST_CLIENT_ID);
    let mut player_info_echo_provenance = VecDeque::new();
    let mut reset_client_performance_pending = false;

    let worker_result: Result<()> = async {
        loop {
            tokio::select! {
                Some(probe) = control_tick_rx.recv() => {
                    await_host_operation_while_forwarding_events(
                        host.control_tick_reached(
                            probe.tick,
                            probe.control_rate,
                            probe.target_fps,
                            probe.reached_at,
                        ),
                        &mut host_events,
                        local_owner,
                        &event_tx,
                        &telemetry_tx,
                        &mut player_info_echo_provenance,
                        &netpuncher_state,
                    )
                    .await?
                    .map_err(|error| anyhow!("host control-tick stamp failed: {error}"))?;
                }
                Some(event) = control_performance_rx.recv() => {
                    match event {
                        ControlPerformanceEvent::Reset => {
                            reset_client_performance_pending = true;
                        }
                        ControlPerformanceEvent::TickConsumed {
                            tick,
                            consumed_at,
                            client_ids,
                        } => {
                            let reset_performance =
                                std::mem::take(&mut reset_client_performance_pending);
                            await_host_operation_while_forwarding_events(
                                host.control_tick_consumed(
                                    tick,
                                    consumed_at,
                                    client_ids,
                                    reset_performance,
                                ),
                                &mut host_events,
                                local_owner,
                                &event_tx,
                                &telemetry_tx,
                                &mut player_info_echo_provenance,
                                &netpuncher_state,
                            )
                            .await?
                            .map_err(|error| {
                                anyhow!("host control-consumption bookkeeping failed: {error}")
                            })?;
                        }
                    }
                }
                maybe_event = host_events.recv() => {
                    match maybe_event {
                        Some(event) => handle_host_event(
                            event,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        ).await?,
                        None => {
                            return Err(anyhow!("host event stream ended"));
                        }
                    }
                }
                Some(command) = command_rx.recv() => {
                    match command {
                    NetworkCommand::InspectRuntimeConnections { completion } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.runtime_connections(),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::InspectLobbyClientTelemetry {
                        client_ids,
                        completion,
                    } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.lobby_client_telemetry(client_ids),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::InspectRuntimeClientStates {
                        tick,
                        probe,
                        completion,
                    } => {
                        // Probes are normally carried on a dedicated
                        // nonblocking channel. Drain its FIFO before the
                        // bundled current-tick probe so inspection cannot
                        // leapfrog an earlier completed tick's EWMA sample.
                        let reset_performance =
                            std::mem::take(&mut reset_client_performance_pending);
                        let inspection = async {
                            while let Ok(queued_probe) = control_tick_rx.try_recv() {
                                host.control_tick_reached(
                                    queued_probe.tick,
                                    queued_probe.control_rate,
                                    queued_probe.target_fps,
                                    queued_probe.reached_at,
                                )
                                .await
                                .map_err(|error| {
                                    format!(
                                        "host queued client-state cadence stamp failed: {error}"
                                    )
                                })?;
                            }
                            if let Some(probe) = probe {
                                host.control_tick_reached(
                                    probe.tick,
                                    probe.control_rate,
                                    probe.target_fps,
                                    probe.reached_at,
                                )
                                .await
                                .map_err(|error| {
                                    format!("host client-state cadence stamp failed: {error}")
                                })?;
                            }
                            host.runtime_client_states(tick, reset_performance)
                                .await
                                .map_err(|error| error.to_string())
                        };
                        let result = await_host_operation_while_forwarding_events(
                            inspection,
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?;
                        let _ = completion.send(result);
                    }
                    NetworkCommand::DisconnectRuntimeConnection {
                        connection_id,
                        completion,
                    } => {
                        let result = host
                            .disconnect_runtime_connection(connection_id)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::PublishRuntimeDynamic {
                        dynamic,
                        dynamic_tick,
                        parameters,
                        completion,
                    } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.publish_runtime_dynamic(*dynamic, dynamic_tick, *parameters),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::RemoveRuntimeDynamic { completion } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.remove_runtime_dynamic(),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::FailPendingJoinData { reason, completion } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.fail_pending_join_data(reason),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
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
                    NetworkCommand::BeginResourceDerive {
                        resource_id,
                        source_path,
                        ownership,
                        completion,
                    } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.begin_resource_derive(resource_id, source_path, ownership),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::FinishResourceDerive {
                        derivation,
                        completion,
                    } => {
                        let result = await_host_operation_while_forwarding_events(
                            host.finish_resource_derive(derivation),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await?
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
                        send_player_info_from_host(&host, info).await?;
                        player_info_echo_provenance.push_back(PlayerInfoEchoProvenance::Normal);
                    }
                    NetworkCommand::BroadcastPreexecutedPlayerInfo {
                        info,
                        join_players_on_echo,
                    } => {
                        let original = info.clone();
                        send_player_info_from_host(&host, info).await?;
                        player_info_echo_provenance.push_back(
                            PlayerInfoEchoProvenance::Preexecuted {
                                original,
                                join_players_on_echo,
                            },
                        );
                    }
                    NetworkCommand::BroadcastLeagueRoundResults(packet) => {
                        host.broadcast_league_round_results(packet)
                            .await
                            .map_err(|error| anyhow!("host league-result broadcast failed: {error}"))?;
                    }
                    NetworkCommand::LeagueAuthenticatePlayer { auth, player, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            if runtime.send_priority(LeagueRuntimeCommand::AuthenticatePlayer {
                                auth,
                                player,
                                completion: completion.clone(),
                            }).await.is_err() {
                                let _ = completion.send(Err("league runtime is unavailable".to_string()));
                            }
                        } else {
                            let _ = completion.send(Err("league runtime is unavailable".to_string()));
                        }
                    }
                    NetworkCommand::LeagueCheckPlayer { player, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            if runtime.send_priority(LeagueRuntimeCommand::CheckPlayer {
                                player,
                                completion: completion.clone(),
                            }).await.is_err() {
                                let _ = completion.send(Err("league runtime is unavailable".to_string()));
                            }
                        } else {
                            let _ = completion.send(Err("league runtime is unavailable".to_string()));
                        }
                    }
                    NetworkCommand::LeagueUpdate { now, reference } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            latest_league_reference = Some(reference.clone());
                            runtime.try_update(now, reference);
                        }
                    }
                    NetworkCommand::LeagueEnd { reference, record, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            latest_league_reference = Some(reference.clone());
                            let _ = completion.send(
                                finish_league_runtime_attempt(runtime, reference, record).await,
                            );
                        } else {
                            let _ = completion.send(Ok(LeagueEndAttempt::Finished(None)));
                        }
                    }
                    NetworkCommand::LeagueFinalizeEndFailure { packet, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            let _ = completion.send(
                                finalize_league_end_failure_runtime(runtime, packet).await,
                            );
                        } else {
                            let _ = completion.send(Ok(None));
                        }
                    }
                    NetworkCommand::LeagueReportDisconnect { reason, players, fbids, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            if runtime.send_priority(LeagueRuntimeCommand::ReportDisconnect {
                                reason,
                                players,
                                fbids,
                                completion: completion.clone(),
                            }).await.is_err() {
                                let _ = completion.send(Err("league runtime is unavailable".to_string()));
                            }
                        } else {
                            let _ = completion.send(Err("league runtime is unavailable".to_string()));
                        }
                    }
                    NetworkCommand::LeagueInvalidate => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            let _ = runtime
                                .send_priority(LeagueRuntimeCommand::Invalidate)
                                .await;
                        }
                    }
                    NetworkCommand::SetMasterserverSignup {
                        enabled,
                        config,
                        reference,
                        completion,
                        mut cancellation,
                        transition,
                    } => {
                        if enabled {
                            if league_runtime.is_some() {
                                if transition
                                    .compare_exchange(
                                        MASTERSERVER_SIGNUP_PENDING,
                                        MASTERSERVER_SIGNUP_FINISHED,
                                        Ordering::AcqRel,
                                        Ordering::Acquire,
                                    )
                                    .is_ok()
                                {
                                    latest_league_reference = Some(reference);
                                    let _ = completion.send(Ok(None));
                                } else {
                                    let _ = completion.send(Err(
                                        "masterserver signup was cancelled".to_string(),
                                    ));
                                }
                            } else {
                                let puncher_address =
                                    reference.summary().netpuncher_address.clone();
                                let live_attempt = async {
                                    let registration = register_league_host(
                                        config,
                                        &reference,
                                        event_tx.clone(),
                                    );
                                    let puncher_init = async {
                                        let addresses = resolve_netpuncher_addresses(
                                            &puncher_address,
                                        )
                                        .await;
                                        if addresses.is_empty() {
                                            Ok(())
                                        } else {
                                            host.init_netpunchers(addresses).await
                                        }
                                    };
                                    tokio::pin!(registration);
                                    tokio::pin!(puncher_init);
                                    let mut registered = None;
                                    let mut puncher_result = None;
                                    let cancelled = loop {
                                        tokio::select! {
                                            // Poll Start first. Once it dispatches and yields,
                                            // puncher initialization runs concurrently with the
                                            // response wait like C4Network2::LeagueStart.
                                            biased;
                                            _ = &mut cancellation => break true,
                                            result = &mut registration, if registered.is_none() => {
                                                let failed = result.is_err();
                                                registered = Some(result);
                                                if failed || puncher_result.is_some() {
                                                    break false;
                                                }
                                            }
                                            result = &mut puncher_init, if puncher_result.is_none() => {
                                                puncher_result = Some(result);
                                                if registered.is_some() {
                                                    break false;
                                                }
                                            }
                                        }
                                    };
                                    (registered, puncher_result, cancelled)
                                };
                                let (registered, puncher_result, cancelled) =
                                    await_host_operation_while_forwarding_events(
                                        live_attempt,
                                        &mut host_events,
                                        local_owner,
                                        &event_tx,
                                        &telemetry_tx,
                                        &mut player_info_echo_provenance,
                                        &netpuncher_state,
                                    )
                                    .await?;
                                if cancelled {
                                    if let Some(Ok((response, runtime))) = registered {
                                        // Cancellation may race a committed Start while the
                                        // concurrent puncher setup is still pending. Once the
                                        // server has created a session, C++ tears it down with
                                        // End before deinitialising the league client.
                                        let cleanup_reference =
                                            league_cleanup_reference_after_start(
                                                &reference,
                                                &response,
                                            )
                                            .unwrap_or_else(|error| {
                                                tracing::error!(
                                                    %error,
                                                    "falling back to the pre-Start cancelled cleanup reference"
                                                );
                                                reference.clone()
                                            });
                                        if let Err(cleanup_error) =
                                            await_host_operation_while_forwarding_events(
                                                finish_league_runtime(
                                                    &runtime,
                                                    cleanup_reference,
                                                    None,
                                                ),
                                                &mut host_events,
                                                local_owner,
                                                &event_tx,
                                                &telemetry_tx,
                                                &mut player_info_echo_provenance,
                                                &netpuncher_state,
                                            )
                                            .await?
                                        {
                                            tracing::error!(
                                                %cleanup_error,
                                                "failed to end cancelled live league registration"
                                            );
                                        }
                                    }
                                    latest_league_reference.take();
                                    let _ = completion.send(Err(
                                        "masterserver signup was cancelled".to_string(),
                                    ));
                                    continue;
                                }
                                let registered = registered
                                    .expect("uncancelled live signup has a Start result");
                                match registered {
                                    Err(error) => {
                                        latest_league_reference.take();
                                        if transition
                                            .compare_exchange(
                                                MASTERSERVER_SIGNUP_PENDING,
                                                MASTERSERVER_SIGNUP_FINISHED,
                                                Ordering::AcqRel,
                                                Ordering::Acquire,
                                            )
                                            .is_ok()
                                        {
                                            let _ = completion.send(Err(error.to_string()));
                                        } else {
                                            let _ = completion.send(Err(
                                                "masterserver signup was cancelled".to_string(),
                                            ));
                                        }
                                    }
                                    Ok((response, runtime)) => {
                                        let updated_reference =
                                            apply_league_start_response_to_reference(
                                                &reference,
                                                &response,
                                            );
                                        if let Err(error) = updated_reference.as_ref() {
                                            let cleanup_reference =
                                                match league_cleanup_reference_after_start(
                                                    &reference,
                                                    &response,
                                                ) {
                                                    Ok(reference) => reference,
                                                    Err(cleanup_reference_error) => {
                                                        tracing::error!(
                                                            error = %cleanup_reference_error,
                                                            "falling back to the pre-Start live cleanup reference"
                                                        );
                                                        reference.clone()
                                                    }
                                                };
                                            if let Err(cleanup_error) =
                                                await_host_operation_while_forwarding_events(
                                                    finish_league_runtime(
                                                        &runtime,
                                                        cleanup_reference,
                                                        None,
                                                    ),
                                                    &mut host_events,
                                                    local_owner,
                                                    &event_tx,
                                                    &telemetry_tx,
                                                    &mut player_info_echo_provenance,
                                                    &netpuncher_state,
                                                )
                                                .await?
                                            {
                                                tracing::error!(
                                                    %cleanup_error,
                                                    "failed to end invalid live league registration"
                                                );
                                            }
                                            if transition.compare_exchange(
                                                MASTERSERVER_SIGNUP_PENDING,
                                                MASTERSERVER_SIGNUP_FINISHED,
                                                Ordering::AcqRel,
                                                Ordering::Acquire,
                                            ).is_ok() {
                                                let _ = completion.send(Err(error.clone()));
                                            } else {
                                                let _ = completion.send(Err(
                                                    "masterserver signup was cancelled".to_string(),
                                                ));
                                            }
                                            continue;
                                        }
                                        if transition
                                            .compare_exchange(
                                                MASTERSERVER_SIGNUP_PENDING,
                                                MASTERSERVER_SIGNUP_FINISHED,
                                                Ordering::AcqRel,
                                                Ordering::Acquire,
                                            )
                                            .is_err()
                                        {
                                            let cleanup_reference =
                                                updated_reference.expect("checked Ok above");
                                            if let Err(cleanup_error) =
                                                await_host_operation_while_forwarding_events(
                                                    finish_league_runtime(
                                                        &runtime,
                                                        cleanup_reference,
                                                        None,
                                                    ),
                                                    &mut host_events,
                                                    local_owner,
                                                    &event_tx,
                                                    &telemetry_tx,
                                                    &mut player_info_echo_provenance,
                                                    &netpuncher_state,
                                                )
                                                .await?
                                            {
                                                tracing::error!(
                                                    %cleanup_error,
                                                    "failed to end cancelled live league registration"
                                                );
                                            }
                                            latest_league_reference.take();
                                            let _ = completion.send(Err(
                                                "masterserver signup was cancelled".to_string(),
                                            ));
                                            continue;
                                        }
                                        if let Some(Err(error)) = puncher_result {
                                            tracing::error!(
                                                %error,
                                                "live masterserver signup could not initialize the netpuncher"
                                            );
                                        }
                                        latest_league_reference =
                                            Some(updated_reference.expect("checked Ok above"));
                                        league_runtime = Some(runtime);
                                        let _ = completion.send(Ok(Some(response)));
                                    }
                                }
                            }
                        } else {
                            // A successful Start may have replaced synchronized
                            // fields such as RandomSeed. End must identify that
                            // committed server reference, even when the app is
                            // compensating for a rejected local application.
                            let end_reference =
                                latest_league_reference.take().unwrap_or(reference);
                            let result = if let Some(runtime) = league_runtime.take() {
                                let ending = async {
                                    let finish = finish_league_runtime(
                                        &runtime,
                                        end_reference,
                                        None,
                                    );
                                    tokio::pin!(finish);
                                    tokio::select! {
                                        biased;
                                        cancellation = &mut cancellation => {
                                            if cancellation.is_ok() {
                                                None
                                            } else {
                                                // The lobby-side handle was dropped during
                                                // teardown without explicitly cancelling this
                                                // committed cleanup. Match C++'s End-before-
                                                // Deinit ordering and finish the request before
                                                // the queued worker shutdown is observed.
                                                Some(finish.await)
                                            }
                                        },
                                        result = &mut finish => Some(result),
                                    }
                                };
                                await_host_operation_while_forwarding_events(
                                    ending,
                                    &mut host_events,
                                    local_owner,
                                    &event_tx,
                                    &telemetry_tx,
                                    &mut player_info_echo_provenance,
                                    &netpuncher_state,
                                )
                                .await?
                            } else {
                                Some(Ok(None))
                            };
                            if result.is_none()
                                || transition
                                    .compare_exchange(
                                        MASTERSERVER_SIGNUP_PENDING,
                                        MASTERSERVER_SIGNUP_FINISHED,
                                        Ordering::AcqRel,
                                        Ordering::Acquire,
                                    )
                                    .is_err()
                            {
                                let _ = completion.send(Err(
                                    "masterserver signup change was cancelled".to_string(),
                                ));
                                continue;
                            }
                            match result.expect("uncancelled signup disable has an End result") {
                                Ok(Some(packet)) => {
                                    if let Err(error) = await_host_operation_while_forwarding_events(
                                        host.broadcast_league_round_results(packet),
                                        &mut host_events,
                                        local_owner,
                                        &event_tx,
                                        &telemetry_tx,
                                        &mut player_info_echo_provenance,
                                        &netpuncher_state,
                                    )
                                    .await?
                                    {
                                        tracing::error!(
                                            %error,
                                            "host league-result broadcast failed while disabling signup"
                                        );
                                    }
                                    let _ = completion.send(Ok(None));
                                }
                                Ok(None) => {
                                    let _ = completion.send(Ok(None));
                                }
                                Err(error) => {
                                    let _ = completion.send(Err(error));
                                }
                            }
                        }
                    }
                    NetworkCommand::SubmitJoinPlayer { tick, join } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::JoinPlayer(join),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitRemovePlayer { tick, remove } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::RemovePlayer(remove),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitClientUpdate(update) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::ClientUpdate(update),
                        )?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host client-update submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitClientRemove(remove) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::ClientRemove(remove),
                        )?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host client-remove submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitControlSet(set) => {
                        let data = encode_control_entry_payload(&set.into_control_packet())?;
                        host.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("host control-set submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitDecidedControl { tick, control, sync } => {
                        if sync {
                            let data = encode_control_entry_payload(&control)?;
                            host.submit_packet(ControlDelivery::Sync, data)
                                .await
                                .map_err(|error| anyhow!("host decided-control submission failed: {error}"))?;
                        } else {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitInitScenarioPlayer { tick, selection } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::InitScenarioPlayer(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitSurrenderPlayer { tick, surrender } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::SurrenderPlayer(surrender),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitInternalPlayerScript { tick, control } => {
                        frame_builder.record_control(tick, control, current_millis());
                    }
                    NetworkCommand::SubmitMessage(message) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::Message(message),
                        )?;
                        host.submit_packet(ControlDelivery::Private, data)
                            .await
                            .map_err(|error| anyhow!("host message submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitVote(vote) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::Vote(vote),
                        )?;
                        host.submit_packet(ControlDelivery::Direct, data)
                            .await
                            .map_err(|error| anyhow!("host vote submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitVoteEnd(result) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::VoteEnd(result),
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
                    // Awaited, not fired and forgotten: the app queues Shutdown
                    // directly behind this, and only the resolved broadcast
                    // guarantees the notice is on every route before the host
                    // loop is torn down.
                    NetworkCommand::BroadcastHostRestarting { rejoin_seconds } => {
                        host.broadcast_host_restarting(rejoin_seconds)
                            .await
                            .map_err(|error| anyhow!("host restart notice failed: {error}"))?;
                    }
                    NetworkCommand::SubmitLocal { owner, event, tick } => {
                        if let Some(control) = control_packet_for_event(owner, event, HOST_CLIENT_ID) {
                            frame_builder.record_control(tick, control, current_millis());
                        }
                    }
                    NetworkCommand::SubmitPlayerCommand { tick, command } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::PlayerCommand(command),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::PlayerSelect(selection),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitScript { tick, script } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::Script(script),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitMessageBoardAnswer { tick, answer } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::MessageBoardAnswer(answer),
                            current_millis(),
                        );
                    }
                    NetworkCommand::SubmitSyncCheck { tick, check } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::SyncCheck(check),
                            current_millis(),
                        );
                    }
                    NetworkCommand::FinalizeTick { tick } => {
                        if let Some(frame) = frame_builder.finalize_tick(tick) {
                            send_frame_to_host(&host, frame).await?;
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
                    NetworkCommand::SetHostPassword {
                        password,
                        completion,
                    } => {
                        let password = (!password.is_empty()).then_some(password);
                        match host.set_password(password).await {
                            Ok(()) => {
                                let _ = completion.send(Ok(()));
                            }
                            Err(error) => {
                                let message = format!("host password change failed: {error}");
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
                    NetworkCommand::BeginGo {
                        status,
                        join_allowed,
                        completion,
                    } => {
                        let result = match await_host_operation_while_forwarding_events(
                            host.begin_go(status, join_allowed),
                            &mut host_events,
                            local_owner,
                            &event_tx,
                            &telemetry_tx,
                            &mut player_info_echo_provenance,
                            &netpuncher_state,
                        )
                        .await
                        {
                            Ok(Ok(())) => Ok(()),
                            Ok(Err(error)) => {
                                Err(format!("host Go transition failed: {error}"))
                            }
                            Err(error) => Err(format!("host Go transition failed: {error}")),
                        };
                        let failure = result.as_ref().err().cloned();
                        let _ = completion.send(result);
                        if let Some(message) = failure {
                            return Err(anyhow!(message));
                        }
                    }
                    NetworkCommand::StatusReachedCurrent => {
                        host.status_reached_current()
                            .await
                            .map_err(|err| anyhow!("host status arrival failed: {err}"))?;
                    }
                    NetworkCommand::StatusReached {
                        status,
                        actual_control_tick,
                    } => {
                        host.status_reached(status, actual_control_tick)
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
        Ok(())
    }
    .await;
    drop(host_events);

    if let (Some(runtime), Some(reference)) =
        (league_runtime.as_ref(), latest_league_reference.take())
    {
        match finish_league_runtime(runtime, reference, None).await {
            Ok(Some(packet)) => {
                if let Err(error) = host.broadcast_league_round_results(packet).await {
                    tracing::error!(%error, "failed to broadcast final league results during shutdown");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "failed to end league registration during shutdown")
            }
        }
    }
    if let Some(runtime) = league_record_runtime.as_ref() {
        shutdown_league_record_runtime(runtime).await;
    }
    host.shutdown().await.ok();
    worker_result
}

async fn await_host_operation_while_forwarding_events<F>(
    operation: F,
    host_events: &mut tokio_mpsc::Receiver<HostEvent>,
    local_owner: i32,
    event_tx: &NetworkEventSender,
    telemetry_tx: &SyncSender<NetworkEvent>,
    player_info_echo_provenance: &mut VecDeque<PlayerInfoEchoProvenance>,
    netpuncher_state: &Arc<Mutex<NetworkNetpuncherState>>,
) -> Result<F::Output>
where
    F: std::future::Future,
{
    tokio::pin!(operation);
    loop {
        tokio::select! {
            output = &mut operation => return Ok(output),
            maybe_event = host_events.recv() => {
                match maybe_event {
                    Some(event) => handle_host_event(
                        event,
                        local_owner,
                        event_tx,
                        telemetry_tx,
                        player_info_echo_provenance,
                        netpuncher_state,
                    ).await?,
                    None => return Err(anyhow!("host event stream ended")),
                }
            }
        }
    }
}

async fn handle_host_event(
    event: HostEvent,
    local_owner: i32,
    event_tx: &NetworkEventSender,
    _telemetry_tx: &SyncSender<NetworkEvent>,
    player_info_echo_provenance: &mut VecDeque<PlayerInfoEchoProvenance>,
    netpuncher_state: &Arc<Mutex<NetworkNetpuncherState>>,
) -> Result<()> {
    match event {
        HostEvent::LocalAddressesChanged { local_addresses } => {
            netpuncher_state.lock().local_addresses = local_addresses;
        }
        HostEvent::NetpuncherStateChanged {
            game_ids,
            local_addresses,
        } => {
            *netpuncher_state.lock() = NetworkNetpuncherState {
                local_addresses: local_addresses.clone(),
                game_ids,
            };
            let _ = event_tx.send(NetworkEvent::NetpuncherStateChanged {
                game_ids,
                local_addresses,
            });
        }
        HostEvent::StatusChanged(status) => {
            let _ = event_tx.send(NetworkEvent::HostStatusChanged(status));
        }
        HostEvent::StatusCommitted(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
        }
        HostEvent::StatusAck { client_id, status } => {
            let _ = event_tx.send(NetworkEvent::HostStatusAck { client_id, status });
        }
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
        HostEvent::ResourceProgress {
            resource_id,
            present_percent,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceProgress {
                resource_id,
                present_percent,
            });
        }
        HostEvent::ResourceComplete {
            resource_id,
            core,
            path,
            local,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path,
                local,
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
        HostEvent::ClientConnectionFailed { client_id } => {
            let _ = event_tx.send(NetworkEvent::PeerConnectionFailed { client_id });
        }
        HostEvent::JoinDataNeeded {
            client_id,
            current_control_tick,
        } => {
            let _ = event_tx.send(NetworkEvent::JoinDataNeeded {
                client_id,
                current_control_tick,
            });
        }
        HostEvent::UnhandledPacket {
            client_id,
            packet_type,
        } => {
            let status = format!("{packet_type:02x}");
            tracing::error!(?client_id, %status, "Unhandled packet");
        }
        HostEvent::RecoverableRouteDiagnostic { client_id, error } => {
            let _ = event_tx.send(NetworkEvent::RecoverableRouteDiagnostic { client_id, error });
        }
        HostEvent::TransportError { client_id, error } => {
            let _ = event_tx.send(NetworkEvent::TransportDiagnostic { client_id, error });
        }
        HostEvent::FatalError { error } => {
            let _ = event_tx.send(NetworkEvent::FatalError(error));
        }
        HostEvent::Direct {
            client_id,
            delivery,
            data,
        } => {
            let player_info = (client_id == clonk_network::BROADCAST_CLIENT_ID)
                .then(|| decode_control_entry_payload(&data).ok())
                .flatten()
                .and_then(|control| match control {
                    clonk_engine::ControlPacket::PlayerInfo(info) => Some(info),
                    _ => None,
                });
            let provenance = player_info
                .as_ref()
                .and_then(|_| player_info_echo_provenance.pop_front());
            match provenance {
                Some(PlayerInfoEchoProvenance::Preexecuted {
                    original,
                    join_players_on_echo,
                }) => {
                    let _ = event_tx.send(NetworkEvent::PreexecutedPlayerInfoEcho {
                        original,
                        info: player_info.expect("preexecuted provenance belongs to PlayerInfo"),
                        join_players_on_echo,
                    });
                }
                Some(PlayerInfoEchoProvenance::Normal) | None => {
                    handle_direct_packet(delivery, data, event_tx)?;
                }
            }
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

async fn wait_for_startup_cancellation(cancellation: Option<&NetworkStartupCancellation>) {
    match cancellation {
        Some(cancellation) => cancellation.cancelled().await,
        None => std::future::pending::<()>().await,
    }
}

// Keep client worker channels explicit so their ownership and shutdown behavior
// remain visible at every production and test entry point.
#[allow(clippy::too_many_arguments)]
async fn run_client_worker(
    settings: ClientSettings,
    local_owner: i32,
    command_rx: &mut tokio_mpsc::Receiver<NetworkCommand>,
    control_tick_rx: &mut tokio_mpsc::UnboundedReceiver<ControlTickProbe>,
    control_performance_rx: &mut tokio_mpsc::UnboundedReceiver<ControlPerformanceEvent>,
    event_tx: NetworkEventSender,
    telemetry_tx: SyncSender<NetworkEvent>,
    local_id_tx: mpsc::Sender<std::result::Result<NetworkWorkerReady, NetworkStartError>>,
    netpuncher_state: Arc<Mutex<NetworkNetpuncherState>>,
    current_frame_source: Arc<AtomicI32>,
    startup_cancellation: Option<NetworkStartupCancellation>,
) -> Result<()> {
    let player_name = settings.player_name.clone();
    let league_transport = settings.league_transport.clone();
    let tcp_enabled = settings.mesh_tcp_bind_address.is_some();
    let udp_enabled = settings.mesh_udp_bind_address.is_some();
    let mesh_punchers = if udp_enabled {
        tokio::select! {
            biased;
            _ = wait_for_startup_cancellation(startup_cancellation.as_ref()) => {
                let _ = local_id_tx.send(Err(NetworkStartError::Cancelled));
                return Ok(());
            }
            resolved = resolve_client_mesh_punchers(
                settings.netpuncher_address.as_deref(),
                settings.netpuncher_game_ids,
            ) => resolved,
        }
    } else {
        Vec::new()
    };
    let participant_kind = if settings.observer {
        ParticipantKind::Observer
    } else {
        ParticipantKind::Player
    };
    let mut client_config = ClientConfig::new(player_name.clone(), participant_kind)
        .with_compatibility_build(settings.compatibility_build)
        .with_group_maker(settings.group_maker)
        .with_password(settings.password)
        .with_resource_directory(settings.resource_directory)
        .with_local_resource_roots(settings.local_resource_roots)
        .with_max_resource_search_recursion(settings.max_resource_search_recursion)
        .with_mesh_punchers(mesh_punchers);
    if let Some(bind_address) = settings.mesh_tcp_bind_address {
        client_config = client_config.with_mesh_tcp_bind_address(bind_address);
    }
    if let Some(bind_address) = settings.mesh_udp_bind_address {
        client_config = client_config.with_mesh_udp_bind_address(bind_address);
    }
    if let Some(system_path) = settings.local_system_path {
        client_config = client_config.with_trusted_local_system_path(system_path);
    }
    let server_addresses = settings
        .server_addresses
        .into_iter()
        .filter(|address| client_join_protocol_enabled(address, tcp_enabled, udp_enabled));
    let client_result = tokio::select! {
        biased;
        _ = wait_for_startup_cancellation(startup_cancellation.as_ref()) => {
            let _ = local_id_tx.send(Err(NetworkStartError::Cancelled));
            return Ok(());
        }
        result = connect_client_addresses(server_addresses, client_config) => result,
    };
    let mut client = match client_result {
        Ok(client) => client,
        Err(error) => {
            let startup_error = client_startup_error(error);
            let detail = startup_error.to_string();
            let _ = local_id_tx.send(Err(startup_error));
            return Err(anyhow!(detail));
        }
    };
    if startup_cancellation
        .as_ref()
        .is_some_and(NetworkStartupCancellation::is_cancelled)
    {
        let _ = local_id_tx.send(Err(NetworkStartError::Cancelled));
        return Ok(());
    }
    let (client_id, initial_status, league_endpoint) =
        announce_connected_client(&mut client, player_name, &event_tx, &local_id_tx)?;
    let league_runtime = league_endpoint.and_then(|endpoint| {
        match spawn_league_client(endpoint, league_transport, event_tx.clone()) {
            Ok(runtime) => Some(runtime),
            Err(error) => {
                tracing::error!(%error, "league disconnect reporter is unavailable");
                None
            }
        }
    });
    let _ = local_id_tx.send(Ok(NetworkWorkerReady {
        local_client_id: client_id,
        control_send_time: client.control_send_time_snapshot(),
        league_start_response: None,
        league_start_failure: None,
        league_runtime_available: league_runtime.is_some(),
        league_record_runtime: None,
        network_io_statistics: client.io_statistics(),
    }));
    netpuncher_state.lock().local_addresses.clear();
    let mut client_events = client.take_event_receiver();
    let mut client_events_open = true;
    let mut frame_builder = ControlFrameAccumulator::new(client_id);
    let mut client_status = ClientStatusState::default();
    client_status.receive_request(initial_status);
    let mut client_activation = ClientActivationState::default();
    let mut rebase_pending_on_activation = false;
    let mut reset_client_performance_pending = false;

    loop {
        let activation_retry_at = client_activation.next_retry_at();
        tokio::select! {
            Some(probe) = control_tick_rx.recv() => {
                await_client_operation_while_forwarding_events(
                    client.control_tick_reached(probe.tick, probe.reached_at),
                    &mut client_events,
                    &mut client_status,
                    &mut client_activation,
                    &mut client_events_open,
                    local_owner,
                    client_id,
                    &event_tx,
                    &telemetry_tx,
                    &netpuncher_state,
                )
                .await?
                .map_err(|error| anyhow!("client control-tick stamp failed: {error}"))?;
            }
            Some(event) = control_performance_rx.recv() => {
                match event {
                    ControlPerformanceEvent::Reset => {
                        reset_client_performance_pending = true;
                    }
                    ControlPerformanceEvent::TickConsumed {
                        tick,
                        consumed_at,
                        client_ids,
                    } => {
                        let reset_performance =
                            std::mem::take(&mut reset_client_performance_pending);
                        await_client_operation_while_forwarding_events(
                            client.control_tick_consumed(
                                tick,
                                consumed_at,
                                client_ids,
                                reset_performance,
                            ),
                            &mut client_events,
                            &mut client_status,
                            &mut client_activation,
                            &mut client_events_open,
                            local_owner,
                            client_id,
                            &event_tx,
                            &telemetry_tx,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| {
                            anyhow!("client control-consumption bookkeeping failed: {error}")
                        })?;
                    }
                }
            }
            maybe_event = client_events.recv(), if client_events_open => {
                handle_client_worker_event(
                    maybe_event,
                    &mut client_status,
                    &mut client_activation,
                    &mut client_events_open,
                    local_owner,
                    client_id,
                    &event_tx,
                    &telemetry_tx,
                    &netpuncher_state,
                ).await?;
            }
            Some(command) = command_rx.recv() => {
                if !client_events_open {
                    let unavailable = "network client is disconnected".to_string();
                    match command {
                        NetworkCommand::InspectRuntimeConnections { completion } => {
                            let _ = completion.send(Err(unavailable.clone()));
                        }
                        NetworkCommand::InspectLobbyClientTelemetry { completion, .. } => {
                            let _ = completion.send(Err(unavailable.clone()));
                        }
                        NetworkCommand::InspectRuntimeClientStates { completion, .. } => {
                            let _ = completion.send(Err(unavailable.clone()));
                        }
                        NetworkCommand::DisconnectRuntimeConnection { completion, .. } => {
                            let _ = completion.send(Err(unavailable.clone()));
                        }
                        NetworkCommand::LeagueReportDisconnect {
                            reason,
                            players,
                            fbids,
                            completion,
                        } => {
                            if let Some(runtime) = league_runtime.as_ref() {
                                if runtime
                                    .send_priority(LeagueRuntimeCommand::ReportDisconnect {
                                        reason,
                                        players,
                                        fbids,
                                        completion: completion.clone(),
                                    })
                                    .await
                                    .is_err()
                                {
                                    let _ = completion.send(Err(
                                        "league runtime is unavailable".to_string(),
                                    ));
                                }
                            } else {
                                let _ = completion.send(Err(
                                    "league runtime is unavailable".to_string(),
                                ));
                            }
                        }
                        NetworkCommand::LeagueAuthenticatePlayer { completion, .. } => {
                            let _ = completion.send(Err(unavailable.clone()));
                        }
                        NetworkCommand::LeagueCheckPlayer { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::PublishRuntimeDynamic { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::RemoveRuntimeDynamic { completion } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::FailPendingJoinData { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::PublishPlayerResource { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::BeginResourceDerive { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::FinishResourceDerive { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::RemoveResource { completion, .. }
                        | NetworkCommand::BeginGo { completion, .. }
                        | NetworkCommand::SetJoinAllowed { completion, .. }
                        | NetworkCommand::SetHostPassword { completion, .. }
                        | NetworkCommand::GracefulPart { completion } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::SetMasterserverSignup { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::LeagueEnd { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::LeagueFinalizeEndFailure { completion, .. } => {
                            let _ = completion.send(Err(unavailable));
                        }
                        NetworkCommand::Shutdown => break,
                        _ => {}
                    }
                    continue;
                }
                match command {
                    NetworkCommand::InspectRuntimeConnections { completion } => {
                        let result = await_client_operation_while_forwarding_events(
                            client.runtime_connections(),
                            &mut client_events,
                            &mut client_status,
                            &mut client_activation,
                            &mut client_events_open,
                            local_owner,
                            client_id,
                            &event_tx,
                            &telemetry_tx,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::InspectLobbyClientTelemetry {
                        client_ids: lobby_client_ids,
                        completion,
                    } => {
                        let result = await_client_operation_while_forwarding_events(
                            client.lobby_client_telemetry(lobby_client_ids),
                            &mut client_events,
                            &mut client_status,
                            &mut client_activation,
                            &mut client_events_open,
                            local_owner,
                            client_id,
                            &event_tx,
                            &telemetry_tx,
                            &netpuncher_state,
                        )
                        .await?
                        .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::InspectRuntimeClientStates {
                        tick,
                        probe,
                        completion,
                    } => {
                        let reset_performance =
                            std::mem::take(&mut reset_client_performance_pending);
                        let inspection = async {
                            while let Ok(queued_probe) = control_tick_rx.try_recv() {
                                client
                                    .control_tick_reached(
                                        queued_probe.tick,
                                        queued_probe.reached_at,
                                    )
                                    .await
                                    .map_err(|error| {
                                        format!(
                                            "queued client-state cadence stamp failed: {error}"
                                        )
                                    })?;
                            }
                            if let Some(probe) = probe {
                                client
                                    .control_tick_reached(probe.tick, probe.reached_at)
                                    .await
                                    .map_err(|error| {
                                        format!("client-state cadence stamp failed: {error}")
                                    })?;
                            }
                            client
                                .runtime_client_states(tick, reset_performance)
                                .await
                                .map_err(|error| error.to_string())
                        };
                        let result = await_client_operation_while_forwarding_events(
                            inspection,
                            &mut client_events,
                            &mut client_status,
                            &mut client_activation,
                            &mut client_events_open,
                            local_owner,
                            client_id,
                            &event_tx,
                            &telemetry_tx,
                            &netpuncher_state,
                        )
                        .await?;
                        let _ = completion.send(result);
                    }
                    NetworkCommand::DisconnectRuntimeConnection {
                        connection_id,
                        completion,
                    } => {
                        let result = client
                            .disconnect_runtime_connection(connection_id)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::PublishRuntimeDynamic { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted to publish a host runtime dynamic".to_string(),
                        ));
                    }
                    NetworkCommand::RemoveRuntimeDynamic { completion } => {
                        let _ = completion.send(Err(
                            "client attempted to remove a host runtime dynamic".to_string(),
                        ));
                    }
                    NetworkCommand::FailPendingJoinData { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted to fail host pending JoinData".to_string(),
                        ));
                    }
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
                    NetworkCommand::BeginResourceDerive {
                        resource_id,
                        source_path,
                        ownership,
                        completion,
                    } => {
                        let result = client
                            .begin_resource_derive(resource_id, source_path, ownership)
                            .await
                            .map_err(|error| error.to_string());
                        let _ = completion.send(result);
                    }
                    NetworkCommand::FinishResourceDerive { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted to finish an official resource derivation"
                                .to_string(),
                        ));
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
                            & clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL
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
                                current_frame_source.load(Ordering::Relaxed),
                            )
                            .await?;
                        }
                    }
                    NetworkCommand::BroadcastPlayerInfo(_)
                    | NetworkCommand::BroadcastPreexecutedPlayerInfo { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to broadcast authoritative PlayerInfo".to_string(),
                        ));
                    }
                    NetworkCommand::BroadcastLeagueRoundResults(_) => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to broadcast league round results".to_string(),
                        ));
                    }
                    NetworkCommand::LeagueAuthenticatePlayer { auth, player, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            if runtime.send_priority(LeagueRuntimeCommand::AuthenticatePlayer {
                                auth,
                                player,
                                completion: completion.clone(),
                            }).await.is_err() {
                                let _ = completion.send(Err("league runtime is unavailable".to_string()));
                            }
                        } else {
                            let _ = completion.send(Err("league runtime is unavailable".to_string()));
                        }
                    }
                    NetworkCommand::LeagueCheckPlayer { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted to check a league player".to_string(),
                        ));
                    }
                    NetworkCommand::LeagueUpdate { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted a host league lifecycle request".to_string(),
                        ));
                    }
                    NetworkCommand::LeagueEnd { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted a host league lifecycle request".to_string(),
                        ));
                    }
                    NetworkCommand::LeagueFinalizeEndFailure { completion, .. } => {
                        let _ = completion.send(Err(
                            "client attempted a host league lifecycle request".to_string(),
                        ));
                    }
                    NetworkCommand::LeagueReportDisconnect { reason, players, fbids, completion } => {
                        if let Some(runtime) = league_runtime.as_ref() {
                            if runtime.send_priority(LeagueRuntimeCommand::ReportDisconnect {
                                reason,
                                players,
                                fbids,
                                completion: completion.clone(),
                            }).await.is_err() {
                                let _ = completion.send(Err("league runtime is unavailable".to_string()));
                            }
                        } else {
                            let _ = completion.send(Err("league runtime is unavailable".to_string()));
                        }
                    }
                    NetworkCommand::LeagueInvalidate => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to invalidate a host league reference".to_string(),
                        ));
                    }
                    NetworkCommand::SetMasterserverSignup { completion, .. } => {
                        let message =
                            "client attempted to change host masterserver signup".to_string();
                        let _ = completion.send(Err(message.clone()));
                        let _ = event_tx.send(NetworkEvent::Error(message));
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
                    NetworkCommand::SubmitControlSet(set) => {
                        let data = encode_control_entry_payload(&set.into_control_packet())?;
                        client.submit_packet(ControlDelivery::Sync, data)
                            .await
                            .map_err(|error| anyhow!("client control-set submission failed: {error}"))?;
                    }
                    NetworkCommand::SubmitDecidedControl { tick, control, .. } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            control,
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitInitScenarioPlayer { tick, selection } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::InitScenarioPlayer(selection),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitSurrenderPlayer { tick, surrender } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::SurrenderPlayer(surrender),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitInternalPlayerScript { tick, control } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            control,
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitMessage(message) => {
                        let data = encode_control_entry_payload(
                            &clonk_engine::ControlPacket::Message(message.clone()),
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
                            &clonk_engine::ControlPacket::Vote(vote),
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
                    NetworkCommand::BroadcastHostRestarting { .. } => {
                        let _ = event_tx.send(NetworkEvent::Error(
                            "client attempted to announce a host round restart".to_string(),
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
                        if let Some(control) = control_packet_for_event(owner, event, client_id) {
                            record_client_control(
                                &client,
                                &mut client_activation,
                                &mut frame_builder,
                                &current_frame_source,
                                tick,
                                control,
                            )
                            .await?;
                        }
                    }
                    NetworkCommand::SubmitPlayerCommand { tick, command } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::PlayerCommand(command),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitPlayerSelect { tick, selection } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::PlayerSelect(selection),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitScript { tick, script } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::Script(script),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitMessageBoardAnswer { tick, answer } => {
                        record_client_control(
                            &client,
                            &mut client_activation,
                            &mut frame_builder,
                            &current_frame_source,
                            tick,
                            clonk_engine::ControlPacket::MessageBoardAnswer(answer),
                        )
                        .await?;
                    }
                    NetworkCommand::SubmitSyncCheck { tick, check } => {
                        frame_builder.record_control(
                            tick,
                            clonk_engine::ControlPacket::SyncCheck(check),
                            current_millis(),
                        );
                    }
                    NetworkCommand::FinalizeTick { tick } => {
                        if !client_activation.can_finalize() {
                            continue;
                        }
                        if std::mem::take(&mut rebase_pending_on_activation) {
                            frame_builder.rebase_pending_to_first_activated_tick(tick);
                        }
                        if let Some(frame) = frame_builder.finalize_tick(tick) {
                            send_frame_to_client(&client, frame).await?;
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
                    NetworkCommand::SetHostPassword { completion, .. } => {
                        let message = "client attempted to change host password".to_string();
                        let _ = completion.send(Err(message.clone()));
                        let _ = event_tx.send(NetworkEvent::Error(message));
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
                    NetworkCommand::BeginGo { completion, .. } => {
                        let message =
                            "client attempted to begin the authoritative game".to_string();
                        let _ = completion.send(Err(message.clone()));
                        let _ = event_tx.send(NetworkEvent::Error(message));
                    }
                    NetworkCommand::StatusReachedCurrent | NetworkCommand::StatusReached { .. } => {
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
                        current_frame_source.store(current_frame, Ordering::Relaxed);
                        client_activation.status_reached();
                        request_client_activation_if_due(
                            &client,
                            &mut client_activation,
                            tokio::time::Instant::now(),
                            current_frame,
                        )
                        .await?;
                    }
                    NetworkCommand::ClientUpdateExecuted(update) => {
                        if let Ok(local_client_id) = i32::try_from(client_id) {
                            rebase_pending_on_activation |= client_activation
                                .apply_executed_client_update(local_client_id, &update);
                        }
                    }
                    NetworkCommand::GracefulPart { completion } => {
                        drop(client_events);
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
            _ = wait_for_activation_retry(activation_retry_at), if client_events_open => {
                request_client_activation_if_due(
                    &client,
                    &mut client_activation,
                    tokio::time::Instant::now(),
                    current_frame_source.load(Ordering::Relaxed),
                )
                .await?;
            }
            else => break,
        }
    }

    drop(client_events);
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
    current_frame: i32,
) -> Result<()> {
    let Some(tick) = activation.request_tick_if_due(now, current_frame) else {
        return Ok(());
    };
    client
        .request_activation(tick)
        .await
        .map_err(|error| anyhow!("client activation request failed: {error}"))?;
    activation.mark_requested(now);
    Ok(())
}

async fn record_client_control(
    client: &ClientHandle,
    activation: &mut ClientActivationState,
    frame_builder: &mut ControlFrameAccumulator,
    current_frame_source: &AtomicI32,
    tick: Tick,
    control: clonk_engine::ControlPacket,
) -> Result<()> {
    if !frame_builder.record_control(tick, control, current_millis()) {
        return Ok(());
    }
    activation.arm_for_queued_control();
    request_client_activation_if_due(
        client,
        activation,
        tokio::time::Instant::now(),
        current_frame_source.load(Ordering::Relaxed),
    )
    .await
}

fn announce_connected_client(
    client: &mut ClientHandle,
    player_name: String,
    event_tx: &NetworkEventSender,
    local_id_tx: &mpsc::Sender<std::result::Result<NetworkWorkerReady, NetworkStartError>>,
) -> Result<(ClientId, NetworkStatus, Option<String>)> {
    let join_data = match client.take_join_data() {
        Some(join_data) => join_data,
        None => {
            let message = "connected client did not retain JoinData".to_string();
            let _ = local_id_tx.send(Err(message.clone().into()));
            return Err(anyhow!(message));
        }
    };
    let initial_status = initial_client_status(&join_data);
    let league_endpoint = (!join_data.parameters.league_address.is_empty()).then(|| {
        join_data
            .parameters
            .league_address
            .to_string_lossy()
            .into_owned()
    });
    let client_id = client.client_id();
    let _ = event_tx.send(NetworkEvent::JoinData(join_data));
    let _ = event_tx.send(NetworkEvent::PeerConnected {
        client_id,
        name: player_name,
        kind: ParticipantKind::Player,
    });
    Ok((client_id, initial_status, league_endpoint))
}

fn initial_client_status(join_data: &clonk_network::JoinDataEnvelope) -> NetworkStatus {
    NetworkStatus {
        target_tick: join_data.start_control_tick,
        ..join_data.status
    }
}

// Event forwarding deliberately receives each independently mutable state
// machine; grouping them would obscure which state an event may update.
#[allow(clippy::too_many_arguments)]
async fn handle_client_worker_event(
    maybe_event: Option<ClientEvent>,
    client_status: &mut ClientStatusState,
    client_activation: &mut ClientActivationState,
    client_events_open: &mut bool,
    local_owner: i32,
    client_id: ClientId,
    event_tx: &NetworkEventSender,
    telemetry_tx: &SyncSender<NetworkEvent>,
    netpuncher_state: &Arc<Mutex<NetworkNetpuncherState>>,
) -> Result<()> {
    match maybe_event {
        Some(ClientEvent::Status(status)) => {
            if client_status.receive_request(status) {
                client_activation.status_requested();
            }
            handle_client_event(
                ClientEvent::Status(status),
                local_owner,
                client_id,
                event_tx,
                telemetry_tx,
            )
            .await?;
        }
        Some(ClientEvent::StatusAck(status)) => {
            if client_status.commit(status) {
                handle_client_event(
                    ClientEvent::StatusAck(status),
                    local_owner,
                    client_id,
                    event_tx,
                    telemetry_tx,
                )
                .await?;
            }
        }
        Some(event) => {
            if let ClientEvent::LocalAddressesChanged { local_addresses } = &event {
                netpuncher_state.lock().local_addresses = local_addresses.clone();
            }
            let disconnected = matches!(&event, ClientEvent::Disconnected { .. });
            handle_client_event(event, local_owner, client_id, event_tx, telemetry_tx).await?;
            if disconnected {
                // Preserve only the command bridge required for the app's
                // synchronous ReportDisconnect call.
                *client_events_open = false;
            }
        }
        None => {
            // Keep the command bridge alive long enough for the app to
            // synchronously report the lost host, then let ChangeToLocal drop
            // this manager and send Shutdown.
            *client_events_open = false;
        }
    }
    Ok(())
}

// The forwarding helper mirrors `handle_client_worker_event` while borrowing
// all state across `select!`; an aggregate borrow would not simplify ownership.
#[allow(clippy::too_many_arguments)]
async fn await_client_operation_while_forwarding_events<F>(
    operation: F,
    client_events: &mut tokio_mpsc::Receiver<ClientEvent>,
    client_status: &mut ClientStatusState,
    client_activation: &mut ClientActivationState,
    client_events_open: &mut bool,
    local_owner: i32,
    client_id: ClientId,
    event_tx: &NetworkEventSender,
    telemetry_tx: &SyncSender<NetworkEvent>,
    netpuncher_state: &Arc<Mutex<NetworkNetpuncherState>>,
) -> Result<F::Output>
where
    F: std::future::Future,
{
    tokio::pin!(operation);
    loop {
        tokio::select! {
            output = &mut operation => return Ok(output),
            maybe_event = client_events.recv(), if *client_events_open => {
                handle_client_worker_event(
                    maybe_event,
                    client_status,
                    client_activation,
                    client_events_open,
                    local_owner,
                    client_id,
                    event_tx,
                    telemetry_tx,
                    netpuncher_state,
                ).await?;
            }
        }
    }
}

async fn handle_client_event(
    event: ClientEvent,
    local_owner: i32,
    _client_id: ClientId,
    event_tx: &NetworkEventSender,
    _telemetry_tx: &SyncSender<NetworkEvent>,
) -> Result<()> {
    match event {
        ClientEvent::LocalAddressesChanged { .. } => {}
        ClientEvent::PingMeasured { round_trip_ms } => {
            let _ = event_tx.send(NetworkEvent::HostPingMeasured { round_trip_ms });
        }
        ClientEvent::Status(status) => {
            let _ = event_tx.send(NetworkEvent::StatusRequested(status));
        }
        ClientEvent::StatusAck(status) => {
            let _ = event_tx.send(NetworkEvent::StatusCommitted(status));
        }
        ClientEvent::LobbyCountdown { packet } => {
            let _ = event_tx.send(NetworkEvent::LobbyCountdown(packet));
        }
        ClientEvent::HostRestarting { rejoin_seconds } => {
            let _ = event_tx.send(NetworkEvent::HostRestarting { rejoin_seconds });
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
        ClientEvent::ResourceProgress {
            resource_id,
            present_percent,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceProgress {
                resource_id,
                present_percent,
            });
        }
        ClientEvent::ResourceComplete {
            resource_id,
            core,
            path,
            local,
        } => {
            let _ = event_tx.send(NetworkEvent::ResourceComplete {
                resource_id,
                core,
                path,
                local,
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

async fn send_frame_to_host(host: &HostHandle, frame: LegacyControlFrame) -> Result<()> {
    // A local codec failure makes the next complete-control barrier
    // unreachable. Propagate to `run_worker` so its sole exit boundary emits
    // FatalError after the worker has actually stopped.
    let packet = encode_control_packet(&frame)
        .map_err(|err| anyhow!("failed to encode host control packet: {err}"))?;
    host.submit_local_control(packet)
        .await
        .map_err(|err| anyhow!("host submit failed: {err}"))
}

async fn send_player_info_from_host(host: &HostHandle, info: PlayerInfoControlData) -> Result<()> {
    // PlayerInfo is host-authored admission state; a log-only failure would
    // leave the already-preexecuted host state different from every client.
    let data = encode_control_entry_payload(&clonk_engine::ControlPacket::PlayerInfo(info))
        .map_err(|err| anyhow!("failed to encode authoritative PlayerInfo: {err}"))?;
    host.submit_packet(ControlDelivery::Direct, data)
        .await
        .map_err(|err| anyhow!("host PlayerInfo broadcast failed: {err}"))
}

async fn send_frame_to_client(client: &ClientHandle, frame: LegacyControlFrame) -> Result<()> {
    // The coordinator cannot complete this tick without the local
    // contribution, so the worker must not survive a compiler failure.
    let packet = encode_control_packet(&frame)
        .map_err(|err| anyhow!("failed to encode client control packet: {err}"))?;
    client
        .submit_control(packet)
        .await
        .map_err(|err| anyhow!("client submit failed: {err}"))
}

fn handle_ready_packet(
    packet: ControlPacket,
    local_owner: i32,
    event_tx: &NetworkEventSender,
) -> Result<()> {
    // PID_Control is the authoritative complete tick, unlike malformed direct
    // peer input which can be isolated to one transport.
    let frame = decode_control_packet(&packet)
        .map_err(|err| anyhow!("failed to decode authoritative control packet: {err}"))?;
    emit_frame_controls(frame, local_owner, event_tx)
}

fn handle_direct_packet(
    delivery: ControlDelivery,
    data: Vec<u8>,
    event_tx: &NetworkEventSender,
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
    event_tx: &NetworkEventSender,
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
    controls: Vec<clonk_engine::ControlPacket>,
    event_tx: &NetworkEventSender,
) -> Result<()> {
    let controls = controls
        .into_iter()
        .filter_map(network_control_for_packet)
        .collect();
    let _ = event_tx.send(NetworkEvent::ScheduledSync { tick, controls });
    Ok(())
}

pub fn network_control_for_packet(control: clonk_engine::ControlPacket) -> Option<NetworkControl> {
    match control {
        clonk_engine::ControlPacket::ClientJoin(join) => Some(NetworkControl::ClientJoin(join)),
        clonk_engine::ControlPacket::ClientUpdate(update) => {
            Some(NetworkControl::ClientUpdate(update))
        }
        clonk_engine::ControlPacket::ClientRemove(remove) => {
            Some(NetworkControl::ClientRemove(remove))
        }
        // Keep the original signed fields through ordered execution. The
        // C++ packet layer counts them before InCom narrows Command to a byte.
        clonk_engine::ControlPacket::PlayerControl(data) => {
            Some(NetworkControl::PlayerControl(data))
        }
        clonk_engine::ControlPacket::PlayerCommand(data) => {
            Some(NetworkControl::PlayerCommand(data))
        }
        clonk_engine::ControlPacket::PlayerSelect(data) => Some(NetworkControl::PlayerSelect(data)),
        clonk_engine::ControlPacket::Script(data) => Some(NetworkControl::Script(data)),
        clonk_engine::ControlPacket::Message(data) => Some(NetworkControl::Message(data)),
        clonk_engine::ControlPacket::MessageBoardAnswer(data) => {
            Some(NetworkControl::MessageBoardAnswer(data))
        }
        clonk_engine::ControlPacket::CustomCommand(data) => {
            Some(NetworkControl::CustomCommand(data))
        }
        clonk_engine::ControlPacket::EmMoveObject(data) => Some(NetworkControl::EmMoveObject(data)),
        clonk_engine::ControlPacket::EmDrawTool(data) => Some(NetworkControl::EmDrawTool(data)),
        clonk_engine::ControlPacket::EmDropDef(data) => Some(NetworkControl::EmDropDef(data)),
        clonk_engine::ControlPacket::Synchronize(data) => Some(NetworkControl::Synchronize(data)),
        clonk_engine::ControlPacket::SyncCheck(packet) => Some(NetworkControl::SyncCheck(packet)),
        clonk_engine::ControlPacket::PlayerInfo(info) => Some(NetworkControl::PlayerInfo(info)),
        clonk_engine::ControlPacket::JoinPlayer(join) => Some(NetworkControl::JoinPlayer(join)),
        clonk_engine::ControlPacket::RemovePlayer(remove) => {
            Some(NetworkControl::RemovePlayer(remove))
        }
        clonk_engine::ControlPacket::InitScenarioPlayer(selection) => {
            Some(NetworkControl::InitScenarioPlayer(selection))
        }
        clonk_engine::ControlPacket::SurrenderPlayer(surrender) => {
            Some(NetworkControl::SurrenderPlayer(surrender))
        }
        clonk_engine::ControlPacket::ActivateGameGoalMenu(control) => {
            Some(NetworkControl::ActivateGameGoalMenu(control))
        }
        clonk_engine::ControlPacket::ToggleHostility(control) => {
            Some(NetworkControl::ToggleHostility(control))
        }
        clonk_engine::ControlPacket::ActivateGameGoalRule(control) => {
            Some(NetworkControl::ActivateGameGoalRule(control))
        }
        clonk_engine::ControlPacket::SetPlayerTeam(control) => {
            Some(NetworkControl::SetPlayerTeam(control))
        }
        clonk_engine::ControlPacket::EliminatePlayer(control) => {
            Some(NetworkControl::EliminatePlayer(control))
        }
        clonk_engine::ControlPacket::Vote(vote) => Some(NetworkControl::Vote(vote)),
        clonk_engine::ControlPacket::VoteEnd(result) => Some(NetworkControl::VoteEnd(result)),
        clonk_engine::ControlPacket::Set(set) => Some(NetworkControl::Set(set.into())),
        clonk_engine::ControlPacket::DebugRecord(data) => Some(NetworkControl::DebugRecord(data)),
        clonk_engine::ControlPacket::Unknown { .. } => None,
    }
}

fn control_packet_for_event(
    owner: i32,
    event: ControlEvent,
    client_id: ClientId,
) -> Option<clonk_engine::ControlPacket> {
    let (command, data) = match event {
        ControlEvent::RawPlayerControl { command, data } => (i32::from(command), data),
        event => (control_command_for_event(event)?, 0),
    };
    let by_client = i32::try_from(client_id).ok()?;
    Some(clonk_engine::ControlPacket::PlayerControl(
        PlayerControlData {
            player: owner,
            command,
            data,
            by_client,
        },
    ))
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
    use std::io::{Read, Write};

    use flate2::read::ZlibDecoder;
    use tokio::io::AsyncReadExt as _;

    use super::*;

    fn test_netpuncher_state() -> Arc<Mutex<NetworkNetpuncherState>> {
        Arc::new(Mutex::new(NetworkNetpuncherState::default()))
    }

    #[test]
    fn failed_client_join_names_the_connect_failure_once() {
        // `ClientError::Connect` already carries the caption, so the startup
        // mapping must not prepend a second copy of it.
        let startup_error = client_startup_error(clonk_network::ClientError::Connect(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "connection attempt timed out"),
        ));

        assert_eq!(
            startup_error.to_string(),
            "failed to connect to host: connection attempt timed out"
        );
    }

    #[test]
    fn production_network_runtime_has_a_bounded_worker_budget() {
        // C++ runs all registered TCP and UDP network procs on one
        // StdSchedulerThread (oracle-src-pinned src/C4InteractiveThread.cpp:48-67;
        // src/StdScheduler.cpp:229-244; src/C4Network2IO.cpp:71-88). Rust keeps
        // enough parallel service capacity for the per-peer UDP tasks without
        // scaling every game process to the host's CPU count. The budget is
        // `Config.General.ThreadPoolThreadCount` on non-Windows targets, which
        // C++ defaults to 8 (C4Config.cpp:406-408; C4Application.cpp:152-159).
        let restore = network_runtime_worker_threads();
        let runtime = build_network_runtime().expect("build production network runtime");
        assert_eq!(
            runtime.metrics().num_workers(),
            DEFAULT_NETWORK_RUNTIME_WORKER_THREADS
        );
        drop(runtime);

        // A configured count sizes the pool; zero keeps the default so tokio is
        // never asked for an invalid worker count.
        set_network_runtime_worker_threads(2);
        let configured = build_network_runtime().expect("build a configured runtime");
        assert_eq!(configured.metrics().num_workers(), 2);
        drop(configured);
        set_network_runtime_worker_threads(0);
        let fallback = build_network_runtime().expect("build the fallback runtime");
        assert_eq!(
            fallback.metrics().num_workers(),
            DEFAULT_NETWORK_RUNTIME_WORKER_THREADS
        );
        drop(fallback);
        set_network_runtime_worker_threads(restore);
    }

    #[test]
    fn client_settings_default_and_override_compatibility_build() {
        // C++ publishes C4XVERBUILD in its reference and requires that exact
        // PID_Conn build (oracle-src-pinned src/C4Network2Reference.cpp:79,100-102;
        // src/C4Network2.cpp:1291-1299).
        let address = SocketAddr::from(([127, 0, 0, 1], 11_112));
        let settings = ClientSettings::new(address, "Alice");
        assert_eq!(
            settings.compatibility_build,
            clonk_network::CURRENT_GAME_BUILD
        );

        assert_eq!(
            settings
                .with_compatibility_build(clonk_network::CURRENT_GAME_BUILD + 2)
                .compatibility_build,
            clonk_network::CURRENT_GAME_BUILD + 2
        );
    }

    #[tokio::test]
    async fn startup_cancellation_retains_early_signal_per_attempt() {
        let cancelled_attempt = NetworkStartupCancellation::new();
        let later_attempt = NetworkStartupCancellation::new();

        assert!(cancelled_attempt.cancel());
        assert!(!cancelled_attempt.cancel());
        tokio::time::timeout(Duration::from_millis(100), cancelled_attempt.cancelled())
            .await
            .expect("cancellation sent before the waiter is retained");
        assert!(!later_attempt.is_cancelled());
    }

    fn runtime_dynamic_fixture() -> clonk_network::LiveNetworkDynamic {
        clonk_network::LiveNetworkDynamic {
            group_filename: "Runtime.c4s".to_string(),
            maker: b"Host".to_vec(),
            packed_bytes: vec![1, 2, 3],
            file_size: 3,
            file_crc: 0x1122_3344,
            contents_crc: 0x5566_7788,
            entries: Vec::new(),
        }
    }

    fn join_parameters_fixture() -> clonk_network::JoinGameParametersEnvelope {
        HostConfig::default()
            .initial_join_snapshot
            .expect("default host JoinData")
            .parameters
    }

    #[test]
    fn synchronize_submission_uses_cpp_runtime_join_flags_and_sync_delivery() {
        let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();

        manager
            .submit_synchronize(23, false, true)
            .expect("queue runtime-join synchronization");

        let Some(NetworkCommand::SubmitDecidedControl {
            tick,
            control:
                clonk_engine::ControlPacket::Synchronize(clonk_engine::SynchronizeControlData {
                    save_player_files,
                    sync_clearance,
                    by_client,
                }),
            sync,
        }) = commands.command_rx.blocking_recv()
        else {
            panic!("expected one synchronized CID_Synchronize command");
        };
        assert_eq!(tick, 23);
        assert!(!save_player_files);
        assert!(sync_clearance);
        assert_eq!(by_client, HOST_CLIENT_ID as i32);
        assert!(sync);
    }

    #[test]
    fn runtime_record_synchronize_uses_ordinary_queue_for_host_and_client() {
        for (client_id, expected_author) in [(HOST_CLIENT_ID, 0), (7, 7)] {
            let (manager, _event_tx, mut commands) =
                NetworkManager::test_stub_with_commands_for_client_id(client_id);

            manager
                .submit_queued_synchronize(29, false, true)
                .expect("queue runtime-record synchronization");

            assert_eq!(
                commands.take_submitted_decided_controls(),
                vec![(
                    29,
                    clonk_engine::ControlPacket::Synchronize(
                        clonk_engine::SynchronizeControlData {
                            save_player_files: false,
                            sync_clearance: true,
                            by_client: expected_author,
                        },
                    ),
                    false,
                )]
            );
        }
    }

    #[test]
    fn runtime_dynamic_manager_wrappers_forward_host_commands() {
        let (manager, _event_tx, mut commands) = NetworkManager::test_stub_with_commands();
        let dynamic = runtime_dynamic_fixture();
        let parameters = join_parameters_fixture();
        let published_core = clonk_engine::NetworkResourceCore {
            id: 41,
            ..Default::default()
        };
        let reason = clonk_engine::LegacyCString::from_bytes(b"dynamic failed".to_vec()).unwrap();
        let expected_dynamic = dynamic.clone();
        let expected_parameters = parameters.clone();
        let expected_core = published_core.clone();
        let expected_reason = reason.clone();
        let responder = std::thread::spawn(move || {
            match commands.command_rx.blocking_recv() {
                Some(NetworkCommand::PublishRuntimeDynamic {
                    dynamic,
                    dynamic_tick,
                    parameters,
                    completion,
                }) => {
                    assert_eq!(*dynamic, expected_dynamic);
                    assert_eq!(dynamic_tick, 23);
                    assert_eq!(*parameters, expected_parameters);
                    completion
                        .send(Ok(expected_core))
                        .expect("complete dynamic publication");
                }
                command => panic!("unexpected runtime-dynamic publication command: {command:?}"),
            }
            match commands.command_rx.blocking_recv() {
                Some(NetworkCommand::RemoveRuntimeDynamic { completion }) => {
                    completion.send(Ok(true)).expect("complete dynamic removal")
                }
                command => panic!("unexpected runtime-dynamic removal command: {command:?}"),
            }
            match commands.command_rx.blocking_recv() {
                Some(NetworkCommand::FailPendingJoinData { reason, completion }) => {
                    assert_eq!(reason, expected_reason);
                    completion
                        .send(Ok(2))
                        .expect("complete pending JoinData failure");
                }
                command => panic!("unexpected pending JoinData command: {command:?}"),
            }
        });

        assert_eq!(
            manager
                .publish_runtime_dynamic(dynamic, 23, parameters)
                .expect("publish runtime dynamic"),
            published_core
        );
        assert!(manager
            .remove_runtime_dynamic()
            .expect("remove runtime dynamic"));
        assert_eq!(
            manager
                .fail_pending_join_data(reason)
                .expect("fail pending JoinData"),
            2
        );
        responder.join().expect("runtime-dynamic responder");
    }

    #[test]
    fn runtime_dynamic_manager_wrappers_reject_client_role() {
        let (manager, _event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);

        assert!(manager
            .publish_runtime_dynamic(runtime_dynamic_fixture(), 23, join_parameters_fixture())
            .unwrap_err()
            .to_string()
            .contains("network host"));
        assert!(manager
            .remove_runtime_dynamic()
            .unwrap_err()
            .to_string()
            .contains("network host"));
        assert!(manager
            .fail_pending_join_data(clonk_engine::LegacyCString::default())
            .unwrap_err()
            .to_string()
            .contains("network host"));
        assert!(manager
            .submit_synchronize(23, false, true)
            .unwrap_err()
            .to_string()
            .contains("network host"));
        assert!(matches!(
            commands.command_rx.try_recv(),
            Err(tokio_mpsc::error::TryRecvError::Empty)
        ));
    }

    async fn reserve_tcp_and_udp_at_same_address(
    ) -> (TcpListener, tokio::net::UdpSocket, SocketAddr) {
        const MAX_ATTEMPTS: usize = 32;

        for attempt in 1..=MAX_ATTEMPTS {
            let udp = tokio::net::UdpSocket::bind("127.0.0.1:0")
                .await
                .unwrap_or_else(|error| {
                    panic!("failed to bind UDP fixture on attempt {attempt}: {error}")
                });
            let address = udp.local_addr().unwrap_or_else(|error| {
                panic!("failed to read UDP fixture address on attempt {attempt}: {error}")
            });
            match TcpListener::bind(address).await {
                Ok(tcp) => return (tcp, udp, address),
                Err(error)
                    if error.kind() == std::io::ErrorKind::AddrInUse && attempt < MAX_ATTEMPTS => {}
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    panic!(
                        "failed to reserve TCP fixture at held UDP endpoints after {MAX_ATTEMPTS} attempts; last endpoint {address}: {error}"
                    );
                }
                Err(error) => {
                    panic!(
                        "failed to bind TCP fixture at held UDP endpoint {address} on attempt {attempt}: {error}"
                    );
                }
            }
        }
        unreachable!("same-address fixture attempt loop always returns or panics")
    }

    #[tokio::test]
    async fn event_pumps_unblock_inner_commands_when_event_channels_are_full() {
        let (host_event_tx, mut host_events) = tokio_mpsc::channel(1);
        host_event_tx
            .send(HostEvent::UnhandledPacket {
                client_id: None,
                packet_type: 0x41,
            })
            .await
            .unwrap();
        let blocked_host_event_tx = host_event_tx.clone();
        let host_operation = async move {
            blocked_host_event_tx
                .send(HostEvent::UnhandledPacket {
                    client_id: None,
                    packet_type: 0x42,
                })
                .await
                .unwrap();
            7
        };
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(1);
        let mut provenance = VecDeque::new();
        let netpuncher_state = test_netpuncher_state();
        let host_result = tokio::time::timeout(
            Duration::from_secs(1),
            await_host_operation_while_forwarding_events(
                host_operation,
                &mut host_events,
                0,
                &event_tx,
                &telemetry_tx,
                &mut provenance,
                &netpuncher_state,
            ),
        )
        .await
        .expect("host event pump should release the blocked operation")
        .unwrap();
        assert_eq!(host_result, 7);

        let (client_event_tx, mut client_events) = tokio_mpsc::channel(1);
        client_event_tx
            .send(ClientEvent::PingMeasured { round_trip_ms: 1 })
            .await
            .unwrap();
        let blocked_client_event_tx = client_event_tx.clone();
        let client_operation = async move {
            blocked_client_event_tx
                .send(ClientEvent::PingMeasured { round_trip_ms: 2 })
                .await
                .unwrap();
            9
        };
        let mut client_status = ClientStatusState::default();
        let mut client_activation = ClientActivationState::default();
        let mut client_events_open = true;
        let client_result = tokio::time::timeout(
            Duration::from_secs(1),
            await_client_operation_while_forwarding_events(
                client_operation,
                &mut client_events,
                &mut client_status,
                &mut client_activation,
                &mut client_events_open,
                0,
                1,
                &event_tx,
                &telemetry_tx,
                &netpuncher_state,
            ),
        )
        .await
        .expect("client event pump should release the blocked operation")
        .unwrap();
        assert_eq!(client_result, 9);
    }

    #[tokio::test]
    async fn runtime_dynamic_host_operation_drains_join_events_under_pressure() {
        let (host_event_tx, mut host_events) = tokio_mpsc::channel(1);
        host_event_tx
            .send(HostEvent::JoinDataNeeded {
                client_id: 7,
                current_control_tick: 23,
            })
            .await
            .unwrap();
        let blocked_host_event_tx = host_event_tx.clone();
        let operation = async move {
            blocked_host_event_tx
                .send(HostEvent::SyncScheduled {
                    control_tick: 23,
                    controls: Vec::new(),
                })
                .await
                .unwrap();
            41
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(1);
        let mut provenance = VecDeque::new();
        let netpuncher_state = test_netpuncher_state();

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            await_host_operation_while_forwarding_events(
                operation,
                &mut host_events,
                0,
                &event_tx,
                &telemetry_tx,
                &mut provenance,
                &netpuncher_state,
            ),
        )
        .await
        .expect("runtime-dynamic command must not deadlock behind host events")
        .unwrap();

        assert_eq!(result, 41);
        assert_eq!(
            event_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            NetworkEvent::JoinDataNeeded {
                client_id: 7,
                current_control_tick: 23,
            }
        );
    }

    fn serve_one_league_record_upload() -> (
        String,
        Receiver<()>,
        Sender<()>,
        thread::JoinHandle<Vec<u8>>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (request_received, request_ready) = mpsc::channel();
        let (respond, response_release) = mpsc::channel();
        let request = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let count = stream.read(&mut buffer).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..count]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let header = std::str::from_utf8(&request[..header_end]).unwrap();
                let content_length = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                    })
                    .unwrap();
                if request.len() >= header_end + 4 + content_length {
                    break;
                }
            }
            request_received.send(()).unwrap();
            response_release
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .unwrap();
            request
        });
        (
            format!("http://{address}/stream?token=x&"),
            request_ready,
            respond,
            request,
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_record_runtime_posts_terminal_zlib_bytes_to_cpp_url() {
        let (endpoint, request_ready, respond, request) = serve_one_league_record_upload();
        let runtime = spawn_league_record_runtime(
            endpoint,
            clonk_network::LeagueHttpTransportConfig::default(),
        )
        .unwrap();
        assert_eq!(runtime.status(), LeagueRecordStreamStatus::default());
        let (started, start_result) = std::sync::mpsc::channel();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Start {
                now: 100,
                completion: started,
            })
            .unwrap();
        start_result
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        assert_eq!(
            runtime.status(),
            LeagueRecordStreamStatus {
                is_streaming: true,
                waiting_raw_bytes: 0,
                input_position: 0,
                pending_compressed_bytes: 0,
                sent_position: 0,
            }
        );

        let source = b"C++ compatible streamed record bytes".to_vec();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Append(source.clone()))
            .unwrap();
        let (finished, finish_result) = std::sync::mpsc::channel();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Finish {
                now: 100,
                completion: finished,
            })
            .unwrap();
        finish_result
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let finishing_status = runtime.status();
        assert!(finishing_status.is_streaming());
        assert_eq!(finishing_status.waiting_raw_bytes(), 0);
        assert_eq!(finishing_status.input_position(), source.len() as u64);
        assert!(finishing_status.pending_compressed_bytes() > 0);
        assert_eq!(finishing_status.sent_position(), 0);
        request_ready.recv_timeout(Duration::from_secs(5)).unwrap();
        respond.send(()).unwrap();
        let (shutdown, shut_down) = tokio::sync::oneshot::channel();
        runtime
            .command_tx
            .send(LeagueRecordRuntimeCommand::Shutdown {
                completion: shutdown,
            })
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), shut_down)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        // C++ clears the stream synchronously before returning from
        // `StopStreaming` (`src/C4Network2.cpp:3099-3112`).
        assert_eq!(runtime.status(), LeagueRecordStreamStatus::default());

        let request = request.join().unwrap();
        let header_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap();
        let header = std::str::from_utf8(&request[..header_end]).unwrap();
        assert!(header.starts_with("POST /stream?token=x&pos=0&end=true HTTP/1."));
        let mut decoded = Vec::new();
        ZlibDecoder::new(&request[header_end + 4..])
            .read_to_end(&mut decoded)
            .unwrap();
        assert_eq!(decoded, source);
    }

    #[test]
    fn control_tick_probe_uses_dedicated_coalesced_channel() {
        let (mut manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let (control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        manager.control_tick_tx = control_tick_tx;

        let reached_at = tokio::time::Instant::now();
        manager.control_tick_reached(7, 2, DEFAULT_CONTROL_TARGET_FPS, reached_at);
        let stored_reached_at = {
            let probe = manager.control_tick_probe.lock();
            let probe = probe.as_ref().unwrap();
            assert!(probe.queued);
            probe.reached_at
        };
        assert_eq!(stored_reached_at, reached_at);
        let queued = control_tick_rx.try_recv().unwrap();
        assert_eq!(queued.tick, 7);
        assert_eq!(queued.control_rate, 2);
        assert_eq!(queued.target_fps, DEFAULT_CONTROL_TARGET_FPS);
        assert_eq!(queued.reached_at, reached_at);

        manager.control_tick_reached(7, 2, DEFAULT_CONTROL_TARGET_FPS, reached_at);
        assert!(matches!(
            control_tick_rx.try_recv(),
            Err(tokio_mpsc::error::TryRecvError::Empty)
        ));
        assert!(matches!(
            commands.command_rx.try_recv(),
            Err(tokio_mpsc::error::TryRecvError::Empty)
        ));

        manager.control_tick_reached(7, 3, 76, reached_at + Duration::from_secs(1));
        let refreshed = control_tick_rx.try_recv().unwrap();
        assert_eq!(
            (refreshed.tick, refreshed.control_rate, refreshed.target_fps),
            (7, 3, 76)
        );
        assert_eq!(refreshed.reached_at, reached_at);
    }

    #[test]
    fn netpuncher_resolution_normalizes_default_port_and_family_order() {
        let addresses = normalize_resolved_netpuncher_addresses([
            "[2001:db8::2]:0".parse().unwrap(),
            "192.0.2.9:0".parse().unwrap(),
            "192.0.2.10:1234".parse().unwrap(),
            "[2001:db8::3]:1234".parse().unwrap(),
        ]);

        assert_eq!(
            addresses,
            vec![
                "192.0.2.9:11115".parse().unwrap(),
                "[2001:db8::2]:11115".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn netpuncher_lookup_accepts_legacy_host_and_ipv6_spellings() {
        assert_eq!(
            netpuncher_lookup_target("puncher.example"),
            Some(("puncher.example".to_string(), 11_115))
        );
        assert_eq!(
            netpuncher_lookup_target("puncher.example:1234"),
            Some(("puncher.example".to_string(), 1_234))
        );
        assert_eq!(
            netpuncher_lookup_target("[::1]"),
            Some(("::1".to_string(), 11_115))
        );
        assert_eq!(
            netpuncher_lookup_target("[::1]:0"),
            Some(("::1".to_string(), 11_115))
        );
        assert_eq!(
            netpuncher_lookup_target("[::1]:1234"),
            Some(("::1".to_string(), 1_234))
        );
        assert_eq!(netpuncher_lookup_target("[::1]:bad"), None);
        assert_eq!(netpuncher_lookup_target("[::1]junk"), None);
        assert_eq!(netpuncher_lookup_target("[::1]:70000"), None);
    }

    #[test]
    fn league_start_addresses_keep_tcp_and_configured_udp_ports_distinct() {
        let tcp = "192.0.2.4:11112".parse().unwrap();
        let udp = "192.0.2.4:11113".parse().unwrap();
        assert_eq!(
            host_registration_addresses(Some(tcp), Some(11_112), Some(udp), Some(11_113)),
            vec![
                NetworkAddress::new(NetworkProtocol::Tcp, tcp),
                NetworkAddress::new(NetworkProtocol::Udp, udp),
            ]
        );
        assert_eq!(
            host_registration_addresses(Some(tcp), Some(11_112), None, Some(11_113)),
            vec![NetworkAddress::new(NetworkProtocol::Tcp, tcp)]
        );
        assert_eq!(
            host_registration_addresses(None, Some(0), Some(udp), Some(11_113)),
            vec![NetworkAddress::new(NetworkProtocol::Udp, udp)]
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn league_start_omits_udp_when_the_prebind_fails() {
        let occupied = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let occupied_address = occupied.local_addr().unwrap();
        let config = HostConfig {
            udp_bind_address: Some(occupied_address),
            configured_udp_port: Some(occupied_address.port()),
            ..HostConfig::default()
        };
        let binding = HostUdpBinding::bind(&config);

        assert_eq!(binding.local_addr(), None);
        assert_eq!(
            host_registration_addresses(
                Some("127.0.0.1:11112".parse().unwrap()),
                Some(11_112),
                binding.local_addr(),
                config.configured_udp_port,
            ),
            vec![NetworkAddress::new(
                NetworkProtocol::Tcp,
                "127.0.0.1:11112".parse().unwrap(),
            )]
        );
    }

    fn minimal_league_reference() -> clonk_network::HostGameReference {
        let config = clonk_network::HostConfig::default();
        let parameters = config
            .initial_join_snapshot
            .as_ref()
            .expect("default JoinData")
            .parameters
            .clone();
        let summary = clonk_network::NetworkGameReference {
            title: clonk_resources::decode_legacy_script_text(parameters.title.as_bytes()),
            host_name: clonk_resources::decode_legacy_script_text(
                config.local_core.name.as_bytes(),
            ),
            host_nick: clonk_resources::decode_legacy_script_text(
                config.local_core.nick.as_bytes(),
            ),
            state: "Lobby".to_string(),
            control_mode: config.initial_status.control_mode,
            start_time: 1,
            join_allowed: false,
            use_fair_crew: parameters.use_fair_crew,
            goals: parameters
                .goals
                .iter()
                .map(|goal| goal.id.as_bytes().iter().copied().map(char::from).collect())
                .collect(),
            league: clonk_resources::decode_legacy_script_text(parameters.league.as_bytes()),
            league_address: clonk_resources::decode_legacy_script_text(
                parameters.league_address.as_bytes(),
            ),
            max_players: parameters.max_players,
            game: "LegacyClonk".to_string(),
            version: clonk_network::CURRENT_GAME_VERSION,
            build: clonk_network::CURRENT_GAME_BUILD,
            source_address: SocketAddr::V6(std::net::SocketAddrV6::new(
                std::net::Ipv6Addr::UNSPECIFIED,
                0,
                0,
                0,
            )),
            ..Default::default()
        };
        clonk_network::HostGameReference::new(
            summary,
            clonk_network::HostGameReferenceMetadata {
                ..Default::default()
            },
            parameters,
        )
        .expect("minimal league reference validates")
    }

    fn minimal_league_reference_with_netpuncher(
        address: SocketAddr,
    ) -> clonk_network::HostGameReference {
        let reference = minimal_league_reference();
        let mut summary = reference.summary().clone();
        summary.netpuncher_address = address.to_string();
        let mut metadata = reference.metadata().clone();
        metadata.netpuncher_address =
            clonk_engine::LegacyCString::from_bytes(address.to_string().into_bytes())
                .expect("socket address has no NUL");
        clonk_network::HostGameReference::new(summary, metadata, reference.parameters().clone())
            .expect("netpuncher reference validates")
    }

    #[test]
    fn league_end_reference_uses_the_latest_projected_gains() {
        let reference = minimal_league_reference();
        let mut parameters = reference.parameters().clone();
        parameters.player_infos.clients = vec![clonk_network::ClientPlayerInfosSnapshot {
            client_id: 5,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 17,
                league_projected_gain: 3,
                ..clonk_engine::ControlPlayerInfoEntry::default()
            }],
        }];
        let reference = reference
            .replacing_parameters(parameters)
            .expect("player-info reference validates");

        let updated =
            reference_with_projected_gains(&reference, &HashMap::from([(17, -8), (99, 42)]))
                .expect("projected gains preserve reference invariants");

        assert_eq!(
            updated.parameters().player_infos.clients[0].players[0].league_projected_gain,
            -8
        );
    }

    fn league_http_fixture_with_before_reply<F>(
        replies: Vec<&'static [u8]>,
        mut before_reply: F,
    ) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>)
    where
        F: FnMut(usize) + Send + 'static,
    {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind fixture");
        listener.set_nonblocking(true).expect("nonblocking fixture");
        let endpoint = format!("http://{}/", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut bodies = Vec::new();
            for (reply_index, reply) in replies.into_iter().enumerate() {
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(connection) => break connection,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "timed out waiting for league request"
                            );
                            std::thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("accept league request: {error}"),
                    }
                };
                stream
                    .set_nonblocking(false)
                    .expect("blocking fixture stream");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let header_end = loop {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).expect("read league request");
                    assert_ne!(count, 0, "request ended before its headers");
                    request.extend_from_slice(&chunk[..count]);
                    if let Some(offset) =
                        request.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        break offset + 4;
                    }
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                    })
                    .expect("Content-Length header");
                while request.len() < header_end + content_length {
                    let mut chunk = [0_u8; 4096];
                    let count = stream.read(&mut chunk).expect("read league body");
                    assert_ne!(count, 0, "request ended before its body");
                    request.extend_from_slice(&chunk[..count]);
                }
                bodies.push(request[header_end..header_end + content_length].to_vec());
                before_reply(reply_index);
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.len()
                )
                .unwrap();
                stream.write_all(reply).unwrap();
            }
            bodies
        });
        (endpoint, server)
    }

    fn league_http_fixture(
        replies: Vec<&'static [u8]>,
    ) -> (String, std::thread::JoinHandle<Vec<Vec<u8>>>) {
        league_http_fixture_with_before_reply(replies, |_| {})
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_runtime_posts_start_due_update_and_latches_end_after_one_upload() {
        let (endpoint, server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\nSeed=12\r\nMaxPlayers=4\r\n",
            b"[Response]\r\nStatus=Failure\r\n",
            b"[Response]\r\nStatus=Success\r\nLeague=done\r\n",
        ]);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let reference = minimal_league_reference();
        let (start, command_tx) = register_league_host(
            PreparedLeagueHostConfig {
                endpoint,
                transport: clonk_network::LeagueHttpTransportConfig::default(),
                update_period_secs: 120,
                league_server_signup: true,
            },
            &reference,
            event_tx,
        )
        .await
        .expect("register league host");
        assert_eq!(start.league.as_bytes(), b"Cup");
        assert_eq!(start.seed, Some(12));
        assert_eq!(start.max_players, 4);
        command_tx.try_update(100, reference.clone());
        command_tx.try_update(100, reference.clone());
        let (first_complete, first_done) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::End {
                reference: reference.clone(),
                record: None,
                completion: first_complete,
            })
            .await
            .unwrap();
        assert!(matches!(
            first_done.await.expect("first End completes"),
            LeagueEndAttempt::Finished(Some(_))
        ));
        let (second_complete, second_done) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::End {
                reference,
                record: None,
                completion: second_complete,
            })
            .await
            .unwrap();
        assert_eq!(
            second_done.await.expect("latched End completes"),
            LeagueEndAttempt::Finished(None)
        );
        drop(command_tx);
        tokio::task::yield_now().await;

        let events = event_rx.try_iter().collect::<Vec<_>>();
        assert!(matches!(events.as_slice(), [NetworkEvent::LeagueUpdate(_)]));
        let bodies = server.join().expect("join league HTTP fixture");
        assert_eq!(bodies.len(), 3, "second Update and End must be latched");
        assert!(bodies[0]
            .windows(b"Action=Start".len())
            .any(|window| window == b"Action=Start"));
        assert!(bodies[1]
            .windows(b"Action=Update".len())
            .any(|window| window == b"Action=Update"));
        assert!(bodies[2]
            .windows(b"Action=End".len())
            .any(|window| window == b"Action=End"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_end_transport_failure_remains_retryable_until_explicit_finalization() {
        let (endpoint, server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\n",
        ]);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let reference = minimal_league_reference();
        let (_start, command_tx) = register_league_host(
            PreparedLeagueHostConfig {
                endpoint,
                transport: clonk_network::LeagueHttpTransportConfig::default(),
                update_period_secs: 120,
                league_server_signup: true,
            },
            &reference,
            event_tx,
        )
        .await
        .expect("register league host");
        assert_eq!(server.join().expect("join one-request fixture").len(), 1);

        for _ in 0..2 {
            let (completion, completed) = tokio::sync::oneshot::channel();
            command_tx
                .send_priority(LeagueRuntimeCommand::End {
                    reference: reference.clone(),
                    record: None,
                    completion,
                })
                .await
                .unwrap();
            assert!(matches!(
                completed.await.expect("retryable End completes"),
                LeagueEndAttempt::Retryable {
                    phase: LeagueEndFailurePhase::Send,
                    ..
                }
            ));
        }

        let (completion, completed) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::FinalizeEndFailure {
                packet: clonk_network::LeagueRoundResultsPacket {
                    success: false,
                    result_string: legacy_runtime_message("Could not send game result: offline"),
                    players: Vec::new(),
                },
                completion,
            })
            .await
            .unwrap();
        let packet = completed
            .await
            .expect("failure finalization completes")
            .expect("failure packet");
        assert!(!packet.success);
        assert_eq!(
            packet.result_string.as_bytes(),
            b"Could not send game result: offline"
        );

        let (completion, completed) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::End {
                reference,
                record: None,
                completion,
            })
            .await
            .unwrap();
        assert_eq!(
            completed.await.expect("latched End completes"),
            LeagueEndAttempt::Finished(None)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_end_rejection_preserves_results_and_latches_only_after_finalization() {
        let rejected = b"[Response]\r\n\
Status=Failure\r\n\
Message=Server says Andr\xe9\r\n\
\r\n\
\x20\x20[PlayerInfos]\r\n\
\r\n\
\x20\x20\x20\x20[Player]\r\n\
\x20\x20\x20\x20ID=17\r\n\
\x20\x20\x20\x20Status=Won\r\n";
        let (endpoint, server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\n",
            rejected,
            rejected,
        ]);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let reference = minimal_league_reference();
        let (_start, command_tx) = register_league_host(
            PreparedLeagueHostConfig {
                endpoint,
                transport: clonk_network::LeagueHttpTransportConfig::default(),
                update_period_secs: 120,
                league_server_signup: true,
            },
            &reference,
            event_tx,
        )
        .await
        .expect("register league host");

        let mut first_rejection = None;
        for _ in 0..2 {
            let (completion, completed) = tokio::sync::oneshot::channel();
            command_tx
                .send_priority(LeagueRuntimeCommand::End {
                    reference: reference.clone(),
                    record: None,
                    completion,
                })
                .await
                .unwrap();
            let LeagueEndAttempt::Rejected(packet) =
                completed.await.expect("rejected End completes")
            else {
                panic!("server rejection must remain an explicit terminal choice");
            };
            assert_eq!(packet.result_string.as_bytes(), b"Server says Andr\xe9");
            assert_eq!(packet.players.len(), 1);
            assert_eq!(packet.players[0].player_info_id, 17);
            first_rejection.get_or_insert(packet);
        }
        assert_eq!(server.join().expect("join rejection fixture").len(), 3);

        let (completion, completed) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::FinalizeEndFailure {
                packet: first_rejection.expect("first rejection packet"),
                completion,
            })
            .await
            .unwrap();
        assert!(completed
            .await
            .expect("rejection finalization completes")
            .is_some());

        let (completion, completed) = tokio::sync::oneshot::channel();
        command_tx
            .send_priority(LeagueRuntimeCommand::End {
                reference,
                record: None,
                completion,
            })
            .await
            .unwrap();
        assert_eq!(
            completed.await.expect("latched rejected End completes"),
            LeagueEndAttempt::Finished(None)
        );
        drop(command_tx);
        tokio::task::yield_now().await;
    }

    #[tokio::test]
    async fn headless_league_end_wraps_the_last_error_and_keeps_prior_results() {
        let (runtime, mut commands, _gate) = league_runtime_channels();
        let observer = tokio::spawn(async move {
            for attempt in 0..10 {
                let LeagueRuntimeCommand::End { completion, .. } =
                    commands.recv().await.expect("headless End attempt")
                else {
                    panic!("unexpected headless league command");
                };
                let outcome = if attempt == 0 {
                    LeagueEndAttempt::Rejected(clonk_network::LeagueRoundResultsPacket {
                        success: false,
                        result_string: legacy_runtime_message("initial rejection"),
                        players: vec![clonk_network::LeagueRoundResultsPlayer {
                            player_info_id: 17,
                            total_playing_time: 0,
                            settlement_score_old: -1,
                            settlement_score_new: -1,
                            league_score_new: -1,
                            league_score_gain: -1,
                            league_rank_new: 0,
                            league_rank_symbol_new: 0,
                            league_progress_data: clonk_engine::LegacyCString::default(),
                            status: clonk_network::LeagueRoundPlayerStatus::Lost,
                        }],
                    })
                } else {
                    LeagueEndAttempt::Retryable {
                        phase: LeagueEndFailurePhase::Send,
                        error: "offline".to_string(),
                    }
                };
                completion.send(outcome).expect("complete headless End");
            }
            let LeagueRuntimeCommand::FinalizeEndFailure { packet, completion } = commands
                .recv()
                .await
                .expect("headless failure finalization")
            else {
                panic!("unexpected final headless league command");
            };
            completion
                .send(Some(packet.clone()))
                .expect("complete headless finalization");
            packet
        });

        let packet = finish_league_runtime(&runtime, minimal_league_reference(), None)
            .await
            .expect("headless End loop")
            .expect("headless failure packet");
        drop(runtime);
        let observed = observer.await.expect("join headless End observer");
        assert_eq!(packet, observed);
        assert_eq!(
            packet.result_string.as_bytes(),
            b"Could not send game result: offline"
        );
        assert_eq!(packet.players.len(), 1);
        assert_eq!(packet.players[0].player_info_id, 17);
    }

    #[tokio::test]
    async fn client_puncher_resolution_applies_default_port_and_family_game_id() {
        let resolved = resolve_client_mesh_punchers(
            Some("127.0.0.1"),
            NetpuncherGameIds {
                ipv4: 0x1234,
                ipv6: 0x5678,
            },
        )
        .await;

        assert_eq!(
            resolved,
            vec![ClientMeshPuncherConfig {
                address: "127.0.0.1:11115".parse().unwrap(),
                game_id: 0x1234,
            }]
        );
    }

    #[tokio::test]
    async fn client_puncher_resolution_preserves_explicit_port_and_zero_id() {
        let resolved = resolve_client_mesh_punchers(
            Some("127.0.0.1:21115"),
            NetpuncherGameIds { ipv4: 0, ipv6: 0 },
        )
        .await;

        assert_eq!(
            resolved,
            vec![ClientMeshPuncherConfig {
                address: "127.0.0.1:21115".parse().unwrap(),
                game_id: 0,
            }]
        );
    }

    #[tokio::test]
    async fn client_puncher_resolution_accepts_bracketed_ipv6_without_a_port() {
        let resolved = resolve_client_mesh_punchers(
            Some("[::1]"),
            NetpuncherGameIds {
                ipv4: 0,
                ipv6: 0x9abc,
            },
        )
        .await;

        assert_eq!(
            resolved,
            vec![ClientMeshPuncherConfig {
                address: "[::1]:11115".parse().unwrap(),
                game_id: 0x9abc,
            }]
        );
    }

    #[test]
    fn configured_zero_port_removes_that_protocol_from_host_join_attempts() {
        let endpoint = SocketAddr::from(([127, 0, 0, 1], 11_112));
        let addresses = [
            NetworkAddress::new(NetworkProtocol::Tcp, endpoint),
            NetworkAddress::new(NetworkProtocol::Udp, endpoint),
            NetworkAddress::new(NetworkProtocol::Unknown(9), endpoint),
        ];

        assert_eq!(
            addresses
                .into_iter()
                .filter(|address| client_join_protocol_enabled(address, false, true))
                .collect::<Vec<_>>(),
            [NetworkAddress::new(NetworkProtocol::Udp, endpoint)]
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn league_runtime_posts_exact_player_auth_and_host_check_requests() {
        let (endpoint, server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=host-session\r\nLeague=Cup\r\nMaxPlayers=4\r\n",
            b"[Response]\r\nStatus=Success\r\nAUID=one-use-token\r\nAccount=Alice\r\nFBID=feedback-token\r\n",
            b"[Response]\r\nStatus=Success\r\nAccount=Alice\r\nLeague=Cup\r\nScore=12\r\nRank=3\r\nRankSymbol=4\r\nProgressData=ready\r\n",
            b"[Response]\r\nStatus=Success\r\n",
        ]);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let reference = minimal_league_reference();
        let (_start, runtime) = register_league_host(
            PreparedLeagueHostConfig {
                endpoint,
                transport: clonk_network::LeagueHttpTransportConfig::default(),
                update_period_secs: 120,
                league_server_signup: true,
            },
            &reference,
            event_tx,
        )
        .await
        .expect("register league host");
        let auth = clonk_network::LeagueAuthRequestHead {
            account: clonk_engine::LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: clonk_engine::LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        let mut player = clonk_engine::ControlPlayerInfoEntry {
            id: 17,
            name: clonk_engine::LegacyCString::from_bytes(b"Player".to_vec()).unwrap(),
            ..Default::default()
        };
        let auth_player = player.clone();
        let (auth_completion, auth_completed) = mpsc::channel();
        runtime
            .send_priority(LeagueRuntimeCommand::AuthenticatePlayer {
                auth: auth.clone(),
                player: auth_player.clone(),
                completion: auth_completion,
            })
            .await
            .unwrap();
        let auth_response = auth_completed
            .recv_timeout(Duration::from_secs(2))
            .expect("Auth completion")
            .expect("Auth exchange");
        assert!(auth_response.apply_player_auth(&mut player));

        let checked_player = player.clone();
        let (check_completion, check_completed) = mpsc::channel();
        runtime
            .send_priority(LeagueRuntimeCommand::CheckPlayer {
                player: checked_player.clone(),
                completion: check_completion,
            })
            .await
            .unwrap();
        let check_response = check_completed
            .recv_timeout(Duration::from_secs(2))
            .expect("Join completion")
            .expect("Join exchange");
        assert!(check_response.head.is_success());
        let disconnect_players = clonk_network::ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 17,
                flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                league_account: clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                ..Default::default()
            }],
        };
        let (disconnect_completion, disconnect_completed) = mpsc::channel();
        runtime
            .send_priority(LeagueRuntimeCommand::ReportDisconnect {
                reason: clonk_network::LeagueDisconnectReason::ConnectionFailed,
                players: disconnect_players.clone(),
                fbids: clonk_network::LeagueFbidRegistry::new(),
                completion: disconnect_completion,
            })
            .await
            .unwrap();
        disconnect_completed
            .recv_timeout(Duration::from_secs(2))
            .expect("ReportDisconnect completion")
            .expect("ReportDisconnect exchange");
        drop(runtime);

        let bodies = server.join().expect("join league HTTP fixture");
        let auth_player_info =
            clonk_network::encode_league_player_info_section(&auth_player).unwrap();
        let expected_auth = clonk_network::encode_league_auth_request(
            &auth,
            &auth_player_info,
            league_checksum_start(),
        )
        .unwrap();
        let check_player_info =
            clonk_network::encode_league_player_info_section(&checked_player).unwrap();
        let expected_check = clonk_network::encode_league_join_request(
            &clonk_network::LeagueJoinRequestHead {
                csid: clonk_engine::LegacyCString::from_bytes(b"host-session".to_vec()).unwrap(),
                auid: clonk_engine::LegacyCString::from_bytes(b"one-use-token".to_vec()).unwrap(),
            },
            &check_player_info,
            league_checksum_start(),
        )
        .unwrap();
        let without_random_checksum = |mut body: Vec<u8>| {
            let checksum = body
                .windows(b"Checksum=".len())
                .position(|window| window == b"Checksum=")
                .expect("Checksum field")
                + b"Checksum=".len();
            assert_ne!(&body[checksum..checksum + 5], b"-----");
            body[checksum..checksum + 5].copy_from_slice(b"-----");
            body
        };
        assert_eq!(
            without_random_checksum(bodies[1].clone()),
            without_random_checksum(expected_auth)
        );
        assert_eq!(
            without_random_checksum(bodies[2].clone()),
            without_random_checksum(expected_check)
        );
        let mut expected_fbids = clonk_network::LeagueFbidRegistry::new();
        expected_fbids.insert(
            clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
            clonk_engine::LegacyCString::from_bytes(b"feedback-token".to_vec()).unwrap(),
        );
        let expected_disconnect = clonk_network::encode_league_report_disconnect_request(
            &clonk_engine::LegacyCString::from_bytes(b"host-session".to_vec()).unwrap(),
            clonk_network::LeagueDisconnectReason::ConnectionFailed,
            &disconnect_players,
            &expected_fbids,
            league_checksum_start(),
        )
        .unwrap();
        assert_eq!(
            without_random_checksum(bodies[3].clone()),
            without_random_checksum(expected_disconnect)
        );
    }

    #[test]
    fn league_player_auth_exchange_is_pollable_and_abort_drops_its_completion() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_league_commands_for_client_id(7);
        let auth = clonk_network::LeagueAuthRequestHead {
            account: clonk_engine::LegacyCString::from_bytes(b"account".to_vec()).unwrap(),
            password: clonk_engine::LegacyCString::from_bytes(b"password".to_vec()).unwrap(),
            ..Default::default()
        };
        let player = clonk_engine::ControlPlayerInfoEntry {
            name: clonk_engine::LegacyCString::from_bytes(b"Player".to_vec()).unwrap(),
            ..Default::default()
        };

        let pending = manager
            .begin_authenticate_league_player(auth.clone(), &player)
            .expect("begin Auth")
            .expect("league runtime available");
        assert!(pending.try_complete().is_none());
        let command = commands.receive_league_player_auth();
        assert_eq!(command.auth, auth);
        assert_eq!(command.player, player);
        let response = clonk_network::decode_league_auth_response(
            b"[Response]\r\nStatus=Success\r\nAUID=one-use-token\r\nMessage=Welcome\r\n",
        );
        assert!(command.complete(Ok(response.clone())));
        assert_eq!(
            pending
                .try_complete()
                .expect("completed Auth is ready")
                .expect("successful Auth"),
            response
        );

        let abandoned = manager
            .begin_authenticate_league_player(
                clonk_network::LeagueAuthRequestHead::default(),
                &player,
            )
            .expect("begin abandoned Auth")
            .expect("league runtime available");
        let command = commands.receive_league_player_auth();
        drop(abandoned);
        assert!(
            !command.complete(Ok(clonk_network::LeagueAuthResponse::default())),
            "Abort abandons the completion without waiting for HTTP"
        );
    }

    fn message_control(by_client: i32) -> MessageControlData {
        MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_PRIVATE,
            player: 4,
            to_player: 9,
            message: clonk_engine::LegacyCString::from_bytes(b"private hello".to_vec())
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_worker_binds_and_advertises_tcp_and_udp_on_one_endpoint() {
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let netpuncher_state = test_netpuncher_state();
        let worker_state = Arc::clone(&netpuncher_state);
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                worker_state,
            )
            .await
        });

        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("host worker readiness");
        assert_eq!(ready.local_client_id, HOST_CLIENT_ID);
        let local_addresses = netpuncher_state.lock().local_addresses.clone();
        assert_eq!(local_addresses.len(), 2);
        assert_eq!(local_addresses[0].protocol, NetworkProtocol::Tcp);
        assert_eq!(local_addresses[1].protocol, NetworkProtocol::Udp);
        assert_eq!(local_addresses[0].endpoint, local_addresses[1].endpoint);
        assert_ne!(local_addresses[0].endpoint.port(), 0);

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop host worker");
        worker
            .await
            .expect("join host worker")
            .expect("host worker exits cleanly");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_host_survives_occupied_tcp_with_configured_udp() {
        let (occupied_tcp, udp_reservation, configured_address) =
            reserve_tcp_and_udp_at_same_address().await;
        drop(udp_reservation);
        let settings = HostSettings {
            bind_addr: configured_address,
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                configured_address.port(),
                configured_address.port(),
                None,
            )),
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let netpuncher_state = test_netpuncher_state();
        let worker_state = Arc::clone(&netpuncher_state);
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                worker_state,
            )
            .await
        });

        local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("UDP fallback host readiness");
        let expected_addresses = vec![NetworkAddress::new(
            NetworkProtocol::Udp,
            configured_address,
        )];
        assert_eq!(netpuncher_state.lock().local_addresses, expected_addresses);
        assert!(matches!(
            event_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetworkEvent::Error(error))
                if error.starts_with("failed to bind host socket at ")
        ));

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop UDP fallback host worker");
        worker
            .await
            .expect("join UDP fallback host worker")
            .expect("UDP fallback host exits cleanly");
        drop(occupied_tcp);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_zero_tcp_port_starts_udp_without_a_bind_error() {
        let (occupied_tcp, udp_reservation, configured_address) =
            reserve_tcp_and_udp_at_same_address().await;
        drop(udp_reservation);
        let settings = HostSettings {
            bind_addr: configured_address,
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                0,
                configured_address.port(),
                None,
            )),
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let netpuncher_state = test_netpuncher_state();
        let worker_state = Arc::clone(&netpuncher_state);
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                worker_state,
            )
            .await
        });

        local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("configured UDP-only host readiness");
        assert_eq!(
            netpuncher_state.lock().local_addresses,
            vec![NetworkAddress::new(
                NetworkProtocol::Udp,
                configured_address,
            )]
        );
        assert!(matches!(
            event_rx.recv_timeout(Duration::from_secs(2)),
            Ok(NetworkEvent::PeerConnected {
                client_id: HOST_CLIENT_ID,
                ..
            })
        ));

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop configured UDP-only host worker");
        worker
            .await
            .expect("join configured UDP-only host worker")
            .expect("configured UDP-only host exits cleanly");
        drop(occupied_tcp);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn prepared_host_fails_only_when_both_transports_are_unavailable() {
        let (occupied_tcp, occupied_udp, configured_address) =
            reserve_tcp_and_udp_at_same_address().await;
        let league_listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        league_listener.set_nonblocking(true).unwrap();
        let league_endpoint = format!("http://{}/", league_listener.local_addr().unwrap());
        let settings = HostSettings {
            bind_addr: configured_address,
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                configured_address.port(),
                configured_address.port(),
                Some(PreparedLeagueHostConfig {
                    endpoint: league_endpoint,
                    transport: clonk_network::LeagueHttpTransportConfig::default(),
                    update_period_secs: 120,
                    league_server_signup: false,
                }),
            )),
        };
        let (_command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let netpuncher_state = test_netpuncher_state();
        let worker_state = Arc::clone(&netpuncher_state);
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                worker_state,
            )
            .await
        });

        let error = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect_err("both unavailable transports must fail startup");
        assert!(matches!(
            error,
            NetworkStartError::Other(message)
                if message.starts_with("failed to bind host socket at ")
                    && message.contains("failed to start reliable-UDP listener")
        ));
        assert!(worker.await.expect("join failed host worker").is_err());
        assert!(netpuncher_state.lock().local_addresses.is_empty());
        assert!(matches!(
            event_rx.try_recv(),
            Err(TryRecvError::Empty | TryRecvError::Disconnected)
        ));
        assert_eq!(
            league_listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "fatal transport setup must not partially publish a league Start"
        );
        drop((occupied_tcp, occupied_udp));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn initial_start_rejection_ends_with_the_server_assigned_seed() {
        let (endpoint, league_server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\nSeed=305419896\r\nMaxPlayers=-1\r\n",
            b"[Response]\r\nStatus=Success\r\n",
        ]);
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                1,
                0,
                Some(PreparedLeagueHostConfig {
                    endpoint,
                    transport: clonk_network::LeagueHttpTransportConfig::default(),
                    update_period_secs: 120,
                    league_server_signup: false,
                }),
            )),
        };
        let (_command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
            )
            .await
        });

        let error = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect_err("negative Start MaxPlayers must reject local admission");
        assert!(matches!(
            error,
            NetworkStartError::Other(message)
                if message.contains("league Start settings are invalid")
        ));
        assert!(worker
            .await
            .expect("join rejected league host worker")
            .is_err());

        let bodies = league_server.join().expect("join league HTTP fixture");
        assert_eq!(bodies.len(), 2);
        assert!(bodies[0]
            .windows(b"Action=Start".len())
            .any(|window| window == b"Action=Start"));
        assert!(bodies[1]
            .windows(b"Action=End".len())
            .any(|window| window == b"Action=End"));
        assert!(
            bodies[1]
                .windows(b"RandomSeed=305419896".len())
                .any(|window| window == b"RandomSeed=305419896"),
            "cleanup must identify the Start-updated registration"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shutting_the_host_worker_down_ends_its_registration() {
        // `C4Network2::Clear` ends whatever registration the session still
        // holds (src/C4Network2.cpp:746-763), and dropping the manager — which
        // queues this Shutdown and joins — is the port's only route to it.
        // `GameApp::request_exit` now leans on that, so it is pinned here.
        let (endpoint, league_server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\nSeed=305419896\r\nMaxPlayers=4\r\n",
            b"[Response]\r\nStatus=Success\r\n",
        ]);
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                1,
                0,
                Some(PreparedLeagueHostConfig {
                    endpoint,
                    transport: clonk_network::LeagueHttpTransportConfig::default(),
                    update_period_secs: 120,
                    league_server_signup: false,
                }),
            )),
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
            )
            .await
        });

        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("host worker readiness");
        assert!(ready.league_runtime_available);

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("shut the registered host worker down");
        worker
            .await
            .expect("join registered host worker")
            .expect("registered host worker exits cleanly");

        let bodies = league_server.join().expect("join league HTTP fixture");
        assert_eq!(bodies.len(), 2, "shutdown must follow Start with End");
        assert!(bodies[1]
            .windows(b"Action=End".len())
            .any(|window| window == b"Action=End"));
        assert!(
            bodies[1]
                .windows(b"CSID=session".len())
                .any(|window| window == b"CSID=session"),
            "the End must name the session the Start committed"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refused_initial_start_keeps_hosting_without_a_registration() {
        // C4Network2::InitHost answers a refused LeagueStart with DeinitLeague
        // and keeps hosting: only the modal's Abort — which is what pCancel
        // carries back — or a console build turns the refusal into a failed
        // host init (src/C4Network2.cpp:259-272,2292-2400). There is also
        // nothing to end, because the server committed no session.
        let (endpoint, league_server) = league_http_fixture(vec![
            b"[Response]\r\nStatus=Failure\r\nMessage=already registered\r\n",
        ]);
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: Some(PreparedHostBootstrap::transport_test_fixture(
                1,
                0,
                Some(PreparedLeagueHostConfig {
                    endpoint,
                    transport: clonk_network::LeagueHttpTransportConfig::default(),
                    update_period_secs: 120,
                    league_server_signup: false,
                }),
            )),
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
            )
            .await
        });

        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("a refused registration must not fail the host session");
        assert!(
            !ready.league_runtime_available,
            "a refused Start leaves no registration to update"
        );
        assert!(
            ready
                .league_start_failure
                .as_deref()
                .is_some_and(|failure| failure.contains("league Start reply was rejected")),
            "the refusal is reported so the caller can present C++'s OK/Abort choice"
        );

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("shut the unregistered host worker down");
        worker
            .await
            .expect("join unregistered host worker")
            .expect("an unregistered host still exits cleanly");

        let bodies = league_server.join().expect("join league HTTP fixture");
        assert_eq!(
            bodies.len(),
            1,
            "a refused Start commits no session, so nothing may be ended"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_worker_returns_live_start_response_and_initializes_puncher() {
        let (second_start_seen_tx, second_start_seen_rx) = mpsc::channel();
        let (release_second_start_tx, release_second_start_rx) = mpsc::channel();
        let (endpoint, league_server) = league_http_fixture_with_before_reply(
            vec![
                b"[Response]\r\nStatus=Failure\r\nMessage=try again\r\n",
                b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\nSeed=305419896\r\nMaxPlayers=4\r\nStreamTo=https://stream.example/upload?\r\n",
                b"[Response]\r\nStatus=Success\r\n",
            ],
            move |reply_index| {
                if reply_index == 1 {
                    second_start_seen_tx
                        .send(())
                        .expect("report gated Start request");
                    release_second_start_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release gated Start response");
                }
            },
        );
        let mut puncher =
            clonk_network::ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
                .expect("bind live puncher fixture");
        let puncher_address = puncher.local_addr();
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
            )
            .await
        });
        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("host worker readiness");
        assert!(!ready.league_runtime_available);

        let failed_reference = minimal_league_reference();
        let reference = minimal_league_reference_with_netpuncher(puncher_address);
        let config = PreparedLeagueHostConfig {
            endpoint,
            transport: clonk_network::LeagueHttpTransportConfig::default(),
            update_period_secs: 120,
            league_server_signup: false,
        };
        let (failed_tx, failed_rx) = mpsc::channel();
        let (_failed_cancel, failed_cancellation) = tokio::sync::oneshot::channel();
        let failed_transition = Arc::new(std::sync::atomic::AtomicU8::new(
            MASTERSERVER_SIGNUP_PENDING,
        ));
        command_tx
            .send(NetworkCommand::SetMasterserverSignup {
                enabled: true,
                config: config.clone(),
                reference: failed_reference,
                completion: failed_tx,
                cancellation: failed_cancellation,
                transition: failed_transition,
            })
            .await
            .expect("attempt masterserver signup");
        failed_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("failed masterserver signup completion")
            .expect_err("failed Start rolls the runtime back");

        let (enabled_tx, enabled_rx) = mpsc::channel();
        let (_enabled_cancel, enabled_cancellation) = tokio::sync::oneshot::channel();
        let enabled_transition = Arc::new(std::sync::atomic::AtomicU8::new(
            MASTERSERVER_SIGNUP_PENDING,
        ));
        command_tx
            .send(NetworkCommand::SetMasterserverSignup {
                enabled: true,
                config: config.clone(),
                reference: reference.clone(),
                completion: enabled_tx,
                cancellation: enabled_cancellation,
                transition: enabled_transition,
            })
            .await
            .expect("enable masterserver signup");
        second_start_seen_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second Start request reaches the HTTP server");
        let mut puncher_stream = tokio::time::timeout(Duration::from_secs(2), puncher.accept())
            .await
            .expect("live signup contacts puncher before Start responds")
            .expect("accept live host puncher session");
        let puncher_request = tokio::time::timeout(Duration::from_secs(2), async {
            let mut header = [0_u8; 5];
            puncher_stream.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], 0xff);
            let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut payload = vec![0; length];
            puncher_stream.read_exact(&mut payload).await.unwrap();
            payload
        })
        .await
        .expect("live puncher ID request arrives before Start responds");
        assert_eq!(
            clonk_network::decode_netpuncher_packet(&puncher_request).unwrap(),
            clonk_network::NetpuncherPacket::IdRequest
        );
        assert!(matches!(enabled_rx.try_recv(), Err(TryRecvError::Empty)));
        release_second_start_tx
            .send(())
            .expect("release successful Start response");
        let response = enabled_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("masterserver signup enable completion")
            .expect("masterserver signup enables")
            .expect("fresh enable returns Start response");
        assert_eq!(response.league.as_bytes(), b"Cup");
        assert_eq!(response.seed, Some(305_419_896));
        assert_eq!(response.max_players, 4);
        assert_eq!(
            response.stream_to.as_bytes(),
            b"https://stream.example/upload?"
        );

        let (disabled_tx, disabled_rx) = mpsc::channel();
        let (_disabled_cancel, disabled_cancellation) = tokio::sync::oneshot::channel();
        let disabled_transition = Arc::new(std::sync::atomic::AtomicU8::new(
            MASTERSERVER_SIGNUP_PENDING,
        ));
        command_tx
            .send(NetworkCommand::SetMasterserverSignup {
                enabled: false,
                config,
                reference,
                completion: disabled_tx,
                cancellation: disabled_cancellation,
                transition: disabled_transition,
            })
            .await
            .expect("disable masterserver signup");
        disabled_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("masterserver signup disable completion")
            .expect("masterserver signup disables");

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop host worker");
        worker
            .await
            .expect("join host worker")
            .expect("host worker exits cleanly");

        let bodies = league_server.join().expect("join league HTTP fixture");
        assert_eq!(bodies.len(), 3, "shutdown must not repeat the live End");
        assert!(bodies[0]
            .windows(b"Action=Start".len())
            .any(|window| window == b"Action=Start"));
        assert!(bodies[1]
            .windows(b"Action=Start".len())
            .any(|window| window == b"Action=Start"));
        assert!(bodies[2]
            .windows(b"Action=End".len())
            .any(|window| window == b"Action=End"));
        assert!(
            bodies[2]
                .windows(b"RandomSeed=305419896".len())
                .any(|window| window == b"RandomSeed=305419896"),
            "End must identify the Start-updated reference, not the pre-Start command copy"
        );
        puncher.shutdown().await.unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn worker_finishes_live_end_when_lobby_drops_cleanup_handle() {
        let (end_seen_tx, end_seen_rx) = mpsc::channel();
        let (release_end_tx, release_end_rx) = mpsc::channel();
        let (endpoint, league_server) = league_http_fixture_with_before_reply(
            vec![
                b"[Response]\r\nStatus=Success\r\nCSID=session\r\nLeague=Cup\r\nSeed=305419896\r\n",
                b"[Response]\r\nStatus=Success\r\n",
            ],
            move |reply_index| {
                if reply_index == 1 {
                    end_seen_tx.send(()).expect("report in-flight End");
                    release_end_rx
                        .recv_timeout(Duration::from_secs(5))
                        .expect("release in-flight End response");
                }
            },
        );
        let settings = HostSettings {
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            player_name: "Host".to_string(),
            prepared: None,
        };
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (_control_tick_tx, mut control_tick_rx) = tokio_mpsc::unbounded_channel();
        let (_control_performance_tx, mut control_performance_rx) = tokio_mpsc::unbounded_channel();
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_host_worker(
                settings,
                0,
                &mut command_rx,
                &mut control_tick_rx,
                &mut control_performance_rx,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
            )
            .await
        });
        local_id_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("host worker readiness timeout")
            .expect("host worker readiness");

        let reference = minimal_league_reference();
        let config = PreparedLeagueHostConfig {
            endpoint,
            transport: clonk_network::LeagueHttpTransportConfig::default(),
            update_period_secs: 120,
            league_server_signup: false,
        };
        let (enabled_tx, enabled_rx) = mpsc::channel();
        let (_enabled_cancel, enabled_cancellation) = tokio::sync::oneshot::channel();
        command_tx
            .send(NetworkCommand::SetMasterserverSignup {
                enabled: true,
                config: config.clone(),
                reference: reference.clone(),
                completion: enabled_tx,
                cancellation: enabled_cancellation,
                transition: Arc::new(std::sync::atomic::AtomicU8::new(
                    MASTERSERVER_SIGNUP_PENDING,
                )),
            })
            .await
            .expect("enable masterserver signup");
        enabled_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("masterserver signup enable completion")
            .expect("masterserver signup enables");

        let (disabled_tx, disabled_rx) = mpsc::channel();
        let (disabled_cancel, disabled_cancellation) = tokio::sync::oneshot::channel();
        let disabled_transition = Arc::new(std::sync::atomic::AtomicU8::new(
            MASTERSERVER_SIGNUP_PENDING,
        ));
        command_tx
            .send(NetworkCommand::SetMasterserverSignup {
                enabled: false,
                config,
                reference,
                completion: disabled_tx,
                cancellation: disabled_cancellation,
                transition: Arc::clone(&disabled_transition),
            })
            .await
            .expect("queue compensating End");
        end_seen_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("End reaches the league server");
        drop(disabled_cancel);
        release_end_tx.send(()).expect("release End response");
        drop(disabled_rx);
        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("queue worker shutdown behind End");
        worker
            .await
            .expect("join host worker")
            .expect("host worker exits cleanly");

        assert_eq!(
            disabled_transition.load(Ordering::Acquire),
            MASTERSERVER_SIGNUP_FINISHED
        );
        let bodies = league_server.join().expect("join league HTTP fixture");
        assert_eq!(bodies.len(), 2, "End must complete before worker shutdown");
        assert!(bodies[1]
            .windows(b"Action=End".len())
            .any(|window| window == b"Action=End"));
        assert!(bodies[1]
            .windows(b"RandomSeed=305419896".len())
            .any(|window| window == b"RandomSeed=305419896"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_worker_adds_udp_route_on_the_configured_server_endpoint() {
        let (listener, udp_proxy, address) = reserve_tcp_and_udp_at_same_address().await;
        let host = start_host(
            listener,
            HostConfig {
                udp_bind_address: Some(SocketAddr::from(([127, 0, 0, 1], 0))),
                ..HostConfig::default()
            },
        )
        .await
        .expect("start dual-transport host");
        let host_udp_addr = host.udp_local_addr().expect("host UDP address");
        let (udp_seen_tx, mut udp_seen_rx) = tokio_mpsc::unbounded_channel();
        let udp_proxy_task = tokio::spawn(async move {
            let mut client_addr = None;
            let mut buffer = vec![0_u8; 65_536];
            loop {
                let (length, source) = udp_proxy
                    .recv_from(&mut buffer)
                    .await
                    .expect("receive proxied UDP datagram");
                if source == host_udp_addr {
                    if let Some(client_addr) = client_addr {
                        udp_proxy
                            .send_to(&buffer[..length], client_addr)
                            .await
                            .expect("forward host UDP datagram");
                    }
                } else {
                    client_addr = Some(source);
                    let _ = udp_seen_tx.send(());
                    udp_proxy
                        .send_to(&buffer[..length], host_udp_addr)
                        .await
                        .expect("forward client UDP datagram");
                }
            }
        });
        let temporary = tempfile::tempdir().expect("temporary client resource directory");
        let mut settings = ClientSettings::new(address, "Alice");
        settings.resource_directory = temporary.path().join("Network");
        let (command_tx, mut command_rx) = tokio_mpsc::channel(8);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
            )
            .await
        });

        local_id_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("client worker readiness timeout")
            .expect("client worker readiness");
        tokio::time::timeout(Duration::from_secs(4), udp_seen_rx.recv())
            .await
            .expect("client worker UDP contact timeout")
            .expect("client worker contacted UDP on the configured server endpoint");

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop client worker");
        worker
            .await
            .expect("join client worker")
            .expect("client worker exits cleanly");
        host.shutdown().await.expect("host shutdown");
        udp_proxy_task.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn disconnected_client_ignores_stale_controls_before_reporting_league_loss() {
        let (endpoint, league_server) =
            league_http_fixture(vec![b"[Response]\r\nStatus=Success\r\n"]);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let address = listener.local_addr().expect("host address");
        let mut host_config = HostConfig::default();
        host_config
            .initial_join_snapshot
            .as_mut()
            .expect("default JoinData")
            .parameters
            .league_address = clonk_engine::LegacyCString::from_bytes(endpoint.into_bytes())
            .expect("fixture endpoint is NUL-free");
        let host = start_host(listener, host_config).await.expect("start host");

        let temporary = tempfile::tempdir().expect("temporary client resource directory");
        let mut settings = ClientSettings::new(address, "Alice");
        settings.resource_directory = temporary.path().join("Network");
        let (command_tx, command_rx) = tokio_mpsc::channel(16);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
            )
            .await
        });

        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("client worker readiness timeout")
            .expect("client worker readiness");
        assert!(ready.league_runtime_available);
        let remove =
            clonk_engine::ControlPacket::ClientRemove(clonk_engine::ClientRemoveControlData {
                client_id: i32::try_from(ready.local_client_id).unwrap(),
                reason: clonk_engine::LegacyCString::default(),
                by_client: i32::try_from(HOST_CLIENT_ID).unwrap(),
            });
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&remove).expect("encode client removal"),
        )
        .await
        .expect("close client connection");
        let disconnect_deadline = std::time::Instant::now() + Duration::from_secs(4);
        let mut seen_events = Vec::new();
        loop {
            let remaining =
                disconnect_deadline.saturating_duration_since(std::time::Instant::now());
            let event = event_rx.recv_timeout(remaining).unwrap_or_else(|error| {
                panic!("client disconnect event timeout ({error}); saw {seen_events:?}")
            });
            if matches!(event, NetworkEvent::PeerDisconnected { client_id: 0, .. }) {
                break;
            }
            seen_events.push(event);
        }

        // A frame already queued by the running game must not touch the dead
        // ClientHandle and terminate the command bridge before this report.
        command_tx
            .send(NetworkCommand::FinalizeTick { tick: 1 })
            .await
            .expect("queue stale frame");
        let (completion, completed) = mpsc::channel();
        command_tx
            .send(NetworkCommand::LeagueReportDisconnect {
                reason: clonk_network::LeagueDisconnectReason::ConnectionFailed,
                players: clonk_network::ClientPlayerInfosSnapshot {
                    client_id: i32::try_from(ready.local_client_id).unwrap(),
                    flags: 0,
                    players: vec![clonk_engine::ControlPlayerInfoEntry {
                        id: 17,
                        flags: clonk_engine::PLAYER_INFO_FLAG_JOINED,
                        ..Default::default()
                    }],
                },
                fbids: clonk_network::LeagueFbidRegistry::new(),
                completion,
            })
            .await
            .expect("queue league disconnect report");
        assert_eq!(
            completed
                .recv_timeout(Duration::from_secs(4))
                .expect("league disconnect completion"),
            Ok(())
        );
        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop disconnected client worker");
        worker
            .await
            .expect("join client worker")
            .expect("client worker exits cleanly");
        host.shutdown().await.expect("stop host");

        let bodies = league_server.join().expect("join league HTTP fixture");
        assert!(bodies[0]
            .windows(b"Action=ReportDisconnect".len())
            .any(|window| window == b"Action=ReportDisconnect"));
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
        let mut client = clonk_network::connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let client_id = client.client_id();
        let wire_client_id = i32::try_from(client_id).expect("client ID fits wire field");
        let name = clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec())
            .expect("static client name is NUL-free");
        let mut parameters = snapshot.parameters;
        parameters.clients = clonk_network::JoinClientRegistrySnapshot {
            clients: vec![
                host_core,
                clonk_engine::ClientCoreControlData {
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
        let expected = clonk_network::JoinDataEnvelope {
            client_id: wire_client_id,
            start_control_tick: snapshot.dynamic_tick,
            status: host_status,
            dynamic: snapshot.dynamic,
            parameters,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
                None,
            )
        );
        assert!(matches!(local_id_rx.try_recv(), Err(TryRecvError::Empty)));
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
        let host_config = HostConfig {
            udp_bind_address: Some(address),
            ..HostConfig::default()
        };
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
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, _local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
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
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: wire_client_id,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_worker_rebases_deactivated_presend_input_to_first_activated_tick() {
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
        let (command_tx, command_rx) = tokio_mpsc::channel(32);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                3,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
            )
            .await
        });
        let ready = local_id_rx
            .recv_timeout(Duration::from_secs(4))
            .expect("client worker readiness timeout")
            .expect("client worker readiness");
        let client_id = ready.local_client_id;
        loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("client join timeout")
            {
                Some(HostEvent::ClientJoined {
                    client_id: joined, ..
                }) if joined == client_id => break,
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("transport error before queued input: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before client join"),
            }
        }

        command_tx
            .send(NetworkCommand::AcknowledgeRequestedStatus {
                status: expected_status,
                current_control_tick: expected_status.target_tick,
                current_frame: 41,
            })
            .await
            .expect("queue reached status");
        command_tx
            .send(NetworkCommand::SubmitLocal {
                owner: 3,
                event: ControlEvent::Press(ControlButton::Right),
                tick: 2,
            })
            .await
            .expect("queue inactive input");
        command_tx
            .send(NetworkCommand::FinalizeTick { tick: 0 })
            .await
            .expect("probe inactive finalization");

        loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("activation request timeout")
            {
                Some(HostEvent::ActivationRequest {
                    client_id: requested,
                    tick,
                    ..
                }) if requested == client_id => {
                    assert_eq!(tick, 41);
                    break;
                }
                Some(HostEvent::Ready { .. }) => {
                    panic!("deactivated client emitted a control contribution")
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("inactive input caused a transport error: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before activation request"),
            }
        }

        let host_frame = |tick| {
            encode_control_packet(&LegacyControlFrame {
                client_id: HOST_CLIENT_ID,
                tick,
                timestamp_ms: 0,
                controls: Vec::new(),
            })
            .expect("encode host control")
        };
        host.submit_local_control(host_frame(0))
            .await
            .expect("advance host-only tick");
        loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("host-only control timeout")
            {
                Some(HostEvent::Ready { packet }) if packet.tick() == 0 => break,
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("host-only tick failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before host-only tick"),
            }
        }

        let update = clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: i32::try_from(client_id).expect("client ID fits i32"),
            data: 1,
            by_client: i32::try_from(HOST_CLIENT_ID).expect("host ID fits i32"),
        };
        host.submit_packet(
            ControlDelivery::Sync,
            encode_control_entry_payload(&clonk_engine::ControlPacket::ClientUpdate(
                update.clone(),
            ))
            .expect("encode activation control"),
        )
        .await
        .expect("submit activation control");
        loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("activation execution timeout")
            {
                Some(HostEvent::SyncScheduled { controls, .. })
                    if controls
                        == vec![clonk_engine::ControlPacket::ClientUpdate(update.clone())] =>
                {
                    break;
                }
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("activation control failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before activation execution"),
            }
        }
        command_tx
            .send(NetworkCommand::ClientUpdateExecuted(update))
            .await
            .expect("report executed activation");
        command_tx
            .send(NetworkCommand::FinalizeTick { tick: 1 })
            .await
            .expect("finalize first activated tick");
        host.submit_local_control(host_frame(1))
            .await
            .expect("complete first activated tick");

        let packet = loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("activated control timeout")
            {
                Some(HostEvent::Ready { packet }) if packet.tick() == 1 => break packet,
                Some(HostEvent::TransportError { error, .. }) => {
                    panic!("activated input failed: {error}")
                }
                Some(_) => continue,
                None => panic!("host event stream ended before activated input"),
            }
        };
        let frame = decode_control_packet(&packet).expect("decode complete activated tick");
        assert!(frame.controls.iter().any(|control| matches!(
            control,
            clonk_engine::ControlPacket::PlayerControl(data)
                if data.by_client == i32::try_from(client_id).unwrap()
                    && data.command == i32::from(COM_RIGHT)
        )));

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn client_join_flow_worker_uses_every_address_and_supplied_password() {
        let closed = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind disposable route");
        let closed_address = closed.local_addr().expect("disposable route address");
        drop(closed);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind passworded host");
        let live_address = listener.local_addr().expect("passworded host address");
        let password = clonk_engine::LegacyCString::from_bytes(b"join secret".to_vec())
            .expect("fixture is NUL-free");
        let host_config = HostConfig {
            password: password.clone(),
            ..Default::default()
        };
        let host = start_host(listener, host_config).await.expect("start host");
        let temporary = tempfile::tempdir().expect("temporary client work directory");
        let mut settings = ClientSettings::new(closed_address, "Alice")
            .with_join_attempts([
                NetworkAddress::new(NetworkProtocol::Tcp, closed_address),
                NetworkAddress::new(NetworkProtocol::Tcp, live_address),
            ])
            .with_password(password);
        settings.resource_directory = temporary.path().join("Network");
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (event_tx, _event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
            )
            .await
        });

        assert!(matches!(
            local_id_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker reports readiness"),
            Ok(NetworkWorkerReady { local_client_id, .. }) if local_client_id > 0
        ));
        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop connected client worker");
        worker
            .await
            .expect("join worker task")
            .expect("stop worker");
        host.shutdown().await.expect("stop host");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn client_worker_uses_the_installed_system_for_cpp_cross_build_join() {
        // C++ cannot transfer NRT_System (src/C4Network2Res.cpp:1458-1461).
        // The Rust app already executes its installed System group, matching
        // Application.SystemGroup ownership after C++ bootstrap
        // (src/C4Application.cpp:127-134; src/C4Game.cpp:2764-2793).
        let temporary = tempfile::tempdir().expect("temporary resource roots");
        let host_root = temporary.path().join("host");
        let client_root = temporary.path().join("client");
        let host_system = host_root.join("System.c4g");
        let client_system = client_root.join("System.c4g");
        std::fs::create_dir_all(&host_system).expect("host System directory");
        std::fs::create_dir_all(&client_system).expect("client System directory");
        std::fs::write(host_system.join("Host.c"), b"C++ host system")
            .expect("host System contents");
        std::fs::write(client_system.join("Client.c"), b"Rust client system")
            .expect("client System contents");
        let publication = clonk_network::build_host_resource_core(
            &host_system,
            &host_root,
            clonk_network::HostResourceCoreSpec::new(
                clonk_network::HostResourceType::System,
                2,
                clonk_engine::LegacyCString::from_bytes(b"System.c4g".to_vec())
                    .expect("static resource name"),
                "C++ host",
            ),
        )
        .expect("publish host System");
        let mut host_config = HostConfig::default();
        let snapshot = host_config
            .initial_join_snapshot
            .as_mut()
            .expect("default JoinData");
        snapshot.dynamic.id = 3;
        snapshot.parameters.scenario.id = 0;
        snapshot
            .parameters
            .game_resources
            .push(publication.core.clone());
        host_config.resource_directory = Some(host_root);
        host_config.resource_files = vec![clonk_network::HostedResourceFile {
            core: publication.core.clone(),
            path: host_system,
            ownership: clonk_network::ResourceFileOwnership::Persistent,
            binary_compatible: false,
        }];
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind C++-style host");
        let address = listener.local_addr().expect("host address");
        let host = start_host(listener, host_config).await.expect("start host");
        let mut settings = ClientSettings::new(address, "Alice")
            .with_join_attempts([NetworkAddress::new(NetworkProtocol::Tcp, address)]);
        settings.mesh_udp_bind_address = None;
        settings.resource_directory = client_root.join("Network");
        settings.local_system_path = Some(client_system.clone());
        let (command_tx, command_rx) = tokio_mpsc::channel(8);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let (local_id_tx, local_id_rx) = mpsc::channel();
        let worker = tokio::spawn(async move {
            let mut command_rx = command_rx;
            run_client_worker(
                settings,
                0,
                &mut command_rx,
                &mut tokio_mpsc::unbounded_channel().1,
                &mut tokio_mpsc::unbounded_channel().1,
                event_tx,
                telemetry_tx,
                local_id_tx,
                test_netpuncher_state(),
                Arc::new(AtomicI32::new(0)),
                None,
            )
            .await
        });

        assert!(matches!(
            local_id_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker reports readiness"),
            Ok(NetworkWorkerReady { local_client_id, .. }) if local_client_id > 0
        ));
        let mut saw_join_data = false;
        let mut saw_system = false;
        while !saw_join_data || !saw_system {
            match event_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("worker emits bootstrap events")
            {
                NetworkEvent::JoinData(_) => saw_join_data = true,
                NetworkEvent::ResourceComplete {
                    resource_id: 2,
                    core,
                    path,
                    local,
                } => {
                    assert_eq!(core, publication.core);
                    assert_eq!(path, client_system);
                    assert!(local);
                    saw_system = true;
                }
                NetworkEvent::Error(error) => panic!("client worker failed: {error}"),
                _ => {}
            }
        }

        command_tx
            .send(NetworkCommand::Shutdown)
            .await
            .expect("stop client worker");
        worker
            .await
            .expect("join worker task")
            .expect("stop worker");
        host.shutdown().await.expect("stop host");
    }

    #[test]
    fn manager_queues_player_info_update_without_fabricating_an_author() {
        // C4PacketPlayerInfoUpdRequest carries C4ClientPlayerInfos unchanged
        // and has no C4ControlPacket ByClient field
        // (src/C4Network2Players.cpp:142-166;
        // src/C4PlayerInfo.cpp:1800-1803).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: 1,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
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
    fn managers_queue_control_set_with_authenticated_local_author() {
        // C4MessageInput submits `/set` through CDT_Decide. During the frozen
        // lobby C4GameControlNetwork resolves that delivery to CDT_Sync, while
        // retaining the submitting client for the setting's HostControl check.
        let set = LegacyControlSet {
            value_type: 2,
            data: 4,
            by_client: -1,
        };

        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.submit_control_set(set)
            .expect("host queues synchronized CID_Set");
        assert_eq!(
            host_commands.take_submitted_control_sets(),
            vec![LegacyControlSet {
                by_client: 0,
                ..set
            }]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_control_set(set)
            .expect("client queues synchronized CID_Set");
        assert_eq!(
            client_commands.take_submitted_control_sets(),
            vec![LegacyControlSet {
                by_client: 7,
                ..set
            }]
        );
    }

    #[test]
    fn decided_controls_stamp_authors_and_clients_always_queue() {
        let set = LegacyControlSet {
            value_type: 1,
            data: 0,
            by_client: -1,
        };

        let (host, _events, mut host_commands) = NetworkManager::test_stub_with_commands();
        host.submit_decided_control_set(12, set, true)
            .expect("queue frozen-host decided control");
        let submitted = host_commands.take_submitted_decided_controls();
        let [(tick, control, sync)] = submitted.as_slice() else {
            panic!("expected one host decided control");
        };
        assert_eq!((*tick, *sync), (12, true));
        assert_eq!(
            LegacyControlSet::from_control_packet(control),
            Some(LegacyControlSet {
                by_client: 0,
                ..set
            })
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_decided_control_set(13, set, true)
            .expect("queue client decided control");
        let submitted = client_commands.take_submitted_decided_controls();
        let [(tick, control, sync)] = submitted.as_slice() else {
            panic!("expected one client decided control");
        };
        assert_eq!((*tick, *sync), (13, false));
        assert_eq!(
            LegacyControlSet::from_control_packet(control),
            Some(LegacyControlSet {
                by_client: 7,
                ..set
            })
        );
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
            target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: clonk_engine::LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
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
            answer: clonk_engine::LegacyCString::from_bytes(b"q\"\\z".to_vec())
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
        let request = clonk_network::ClientPlayerResourceRequest {
            source_path: PathBuf::from("Alice.c4p"),
            wire_name: clonk_engine::LegacyCString::from_bytes(b"Alice.c4p".to_vec())
                .expect("fixture filename is NUL-free"),
            group_maker: clonk_engine::LegacyCString::from_bytes(b"Alice".to_vec())
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
        let core = clonk_engine::NetworkResourceCore {
            resource_type: clonk_network::HostResourceType::Player as u8,
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
    fn client_manager_waits_until_the_mutable_resource_is_protected() {
        // C4Player::Save may replace a synchronized player file only after
        // C4Network2Res::Derive has rescued the parent's serving bytes and
        // installed an anonymous derived resource (src/C4Player.cpp:452-461;
        // src/C4Network2Res.cpp:718-776).
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let source_path = PathBuf::from("Alice.c4p");
        let expected_path = source_path.clone();
        let caller = thread::spawn(move || {
            manager.begin_resource_derive(
                23,
                source_path,
                clonk_network::ResourceFileOwnership::Persistent,
            )
        });

        let NetworkCommand::BeginResourceDerive {
            resource_id,
            source_path,
            ownership,
            completion,
        } = commands
            .command_rx
            .blocking_recv()
            .expect("resource derivation command")
        else {
            panic!("expected resource derivation command");
        };
        assert_eq!(resource_id, 23);
        assert_eq!(source_path, expected_path);
        assert_eq!(ownership, clonk_network::ResourceFileOwnership::Persistent);
        assert!(!caller.is_finished(), "the source is not protected yet");
        completion
            .send(Err("rescue failed".to_owned()))
            .expect("complete resource derivation");

        assert_eq!(
            caller
                .join()
                .expect("resource derivation caller exits")
                .expect_err("the backend error is retained")
                .to_string(),
            "rescue failed"
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
    fn client_manager_can_request_merged_dynamic_removal_without_waiting() {
        let (manager, _events, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);

        let removed = manager
            .remove_client_resource_async(23)
            .expect("queue resource removal");
        let (resource_id, completion) = commands.receive_resource_removal();
        assert_eq!(resource_id, 23);
        assert_eq!(removed.try_recv(), Err(TryRecvError::Empty));

        completion
            .send(Err("remove failed".to_owned()))
            .expect("complete resource removal");
        assert_eq!(
            removed.recv().expect("resource-removal result"),
            Err("remove failed".to_owned())
        );
    }

    #[test]
    fn host_manager_cannot_request_async_client_resource_removal() {
        let (manager, _events, _commands) = NetworkManager::test_stub_with_commands();

        let error = manager
            .remove_client_resource_async(23)
            .expect_err("host must not queue a client resource removal");

        assert_eq!(
            error.to_string(),
            "only a network client may remove a network resource"
        );
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
                clonk_engine::InitScenarioPlayerControlData {
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
                clonk_engine::InitScenarioPlayerControlData {
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
                clonk_engine::RemovePlayerControlData {
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
                clonk_engine::SurrenderPlayerControlData {
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
                clonk_engine::SurrenderPlayerControlData {
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
    fn host_manager_waits_for_atomic_go_and_surfaces_worker_failure() {
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        let caller = thread::spawn(move || {
            let applied = manager.begin_go(status, false);
            let rejected = manager.begin_go(status, true);
            (applied, rejected)
        });

        assert_eq!(
            commands.complete_lobby_start(Ok(())),
            vec![TestLobbyStartCommand::BeginGo {
                status,
                join_allowed: false,
            }]
        );
        assert!(
            !caller.is_finished(),
            "the second transition still awaits its worker result"
        );
        assert_eq!(
            commands.complete_lobby_start(Err("host loop rejected Go".to_string())),
            vec![TestLobbyStartCommand::BeginGo {
                status,
                join_allowed: true,
            }]
        );
        let (applied, rejected) = caller.join().expect("Go caller");
        applied.expect("first atomic transition applies");
        assert_eq!(
            rejected
                .expect_err("worker rejection reaches caller")
                .to_string(),
            "host loop rejected Go"
        );
    }

    #[test]
    fn host_manager_queues_cpp_status_change_and_local_reach() {
        // ChangeGameStatus is host-only and CheckStatusReached records the
        // host's local arrival independently of remote acknowledgements
        // (src/C4Network2.cpp:2017-2051,2053-2086).
        let (manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };

        manager
            .change_status(status)
            .expect("host queues status change");
        assert_eq!(commands.take_status_changes(), vec![status]);

        manager
            .status_reached(status, status.target_tick)
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
            state: clonk_network::NETWORK_STATE_GO,
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
            state: clonk_network::NETWORK_STATE_PAUSE,
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
    fn client_manager_rejects_a_drained_stale_status_before_acking_the_latest() {
        let (mut manager, event_tx, mut commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        let first = NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: 2,
            target_tick: 41,
        };
        let latest = NetworkStatus {
            target_tick: 44,
            ..first
        };
        event_tx
            .send(NetworkEvent::StatusRequested(first))
            .expect("queue first host status");
        event_tx
            .send(NetworkEvent::StatusRequested(latest))
            .expect("queue retargeted host status");
        assert_eq!(
            manager.poll_events(),
            vec![
                NetworkEvent::StatusRequested(first),
                NetworkEvent::StatusRequested(latest),
            ]
        );

        assert_eq!(
            manager
                .acknowledge_expected_status_at_frame(first, 44, 123)
                .expect_err("the drained first event must not consume the newer request"),
            NetworkStatusCommandError::NoRequestedStatus
        );
        assert_eq!(manager.client_status.requested, Some(latest));
        assert_eq!(manager.client_status.awaiting_commit, None);
        assert!(commands.take_status_acknowledgements().is_empty());

        manager
            .acknowledge_expected_status_at_frame(latest, 44, 123)
            .expect("the latest request remains acknowledgeable");
        assert_eq!(
            commands.take_framed_status_acknowledgements(),
            vec![(latest, 123)]
        );
        assert_eq!(manager.client_status.awaiting_commit, Some(latest));
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
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
        };
        let now = tokio::time::Instant::now();

        activation.arm_for_queued_player_info(&request);
        assert_eq!(activation.request_tick_if_due(now, 40), None);

        activation.status_reached();
        assert_eq!(activation.request_tick_if_due(now, 41), Some(41));
    }

    #[test]
    fn queued_control_arms_only_a_deactivated_non_observer_client() {
        let mut activation = ClientActivationState::default();
        let now = tokio::time::Instant::now();
        activation.status_reached();

        activation.arm_for_queued_control();
        assert_eq!(activation.request_tick_if_due(now, 123), Some(123));
        assert!(!activation.can_finalize());

        let activate = clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 7,
            data: 1,
            by_client: 0,
        };
        activation.apply_executed_client_update(7, &activate);
        activation.arm_for_queued_control();
        assert_eq!(activation.request_tick_if_due(now, 124), None);
        assert!(activation.can_finalize());

        activation.apply_executed_client_update(
            7,
            &clonk_engine::ClientUpdateControlData {
                data: 0,
                ..activate.clone()
            },
        );
        activation.arm_for_queued_control();
        assert_eq!(activation.request_tick_if_due(now, 125), Some(125));
        assert!(!activation.can_finalize());

        activation.apply_executed_client_update(
            7,
            &clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                data: 1,
                ..activate
            },
        );
        activation.arm_for_queued_control();
        assert_eq!(activation.request_tick_if_due(now, 126), None);
        assert!(!activation.can_finalize());
    }

    #[test]
    fn client_activation_retries_at_five_seconds_with_the_latest_frame() {
        // A non-host with an outstanding activation request calls
        // RequestActivate again from Execute. The strict interval check allows
        // the request at exactly 5,000 ms, and each packet carries the then
        // current Game.FrameCounter (src/C4Network2.cpp:739-743,2116-2145;
        // src/C4Network2.h:57-60).
        let mut activation = ClientActivationState::default();
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached();
        activation.mark_requested(first_sent_at);

        assert_eq!(
            activation.request_tick_if_due(
                first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL - Duration::from_millis(1),
                51,
            ),
            None
        );
        assert_eq!(
            activation.request_tick_if_due(first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL, 52,),
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
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached();
        activation.mark_requested(first_sent_at);

        activation.status_requested();
        let overdue = first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL;
        assert_eq!(activation.request_tick_if_due(overdue, 60), None);

        activation.status_reached();
        assert_eq!(activation.request_tick_if_due(overdue, 61), Some(61));
    }

    #[test]
    fn client_activation_clears_only_for_executed_host_local_activation() {
        // CUT_Activate is trusted only from the host and changes the local
        // activation state when C4ControlClientUpdate executes. A false update,
        // another client's update, or a client-authored update cannot stop the
        // outstanding RequestActivate retries (src/C4Control.cpp:578-606;
        // src/C4Network2.cpp:2116-2145).
        let mut activation = ClientActivationState::default();
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached();
        activation.mark_requested(first_sent_at);
        let retry_at = first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL;

        for update in [
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 1,
                by_client: 3,
            },
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 8,
                data: 1,
                by_client: 0,
            },
            clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
        ] {
            activation.apply_executed_client_update(7, &update);
            assert_eq!(activation.request_tick_if_due(retry_at, 41), Some(41));
        }

        activation.apply_executed_client_update(
            7,
            &clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
                client_id: 7,
                data: 1,
                by_client: 0,
            },
        );
        assert_eq!(activation.request_tick_if_due(retry_at, 41), None);
    }

    #[test]
    fn empty_initial_player_info_never_arms_client_activation() {
        // The empty initial packet is still sent so the host can return all
        // player infos, but JoinLocalPlayer calls RequestActivate only when at
        // least one player was present (src/C4Network2Players.cpp:124-136).
        let mut activation = ClientActivationState::default();
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: Vec::new(),
        };
        activation.arm_for_queued_player_info(&request);
        activation.status_reached();

        assert_eq!(
            activation.request_tick_if_due(tokio::time::Instant::now(), 41),
            None
        );
    }

    #[test]
    fn executed_host_observer_update_clears_client_activation() {
        // CUT_SetObserver deactivates the local client and RequestActivate then
        // clears its outstanding retry state (src/C4Control.cpp:607-619;
        // src/C4Network2.cpp:2116-2122).
        let mut activation = ClientActivationState::default();
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
        };
        let first_sent_at = tokio::time::Instant::now();
        activation.arm_for_queued_player_info(&request);
        activation.status_reached();
        activation.mark_requested(first_sent_at);

        activation.apply_executed_client_update(
            7,
            &clonk_engine::ClientUpdateControlData {
                update_type: clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                client_id: 7,
                data: 0,
                by_client: 0,
            },
        );

        assert_eq!(
            activation.request_tick_if_due(first_sent_at + CLIENT_ACTIVATION_RETRY_INTERVAL, 41,),
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
            state: clonk_network::NETWORK_STATE_LOBBY,
            control_mode: 0,
            target_tick: 23,
        };
        let request = clonk_network::PlayerInfoUpdateRequest {
            client_id: 7,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![clonk_engine::ControlPlayerInfoEntry::default()],
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
        let update = clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
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
        let join_data = clonk_network::JoinDataEnvelope {
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
            state: clonk_network::NETWORK_STATE_GO,
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
            state: clonk_network::NETWORK_STATE_GO,
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
            state: clonk_network::NETWORK_STATE_GO,
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
                .status_reached(status, status.target_tick)
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
        let synchronize = clonk_engine::SynchronizeControlData {
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
                clonk_engine::ControlPacket::PlayerControl(right.clone()),
                clonk_engine::ControlPacket::Synchronize(synchronize.clone()),
                clonk_engine::ControlPacket::SyncCheck(check.clone()),
                clonk_engine::ControlPacket::PlayerControl(left.clone()),
            ],
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

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
        let (event_tx, event_rx) = NetworkEventSender::channel();

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
    fn debug_record_projection_round_trips_opaque_bytes() {
        let packet =
            clonk_engine::ControlPacket::DebugRecord(clonk_engine::DebugRecordControlData {
                data: vec![0x00, 0xff, b'C', b'4'],
            });

        let projected = network_control_for_packet(packet.clone())
            .expect("known CID_DebugRec remains in the execution stream");
        assert_eq!(
            projected,
            NetworkControl::DebugRecord(clonk_engine::DebugRecordControlData {
                data: vec![0x00, 0xff, b'C', b'4'],
            })
        );
        assert_eq!(projected.into_packet(), Some(packet));
    }

    #[test]
    fn ready_frame_retains_admission_controls_in_decoded_order() {
        // C4Control executes the same list order used by PreExecute, and the
        // complete network control preserves each client's packet order
        // (src/C4Control.cpp:73-109;
        // src/C4GameControlNetwork.cpp:741-769).
        let info = clonk_engine::PlayerInfoControlData {
            client_id: 3,
            by_client: 4,
            ..Default::default()
        };
        let join = clonk_engine::JoinPlayerControlData {
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
                clonk_engine::ControlPacket::PlayerInfo(info.clone()),
                clonk_engine::ControlPacket::PlayerControl(player.clone()),
                clonk_engine::ControlPacket::JoinPlayer(join.clone()),
            ],
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

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
        let surrender = clonk_engine::SurrenderPlayerControlData {
            player: 7,
            by_client: 3,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![clonk_engine::ControlPacket::SurrenderPlayer(surrender)],
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

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
        let selection = clonk_engine::InitScenarioPlayerControlData {
            team: 2,
            player: 4,
            by_client: 7,
        };
        let frame = LegacyControlFrame {
            client_id: HOST_CLIENT_ID,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![clonk_engine::ControlPacket::InitScenarioPlayer(selection)],
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

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
        let update = clonk_engine::ClientUpdateControlData {
            update_type: clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        };
        let remove = clonk_engine::ClientRemoveControlData {
            client_id: 4,
            reason: clonk_engine::LegacyCString::from_bytes(b"bye".to_vec()).expect("valid reason"),
            by_client: 0,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

        emit_scheduled_sync_controls(
            23,
            vec![
                clonk_engine::ControlPacket::ClientUpdate(update.clone()),
                clonk_engine::ControlPacket::ClientRemove(remove.clone()),
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
        let result = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 0,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();

        emit_scheduled_sync_controls(
            23,
            vec![clonk_engine::ControlPacket::VoteEnd(result)],
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
    async fn host_recoverable_route_diagnostic_stays_typed_for_the_app() {
        // C++ gives only a failed speculative/secondary connection warning
        // severity because it may have no player impact; ordinary packet
        // failures remain errors (src/C4Network2IO.cpp:252-261,808-834).
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();
        let error = "secondary TCP route closed".to_string();

        handle_host_event(
            HostEvent::RecoverableRouteDiagnostic {
                client_id: Some(7),
                error: error.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward recoverable route diagnostic");

        assert_eq!(
            event_rx.recv().expect("recoverable route diagnostic"),
            NetworkEvent::RecoverableRouteDiagnostic {
                client_id: Some(7),
                error,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_transport_diagnostic_preserves_peer_identity_for_the_app() {
        // Malformed peer input closes only that connection; the host packet
        // loop remains available to every other client
        // (src/C4Network2IO.cpp:808-834).
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();
        let error = "failed to decode direct control packet".to_string();

        handle_host_event(
            HostEvent::TransportError {
                client_id: Some(7),
                error: error.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward transport diagnostic");

        assert_eq!(
            event_rx.recv().expect("transport diagnostic"),
            NetworkEvent::TransportDiagnostic {
                client_id: Some(7),
                error,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fatal_host_lockstep_event_stays_fatal_for_the_app() {
        // PackCompleteCtrl is the authoritative host path for advancing a
        // complete control tick. If that path fails after coordination has
        // advanced, the app must clear network control rather than presenting
        // a recoverable peer warning
        // (src/C4GameControlNetwork.cpp:741-777;
        // src/C4Network2.cpp:746-789).
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();
        let error = "failed to aggregate ready tick 23".to_string();

        handle_host_event(
            HostEvent::FatalError {
                error: error.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward fatal host event");

        assert_eq!(
            event_rx.recv().expect("fatal app event"),
            NetworkEvent::FatalError(error)
        );
    }

    fn unsupported_local_control_frame(client_id: ClientId) -> LegacyControlFrame {
        LegacyControlFrame {
            client_id,
            tick: 23,
            timestamp_ms: 0,
            controls: vec![clonk_engine::ControlPacket::Unknown {
                id: clonk_engine::ControlPacketId(0xfe),
                name: None,
                fields: HashMap::new(),
            }],
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_host_control_encode_failure_stops_the_worker_path() {
        // C4GameControlNetwork cannot advance a tick after its local
        // C4GameControlPacket fails to compile. Application failure clears
        // C4Network2 rather than leaving the installed lockstep clock waiting
        // forever (src/C4GameControlNetwork.cpp:741-777;
        // src/C4Network2.cpp:475-510,746-789).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");

        let error = send_frame_to_host(&host, unsupported_local_control_frame(HOST_CLIENT_ID))
            .await
            .expect_err("unencodable local host control must stop the worker path");

        assert!(error
            .to_string()
            .starts_with("failed to encode host control packet:"));
        host.shutdown().await.expect("shutdown host");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_client_control_encode_failure_stops_the_worker_path() {
        // A client which cannot compile its own control contribution cannot
        // satisfy the next complete-control barrier. Native failure tears down
        // C4Network2 instead of treating that condition as a log-only peer
        // diagnostic (src/C4GameControlNetwork.cpp:48-60,741-777;
        // src/C4Network2.cpp:746-789).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let address = listener.local_addr().expect("host address");
        let host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let client = clonk_network::connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("connect client");

        let error =
            send_frame_to_client(&client, unsupported_local_control_frame(client.client_id()))
                .await
                .expect_err("unencodable local client control must stop the worker path");

        assert!(error
            .to_string()
            .starts_with("failed to encode client control packet:"));
        client.graceful_part().await.expect("part client");
        host.shutdown().await.expect("shutdown host");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn authoritative_player_info_encode_failure_stops_the_host_worker_path() {
        // C4ControlPlayerInfo is host-authored synchronized state. If it cannot
        // be compiled, continuing the session would leave host and clients
        // with different admission state (src/C4Network2Players.cpp:512-544;
        // src/C4GameControlNetwork.cpp:741-777).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let info = PlayerInfoControlData {
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 17,
                flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: None,
                ..Default::default()
            }],
            ..Default::default()
        };

        let error = send_player_info_from_host(&host, info)
            .await
            .expect_err("unencodable authoritative PlayerInfo must stop the worker path");

        assert!(error
            .to_string()
            .starts_with("failed to encode authoritative PlayerInfo:"));
        host.shutdown().await.expect("shutdown host");
    }

    #[test]
    fn production_worker_reports_authoritative_encode_failure_as_fatal() {
        // The network-thread boundary must translate an unrecoverable local
        // compiler failure into FatalError only after the worker has exited.
        // The application can then follow C4Network2::Clear instead of
        // retaining a lockstep clock with no producer
        // (src/C4Network2.cpp:475-510,746-789).
        let manager = NetworkManager::for_mode(
            NetworkMode::Host(HostSettings {
                bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
                player_name: "Host".to_string(),
                prepared: None,
            }),
            0,
        )
        .expect("start production host worker");
        manager
            .broadcast_player_info(PlayerInfoControlData {
                players: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 17,
                    flags: clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE,
                    resource: None,
                    ..Default::default()
                }],
                ..Default::default()
            })
            .expect("queue malformed authoritative PlayerInfo");

        let fatal = loop {
            match manager.event_rx.recv_timeout(Duration::from_secs(2)) {
                Ok(NetworkEvent::FatalError(error)) => break error,
                Ok(_) => continue,
                Err(error) => panic!("fatal worker event was not delivered: {error}"),
            }
        };

        assert!(fatal.starts_with("failed to encode authoritative PlayerInfo:"));
    }

    #[test]
    fn authoritative_ready_decode_failure_stops_the_worker_path() {
        // PID_Control is the one complete, authoritative tick released to
        // C4GameControl. A packet which cannot be decoded cannot be skipped:
        // the application loop must fail and C4Network2::Clear must remove the
        // dead network-control clock (src/C4GameControlNetwork.cpp:741-777;
        // src/C4Network2.cpp:475-510,746-789).
        let packet = ControlPacket::builder(clonk_network::BROADCAST_CLIENT_ID, 23)
            .timestamp_ms(0)
            .payload(Vec::new());
        let (event_tx, event_rx) = NetworkEventSender::channel();

        let error = handle_ready_packet(packet, 0, &event_tx)
            .expect_err("malformed authoritative control must stop the worker path");

        assert!(error
            .to_string()
            .starts_with("failed to decode authoritative control packet:"));
        assert!(
            event_rx.try_recv().is_err(),
            "the worker boundary owns the single FatalError notification"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn final_route_loss_maps_the_complete_nonfatal_cleanup_sequence() {
        // Native OnDisconnect logs the lost route, calls OnClientDisconnect
        // only after the client's final connection is gone, and keeps the
        // host/lobby alive while CtrlRemove performs synchronized cleanup
        // (src/C4Network2.cpp:1774-1824).
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind host listener");
        let address = listener.local_addr().expect("host address");
        let mut host = start_host(listener, HostConfig::default())
            .await
            .expect("start host");
        let mut host_events = host.take_event_receiver();
        let client = clonk_network::connect_client(
            address,
            ClientConfig::new("Alice", ParticipantKind::Player),
        )
        .await
        .expect("connect client");
        let client_id = client.client_id();
        loop {
            match tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("client join timeout")
            {
                Some(HostEvent::ClientJoined {
                    client_id: joined, ..
                }) if joined == client_id => break,
                Some(_) => continue,
                None => panic!("host event stream ended before client join"),
            }
        }

        client
            .graceful_part()
            .await
            .expect("part client with native reason");
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();
        let netpuncher_state = test_netpuncher_state();
        let mut forwarded = Vec::new();
        loop {
            let event = tokio::time::timeout(Duration::from_secs(2), host_events.recv())
                .await
                .expect("disconnect event timeout")
                .expect("host event stream during disconnect");
            handle_host_event(
                event,
                0,
                &event_tx,
                &telemetry_tx,
                &mut player_info_echo_provenance,
                &netpuncher_state,
            )
            .await
            .expect("forward disconnect event");
            forwarded.extend(event_rx.try_iter());
            if forwarded.iter().any(|event| {
                matches!(
                    event,
                    NetworkEvent::RecoverableRouteDiagnostic { .. }
                        | NetworkEvent::TransportDiagnostic { .. }
                        | NetworkEvent::Error(_)
                )
            }) {
                break;
            }
        }

        assert!(
            !forwarded
                .iter()
                .any(|event| matches!(event, NetworkEvent::Error(_))),
            "ordinary final-route loss became an app-fatal network error: {forwarded:?}"
        );
        let failure = forwarded
            .iter()
            .position(|event| {
                matches!(
                    event,
                    NetworkEvent::PeerConnectionFailed { client_id: failed }
                        if *failed == client_id
                )
            })
            .expect("peer failure notification");
        let departure = forwarded
            .iter()
            .position(|event| {
                matches!(
                    event,
                    NetworkEvent::PeerDisconnected {
                        client_id: departed,
                        ..
                    } if *departed == client_id
                )
            })
            .expect("peer departure notification");
        let diagnostic = forwarded
            .iter()
            .position(|event| {
                matches!(
                    event,
                    NetworkEvent::RecoverableRouteDiagnostic {
                        client_id: Some(source),
                        error,
                    } if *source == client_id && error == "removing client"
                )
            })
            .expect("typed final-route diagnostic");
        assert!(failure < departure && departure < diagnostic);

        host.shutdown().await.expect("shutdown host");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_status_change_is_forwarded_to_the_app() {
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_PAUSE,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::StatusChanged(status),
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward requested status");

        assert_eq!(
            event_rx.recv().expect("status event"),
            NetworkEvent::HostStatusChanged(status)
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runtime_join_data_request_is_forwarded_to_the_app() {
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::JoinDataNeeded {
                client_id: 7,
                current_control_tick: 23,
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward runtime JoinData request");

        assert_eq!(
            event_rx.recv().expect("runtime JoinData event"),
            NetworkEvent::JoinDataNeeded {
                client_id: 7,
                current_control_tick: 23,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_status_ack_is_forwarded_to_the_app_with_client_identity() {
        let client_id = 7;
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::StatusAck { client_id, status },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward client status acknowledgement");

        assert_eq!(
            event_rx.recv().expect("status acknowledgement event"),
            NetworkEvent::HostStatusAck { client_id, status }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_puncher_assignment_publishes_one_atomic_app_snapshot() {
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let state = test_netpuncher_state();
        let mut provenance = VecDeque::new();
        let addresses = vec![NetworkAddress::new(
            NetworkProtocol::Udp,
            "198.51.100.9:43123".parse().unwrap(),
        )];

        handle_host_event(
            HostEvent::LocalAddressesChanged {
                local_addresses: addresses.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut provenance,
            &state,
        )
        .await
        .unwrap();
        assert_eq!(state.lock().local_addresses, addresses);
        assert!(matches!(event_rx.try_recv(), Err(TryRecvError::Empty)));

        let game_ids = clonk_network::NetpuncherGameIds {
            ipv4: 0x1122_3344,
            ipv6: 0,
        };
        handle_host_event(
            HostEvent::NetpuncherStateChanged {
                game_ids,
                local_addresses: addresses.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut provenance,
            &state,
        )
        .await
        .unwrap();

        let snapshot = state.lock().clone();
        assert_eq!(snapshot.game_ids, game_ids);
        assert_eq!(snapshot.local_addresses, addresses);
        assert_eq!(
            event_rx.recv().unwrap(),
            NetworkEvent::NetpuncherStateChanged {
                game_ids,
                local_addresses: addresses,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_status_commit_is_forwarded_to_the_app() {
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::StatusCommitted(status),
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward committed status");

        assert_eq!(
            event_rx.recv().expect("status event"),
            NetworkEvent::StatusCommitted(status)
        );
    }

    /// The notice is queued on the same command channel the teardown uses, so
    /// FIFO ordering — not a completion the app would have to block on — is
    /// what keeps `Shutdown` behind it.
    #[test]
    fn only_a_host_broadcasts_a_restart_notice() {
        let (host, _host_events, mut host_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(0);
        let (client, _client_events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);

        host.broadcast_host_restarting(30)
            .expect("host queues the restart notice");
        assert!(client.broadcast_host_restarting(30).is_err());

        assert_eq!(host_commands.take_host_restart_broadcasts(), vec![30]);
        assert!(client_commands.take_host_restart_broadcasts().is_empty());
    }

    /// The app is the only layer that can act on a restart notice — it owns the
    /// round teardown and the rejoin — so the worker must surface it verbatim
    /// rather than folding it into the disconnect that follows.
    #[tokio::test(flavor = "current_thread")]
    async fn a_host_restart_notice_is_forwarded_to_the_app() {
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);

        handle_client_event(
            ClientEvent::HostRestarting { rejoin_seconds: 30 },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward host restart notice");

        assert_eq!(
            event_rx.recv().expect("restart notice event"),
            NetworkEvent::HostRestarting { rejoin_seconds: 30 }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn lobby_countdown_is_forwarded_to_the_app_for_host_and_client() {
        // The host applies its locally constructed packet directly and each
        // client receives the same PID_LobbyCountdown through MainDlg
        // (src/C4GameLobby.cpp:392-418,1111-1131).
        let packet = clonk_network::LobbyCountdownPacket::new(5);
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::LobbyCountdown { packet },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
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
    async fn resource_chunk_progress_is_forwarded_to_the_app_for_host_and_client() {
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::ResourceProgress {
                resource_id: 17,
                present_percent: 40,
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("forward host resource progress");
        handle_client_event(
            ClientEvent::ResourceProgress {
                resource_id: 23,
                present_percent: 75,
            },
            0,
            7,
            &event_tx,
            &telemetry_tx,
        )
        .await
        .expect("forward client resource progress");

        assert_eq!(
            event_rx.recv().expect("host resource progress event"),
            NetworkEvent::ResourceProgress {
                resource_id: 17,
                present_percent: 40,
            }
        );
        assert_eq!(
            event_rx.recv().expect("client resource progress event"),
            NetworkEvent::ResourceProgress {
                resource_id: 23,
                present_percent: 75,
            }
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn host_ready_check_is_forwarded_to_the_app_unchanged() {
        // C4Network2::HandlePacket passes the compiled packet, including its
        // claimed Client field, directly to HandleReadyCheck
        // (src/C4Network2.cpp:949-953,1625-1635).
        let packet = clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::ReadyCheck { packet },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
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
        host.submit_ready_check(clonk_network::ReadyCheckData::Ready)
            .expect("host submits ready state");
        assert_eq!(
            host_commands.take_submitted_ready_checks(),
            vec![clonk_network::ReadyCheckPacket {
                client_id: 0,
                data: clonk_network::ReadyCheckData::Ready,
            }]
        );

        let (client, _events, mut client_commands) =
            NetworkManager::test_stub_with_commands_for_client_id(7);
        client
            .submit_ready_check(clonk_network::ReadyCheckData::NotReady)
            .expect("client submits not-ready state");
        assert_eq!(
            client_commands.take_submitted_ready_checks(),
            vec![clonk_network::ReadyCheckPacket {
                client_id: 7,
                data: clonk_network::ReadyCheckData::NotReady,
            }]
        );
    }

    #[test]
    fn host_manager_queues_cpp_lobby_countdown_packet() {
        // Countdown's constructor broadcasts the initial timer verbatim as
        // PID_LobbyCountdown before installing its one-second callback
        // (src/C4GameLobby.cpp:1111-1130).
        let (host, _events, mut commands) = NetworkManager::test_stub_with_commands();
        let packet = clonk_network::LobbyCountdownPacket::new(5);

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
            .submit_lobby_countdown(clonk_network::LobbyCountdownPacket::new(5))
            .is_err());
        assert!(commands.take_submitted_lobby_countdowns().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn client_status_request_waits_for_app_preparation() {
        // HandleStatus stores the host-authored status, but the client sends
        // PID_StatusAck only after CheckStatusReached observes local arrival
        // (src/C4Network2.cpp:2017-2051,2053-2086).
        let status = NetworkStatus {
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 1,
            target_tick: 23,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
        let packet = clonk_network::ReadyCheckPacket {
            client_id: 9,
            data: clonk_network::ReadyCheckData::NotReady,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
        let packet = clonk_network::LeagueRoundResultsPacket {
            success: true,
            result_string: clonk_engine::LegacyCString::from_bytes(b"Result:\xe4".to_vec())
                .unwrap(),
            players: vec![clonk_network::LeagueRoundResultsPlayer {
                player_info_id: 17,
                total_playing_time: 1_234,
                settlement_score_old: -2,
                settlement_score_new: 300,
                league_score_new: 1_500,
                league_score_gain: 25,
                league_rank_new: 3,
                league_rank_symbol_new: 9,
                league_progress_data: clonk_engine::LegacyCString::from_bytes(b"A=1\xff".to_vec())
                    .unwrap(),
                status: clonk_network::LeagueRoundPlayerStatus::Won,
            }],
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
            state: clonk_network::NETWORK_STATE_GO,
            control_mode: 2,
            target_tick: 41,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
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
        let countdown = clonk_network::LobbyCountdownPacket::new(10);
        let request = clonk_network::ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: clonk_network::ReadyCheckData::Request,
        };
        let ready = clonk_network::ReadyCheckPacket {
            client_id: 7,
            data: clonk_network::ReadyCheckData::Ready,
        };
        let host_ready = clonk_network::ReadyCheckPacket {
            client_id: HOST_CLIENT_ID as i32,
            data: clonk_network::ReadyCheckData::Ready,
        };
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut player_info_echo_provenance = VecDeque::new();

        handle_host_event(
            HostEvent::LobbyCountdown { packet: countdown },
            0,
            &event_tx,
            &telemetry_tx,
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
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
            &mut player_info_echo_provenance,
            &test_netpuncher_state(),
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
        let countdown = clonk_network::LobbyCountdownPacket::new(30);
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
                clonk_network::ReadyCheckPacket {
                    client_id: 0,
                    data: clonk_network::ReadyCheckData::Request,
                }
            ))
        ));
        assert!(matches!(
            host_commands.command_rx.try_recv(),
            Ok(NetworkCommand::SubmitReadyCheck(
                clonk_network::ReadyCheckPacket {
                    client_id: 0,
                    data: clonk_network::ReadyCheckData::Ready,
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
                clonk_network::ReadyCheckPacket {
                    client_id: 7,
                    data: clonk_network::ReadyCheckData::Ready,
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
            state: clonk_network::NETWORK_STATE_GO,
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
        let payload = clonk_network::encode_control_entry_payload(
            &clonk_engine::ControlPacket::PlayerInfo(info.clone()),
        )
        .expect("encode direct PlayerInfo payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();

        handle_direct_packet(clonk_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct PlayerInfo");

        let NetworkEvent::DirectControl(NetworkControl::PlayerInfo(actual)) =
            event_rx.recv().expect("direct control event")
        else {
            panic!("expected one immediate PlayerInfo event");
        };
        assert_eq!(actual, info);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn remote_player_info_does_not_consume_preexecuted_broadcast_provenance() {
        let info = PlayerInfoControlData {
            client_id: 3,
            by_client: 0,
            ..Default::default()
        };
        let payload = clonk_network::encode_control_entry_payload(
            &clonk_engine::ControlPacket::PlayerInfo(info.clone()),
        )
        .expect("encode direct PlayerInfo payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();
        let (telemetry_tx, _telemetry_rx) = mpsc::sync_channel(NETWORK_TELEMETRY_CAPACITY);
        let mut provenance = VecDeque::from([PlayerInfoEchoProvenance::Preexecuted {
            original: info.clone(),
            join_players_on_echo: vec![clonk_engine::ControlPlayerInfoEntry {
                id: 41,
                ..Default::default()
            }],
        }]);

        handle_host_event(
            HostEvent::Direct {
                client_id: 7,
                delivery: clonk_network::ControlDelivery::Direct,
                data: payload.clone(),
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("handle remote PlayerInfo");
        handle_host_event(
            HostEvent::Direct {
                client_id: clonk_network::BROADCAST_CLIENT_ID,
                delivery: clonk_network::ControlDelivery::Direct,
                data: payload,
            },
            0,
            &event_tx,
            &telemetry_tx,
            &mut provenance,
            &test_netpuncher_state(),
        )
        .await
        .expect("handle preexecuted PlayerInfo loopback");

        assert_eq!(
            event_rx.recv().expect("remote direct event"),
            NetworkEvent::DirectControl(NetworkControl::PlayerInfo(info.clone()))
        );
        assert_eq!(
            event_rx.recv().expect("preexecuted loopback event"),
            NetworkEvent::PreexecutedPlayerInfoEcho {
                original: info.clone(),
                info,
                join_players_on_echo: vec![clonk_engine::ControlPlayerInfoEntry {
                    id: 41,
                    ..Default::default()
                }],
            }
        );
        assert!(provenance.is_empty());
    }

    #[test]
    fn private_message_emits_an_immediate_control_event() {
        // Despite the per-message Private subtype, C4MessageInput uses
        // CDT_Private for every C4ControlMessage. HandleControlPkt executes
        // that delivery immediately instead of entering SyncControl
        // (src/C4MessageInput.cpp:423-426;
        // src/C4GameControlNetwork.cpp:558-566).
        let message = message_control(7);
        let payload = clonk_network::encode_control_entry_payload(
            &clonk_engine::ControlPacket::Message(message.clone()),
        )
        .expect("encode private CID_Message payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();

        handle_direct_packet(clonk_network::ControlDelivery::Private, payload, &event_tx)
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
        let join = clonk_engine::ClientJoinControlData {
            core: clonk_engine::ClientCoreControlData {
                client_id: 7,
                name: clonk_engine::LegacyCString::from_bytes(b"Client".to_vec()).unwrap(),
                ..Default::default()
            },
            by_client: 0,
        };
        let payload = clonk_network::encode_control_entry_payload(
            &clonk_engine::ControlPacket::ClientJoin(join.clone()),
        )
        .expect("encode direct ClientJoin payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();

        handle_direct_packet(clonk_network::ControlDelivery::Direct, payload, &event_tx)
            .expect("handle direct ClientJoin");

        let NetworkEvent::DirectControl(NetworkControl::ClientJoin(actual)) =
            event_rx.recv().expect("direct control event")
        else {
            panic!("expected one immediate ClientJoin event");
        };
        let mut expected = join;
        // C4ClientCore's binary reader always runs VAL_NameNoEmpty, including
        // for an explicitly encoded empty Nick.
        expected.core.nick = clonk_engine::LegacyCString::from_bytes(b"empty".to_vec()).unwrap();
        assert_eq!(actual, expected);
        assert!(event_rx.try_recv().is_err());
    }

    #[test]
    fn direct_vote_emits_an_immediate_authenticated_control_event() {
        // C4Network2::Vote submits CID_Vote through CDT_Direct, so the
        // authenticated ByClient ballot executes immediately instead of
        // entering the synchronized control queue
        // (src/C4Network2.cpp:2842-2868;
        // src/C4GameControlNetwork.cpp:449-490,558-566).
        let vote = clonk_engine::VoteControlData {
            vote_type: clonk_engine::VOTE_TYPE_KICK,
            approve: true,
            data: 7,
            by_client: 7,
        };
        let payload =
            clonk_network::encode_control_entry_payload(&clonk_engine::ControlPacket::Vote(vote))
                .expect("encode direct Vote payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();

        handle_direct_packet(clonk_network::ControlDelivery::Direct, payload, &event_tx)
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
        let payload = clonk_network::encode_control_entry_payload(&set.into_control_packet())
            .expect("encode direct CID_Set payload");
        let (event_tx, event_rx) = NetworkEventSender::channel();

        handle_direct_packet(clonk_network::ControlDelivery::Direct, payload, &event_tx)
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
        let data = clonk_engine::C4MN_ADJUST_POSITION | 1;
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
            clonk_engine::ControlPacket::PlayerControl(expected.clone())
        );
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::PlayerControl(
                expected.clone()
            )),
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
            network_control_for_packet(clonk_engine::ControlPacket::PlayerControl(raw.clone())),
            Some(NetworkControl::PlayerControl(raw)),
            "decoded replay keeps the original signed command for CountControl"
        );
    }

    #[test]
    fn every_in_com_byte_survives_event_codec_and_network_dispatch() {
        let controls = (1..=u8::MAX)
            .map(|command| {
                let event = clonk_engine::interpret_player_control_command(i32::from(command))
                    .unwrap_or_else(|| panic!("command {command} was dropped"));
                let packet = control_packet_for_event(7, event, 3)
                    .unwrap_or_else(|| panic!("command {command} was not encoded"));
                assert_eq!(
                    packet,
                    clonk_engine::ControlPacket::PlayerControl(PlayerControlData {
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
            network_control_for_packet(clonk_engine::ControlPacket::PlayerCommand(command)),
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
            network_control_for_packet(clonk_engine::ControlPacket::PlayerSelect(
                selection.clone()
            )),
            Some(NetworkControl::PlayerSelect(selection))
        );
    }

    #[test]
    fn decoded_script_is_retained_for_scheduled_execution() {
        let script = ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_CONSOLE,
            strictness: clonk_engine::ScriptStrictness::Strict2,
            script: clonk_engine::LegacyCString::from_bytes(b"1+2".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::Script(script.clone())),
            Some(NetworkControl::Script(script))
        );
    }

    #[test]
    fn decoded_message_board_answer_is_retained_for_scheduled_execution() {
        let answer = MessageBoardAnswerControlData {
            object: 42,
            answer: clonk_engine::LegacyCString::from_bytes(b"typed answer".to_vec())
                .expect("answer is NUL-free"),
            player: 3,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::MessageBoardAnswer(
                answer.clone(),
            )),
            Some(NetworkControl::MessageBoardAnswer(answer))
        );
    }

    #[test]
    fn decoded_custom_command_is_retained_for_scheduled_execution() {
        let command = clonk_engine::CustomCommandControlData {
            command: clonk_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("command is NUL-free"),
            argument: clonk_engine::LegacyCString::from_bytes(b"arg".to_vec())
                .expect("argument is NUL-free"),
            player: 3,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(
                clonk_engine::ControlPacket::CustomCommand(command.clone(),)
            ),
            Some(NetworkControl::CustomCommand(command))
        );
    }

    #[test]
    fn decoded_em_move_object_is_retained_for_ordered_execution() {
        let control = clonk_engine::EmMoveObjectControlData {
            action: clonk_engine::EMMO_SCRIPT,
            tx: -12,
            ty: 34,
            target_object: 42,
            objects: vec![7, 9],
            strictness: clonk_engine::ScriptStrictness::Strict2,
            script: clonk_engine::LegacyCString::from_bytes(b"SetXDir(0)".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::EmMoveObject(control.clone(),)),
            Some(NetworkControl::EmMoveObject(control))
        );
    }

    #[test]
    fn decoded_em_draw_tool_is_retained_for_ordered_execution() {
        let control = clonk_engine::EmDrawToolControlData {
            action: clonk_engine::EMDT_LINE,
            mode: clonk_engine::LANDSCAPE_MODE_EXACT,
            x: -12,
            y: 34,
            x2: 56,
            y2: -78,
            grade: 9,
            ift: true,
            material: clonk_engine::LegacyCString::from_bytes(b"Earth".to_vec())
                .expect("material is NUL-free"),
            texture: clonk_engine::LegacyCString::from_bytes(b"Smooth".to_vec())
                .expect("texture is NUL-free"),
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::EmDrawTool(control.clone(),)),
            Some(NetworkControl::EmDrawTool(control))
        );
    }

    #[test]
    fn decoded_em_drop_def_is_retained_for_ordered_execution() {
        let control = clonk_engine::EmDropDefControlData {
            id: *b"HUT2",
            x: -130,
            y: 130,
            by_client: 4,
        };
        assert_eq!(
            network_control_for_packet(clonk_engine::ControlPacket::EmDropDef(control)),
            Some(NetworkControl::EmDropDef(control))
        );
    }

    #[test]
    fn decoded_internal_player_scripts_are_retained_for_ordered_execution() {
        let cases = vec![
            (
                clonk_engine::ControlPacket::ActivateGameGoalMenu(
                    clonk_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ActivateGameGoalMenu(
                    clonk_engine::ActivateGameGoalMenuControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
            ),
            (
                clonk_engine::ControlPacket::ToggleHostility(
                    clonk_engine::ToggleHostilityControlData {
                        opponent: 5,
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ToggleHostility(clonk_engine::ToggleHostilityControlData {
                    opponent: 5,
                    player: 3,
                    by_client: 4,
                }),
            ),
            (
                clonk_engine::ControlPacket::ActivateGameGoalRule(
                    clonk_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::ActivateGameGoalRule(
                    clonk_engine::ActivateGameGoalRuleControlData {
                        object: 42,
                        player: 3,
                        by_client: 4,
                    },
                ),
            ),
            (
                clonk_engine::ControlPacket::SetPlayerTeam(
                    clonk_engine::SetPlayerTeamControlData {
                        team: 6,
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::SetPlayerTeam(clonk_engine::SetPlayerTeamControlData {
                    team: 6,
                    player: 3,
                    by_client: 4,
                }),
            ),
            (
                clonk_engine::ControlPacket::EliminatePlayer(
                    clonk_engine::EliminatePlayerControlData {
                        player: 3,
                        by_client: 4,
                    },
                ),
                NetworkControl::EliminatePlayer(clonk_engine::EliminatePlayerControlData {
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
                    clonk_engine::ControlPacket::ActivateGameGoalMenu(
                        clonk_engine::ActivateGameGoalMenuControlData {
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    21,
                    clonk_engine::ControlPacket::ToggleHostility(
                        clonk_engine::ToggleHostilityControlData {
                            opponent: 4,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    22,
                    clonk_engine::ControlPacket::ActivateGameGoalRule(
                        clonk_engine::ActivateGameGoalRuleControlData {
                            object: 42,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    23,
                    clonk_engine::ControlPacket::SetPlayerTeam(
                        clonk_engine::SetPlayerTeamControlData {
                            team: 5,
                            player: 3,
                            by_client: 7,
                        },
                    ),
                ),
                (
                    24,
                    clonk_engine::ControlPacket::EliminatePlayer(
                        clonk_engine::EliminatePlayerControlData {
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
        let command = clonk_engine::CustomCommandControlData {
            command: clonk_engine::LegacyCString::from_bytes(b"push".to_vec())
                .expect("command is NUL-free"),
            argument: clonk_engine::LegacyCString::from_bytes(b"arg".to_vec())
                .expect("argument is NUL-free"),
            player: 3,
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            clonk_engine::ControlPacket::CustomCommand(command.clone()),
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
            vec![clonk_engine::ControlPacket::CustomCommand(command)]
        );
    }

    #[test]
    fn message_board_answer_frame_roundtrips_through_the_tick_accumulator() {
        let answer = MessageBoardAnswerControlData {
            object: 42,
            answer: clonk_engine::LegacyCString::from_bytes(b"typed answer".to_vec())
                .expect("answer is NUL-free"),
            player: 3,
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(
            12,
            clonk_engine::ControlPacket::MessageBoardAnswer(answer.clone()),
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
            vec![clonk_engine::ControlPacket::MessageBoardAnswer(answer)]
        );
    }

    #[test]
    fn script_frame_roundtrips_through_the_tick_accumulator() {
        let script = ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_GLOBAL,
            strictness: clonk_engine::ScriptStrictness::Strict3,
            script: clonk_engine::LegacyCString::from_bytes(b"SetGravity(77)".to_vec())
                .expect("script is NUL-free"),
            by_client: 4,
        };
        let mut accumulator = ControlFrameAccumulator::new(4);
        accumulator.record_control(12, clonk_engine::ControlPacket::Script(script.clone()), 100);
        let frame = accumulator
            .finalize_tick(12)
            .expect("script control produces a control frame");

        let encoded = encode_control_packet(&frame).expect("encode accumulated frame");
        assert_eq!(
            decode_control_packet(&encoded).expect("decode accumulated frame"),
            frame
        );
        assert_eq!(
            frame.controls,
            vec![clonk_engine::ControlPacket::Script(script)]
        );
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
        accumulator.record_control(12, clonk_engine::ControlPacket::PlayerCommand(command), 100);
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
            vec![clonk_engine::ControlPacket::PlayerCommand(command)]
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
            clonk_engine::ControlPacket::PlayerSelect(selection.clone()),
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
            vec![clonk_engine::ControlPacket::PlayerSelect(selection)]
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
    fn control_clock_display_tick_retains_the_executed_tick_between_cadence_frames() {
        let mut clock = NetworkControlClock::new(40, 4);
        assert_eq!(clock.display_control_tick_for_frame(0), 40);
        clock.complete_control_frame();
        assert_eq!(clock.current_tick(), 41);
        for frame in 1..4 {
            assert_eq!(clock.display_control_tick_for_frame(frame), 40);
        }
        assert_eq!(clock.display_control_tick_for_frame(4), 41);
    }

    #[test]
    fn runtime_connection_api_fails_promptly_without_a_worker() {
        let (host, _events, _commands) = NetworkManager::test_stub_with_commands();
        assert!(host
            .runtime_connections()
            .expect_err("stub has no live route worker")
            .to_string()
            .contains("without a network worker"));
        assert!(host
            .disconnect_runtime_connection(7)
            .expect_err("stub cannot retire a live route")
            .to_string()
            .contains("without a network worker"));
    }

    #[test]
    fn manager_control_consumption_does_not_wait_for_the_worker_actor() {
        // C4GameControlNetwork::CalcPerformance samples the already-owned
        // route topology synchronously in GetControl; the later network
        // bookkeeping must not make the game thread wait for scheduler work
        // (oracle-src-pinned src/C4GameControlNetwork.cpp:382-447).
        let (mut manager, _events, mut commands) = NetworkManager::test_stub_with_commands();
        manager.control_send_time =
            clonk_network::ControlSendTimeSnapshot::from_preferred_message_routes(
                0,
                HOST_CLIENT_ID,
                [HOST_CLIENT_ID, 7, 8],
                [(7, 100), (8, 300)],
            );
        manager.worker = Some(std::thread::spawn(|| {}));
        let (result_tx, result_rx) = mpsc::channel();
        let caller = std::thread::spawn(move || {
            let before = tokio::time::Instant::now();
            let sample = manager.control_tick_consumed(7, vec![HOST_CLIENT_ID, 7, 8]);
            let after = tokio::time::Instant::now();
            let _ = result_tx.send((sample, before, after));
            manager
        });

        let observed = result_rx.recv_timeout(Duration::from_millis(250));
        let queued = commands.control_performance_rx.try_recv();
        let queued_consumed_at = match &queued {
            Ok(ControlPerformanceEvent::TickConsumed {
                tick: 7,
                consumed_at,
                client_ids,
                ..
            }) if client_ids == &vec![HOST_CLIENT_ID, 7, 8] => Some(*consumed_at),
            _ => None,
        };
        drop(queued);
        drop(commands);
        let manager = caller.join().expect("join game-thread probe");
        drop(manager);

        let (sample, before, after) =
            observed.expect("control consumption must not wait for worker command processing");
        assert_eq!(
            sample.map(|cost| cost.send_time_ms),
            Some(66),
            "the game thread reads the latest topology snapshot directly"
        );
        let consumed_at =
            queued_consumed_at.expect("consumption bookkeeping remains queued in order");
        assert!((before..=after).contains(&consumed_at));

        let mut clock = NetworkControlClock::new(0, 1);
        clock.observe_control_send_time_ms(sample.expect("live worker sample").send_time_ms);
        // 66ms of link is 2 frames at 38 fps, plus C++'s one-frame floor. The
        // ACT average beside it stays C++'s exact 1/150 EWMA of the same sample.
        assert_eq!(
            clock.calculate_performance(),
            Some(ControlPreSendChange {
                control_presend: 3,
                target_fps: 38,
            })
        );
        assert_eq!(clock.avg_control_send_time(), 440);
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
    fn control_presend_uses_cpp_rolling_average_and_one_to_fifteen_clamp() {
        // The ACT field keeps C++'s exact rolling average: CalcPerformance
        // retains 149/150 of the previous microsecond average and adds 1/150 of
        // the latest topology-aware millisecond control-send sample
        // (src/C4GameControlNetwork.cpp:382-447). PreSend itself no longer
        // reads that average -- see the PORT_STATUS divergence entry -- so this
        // pins the average and the 1..15 clamp, and the sizing is pinned below.
        let mut clock = NetworkControlClock::new(0, 1);
        assert_eq!(clock.control_presend(), 1);
        assert_eq!(clock.avg_control_send_time(), 0);
        clock.observe_control_send_time_ms(300);
        for _ in 0..14 {
            clock.calculate_performance();
            clock.complete_control_frame();
        }
        assert_eq!(
            clock.avg_control_send_time(),
            26_813,
            "the script- and dialog-visible average stays bit-exact with C++"
        );

        let mut saturated = NetworkControlClock::new(0, 1);
        saturated.observe_control_send_time_ms(1_000);
        saturated.calculate_performance();
        saturated.complete_control_frame();
        assert_eq!(
            saturated.control_presend(),
            15,
            "a one-second link saturates the C++ 1..15 clamp"
        );
        for _ in 0..150 {
            saturated.calculate_performance();
            saturated.complete_control_frame();
        }
        assert_eq!(saturated.control_presend(), 15);
    }

    /// A slow *machine* is invisible to ping, and therefore invisible to
    /// PreSend.
    ///
    /// `CalcPerformance` sizes the horizon from `pConn->getPingTime()` alone
    /// (src/C4GameControlNetwork.cpp:404-430), and `iTargetFPS` is a hardcoded
    /// constant rather than a measurement, so a client whose frame loop is late
    /// — healthy ping, late control — never buys itself any headroom. Its input
    /// then misses the async deadline on essentially every tick and is dropped
    /// silently. Measured with `cargo xtask chaos`: a Pi-class machine on a
    /// *good* link leaves the healthy players blocked on 91% of ticks and 10 s
    /// behind over an 11 s session, while a dial-up link on a good machine costs
    /// them almost nothing.
    ///
    /// C++ already computes the right quantity — `AddPerf(pCtrl->getTime() -
    /// iWaitStart)`, real per-client control arrival — and spends it only on an
    /// F7 display string.
    #[test]
    fn a_slow_machine_with_a_healthy_ping_still_grows_presend() {
        let mut ping_only = NetworkControlClock::new(0, 1);
        ping_only.observe_control_send_time_ms(20);
        ping_only.calculate_performance();
        ping_only.complete_control_frame();

        let mut measured = NetworkControlClock::new(0, 1);
        measured.observe_control_send_time_ms(20);
        measured.observe_control_lateness_ms(300);
        measured.calculate_performance();
        measured.complete_control_frame();

        assert!(
            measured.control_presend() > ping_only.control_presend(),
            "a 300ms late tick behind a 20ms ping must widen the horizon: \
             ping-only chose {}, measured chose {}",
            ping_only.control_presend(),
            measured.control_presend()
        );
    }

    /// The horizon takes the *larger* of the two estimates, never a replacement.
    ///
    /// A client that is never late keeps exactly the horizon C++ would have
    /// chosen, so this change cannot regress a healthy link no matter how the
    /// lateness signal behaves. It also means the input-latency cost is charged
    /// only where it buys something, which is the same rule the envelope
    /// estimator was accepted under.
    #[test]
    fn measured_lateness_never_shrinks_the_horizon_below_the_ping_estimate() {
        let mut ping_only = NetworkControlClock::new(0, 1);
        let mut punctual = NetworkControlClock::new(0, 1);
        for _ in 0..8 {
            ping_only.observe_control_send_time_ms(300);
            ping_only.calculate_performance();
            ping_only.complete_control_frame();

            punctual.observe_control_send_time_ms(300);
            // Control keeps arriving before the client needs it.
            punctual.observe_control_lateness_ms(0);
            punctual.calculate_performance();
            punctual.complete_control_frame();
        }

        assert_eq!(
            punctual.control_presend(),
            ping_only.control_presend(),
            "a punctual client must keep the ping-derived horizon exactly"
        );
        assert_eq!(
            punctual.avg_control_send_time(),
            ping_only.avg_control_send_time(),
            "and the script-visible ACT must stay C++'s ping-derived average"
        );
    }

    /// Both inputs to the horizon are measured every control tick and neither
    /// could be read back out. The diagnostics overlay reports the estimate a
    /// stalling player's PreSend is actually being sized from, which is what
    /// separates a slow link from a slow machine from the outside — exactly
    /// the distinction `ACT` alone cannot draw, because it is ping-derived.
    #[test]
    fn the_measured_horizon_inputs_are_readable_after_a_control_tick() {
        let mut clock = NetworkControlClock::new(0, 1);
        assert_eq!(clock.control_lateness_ms(), None);
        assert_eq!(clock.control_latency_budget(), Duration::ZERO);

        clock.observe_control_send_time_ms(40);
        clock.observe_control_lateness_ms(300);
        clock.calculate_performance();
        clock.complete_control_frame();

        assert_eq!(clock.control_lateness_ms(), Some(300));
        assert_eq!(
            clock.control_latency_budget(),
            Duration::from_millis(300),
            "the envelope attacks straight to the larger of ping and lateness"
        );
    }

    /// The divergence itself: C++ needs 14 identical 300ms samples before
    /// PreSend leaves 1, and ~150 before it reflects the link at all
    /// (src/C4GameControlNetwork.cpp:382-447). Every control tick in that
    /// window stalls the whole session, so the port sizes the horizon from the
    /// first sample instead.
    #[test]
    fn control_presend_covers_the_link_from_the_first_sample() {
        let mut clock = NetworkControlClock::new(0, 1);
        clock.observe_control_send_time_ms(300);
        let change = clock
            .calculate_performance()
            .expect("the first sample already re-sizes PreSend");
        // 38 fps * 300ms = 11 frames of link, plus C++'s one-frame floor.
        assert_eq!(change.control_presend, 12);
        assert_eq!(clock.control_presend(), 12);

        // A steady link is not charged a variance premium on top.
        for _ in 0..200 {
            clock.calculate_performance();
            clock.complete_control_frame();
        }
        assert_eq!(clock.control_presend(), 12);
    }

    /// A link that gets slower is covered at once rather than after C++'s
    /// ~8 second rolling-average ramp; a link that recovers gives the latency
    /// back gradually, so one spike cannot pin the horizon high.
    #[test]
    fn control_presend_attacks_fast_and_decays_slowly() {
        let mut clock = NetworkControlClock::new(0, 1);
        clock.observe_control_send_time_ms(30);
        clock.calculate_performance();
        clock.complete_control_frame();
        let settled = clock.control_presend();

        clock.observe_control_send_time_ms(300);
        clock.calculate_performance();
        clock.complete_control_frame();
        let spiked = clock.control_presend();
        assert!(
            spiked >= 12,
            "a 300ms sample must be covered immediately, got {spiked}"
        );

        clock.observe_control_send_time_ms(30);
        for _ in 0..10 {
            clock.calculate_performance();
            clock.complete_control_frame();
        }
        assert!(
            clock.control_presend() >= spiked - 1,
            "ten fast samples may shave a frame but must not collapse the \
             horizon: {} from {spiked}",
            clock.control_presend()
        );
        for _ in 0..600 {
            clock.calculate_performance();
            clock.complete_control_frame();
        }
        assert_eq!(
            clock.control_presend(),
            settled,
            "but a sustained recovery returns the latency"
        );
    }

    #[test]
    fn control_presend_uses_live_target_fps_and_reports_only_changes() {
        // The horizon is a frame count, so the same link converts to a
        // different PreSend at a different target FPS. A 40ms link is 1.5
        // frames at 38 fps and 3 frames at 76 fps, plus C++'s one-frame floor.
        let mut clock = NetworkControlClock::new(0, 1);
        clock.observe_control_send_time_ms(40);
        assert_eq!(
            clock.calculate_performance(),
            Some(ControlPreSendChange {
                control_presend: 2,
                target_fps: 38,
            })
        );
        clock.complete_control_frame();

        clock.set_target_fps(76);
        assert_eq!(
            clock.calculate_performance(),
            Some(ControlPreSendChange {
                control_presend: 4,
                target_fps: 76,
            }),
            "setTargetFPS alone re-converts the same link"
        );
        clock.complete_control_frame();

        // C++ reports a change only when the value actually moves; repeated
        // identical samples stay silent (src/C4GameControlNetwork.cpp:382-447).
        for _ in 0..20 {
            assert_eq!(clock.calculate_performance(), None);
            clock.complete_control_frame();
        }
        assert_eq!(clock.avg_control_send_time(), 5_463);
    }

    #[test]
    fn control_presend_preserves_native_int32_target_arithmetic() {
        let mut clock = NetworkControlClock::new(0, 1);
        clock.set_target_fps(i32::MAX);
        clock.observe_control_send_time_ms(1);

        // Native stores the first EWMA sample as 6us, then evaluates the
        // int32 product INT_MAX * 6 as -6. Widening that product would
        // incorrectly saturate PreSend to 15.
        assert_eq!(clock.calculate_performance(), None);
        assert_eq!(clock.avg_control_send_time(), 6);
        assert_eq!(clock.control_presend(), 1);

        let mut ewma = NetworkControlClock::new(0, 1);
        ewma.observe_control_send_time_ms(i32::MAX);
        assert_eq!(ewma.calculate_performance(), None);
        assert_eq!(ewma.avg_control_send_time(), -6);
    }

    #[test]
    fn control_presend_stops_inclusively_at_target_tick_and_while_inactive() {
        let mut clock = NetworkControlClock::new(9, 2);

        assert!(clock.take_due_ticks(0, false).is_empty());
        assert_eq!(clock.take_due_ticks(0, true), vec![9]);
        clock.complete_control_frame();

        clock.set_target_tick(Some(10));
        assert_eq!(
            clock.take_due_ticks(1, true),
            vec![10],
            "presend emits the target tick itself"
        );
        assert!(clock.take_due_ticks(2, true).is_empty());
        clock.complete_control_frame();
        assert!(clock.take_due_ticks(3, true).is_empty());

        clock.set_target_tick(None);
        assert_eq!(
            clock.take_due_ticks(3, true),
            vec![11],
            "clearing the target resumes one-frame lookahead"
        );
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

    #[test]
    fn accumulator_rebases_all_held_input_to_the_first_activated_tick() {
        let control = control_packet_for_event(1, ControlEvent::Press(ControlButton::Right), 1)
            .expect("build control packet");
        for queued_tick in [9, 15] {
            let mut held = ControlFrameAccumulator::new(1);
            assert!(held.record_control(queued_tick, control.clone(), 30));
            held.rebase_pending_to_first_activated_tick(13);
            let frame = held
                .finalize_tick(13)
                .expect("activation tick emits held input");
            assert_eq!(frame.tick, 13);
            assert_eq!(frame.controls, vec![control.clone()]);
        }

        let mut ordinarily_scheduled = ControlFrameAccumulator::new(1);
        assert!(ordinarily_scheduled.record_control(15, control.clone(), 40));
        assert!(ordinarily_scheduled
            .finalize_tick(13)
            .expect("earlier empty contribution")
            .controls
            .is_empty());
        assert_eq!(
            ordinarily_scheduled
                .finalize_tick(15)
                .expect("future input keeps its tick")
                .controls,
            vec![control]
        );
    }
}
