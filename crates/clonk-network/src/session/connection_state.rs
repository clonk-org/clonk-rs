//! Per-connection state: outbound senders, route ping/lag, client resource & control state.
//!
//! This child module shares the parent session's private protocol machinery;
//! `session.rs` re-exports its crate-facing surface under the original paths.

use super::*;
use std::sync::atomic::AtomicBool;

#[derive(Debug)]
struct PostFailureBufferState<T> {
    accepting: bool,
    messages: VecDeque<T>,
}

#[derive(Debug)]
pub(crate) struct PostFailureBuffer<T> {
    state: Arc<Mutex<PostFailureBufferState<T>>>,
    accepting: Arc<AtomicBool>,
}

impl<T> Clone for PostFailureBuffer<T> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            accepting: self.accepting.clone(),
        }
    }
}

impl<T> Default for PostFailureBuffer<T> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(PostFailureBufferState {
                accepting: true,
                messages: VecDeque::new(),
            })),
            accepting: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl<T> PostFailureBuffer<T> {
    pub(crate) fn retain(&self, message: T) -> Result<(), T> {
        let mut state = self.state.lock().expect("post-failure buffer poisoned");
        if !state.accepting {
            return Err(message);
        }
        state.messages.push_back(message);
        Ok(())
    }

    pub(crate) fn is_accepting(&self) -> bool {
        self.accepting.load(AtomicOrdering::Acquire)
    }

    pub(crate) fn close_and_drain(&self) -> VecDeque<T> {
        self.accepting.store(false, AtomicOrdering::Release);
        let mut state = self.state.lock().expect("post-failure buffer poisoned");
        state.accepting = false;
        std::mem::take(&mut state.messages)
    }
}

#[derive(Debug)]
pub(crate) enum HostOutboundMessage {
    Message(ControlMessage),
    Raw(Vec<u8>),
}

pub(crate) struct HostOutboundReceiver {
    sender: mpsc::UnboundedSender<HostOutboundMessage>,
    messages: mpsc::UnboundedReceiver<HostOutboundMessage>,
    close: watch::Receiver<Option<crate::ConnectionReply>>,
}

impl HostOutboundReceiver {
    #[cfg(test)]
    pub(crate) async fn recv(&mut self) -> Option<HostOutboundMessage> {
        self.messages.recv().await
    }

    #[cfg(test)]
    pub(crate) fn try_recv(&mut self) -> Result<HostOutboundMessage, mpsc::error::TryRecvError> {
        self.messages.try_recv()
    }

    pub(crate) fn into_parts(
        self,
    ) -> (
        mpsc::UnboundedSender<HostOutboundMessage>,
        mpsc::UnboundedReceiver<HostOutboundMessage>,
        watch::Receiver<Option<crate::ConnectionReply>>,
    ) {
        (self.sender, self.messages, self.close)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct HostOutboundSender {
    sender: mpsc::UnboundedSender<HostOutboundMessage>,
    close: watch::Sender<Option<crate::ConnectionReply>>,
    retire: watch::Sender<bool>,
    post_failure: PostFailureBuffer<HostOutboundMessage>,
    udp: Option<crate::udp_session::ReliableUdpRouteSender>,
}

fn publish_udp_route_close(
    close: &watch::Sender<Option<crate::ConnectionReply>>,
    reply: crate::ConnectionReply,
    queue_physical_close: impl FnOnce(crate::ConnectionReply),
) {
    close.send_replace(Some(reply.clone()));
    queue_physical_close(reply);
}

impl HostOutboundSender {
    pub(crate) fn channel() -> (Self, HostOutboundReceiver) {
        // C++ TCP appends pending packets to OBuf and C4Network2Client::SendMsg
        // does not impose an app-message count limit. The reliable-UDP layer
        // separately owns its 10,000-packet ACK/retransmit window
        // (oracle-src-pinned src/C4Network2Client.cpp:121-124;
        // src/C4NetIO.cpp:1345-1357,1916,2788-2808). Keep this scheduler-facing
        // queue lossless and immediately enqueueing so a slow route cannot
        // block or corrupt fanout to the other routes.
        let (sender, receiver) = mpsc::unbounded_channel();
        let (close, close_rx) = watch::channel(None);
        let (retire, _) = watch::channel(false);
        let post_failure = PostFailureBuffer::default();
        (
            Self {
                sender: sender.clone(),
                close,
                retire,
                post_failure,
                udp: None,
            },
            HostOutboundReceiver {
                sender,
                messages: receiver,
                close: close_rx,
            },
        )
    }

    pub(crate) fn from_udp(outbound: crate::udp_session::ReliableUdpRouteSender) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        drop(receiver);
        let (close, _) = watch::channel(None);
        let (retire, _) = watch::channel(false);
        Self {
            sender,
            close,
            retire,
            post_failure: PostFailureBuffer::default(),
            udp: Some(outbound),
        }
    }

    #[cfg(test)]
    pub(crate) async fn send(
        &self,
        message: ControlMessage,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        self.try_send(message)
    }

    pub(crate) fn try_send_raw(
        &self,
        packet: Vec<u8>,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        self.send_or_retain(HostOutboundMessage::Raw(packet))
    }

    pub(crate) fn try_close(
        &self,
        reply: crate::ConnectionReply,
    ) -> Result<(), watch::error::SendError<Option<crate::ConnectionReply>>> {
        if let Some(udp) = &self.udp {
            publish_udp_route_close(&self.close, reply, |reply| udp.close_with_reply(reply));
            return Ok(());
        }
        self.close.send(Some(reply))
    }

    pub(crate) fn try_send(
        &self,
        message: ControlMessage,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        self.send_or_retain(HostOutboundMessage::Message(message))
    }

    pub(crate) fn set_voice_receive_cookie(&self, cookie: crate::voice::VoiceRouteCookie) {
        if let Some(udp) = &self.udp {
            udp.set_voice_receive_cookie(cookie);
        }
    }

    fn send_or_retain(
        &self,
        message: HostOutboundMessage,
    ) -> Result<(), mpsc::error::SendError<HostOutboundMessage>> {
        if let Some(udp) = &self.udp {
            let message = match message {
                HostOutboundMessage::Message(message) => match udp.try_send(message) {
                    Ok(()) => return Ok(()),
                    Err(message) => HostOutboundMessage::Message(message),
                },
                HostOutboundMessage::Raw(packet) => match udp.try_send_raw(packet) {
                    Ok(()) => return Ok(()),
                    Err(packet) => HostOutboundMessage::Raw(packet),
                },
            };
            return self
                .post_failure
                .retain(message)
                .map_err(mpsc::error::SendError);
        }
        match self.sender.send(message) {
            Ok(()) => Ok(()),
            Err(mpsc::error::SendError(message)) => self
                .post_failure
                .retain(message)
                .map_err(mpsc::error::SendError),
        }
    }

    pub(crate) fn same_channel(&self, other: &Self) -> bool {
        match (&self.udp, &other.udp) {
            (Some(left), Some(right)) => left.same_route(right),
            (None, None) => self.sender.same_channel(&other.sender),
            _ => false,
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.close.borrow().is_some() || self.is_retiring() || !self.post_failure.is_accepting()
    }

    pub(crate) fn is_retiring(&self) -> bool {
        *self.retire.borrow()
    }

    pub(crate) fn accepts_post_failure_fifo(&self) -> bool {
        self.close.borrow().is_none() && self.post_failure.is_accepting()
    }

    pub(crate) fn is_round_restart_route_live(&self) -> bool {
        self.accepts_post_failure_fifo()
            && !self.is_retiring()
            && self
                .udp
                .as_ref()
                .map_or_else(|| !self.sender.is_closed(), |udp| udp.is_accepting())
    }

    #[cfg(test)]
    pub(crate) fn writer_channel_is_closed(&self) -> bool {
        self.sender.is_closed()
    }

    pub(crate) fn retire(&self) {
        // Keep accepting logical sends until the owning Disconnected handler
        // removes this route and drains the post-failure suffix. C++ likewise
        // leaves the failed connection selected until Ev_Net_Disconn performs
        // its atomic route-removal/PostMortem handoff.
        if let Some(udp) = &self.udp {
            udp.retire();
        }
        self.retire.send_replace(true);
    }

    pub(crate) fn retire_and_take_post_failure(&self) -> VecDeque<HostOutboundMessage> {
        if let Some(udp) = &self.udp {
            udp.retire();
        }
        let messages = self.post_failure.close_and_drain();
        self.retire.send_replace(true);
        messages
    }

    pub(crate) fn subscribe_retire(&self) -> watch::Receiver<bool> {
        self.retire.subscribe()
    }

    pub(crate) fn subscribe_close(&self) -> watch::Receiver<Option<crate::ConnectionReply>> {
        self.close.subscribe()
    }

    pub(crate) async fn wait_udp_drained(&self) {
        if let Some(udp) = &self.udp {
            udp.wait_drained().await;
        }
    }

    pub(crate) fn try_send_many(routes: &[Self], message: ControlMessage) -> Option<Vec<bool>> {
        let udp = routes
            .iter()
            .map(|route| route.udp.clone())
            .collect::<Option<Vec<_>>>()?;
        let accepted =
            crate::udp_session::ReliableUdpRouteSender::try_send_many(&udp, message.clone())?;
        Some(
            routes
                .iter()
                .zip(accepted)
                .map(|(route, accepted)| {
                    accepted
                        || route
                            .post_failure
                            .retain(HostOutboundMessage::Message(message.clone()))
                            .is_ok()
                })
                .collect(),
        )
    }
}

pub(crate) async fn wait_for_route_retirement(retire_rx: &mut watch::Receiver<bool>) {
    loop {
        if *retire_rx.borrow_and_update() {
            return;
        }
        if retire_rx.changed().await.is_err() {
            // Sender teardown is not a retirement request. Leave this branch
            // inert so already-queued graceful traffic can still drain.
            std::future::pending::<()>().await;
        }
    }
}

#[derive(Debug)]
pub(crate) struct ClientConnection {
    pub(crate) outbound: HostOutboundSender,
    pub(crate) core: clonk_engine::ClientCoreControlData,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) join_data_sent: bool,
    pub(crate) join_data_needed_emitted: bool,
}

/// Route-side mirror of the C++ per-connection ping counters.
///
/// `measured_ms` is `C4Network2IOConnection::iPingTime` (`-1` until the first
/// pong, `SetPingTime` on each pong; src/C4Network2IO.cpp:1335-1341) and feeds
/// `getPingTime()` consumers such as the debug status text
/// (src/C4Network2.cpp:1212-1218) and activation requests. `outstanding_since`
/// mirrors `iLastPing`/`iLastPong`: `OnPing` keeps the FIRST unanswered ping
/// timestamp (src/C4Network2IO.cpp:1326-1333) and a pong clears it, so
/// [`Self::lag_ms`] can reproduce `getLag()`
/// (src/C4Network2IO.cpp:1283-1295) at snapshot time.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RoutePingLag {
    measured_ms: Option<i32>,
    outstanding_since: Option<Instant>,
}

impl RoutePingLag {
    /// `OnPing` after a dispatched probe: only the first unanswered ping
    /// stamps the outstanding timestamp.
    pub(crate) fn record_ping_dispatched(&mut self, now: Instant) {
        self.outstanding_since.get_or_insert(now);
    }

    /// `SetPingTime`: a pong stores the travel time and answers the
    /// outstanding ping.
    pub(crate) fn record_pong(&mut self, round_trip_ms: i32) {
        self.measured_ms = Some(round_trip_ms);
        self.outstanding_since = None;
    }

    /// `getPingTime()`.
    pub(crate) fn ping_ms(self) -> i32 {
        self.measured_ms.unwrap_or(-1)
    }

    /// `getLag()`: while a ping is unanswered and an RTT was ever measured,
    /// the elapsed wait replaces the measurement once it grows past it.
    pub(crate) fn lag_ms(self, now: Instant) -> i32 {
        match (self.measured_ms, self.outstanding_since) {
            (Some(measured), Some(since)) => {
                let unanswered_ms = i32::try_from(now.saturating_duration_since(since).as_millis())
                    .unwrap_or(i32::MAX);
                unanswered_ms.max(measured)
            }
            _ => self.ping_ms(),
        }
    }

    pub(crate) fn apply(&mut self, update: RoutePingUpdate, now: Instant) {
        match update {
            RoutePingUpdate::Dispatched => self.record_ping_dispatched(now),
            RoutePingUpdate::Measured(round_trip_ms) => self.record_pong(round_trip_ms),
        }
    }
}

/// One transport-task ping transition, forwarded to the owning route registry.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RoutePingUpdate {
    /// A ping probe went out (`OnPing`; src/C4Network2IO.cpp:1326-1333).
    Dispatched,
    /// A pong measured this round trip (`SetPingTime`;
    /// src/C4Network2IO.cpp:1335-1341).
    Measured(i32),
}

/// One accepted transport route, separate from its logical network client.
/// C++ keeps every route in `C4Network2IO::pConnList` and assigns message/data
/// ownership on `C4Network2Client` (`src/C4Network2IO.h:69-74,228-264`;
/// `src/C4Network2Client.h:82-84,127-133`).
#[derive(Debug)]
pub(crate) struct AcceptedConnectionRoute {
    pub(crate) client_id: ClientId,
    pub(crate) remote_connection_id: u32,
    pub(crate) peer_addr: SocketAddr,
    pub(crate) protocol: crate::NetworkProtocol,
    pub(crate) ping: RoutePingLag,
    pub(crate) outbound: HostOutboundSender,
    pub(crate) voice_auth: crate::voice::VoiceRouteAuthentication,
    pub(crate) peer_is_port: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionTrafficClass {
    Message,
    Data,
}

/// C4GameControlNetwork::CalcPerformance's topology-aware control send time.
///
/// Every input is an activated, known, nonlocal network client. `Some(ping)`
/// is its preferred message connection and `None` is a host tunnel. C++ skips
/// local clients and control clients which have no corresponding
/// C4Network2Client before reaching this calculation
/// (oracle-src-pinned src/C4GameControlNetwork.cpp:382-435).
pub(crate) fn control_send_time_ms(
    control_mode: i32,
    remote_clients: impl IntoIterator<Item = (ClientId, Option<i32>)>,
) -> i32 {
    let mut clients_ping = 0_i32;
    let mut ping_client_count = 0_i32;
    let mut tunnel_count = 0_i32;
    let mut host_ping = 0_i32;

    for (client_id, ping_ms) in remote_clients {
        let Some(ping_ms) = ping_ms else {
            tunnel_count = tunnel_count.wrapping_add(1);
            continue;
        };
        if client_id == HOST_CLIENT_ID {
            host_ping = ping_ms;
        } else {
            clients_ping = clients_ping.wrapping_add(ping_ms);
            ping_client_count = ping_client_count.wrapping_add(1);
        }
    }

    if control_mode != 0 {
        return host_ping;
    }

    let numerator = clients_ping.wrapping_add(host_ping.wrapping_mul(tunnel_count.wrapping_add(1)));
    let denominator = ping_client_count.wrapping_add(tunnel_count).wrapping_add(1);
    let mut control_send_time = numerator / denominator;
    if tunnel_count == 0 {
        control_send_time /= 2;
    }
    control_send_time
}

#[derive(Debug, Default)]
struct ControlSendTimeTopology {
    control_mode: i32,
    local_client_id: ClientId,
    known_clients: BTreeSet<ClientId>,
    preferred_message_ping_ms: BTreeMap<ClientId, i32>,
}

/// Last complete route-registry view published by the owning session loop.
///
/// C++ reads this topology synchronously while holding the network locks.
/// Rust publishes the same compact view after each loop operation so the game
/// thread can sample it without waiting behind socket-event or UI-event
/// backpressure.
#[derive(Clone, Debug, Default)]
pub struct ControlSendTimeSnapshot {
    topology: Arc<std::sync::RwLock<ControlSendTimeTopology>>,
}

impl ControlSendTimeSnapshot {
    /// Builds a snapshot from one complete preferred-message-route topology.
    ///
    /// `known_clients` includes the local client. A remote known client absent
    /// from `preferred_message_ping_ms` is sampled as a host tunnel, matching
    /// `C4GameControlNetwork::CalcPerformance`.
    pub fn from_preferred_message_routes(
        control_mode: i32,
        local_client_id: ClientId,
        known_clients: impl IntoIterator<Item = ClientId>,
        preferred_message_ping_ms: impl IntoIterator<Item = (ClientId, i32)>,
    ) -> Self {
        let snapshot = Self::default();
        snapshot.publish(
            control_mode,
            local_client_id,
            known_clients.into_iter().collect(),
            preferred_message_ping_ms.into_iter().collect(),
        );
        snapshot
    }

    pub(crate) fn publish(
        &self,
        control_mode: i32,
        local_client_id: ClientId,
        known_clients: BTreeSet<ClientId>,
        preferred_message_ping_ms: BTreeMap<ClientId, i32>,
    ) {
        let mut topology = self
            .topology
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *topology = ControlSendTimeTopology {
            control_mode,
            local_client_id,
            known_clients,
            preferred_message_ping_ms,
        };
    }

    /// Samples the C++ control-send time for the activated client registry.
    ///
    /// This read is synchronous and never waits for a session command to make
    /// a round trip through the network actor.
    pub fn sample(&self, activated_client_ids: &[ClientId]) -> i32 {
        let topology = self
            .topology
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        control_send_time_ms(
            topology.control_mode,
            activated_client_ids
                .iter()
                .copied()
                .filter(|client_id| *client_id != topology.local_client_id)
                .filter(|client_id| topology.known_clients.contains(client_id))
                .map(|client_id| {
                    (
                        client_id,
                        topology.preferred_message_ping_ms.get(&client_id).copied(),
                    )
                }),
        )
    }
}

#[derive(Debug)]
pub(crate) struct ClientResourceState {
    pub(crate) catalog: crate::ResourceCatalog,
    pub(crate) backend: Option<crate::ResourceTransferBackend>,
    pub(crate) local_resource_sources: BTreeMap<PathBuf, clonk_engine::NetworkResourceCore>,
    pub(crate) host_peer_id: i32,
    pub(crate) initial_complete_resources: Vec<(clonk_engine::NetworkResourceCore, PathBuf, bool)>,
    pub(crate) initial_packets: Vec<ResourcePacket>,
    pub(crate) initial_controls: Vec<ControlPacket>,
    pub(crate) initial_ready_checks: Vec<ReadyCheckPacket>,
    pub(crate) initial_lobby_countdowns: Vec<LobbyCountdownPacket>,
    #[cfg(test)]
    pub(crate) liveness: ConnectionLivenessState,
    pub(crate) resource_epoch: Instant,
    resource_directory: Option<PathBuf>,
    resource_resolver: crate::client_bootstrap::ClientBootstrapResolver,
    pub(crate) control: ClientControlState,
    pub(crate) next_control_request_at: tokio::time::Instant,
}

#[derive(Debug)]
pub(crate) struct ClientControlState {
    pub(crate) mode: i32,
    pub(crate) coordinator: ControlCoordinator,
    pending_unregistered: BTreeMap<ClientId, BTreeMap<Tick, ControlPacket>>,
    target_tick: Option<Tick>,
    runtime_recovery_horizon: Option<Tick>,
    central_expected_tick: Tick,
    // The bool records whether central mode already surfaced this future
    // complete packet before the contiguous ready cursor reached it.
    pending_complete: BTreeMap<Tick, (ControlPacket, bool)>,
}

impl ClientControlState {
    #[cfg(test)]
    pub(crate) fn central(start_tick: Tick) -> Self {
        Self {
            mode: 1,
            coordinator: ControlCoordinator::with_start_tick(CLIENT_BACKLOG_LIMIT, start_tick),
            pending_unregistered: BTreeMap::new(),
            target_tick: None,
            runtime_recovery_horizon: None,
            central_expected_tick: start_tick,
            pending_complete: BTreeMap::new(),
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
            target_tick: if matches!(
                join_data.status.state,
                NETWORK_STATE_GO | NETWORK_STATE_PAUSE
            ) {
                Tick::try_from(join_data.status.target_tick).ok()
            } else {
                None
            },
            runtime_recovery_horizon: None,
            central_expected_tick: start_tick,
            pending_complete: BTreeMap::new(),
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

    pub(crate) fn change_mode(
        &mut self,
        mode: i32,
        current_tick: Tick,
    ) -> Result<(bool, Vec<ControlPacket>), String> {
        if self.mode == mode {
            return Ok((false, Vec::new()));
        }
        let old_expected_tick = self.expected_tick();
        self.mode = mode;
        let next_tick = current_tick.max(old_expected_tick);
        let ready = if mode == 0 {
            let ready = self.coordinator.advance_to(next_tick);
            self.resolve_ready_with_completes(ready)?
        } else {
            // Central and asynchronous clients may only advance on complete
            // broadcast controls. Buffered per-client contributions must not
            // be packed locally while changing modes.
            self.coordinator.skip_to(next_tick);
            self.central_expected_tick = next_tick;
            self.drain_central_completes()
        };
        for pending in self.pending_unregistered.values_mut() {
            pending.retain(|tick, _| *tick >= current_tick);
        }
        Ok((true, ready))
    }

    pub(crate) fn set_status_target(&mut self, status: NetworkStatus) {
        self.target_tick = if matches!(status.state, NETWORK_STATE_GO | NETWORK_STATE_PAUSE) {
            Tick::try_from(status.target_tick).ok()
        } else {
            None
        };
    }

    pub(crate) fn clear_target(&mut self) {
        self.target_tick = None;
    }

    pub(crate) fn note_runtime_control_tick_reached(&mut self, tick: Tick) {
        self.runtime_recovery_horizon = Some(
            self.runtime_recovery_horizon
                .map_or(tick, |horizon| horizon.max(tick)),
        );
    }

    pub(crate) fn expected_tick(&self) -> Tick {
        if self.mode == 0 {
            self.coordinator.current_tick()
        } else {
            self.central_expected_tick
        }
    }

    pub(crate) fn recovery_tick(&self) -> Option<Tick> {
        let target_tick = self
            .target_tick
            .into_iter()
            .chain(self.runtime_recovery_horizon)
            .max()?;
        let expected_tick = self.expected_tick();
        let request_tick = if self.mode == 0 {
            self.coordinator
                .missing_ranges()
                .into_iter()
                .map(|range| range.from())
                .min()
                .unwrap_or(expected_tick)
        } else {
            expected_tick
        };
        (request_tick <= target_tick).then_some(request_tick)
    }

    pub(crate) fn is_registered(&self, client_id: ClientId) -> bool {
        self.coordinator.client_ids().any(|id| id == client_id)
    }

    pub(crate) fn register(&mut self, client_id: ClientId) -> Result<Vec<ControlPacket>, String> {
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
        self.resolve_ready_with_completes(ready)
    }

    fn unregister(&mut self, client_id: ClientId) -> Result<Vec<ControlPacket>, String> {
        if !self.coordinator.client_ids().any(|id| id == client_id) {
            return Ok(Vec::new());
        }
        let ready = self
            .coordinator
            .remove_client(client_id)
            .map_err(|error| error.to_string())?;
        self.resolve_ready_with_completes(ready)
    }

    pub(crate) fn apply_membership(
        &mut self,
        control: &clonk_engine::ControlPacket,
    ) -> Result<Vec<ControlPacket>, String> {
        match control {
            clonk_engine::ControlPacket::ClientJoin(join)
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
            clonk_engine::ControlPacket::ClientUpdate(update)
                if update.by_client == HOST_CLIENT_ID as i32 =>
            {
                let Ok(client_id) = ClientId::try_from(update.client_id) else {
                    return Ok(Vec::new());
                };
                match update.update_type {
                    clonk_engine::CLIENT_UPDATE_ACTIVATE if update.data != 0 => {
                        self.register(client_id)
                    }
                    clonk_engine::CLIENT_UPDATE_ACTIVATE
                    | clonk_engine::CLIENT_UPDATE_SET_OBSERVER => self.unregister(client_id),
                    _ => Ok(Vec::new()),
                }
            }
            clonk_engine::ControlPacket::ClientRemove(remove)
                if remove.by_client == HOST_CLIENT_ID as i32 =>
            {
                ClientId::try_from(remove.client_id)
                    .map_or_else(|_| Ok(Vec::new()), |client_id| self.unregister(client_id))
            }
            _ => Ok(Vec::new()),
        }
    }

    pub(crate) fn ingest_contribution(
        &mut self,
        packet: ControlPacket,
    ) -> Result<Vec<ControlPacket>, String> {
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
        self.resolve_ready_with_completes(outcome.ready)
    }

    pub(crate) fn accept_network(
        &mut self,
        packet: ControlPacket,
    ) -> Result<Vec<ControlPacket>, String> {
        validate_control_envelope(&packet).map_err(|error| error.to_string())?;
        if self.mode == 0 {
            if packet.client_id() != BROADCAST_CLIENT_ID {
                return self.ingest_contribution(packet);
            }
            if packet.tick() < self.coordinator.current_tick() {
                return Ok(Vec::new());
            }
            self.pending_complete
                .entry(packet.tick())
                .or_insert((packet, false));
            return self.resolve_ready_with_completes(Vec::new());
        }
        if packet.client_id() != BROADCAST_CLIENT_ID {
            return Ok(Vec::new());
        }
        if packet.tick() < self.central_expected_tick {
            return Ok(Vec::new());
        }
        self.pending_complete
            .entry(packet.tick())
            .or_insert_with(|| (packet.clone(), true));
        while self
            .pending_complete
            .remove(&self.central_expected_tick)
            .is_some()
        {
            self.central_expected_tick = self.central_expected_tick.saturating_add(1);
        }
        Ok(vec![packet])
    }

    fn resolve_ready_with_completes(
        &mut self,
        ready: Vec<ReadyBatch>,
    ) -> Result<Vec<ControlPacket>, String> {
        // A stored C4ClientIDAll packet wins over locally available partials.
        // Future complete replies remain pending until every earlier tick has
        // advanced, matching CheckCompleteCtrl's complete-first walk.
        let mut batches = VecDeque::from(ready);
        let mut complete = Vec::new();
        loop {
            while let Some(batch) = batches.pop_front() {
                if let Some((packet, published)) = self.pending_complete.remove(&batch.tick()) {
                    if !published {
                        complete.push(packet);
                    }
                } else {
                    complete
                        .push(aggregate_ready_batch(&batch).map_err(|error| error.to_string())?);
                }
            }

            let tick = self.coordinator.current_tick();
            let Some((packet, published)) = self.pending_complete.remove(&tick) else {
                break;
            };
            if !published {
                complete.push(packet);
            }
            let next_tick = tick.saturating_add(1);
            if next_tick == tick {
                break;
            }
            batches.extend(self.coordinator.advance_to(next_tick));
        }
        let current_tick = self.coordinator.current_tick();
        self.pending_complete
            .retain(|tick, _| *tick >= current_tick);
        Ok(complete)
    }

    fn drain_central_completes(&mut self) -> Vec<ControlPacket> {
        let mut complete = Vec::new();
        loop {
            let tick = self.central_expected_tick;
            let Some((packet, published)) = self.pending_complete.remove(&tick) else {
                break;
            };
            if !published {
                complete.push(packet);
            }
            let next_tick = tick.saturating_add(1);
            if next_tick == tick {
                break;
            }
            self.central_expected_tick = next_tick;
        }
        complete
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClientBootstrapRegistration {
    AlreadyPresent,
    Registered,
    UnavailableNonLoadable,
}

#[derive(Debug, Default)]
pub(crate) struct LoadedAuthoritativePlayerResources {
    pub(crate) local_sources: Vec<(PathBuf, clonk_engine::NetworkResourceCore)>,
    pub(crate) newly_loading_resource_ids: Vec<i32>,
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
        crate::ClientBootstrapResourceSource::TrustedLocalSystem(path) => {
            if let Some(backend) = backend {
                backend
                    .register_local_logical(resource.core.clone(), path)
                    .map_err(|error| error.to_string())?;
            }
            (false, false)
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
        local.standalone_path().map(std::path::Path::to_path_buf)
    } else {
        Some(local.source_path().to_path_buf())
    }
}

fn resource_is_registered(
    catalog: &crate::ResourceCatalog,
    backend: Option<&crate::ResourceTransferBackend>,
    resource_id: i32,
) -> bool {
    catalog.contains_resource(resource_id)
        || backend.is_some_and(|backend| backend.catalog().contains_resource(resource_id))
}

pub(crate) fn round_resource_cores(
    dynamic: &clonk_engine::NetworkResourceCore,
    parameters: &crate::JoinGameParametersEnvelope,
) -> BTreeMap<i32, clonk_engine::NetworkResourceCore> {
    let mut resources = BTreeMap::new();
    for core in &parameters.game_resources {
        resources.insert(core.id, core.clone());
    }
    resources.insert(dynamic.id, dynamic.clone());
    for player in parameters
        .player_infos
        .clients
        .iter()
        .flat_map(|client| &client.players)
    {
        let flags = player.flags;
        if flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0
            || flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
            || flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0
        {
            continue;
        }
        if let Some(core) = &player.resource {
            resources.insert(core.id, core.clone());
        }
    }
    resources.insert(parameters.scenario.id, parameters.scenario.clone());
    resources
}

pub(crate) fn load_authoritative_player_resources(
    resolver: &crate::client_bootstrap::ClientBootstrapResolver,
    catalog: &mut crate::ResourceCatalog,
    mut backend: Option<&mut crate::ResourceTransferBackend>,
    info: &mut clonk_engine::PlayerInfoControlData,
) -> LoadedAuthoritativePlayerResources {
    let mut loaded = LoadedAuthoritativePlayerResources::default();
    for player in &mut info.players {
        let flags = player.flags;
        if flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0
            || flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
        {
            continue;
        }
        if flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        }
        let Some(core) = player.resource.as_ref() else {
            crate::client_bootstrap::clear_player_resource(player);
            continue;
        };
        let existing_core = backend
            .as_deref()
            .and_then(|backend| backend.core(core.id))
            .or_else(|| catalog.resource_core(core.id))
            .cloned();
        let already_registered = resource_is_registered(catalog, backend.as_deref(), core.id);
        // AddByCore returns an existing ID before comparing cores or probing
        // local files (src/C4Network2Res.cpp:1473-1477).
        let replace_stale = existing_core
            .as_ref()
            .map_or(already_registered, |existing| {
                !already_registered || existing != core
            });
        if already_registered && !replace_stale {
            continue;
        }
        // The C++ list is keyed by ID, but a reused ID can leave a stale
        // round resource in the Rust filesystem backend after its catalog
        // entry has expired. Remove that stale registration before resolving
        // the authoritative player core, otherwise no completion event can
        // reach the exact-core admission gate.
        if replace_stale {
            catalog.forget_resource(core.id);
            if let Some(backend) = backend.as_deref_mut() {
                backend.forget_resource(core.id);
            }
        }
        let registered = resolver
            .resolve(crate::ClientBootstrapResourceRole::Player, core)
            .ok()
            .and_then(|resource| {
                let registration =
                    add_resolved_resource(catalog, backend.as_deref_mut(), &resource).ok()?;
                if registration == ClientBootstrapRegistration::Registered {
                    match &resource.source {
                        crate::ClientBootstrapResourceSource::Local(local) => {
                            if let Some(path) = local_resource_lookup_path(local) {
                                loaded.local_sources.push((path, resource.core.clone()));
                            }
                        }
                        crate::ClientBootstrapResourceSource::Download => {
                            loaded.newly_loading_resource_ids.push(resource.core.id);
                        }
                        crate::ClientBootstrapResourceSource::TrustedLocalSystem(_)
                        | crate::ClientBootstrapResourceSource::UnavailableNonLoadable(_) => {}
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
    loaded
}

impl ClientResourceState {
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
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
            next_control_request_at: tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL,
        }
    }

    pub(crate) fn new(
        join_data: &JoinDataEnvelope,
        host_peer_id: i32,
        initial_packets: Vec<ResourcePacket>,
        initial_controls: Vec<ControlPacket>,
        _liveness: ConnectionLivenessState,
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
            #[cfg(test)]
            liveness: _liveness,
            resource_epoch: Instant::now(),
            resource_directory,
            resource_resolver: crate::client_bootstrap::ClientBootstrapResolver::new(
                &local_candidates,
                standalone_directory,
            ),
            control,
            next_control_request_at: tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL,
        })
    }

    pub(crate) fn on_request_send_failed(
        &mut self,
        peer_id: i32,
        request: &crate::ResourceRequestPacket,
        unavailable_peers: &BTreeSet<i32>,
    ) -> Vec<crate::ResourceCatalogAction> {
        let now_seconds = self.resource_epoch.elapsed().as_secs();
        let mut random = resource_safe_random;
        if let Some(backend) = self.backend.as_mut() {
            backend.on_request_send_failed(
                peer_id,
                request,
                now_seconds,
                unavailable_peers,
                &mut random,
            )
        } else {
            self.catalog.on_request_send_failed(
                peer_id,
                request,
                now_seconds,
                unavailable_peers,
                &mut random,
            )
        }
    }

    pub(crate) fn retain_resource_resolver(
        &mut self,
        resolver: crate::client_bootstrap::ClientBootstrapResolver,
    ) {
        self.resource_resolver = resolver;
    }

    pub(crate) fn publish_player_resource_with_path(
        &mut self,
        request: crate::ClientPlayerResourceRequest,
    ) -> Result<crate::PublishedPlayerResource, String> {
        if let Some(core) = self.local_resource_sources.get(&request.source_path) {
            return Ok(crate::PublishedPlayerResource {
                core: core.clone(),
                local_path: request.source_path,
            });
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
        let publication =
            crate::publish_client_player_resource(crate::ClientPlayerResourcePublicationSpec {
                resource_id,
                source_path: request.source_path,
                wire_name: request.wire_name,
                network_directory,
                group_maker: request.group_maker,
            })
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
            .insert(effective_source_path.clone(), core.clone());
        Ok(crate::PublishedPlayerResource {
            core,
            local_path: effective_source_path,
        })
    }

    pub(crate) fn begin_resource_derive(
        &mut self,
        resource_id: i32,
        source_path: PathBuf,
        ownership: crate::ResourceFileOwnership,
    ) -> Result<crate::ResourceDerivation, String> {
        let now_seconds = self.resource_epoch.elapsed().as_secs();
        let backend = self
            .backend
            .as_mut()
            .ok_or_else(|| "client has no filesystem resource backend".to_string())?;
        let derivation = backend
            .begin_derive(resource_id, source_path, ownership, now_seconds)
            .map_err(|error| error.to_string())?;
        self.catalog
            .register_anonymous_derived_at(resource_id, true, now_seconds);
        Ok(derivation)
    }

    pub(crate) fn remove_resource(&mut self, resource_id: i32) -> Result<(), String> {
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
        resource_is_registered(&self.catalog, self.backend.as_ref(), resource_id)
    }

    fn bootstrap_resource_matches(&self, core: &clonk_engine::NetworkResourceCore) -> bool {
        self.backend
            .as_ref()
            .and_then(|backend| backend.core(core.id))
            .or_else(|| self.catalog.resource_core(core.id))
            == Some(core)
    }

    fn forget_bootstrap_resource(&mut self, resource_id: i32) {
        self.catalog.forget_resource(resource_id);
        if let Some(backend) = self.backend.as_mut() {
            backend.forget_resource(resource_id);
        }
        self.local_resource_sources
            .retain(|_, core| core.id != resource_id);
        self.initial_complete_resources
            .retain(|(core, _, _)| core.id != resource_id);
    }

    fn resolve_restarted_resource(
        &mut self,
        resolver: &crate::client_bootstrap::ClientBootstrapResolver,
        role: crate::ClientBootstrapResourceRole,
        core: &clonk_engine::NetworkResourceCore,
    ) -> Result<ClientBootstrapRegistration, String> {
        if self.contains_bootstrap_resource(core.id) {
            if self.bootstrap_resource_matches(core) {
                return Ok(ClientBootstrapRegistration::AlreadyPresent);
            }
            self.forget_bootstrap_resource(core.id);
        }
        self.resolve_and_add_bootstrap_resource(resolver, role, core)
    }

    pub(crate) fn apply_restart_join_data(
        &mut self,
        mut join_data: JoinDataEnvelope,
    ) -> Result<JoinDataEnvelope, String> {
        if join_data.client_id != self.catalog.local_client_id() {
            return Err(format!(
                "restarted JoinData reassigned client {} to {}",
                self.catalog.local_client_id(),
                join_data.client_id
            ));
        }
        let resolver = self.resource_resolver.clone();
        for core in &join_data.parameters.game_resources {
            self.resolve_restarted_resource(
                &resolver,
                crate::ClientBootstrapResourceRole::GameResource,
                core,
            )?;
        }
        self.resolve_restarted_resource(
            &resolver,
            crate::ClientBootstrapResourceRole::Dynamic,
            &join_data.dynamic,
        )?;
        for player in join_data
            .parameters
            .player_infos
            .clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
        {
            let flags = player.flags;
            if flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED != 0
                || flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE == 0
            {
                continue;
            }
            if flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE != 0 {
                crate::client_bootstrap::clear_player_resource(player);
                continue;
            }
            let Some(core) = player.resource.clone() else {
                crate::client_bootstrap::clear_player_resource(player);
                continue;
            };
            match self.resolve_restarted_resource(
                &resolver,
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
        self.resolve_restarted_resource(
            &resolver,
            crate::ClientBootstrapResourceRole::Scenario,
            &join_data.parameters.scenario,
        )?;
        let retained_resources = round_resource_cores(&join_data.dynamic, &join_data.parameters);
        let retained_resource_ids = retained_resources.keys().copied().collect();
        self.catalog.retain_resource_ids(&retained_resource_ids);
        if let Some(backend) = self.backend.as_mut() {
            backend
                .retain_resources(&retained_resources)
                .map_err(|error| error.to_string())?;
            backend.set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE);
        }
        let retained_complete = self
            .backend
            .as_ref()
            .into_iter()
            .flat_map(|backend| {
                retained_resources.values().filter_map(|core| {
                    (backend.core(core.id) == Some(core) && backend.is_complete(core.id))
                        .then_some(core)
                        .and_then(|core| {
                            backend.path(core.id).map(|path| {
                                (core.clone(), path.to_path_buf(), backend.is_local(core.id))
                            })
                        })
                })
            })
            .collect::<Vec<_>>();
        let already_reported = self
            .initial_complete_resources
            .iter()
            .map(|(core, _, _)| core.id)
            .collect::<BTreeSet<_>>();
        self.initial_complete_resources.extend(
            retained_complete
                .into_iter()
                .filter(|(core, _, _)| !already_reported.contains(&core.id)),
        );
        self.catalog
            .set_max_loads_per_peer(crate::RESOURCE_MAX_LOAD_PER_PEER_PER_FILE);
        self.local_resource_sources
            .retain(|_, core| retained_resource_ids.contains(&core.id));
        self.control = ClientControlState::from_join_data(&join_data)?;
        self.next_control_request_at = tokio::time::Instant::now() + CONTROL_REQUEST_INTERVAL;
        Ok(join_data)
    }

    pub(crate) fn add_bootstrap_resource(
        &mut self,
        resource: &crate::ClientBootstrapResourcePlan,
    ) -> Result<ClientBootstrapRegistration, String> {
        let registration =
            add_resolved_resource(&mut self.catalog, self.backend.as_mut(), resource)?;
        if registration == ClientBootstrapRegistration::Registered {
            match &resource.source {
                crate::ClientBootstrapResourceSource::Local(local) => {
                    self.initial_complete_resources.push((
                        resource.core.clone(),
                        local.path().to_path_buf(),
                        true,
                    ));
                    if let Some(path) = local_resource_lookup_path(local) {
                        self.local_resource_sources
                            .insert(path, resource.core.clone());
                    }
                }
                crate::ClientBootstrapResourceSource::TrustedLocalSystem(path) => {
                    self.initial_complete_resources.push((
                        resource.core.clone(),
                        path.clone(),
                        true,
                    ));
                }
                crate::ClientBootstrapResourceSource::Download
                | crate::ClientBootstrapResourceSource::UnavailableNonLoadable(_) => {}
            }
        }
        Ok(registration)
    }

    pub(crate) fn resolve_and_add_bootstrap_resource(
        &mut self,
        resolver: &crate::client_bootstrap::ClientBootstrapResolver,
        role: crate::ClientBootstrapResourceRole,
        core: &clonk_engine::NetworkResourceCore,
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

    pub(crate) fn load_authoritative_player_resources(
        &mut self,
        info: &mut clonk_engine::PlayerInfoControlData,
    ) -> Vec<(PathBuf, clonk_engine::NetworkResourceCore)> {
        let loaded = load_authoritative_player_resources(
            &self.resource_resolver,
            &mut self.catalog,
            self.backend.as_mut(),
            info,
        );
        self.local_resource_sources
            .extend(loaded.local_sources.iter().cloned());
        loaded.local_sources
    }

    #[cfg(test)]
    pub(crate) fn from_join_data(
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

#[cfg(test)]
mod udp_sender_tests {
    use super::*;

    #[test]
    fn udp_close_is_published_before_physical_close_is_queued() {
        // CloseConns publishes the intentional removal before its best-effort
        // ConnRe send closes the route (oracle-src-pinned
        // src/C4Network2Client.cpp:104-119,457-492).
        let (close, close_rx) = watch::channel(None);
        let reply = crate::ConnectionReply {
            ok: false,
            message: clonk_engine::LegacyCString::from_bytes(b"closed".to_vec()).unwrap(),
            wrong_password: false,
            port_protocol: false,
        };
        let queued = std::cell::Cell::new(false);

        publish_udp_route_close(&close, reply.clone(), |queued_reply| {
            assert_eq!(close_rx.borrow().as_ref(), Some(&reply));
            assert_eq!(queued_reply, reply);
            queued.set(true);
        });

        assert!(queued.get());
    }

    #[test]
    fn udp_close_before_route_task_subscription_retains_the_reply() {
        // CloseConns publishes the best-effort ConnRe before route teardown;
        // task scheduling cannot lose that state transition
        // (oracle-src-pinned src/C4Network2Client.cpp:104-118;
        // src/C4NetIO.cpp:1458-1468).
        let outbound =
            HostOutboundSender::from_udp(crate::udp_session::ReliableUdpRouteSender::test_sender());
        let reply = crate::ConnectionReply {
            ok: false,
            message: clonk_engine::LegacyCString::from_bytes(b"closed".to_vec()).unwrap(),
            wrong_password: false,
            port_protocol: false,
        };

        assert!(outbound.try_close(reply.clone()).is_ok());
        let close = outbound.subscribe_close();
        assert_eq!(close.borrow().as_ref(), Some(&reply));
    }
}
