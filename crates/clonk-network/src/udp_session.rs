//! Session-facing streams over the packet-oriented reliable-UDP driver.
//!
//! `ControlTransport` speaks C4NetIOTCP's internal `0xff + native u32`
//! framing. C4NetIOUDP instead carries the complete packet body directly in
//! one reliable packet. This module owns the shared UDP socket and presents
//! each connected peer as an `AsyncRead + AsyncWrite` stream which adds that
//! framing on receive and removes it on send. The framing is therefore only
//! an in-process adapter; it is never emitted on the UDP wire.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt,
    future::Future,
    io,
    net::SocketAddr,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::Duration,
};

use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{mpsc, oneshot, Notify},
    task::JoinHandle,
};

use crate::udp_runtime::ReliableUdpPollReady;
use crate::{
    canonical_reliable_udp_peer_address, NetpuncherAddressFamily, NetpuncherIoEvent,
    NetpuncherPacket, NetpuncherRole, ReliableUdpDisconnectReason, ReliableUdpDriverError,
    ReliableUdpEvent, ReliableUdpSocketDriver,
};

const TCP_FRAME_PREFIX: u8 = 0xff;
const TCP_FRAME_HEADER_SIZE: usize = 5;
const HUB_COMMAND_CAPACITY: usize = 64;
const INCOMING_PEER_CAPACITY: usize = 32;
const PUNCHER_EVENT_CAPACITY: usize = 16;
/// At 50 frames/s, each hub direction queues at most 160 ms of encoded speech.
const VOICE_MEDIA_CAPACITY: usize = 8;
const PEER_INBOUND_PACKET_CAPACITY: usize = 64;
const ABANDONED_PEER_MAINTENANCE_INTERVAL: Duration = Duration::from_millis(100);

type HubCommandPermitFuture = Pin<
    Box<
        dyn Future<Output = Result<mpsc::OwnedPermit<HubCommand>, mpsc::error::SendError<()>>>
            + Send,
    >,
>;

enum HubCommand {
    InitPuncher {
        address: SocketAddr,
        role: NetpuncherRole,
        response: oneshot::Sender<io::Result<()>>,
    },
    SendPuncherPacket {
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
        response: oneshot::Sender<io::Result<()>>,
    },
    ClosePuncher {
        address: SocketAddr,
        response: oneshot::Sender<io::Result<()>>,
    },
    Connect {
        peer: SocketAddr,
        response: oneshot::Sender<io::Result<ReliableUdpPeerStream>>,
    },
    BindStatistics {
        peer: SocketAddr,
        generation: u64,
        connection_id: u32,
        response: oneshot::Sender<io::Result<()>>,
    },
    PromoteOutbound {
        peer: SocketAddr,
        generation: u64,
        packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
        response: oneshot::Sender<io::Result<ReliableUdpRouteSender>>,
    },
    Send {
        peer: SocketAddr,
        generation: u64,
        payload: Vec<u8>,
    },
    Close {
        peer: SocketAddr,
        generation: u64,
    },
    InboundCapacity {
        peer: SocketAddr,
        generation: u64,
    },
    Shutdown {
        completion: Option<oneshot::Sender<()>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReliableUdpVoiceDatagram {
    pub peer: SocketAddr,
    pub payload: Vec<u8>,
}

struct HubVoiceMedia {
    outgoing: mpsc::Receiver<ReliableUdpVoiceDatagram>,
    incoming: mpsc::Sender<ReliableUdpVoiceDatagram>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct UdpOutboxRouteId(u64);

struct UdpOutboxRouteState {
    accepting: bool,
    retirement_started: bool,
    peer: SocketAddr,
    generation: u64,
    packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
    drained: Arc<UdpRouteDrain>,
    voice_receive_cookie: Option<crate::voice::VoiceRouteCookie>,
}

#[derive(Clone)]
enum UdpPreparedPayload {
    Packet(Arc<[u8]>),
    Failed(Arc<str>),
}

enum UdpOutboxWork {
    Single {
        route: UdpOutboxRouteId,
        payload: UdpPreparedPayload,
    },
    Many {
        routes: VecDeque<UdpOutboxRouteId>,
        payload: UdpPreparedPayload,
    },
    Retire {
        route: UdpOutboxRouteId,
    },
    Close {
        route: UdpOutboxRouteId,
        payload: UdpPreparedPayload,
    },
}

enum UdpOutboxAction {
    Send {
        route: UdpOutboxRouteId,
        peer: SocketAddr,
        generation: u64,
        payload: Arc<[u8]>,
    },
    Failed {
        route: UdpOutboxRouteId,
        peer: SocketAddr,
        generation: u64,
        error: Arc<str>,
    },
    Retired {
        route: UdpOutboxRouteId,
        peer: SocketAddr,
        generation: u64,
    },
    Close {
        route: UdpOutboxRouteId,
        peer: SocketAddr,
        generation: u64,
        payload: Option<Arc<[u8]>>,
    },
}

#[derive(Default)]
struct UdpRouteDrain {
    complete: AtomicBool,
    notify: Notify,
}

impl UdpRouteDrain {
    fn finish(&self) {
        if !self.complete.swap(true, Ordering::AcqRel) {
            self.notify.notify_waiters();
        }
    }

    async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            if self.complete.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Default)]
struct UdpLogicalOutbox {
    next_route: u64,
    routes: BTreeMap<UdpOutboxRouteId, UdpOutboxRouteState>,
    queue: VecDeque<UdpOutboxWork>,
    #[cfg(test)]
    wake_count: usize,
}

impl UdpLogicalOutbox {
    #[cfg(test)]
    fn register_route(
        &mut self,
        packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
    ) -> UdpOutboxRouteId {
        self.register_live_route(
            SocketAddr::from(([0, 0, 0, 0], 0)),
            0,
            packet_log,
            Arc::new(UdpRouteDrain::default()),
        )
    }

    fn register_live_route(
        &mut self,
        peer: SocketAddr,
        generation: u64,
        packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
        drained: Arc<UdpRouteDrain>,
    ) -> UdpOutboxRouteId {
        let route = UdpOutboxRouteId(self.next_route);
        self.next_route = self.next_route.wrapping_add(1);
        let replaced = self.routes.insert(
            route,
            UdpOutboxRouteState {
                accepting: true,
                retirement_started: false,
                peer,
                generation,
                packet_log,
                drained,
                voice_receive_cookie: None,
            },
        );
        debug_assert!(replaced.is_none());
        route
    }

    #[cfg(test)]
    fn enqueue_single(
        &mut self,
        route: UdpOutboxRouteId,
        payload: Arc<[u8]>,
    ) -> Result<(), Arc<[u8]>> {
        self.enqueue_prepared(route, UdpPreparedPayload::Packet(payload))
            .map_err(|payload| match payload {
                UdpPreparedPayload::Packet(payload) => payload,
                UdpPreparedPayload::Failed(_) => {
                    unreachable!("packet enqueue returned a prepared failure")
                }
            })
    }

    fn enqueue_prepared(
        &mut self,
        route: UdpOutboxRouteId,
        payload: UdpPreparedPayload,
    ) -> Result<(), UdpPreparedPayload> {
        if !self.routes.get(&route).is_some_and(|state| state.accepting) {
            return Err(payload);
        }
        self.push(UdpOutboxWork::Single { route, payload });
        Ok(())
    }

    #[cfg(test)]
    fn enqueue_many(
        &mut self,
        routes: impl IntoIterator<Item = UdpOutboxRouteId>,
        payload: Arc<[u8]>,
    ) -> Vec<UdpOutboxRouteId> {
        self.enqueue_many_prepared(routes, UdpPreparedPayload::Packet(payload))
    }

    fn enqueue_many_prepared(
        &mut self,
        routes: impl IntoIterator<Item = UdpOutboxRouteId>,
        payload: UdpPreparedPayload,
    ) -> Vec<UdpOutboxRouteId> {
        let routes = routes
            .into_iter()
            .filter(|route| self.routes.get(route).is_some_and(|state| state.accepting))
            .collect::<VecDeque<_>>();
        let accepted = routes.iter().copied().collect::<Vec<_>>();
        if !routes.is_empty() {
            self.push(UdpOutboxWork::Many { routes, payload });
        }
        accepted
    }

    fn retire(&mut self, route: UdpOutboxRouteId) -> bool {
        let Some(state) = self.routes.get_mut(&route) else {
            return false;
        };
        if state.retirement_started || state.drained.complete.load(Ordering::Acquire) {
            return false;
        }
        state.accepting = false;
        state.retirement_started = true;
        self.push(UdpOutboxWork::Retire { route });
        true
    }

    fn fail_route(&mut self, route: UdpOutboxRouteId) -> bool {
        let Some(state) = self.routes.get_mut(&route) else {
            return false;
        };
        state.accepting = false;

        let mut retained = VecDeque::with_capacity(self.queue.len());
        while let Some(work) = self.queue.pop_front() {
            match work {
                UdpOutboxWork::Single {
                    route: work_route,
                    payload,
                } if work_route == route => self.record_prepared(route, &payload),
                UdpOutboxWork::Many {
                    mut routes,
                    payload,
                } => {
                    let removed = routes.iter().filter(|&&target| target == route).count();
                    routes.retain(|&target| target != route);
                    for _ in 0..removed {
                        self.record_prepared(route, &payload);
                    }
                    if !routes.is_empty() {
                        retained.push_back(UdpOutboxWork::Many { routes, payload });
                    }
                }
                UdpOutboxWork::Retire { route: work_route } if work_route == route => {}
                UdpOutboxWork::Close {
                    route: work_route, ..
                } if work_route == route => {}
                work => retained.push_back(work),
            }
        }
        self.queue = retained;
        if let Some(state) = self.routes.remove(&route) {
            state.drained.finish();
        }
        true
    }

    fn next_action(&mut self) -> Option<UdpOutboxAction> {
        loop {
            match self.queue.pop_front()? {
                UdpOutboxWork::Single { route, payload } => {
                    if let Some(action) = self.prepare_action(route, payload) {
                        return Some(action);
                    }
                }
                UdpOutboxWork::Many {
                    mut routes,
                    payload,
                } => {
                    let Some(route) = routes.pop_front() else {
                        continue;
                    };
                    if !routes.is_empty() {
                        self.queue.push_front(UdpOutboxWork::Many {
                            routes,
                            payload: payload.clone(),
                        });
                    }
                    if let Some(action) = self.prepare_action(route, payload) {
                        return Some(action);
                    }
                }
                UdpOutboxWork::Retire { route } => {
                    if let Some((peer, generation)) = self.route_identity(route) {
                        return Some(UdpOutboxAction::Retired {
                            route,
                            peer,
                            generation,
                        });
                    }
                }
                UdpOutboxWork::Close { route, payload } => {
                    if let Some((peer, generation)) = self.route_identity(route) {
                        let payload = match payload {
                            UdpPreparedPayload::Packet(payload) => {
                                self.record(route, &payload);
                                Some(payload)
                            }
                            UdpPreparedPayload::Failed(_) => None,
                        };
                        return Some(UdpOutboxAction::Close {
                            route,
                            peer,
                            generation,
                            payload,
                        });
                    }
                }
            }
        }
    }

    fn close_route(&mut self, route: UdpOutboxRouteId, payload: UdpPreparedPayload) -> bool {
        let Some(state) = self.routes.get_mut(&route) else {
            return false;
        };
        if state.retirement_started {
            return false;
        }
        state.accepting = false;
        state.retirement_started = true;

        let mut retained = VecDeque::with_capacity(self.queue.len());
        while let Some(work) = self.queue.pop_front() {
            match work {
                UdpOutboxWork::Single {
                    route: work_route, ..
                }
                | UdpOutboxWork::Retire { route: work_route }
                | UdpOutboxWork::Close {
                    route: work_route, ..
                } if work_route == route => {}
                UdpOutboxWork::Many {
                    mut routes,
                    payload,
                } => {
                    routes.retain(|&target| target != route);
                    if !routes.is_empty() {
                        retained.push_back(UdpOutboxWork::Many { routes, payload });
                    }
                }
                work => retained.push_back(work),
            }
        }
        retained.push_front(UdpOutboxWork::Close { route, payload });
        self.queue = retained;
        true
    }

    fn fail_all(&mut self) {
        let routes = self.routes.keys().copied().collect::<Vec<_>>();
        for route in routes {
            self.fail_route(route);
        }
        self.queue.clear();
    }

    fn prepare_action(
        &self,
        route: UdpOutboxRouteId,
        payload: UdpPreparedPayload,
    ) -> Option<UdpOutboxAction> {
        let (peer, generation) = self.route_identity(route)?;
        Some(match payload {
            UdpPreparedPayload::Packet(payload) => {
                self.record(route, &payload);
                UdpOutboxAction::Send {
                    route,
                    peer,
                    generation,
                    payload,
                }
            }
            UdpPreparedPayload::Failed(error) => UdpOutboxAction::Failed {
                route,
                peer,
                generation,
                error,
            },
        })
    }

    fn route_identity(&self, route: UdpOutboxRouteId) -> Option<(SocketAddr, u64)> {
        self.routes
            .get(&route)
            .map(|state| (state.peer, state.generation))
    }

    fn finish_retire(&mut self, route: UdpOutboxRouteId) {
        if let Some(state) = self.routes.remove(&route) {
            state.drained.finish();
        }
    }

    fn push(&mut self, work: UdpOutboxWork) {
        #[cfg(test)]
        if self.queue.is_empty() {
            self.wake_count += 1;
        }
        self.queue.push_back(work);
    }

    fn record(&self, route: UdpOutboxRouteId, payload: &Arc<[u8]>) {
        if let Some(state) = self.routes.get(&route) {
            state
                .packet_log
                .lock()
                .expect("UDP outbox packet log poisoned")
                .record_shared_outbound(payload.clone());
        }
    }

    fn record_prepared(&self, route: UdpOutboxRouteId, payload: &UdpPreparedPayload) {
        if let UdpPreparedPayload::Packet(payload) = payload {
            self.record(route, payload);
        }
    }

    #[cfg(test)]
    fn wake_count(&self) -> usize {
        self.wake_count
    }

    #[cfg(test)]
    fn route_count(&self) -> usize {
        self.routes.len()
    }
}

#[derive(Default)]
struct UdpSharedOutbox {
    state: Mutex<UdpLogicalOutbox>,
    ready: Notify,
}

impl UdpSharedOutbox {
    fn register_route(
        self: &Arc<Self>,
        peer: SocketAddr,
        generation: u64,
        packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
    ) -> Option<ReliableUdpRouteSender> {
        let drained = Arc::new(UdpRouteDrain::default());
        let mut state = self.state.lock().expect("UDP outbox poisoned");
        if state
            .routes
            .values()
            .any(|route| route.peer == peer && route.generation == generation)
        {
            return None;
        }
        let route = state.register_live_route(peer, generation, packet_log, drained.clone());
        drop(state);
        Some(ReliableUdpRouteSender {
            lease: Arc::new(UdpRouteLease {
                outbox: self.clone(),
                route,
                drained,
            }),
        })
    }

    fn enqueue(
        &self,
        route: UdpOutboxRouteId,
        payload: UdpPreparedPayload,
    ) -> Result<(), UdpPreparedPayload> {
        let mut state = self.state.lock().expect("UDP outbox poisoned");
        let was_empty = state.queue.is_empty();
        let result = state.enqueue_prepared(route, payload);
        drop(state);
        if was_empty && result.is_ok() {
            self.ready.notify_one();
        }
        result
    }

    fn set_voice_receive_cookie(
        &self,
        route: UdpOutboxRouteId,
        cookie: crate::voice::VoiceRouteCookie,
    ) {
        if let Some(state) = self
            .state
            .lock()
            .expect("UDP outbox poisoned")
            .routes
            .get_mut(&route)
            .filter(|state| state.accepting)
        {
            state.voice_receive_cookie = Some(cookie);
        }
    }

    fn voice_receive_cookie(
        &self,
        peer: SocketAddr,
        generation: u64,
    ) -> Option<crate::voice::VoiceRouteCookie> {
        self.state
            .lock()
            .expect("UDP outbox poisoned")
            .routes
            .values()
            .find(|route| route.accepting && route.peer == peer && route.generation == generation)
            .and_then(|route| route.voice_receive_cookie)
    }

    fn enqueue_many(
        &self,
        routes: impl IntoIterator<Item = UdpOutboxRouteId>,
        payload: UdpPreparedPayload,
    ) -> Vec<UdpOutboxRouteId> {
        let mut state = self.state.lock().expect("UDP outbox poisoned");
        let was_empty = state.queue.is_empty();
        let accepted = state.enqueue_many_prepared(routes, payload);
        drop(state);
        if was_empty && !accepted.is_empty() {
            self.ready.notify_one();
        }
        accepted
    }

    fn retire(&self, route: UdpOutboxRouteId) {
        let mut state = self.state.lock().expect("UDP outbox poisoned");
        let was_empty = state.queue.is_empty();
        let queued = state.retire(route);
        drop(state);
        if was_empty && queued {
            self.ready.notify_one();
        }
    }

    fn close(&self, route: UdpOutboxRouteId, payload: UdpPreparedPayload) {
        let mut state = self.state.lock().expect("UDP outbox poisoned");
        let was_empty = state.queue.is_empty();
        let queued = state.close_route(route, payload);
        drop(state);
        if was_empty && queued {
            self.ready.notify_one();
        }
    }

    async fn wait_ready(&self) {
        loop {
            let notified = self.ready.notified();
            if !self
                .state
                .lock()
                .expect("UDP outbox poisoned")
                .queue
                .is_empty()
            {
                return;
            }
            notified.await;
        }
    }

    fn take_action(&self) -> Option<UdpOutboxAction> {
        self.state
            .lock()
            .expect("UDP outbox poisoned")
            .next_action()
    }

    #[cfg(test)]
    async fn next_action(&self) -> UdpOutboxAction {
        loop {
            self.wait_ready().await;
            if let Some(action) = self.take_action() {
                return action;
            }
        }
    }

    fn fail_route(&self, route: UdpOutboxRouteId) {
        self.state
            .lock()
            .expect("UDP outbox poisoned")
            .fail_route(route);
    }

    fn finish_retire(&self, route: UdpOutboxRouteId) {
        self.state
            .lock()
            .expect("UDP outbox poisoned")
            .finish_retire(route);
    }

    fn is_accepting(&self, route: UdpOutboxRouteId) -> bool {
        self.state
            .lock()
            .expect("UDP outbox poisoned")
            .routes
            .get(&route)
            .is_some_and(|state| state.accepting)
    }

    fn fail_all(&self) {
        match self.state.lock() {
            Ok(mut state) => state.fail_all(),
            Err(poisoned) => poisoned.into_inner().fail_all(),
        }
    }

    #[cfg(test)]
    fn route_count(&self) -> usize {
        self.state.lock().expect("UDP outbox poisoned").routes.len()
    }
}

struct UdpOutboxRunGuard(Arc<UdpSharedOutbox>);

impl Drop for UdpOutboxRunGuard {
    fn drop(&mut self) {
        self.0.fail_all();
    }
}

struct UdpRouteLease {
    outbox: Arc<UdpSharedOutbox>,
    route: UdpOutboxRouteId,
    drained: Arc<UdpRouteDrain>,
}

impl Drop for UdpRouteLease {
    fn drop(&mut self) {
        self.outbox.retire(self.route);
    }
}

/// Cloneable established-route sender backed by the one logical UDP outbox
/// owned by its socket endpoint.
#[derive(Clone)]
pub(crate) struct ReliableUdpRouteSender {
    lease: Arc<UdpRouteLease>,
}

impl fmt::Debug for ReliableUdpRouteSender {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpRouteSender")
            .field("route", &self.lease.route)
            .finish_non_exhaustive()
    }
}

impl ReliableUdpRouteSender {
    #[cfg(test)]
    pub(crate) fn test_sender() -> Self {
        Arc::new(UdpSharedOutbox::default())
            .register_route(
                SocketAddr::from(([127, 0, 0, 1], 1)),
                0,
                Arc::new(Mutex::new(crate::RecoverablePacketLog::default())),
            )
            .expect("test route is unique")
    }

    #[cfg(test)]
    pub(crate) fn test_fail(&self) {
        self.lease.outbox.fail_route(self.lease.route);
    }

    pub(crate) fn try_send(
        &self,
        message: crate::ControlMessage,
    ) -> Result<(), crate::ControlMessage> {
        let prepared = prepare_udp_message(message.clone());
        self.lease
            .outbox
            .enqueue(self.lease.route, prepared)
            .map_err(|_| message)
    }

    pub(crate) fn set_voice_receive_cookie(&self, cookie: crate::voice::VoiceRouteCookie) {
        self.lease
            .outbox
            .set_voice_receive_cookie(self.lease.route, cookie);
    }

    pub(crate) fn try_send_raw(&self, packet: Vec<u8>) -> Result<(), Vec<u8>> {
        let prepared = match u32::try_from(packet.len()) {
            Ok(_) => UdpPreparedPayload::Packet(Arc::from(packet.clone())),
            Err(_) => UdpPreparedPayload::Failed(Arc::from(
                "send failed: malformed packet: packet exceeds C++ uint32 frame size",
            )),
        };
        self.lease
            .outbox
            .enqueue(self.lease.route, prepared)
            .map_err(|_| packet)
    }

    pub(crate) fn try_send_many(
        routes: &[Self],
        message: crate::ControlMessage,
    ) -> Option<Vec<bool>> {
        let first = routes.first()?;
        if routes
            .iter()
            .any(|route| !Arc::ptr_eq(&first.lease.outbox, &route.lease.outbox))
        {
            return None;
        }
        let accepted = first.lease.outbox.enqueue_many(
            routes.iter().map(|route| route.lease.route),
            prepare_udp_message(message),
        );
        let accepted = accepted.into_iter().collect::<BTreeSet<_>>();
        Some(
            routes
                .iter()
                .map(|route| accepted.contains(&route.lease.route))
                .collect(),
        )
    }

    pub(crate) fn retire(&self) {
        self.lease.outbox.retire(self.lease.route);
    }

    pub(crate) fn is_accepting(&self) -> bool {
        self.lease.outbox.is_accepting(self.lease.route)
    }

    pub(crate) async fn wait_drained(&self) {
        self.lease.drained.wait().await;
    }

    pub(crate) fn same_route(&self, other: &Self) -> bool {
        self.lease.route == other.lease.route
            && Arc::ptr_eq(&self.lease.outbox, &other.lease.outbox)
    }

    pub(crate) fn close_with_reply(&self, reply: crate::ConnectionReply) {
        self.lease.outbox.close(
            self.lease.route,
            prepare_udp_message(crate::ControlMessage::ConnectionReply(reply)),
        );
    }
}

fn prepare_udp_message(message: crate::ControlMessage) -> UdpPreparedPayload {
    match crate::transport::encode_complete_message(message) {
        Ok(packet) => UdpPreparedPayload::Packet(Arc::from(packet)),
        Err(error) => UdpPreparedPayload::Failed(Arc::from(format!("send failed: {error}"))),
    }
}

pub(crate) struct ReliableUdpOutboxRegistration {
    peer: SocketAddr,
    generation: u64,
    commands: mpsc::Sender<HubCommand>,
}

impl ReliableUdpOutboxRegistration {
    pub(crate) async fn promote(
        self,
        packet_log: Arc<Mutex<crate::RecoverablePacketLog>>,
    ) -> io::Result<ReliableUdpRouteSender> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::PromoteOutbound {
                peer: self.peer,
                generation: self.generation,
                packet_log,
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while promoting an outbound route",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped while promoting an outbound route",
            )
        })?
    }
}

enum PeerInbound {
    Packet(Vec<u8>),
}

#[derive(Clone)]
enum PeerTerminal {
    Disconnected(ReliableUdpDisconnectReason),
    Failed(String),
    Closed,
}

struct PeerTerminalState {
    closed: AtomicBool,
    reason: Mutex<Option<PeerTerminal>>,
}

impl PeerTerminalState {
    fn open() -> Self {
        Self {
            closed: AtomicBool::new(false),
            reason: Mutex::new(None),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn close(&self, reason: PeerTerminal) {
        let mut retained = self.reason.lock().expect("peer terminal state poisoned");
        if retained.is_none() {
            *retained = Some(reason);
        }
        self.closed.store(true, Ordering::Release);
    }

    fn reason(&self) -> Option<PeerTerminal> {
        self.reason
            .lock()
            .expect("peer terminal state poisoned")
            .clone()
    }
}

fn terminal_read_result(terminal: Option<PeerTerminal>) -> io::Result<()> {
    match terminal {
        Some(PeerTerminal::Disconnected(reason)) => match reason {
            ReliableUdpDisconnectReason::Closed | ReliableUdpDisconnectReason::ClosedByPeer => {
                Ok(())
            }
            ReliableUdpDisconnectReason::ConnectionTimeout => {
                Err(io::Error::new(io::ErrorKind::TimedOut, reason.as_str()))
            }
            ReliableUdpDisconnectReason::ConnectionReset
            | ReliableUdpDisconnectReason::Reconnect => Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                reason.as_str(),
            )),
            ReliableUdpDisconnectReason::Starvation => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                reason.as_str(),
            )),
        },
        Some(PeerTerminal::Failed(error)) => {
            Err(io::Error::new(io::ErrorKind::ConnectionAborted, error))
        }
        Some(PeerTerminal::Closed) | None => Ok(()),
    }
}

struct ConnectedPeer {
    generation: u64,
    inbound: mpsc::Sender<PeerInbound>,
    staged: VecDeque<PeerInbound>,
    terminal: Arc<PeerTerminalState>,
}

struct PeerInboundServiceSinks<'a> {
    commands: &'a mpsc::Sender<HubCommand>,
    incoming: &'a mpsc::Sender<io::Result<ReliableUdpPeerStream>>,
    puncher_events: &'a mpsc::Sender<NetpuncherIoEvent>,
}

/// One reliable-UDP peer exposed through the stream contract expected by
/// `ControlTransport`.
pub struct ReliableUdpPeerStream {
    peer: SocketAddr,
    generation: u64,
    commands: mpsc::Sender<HubCommand>,
    inbound: mpsc::Receiver<PeerInbound>,
    terminal: Arc<PeerTerminalState>,
    read_frame: Vec<u8>,
    read_offset: usize,
    write_buffer: Vec<u8>,
    pending_send: Option<Vec<u8>>,
    send_reservation: Option<HubCommandPermitFuture>,
    inbound_capacity_pending: bool,
    inbound_capacity_reservation: Option<HubCommandPermitFuture>,
    read_closed: bool,
    write_closed: bool,
    close_requested: bool,
}

impl ReliableUdpPeerStream {
    fn new(
        peer: SocketAddr,
        generation: u64,
        commands: mpsc::Sender<HubCommand>,
        inbound: mpsc::Receiver<PeerInbound>,
        terminal: Arc<PeerTerminalState>,
    ) -> Self {
        Self {
            peer,
            generation,
            commands,
            inbound,
            terminal,
            read_frame: Vec::new(),
            read_offset: 0,
            write_buffer: Vec::new(),
            pending_send: None,
            send_reservation: None,
            inbound_capacity_pending: false,
            inbound_capacity_reservation: None,
            read_closed: false,
            write_closed: false,
            close_requested: false,
        }
    }

    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    pub(crate) fn outbox_registration(&self) -> ReliableUdpOutboxRegistration {
        ReliableUdpOutboxRegistration {
            peer: self.peer,
            generation: self.generation,
            commands: self.commands.clone(),
        }
    }

    /// Binds this physical UDP peer to its high-level Network2 connection ID.
    /// The hub keeps accounting below the synthetic stream framing layer.
    pub fn bind_statistics_connection(
        &self,
        connection_id: u32,
    ) -> impl Future<Output = io::Result<()>> + Send + 'static {
        let terminal_closed = self.terminal.is_closed();
        let commands = self.commands.clone();
        let peer = self.peer;
        let generation = self.generation;
        async move {
            if terminal_closed {
                return Err(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "reliable-UDP peer stream is closed",
                ));
            }
            let (response, completed) = oneshot::channel();
            commands
                .send(HubCommand::BindStatistics {
                    peer,
                    generation,
                    connection_id,
                    response,
                })
                .await
                .map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "reliable-UDP session hub stopped while binding statistics",
                    )
                })?;
            completed.await.map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while binding statistics",
                )
            })?
        }
    }

    fn buffered_frame_size(&self) -> io::Result<Option<usize>> {
        if self.write_buffer.is_empty() {
            return Ok(None);
        }
        if self.write_buffer[0] != TCP_FRAME_PREFIX {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP session stream received an invalid TCP frame prefix",
            ));
        }
        if self.write_buffer.len() < TCP_FRAME_HEADER_SIZE {
            return Ok(None);
        }
        let packet_size = u32::from_ne_bytes(
            self.write_buffer[1..TCP_FRAME_HEADER_SIZE]
                .try_into()
                .expect("TCP frame header length checked"),
        ) as usize;
        TCP_FRAME_HEADER_SIZE
            .checked_add(packet_size)
            .map(Some)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TCP frame size overflow"))
    }

    fn poll_pending_send(&mut self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.terminal.is_closed() {
            self.mark_write_closed();
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP peer stream is closed",
            )));
        }
        if self.pending_send.is_none() {
            return Poll::Ready(Ok(()));
        }
        if self.send_reservation.is_none() {
            self.send_reservation = Some(Box::pin(self.commands.clone().reserve_owned()));
        }
        let reservation = self
            .send_reservation
            .as_mut()
            .expect("pending send has a command reservation");
        match reservation.as_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(permit)) => {
                self.send_reservation = None;
                let payload = self.pending_send.take().expect("pending send payload");
                permit.send(HubCommand::Send {
                    peer: self.peer,
                    generation: self.generation,
                    payload,
                });
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) => {
                self.send_reservation = None;
                self.pending_send = None;
                self.mark_transport_closed();
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped",
                )))
            }
        }
    }

    fn mark_transport_closed(&mut self) {
        self.terminal.close(PeerTerminal::Closed);
        self.read_closed = true;
        self.inbound_capacity_pending = false;
        self.inbound_capacity_reservation = None;
        self.mark_write_closed();
        self.inbound.close();
    }

    fn request_inbound_capacity(&mut self) {
        if self.inbound_capacity_pending {
            return;
        }
        let command = HubCommand::InboundCapacity {
            peer: self.peer,
            generation: self.generation,
        };
        match self.commands.try_send(command) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) | Err(mpsc::error::TrySendError::Closed(_)) => {
                // The old all-peer sweep happened to mask a dropped wakeup.
                // Retain it explicitly so a core-held ordered packet cannot
                // remain stalled after the application frees mailbox credit.
                self.inbound_capacity_pending = true;
            }
        }
    }

    fn poll_inbound_capacity_notification(
        &mut self,
        context: &mut Context<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.inbound_capacity_pending {
            return Poll::Ready(Ok(()));
        }
        if self.inbound_capacity_reservation.is_none() {
            self.inbound_capacity_reservation =
                Some(Box::pin(self.commands.clone().reserve_owned()));
        }
        let reservation = self
            .inbound_capacity_reservation
            .as_mut()
            .expect("pending inbound-capacity wakeup has a command reservation");
        match reservation.as_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(permit)) => {
                self.inbound_capacity_reservation = None;
                self.inbound_capacity_pending = false;
                permit.send(HubCommand::InboundCapacity {
                    peer: self.peer,
                    generation: self.generation,
                });
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(_)) => {
                self.inbound_capacity_reservation = None;
                self.inbound_capacity_pending = false;
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while restoring inbound capacity",
                )))
            }
        }
    }

    fn mark_write_closed(&mut self) {
        self.write_closed = true;
        self.write_buffer.clear();
        self.pending_send = None;
        self.send_reservation = None;
        self.request_close();
    }

    fn request_close(&mut self) {
        if self.close_requested {
            return;
        }
        self.close_requested = true;
        let _ = self.commands.try_send(HubCommand::Close {
            peer: self.peer,
            generation: self.generation,
        });
    }

    fn install_read_frame(&mut self, payload: Vec<u8>) -> io::Result<()> {
        let packet_size = u32::try_from(payload.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP session packet length exceeds u32",
            )
        })?;
        let frame_size = TCP_FRAME_HEADER_SIZE
            .checked_add(payload.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "TCP frame size overflow"))?;
        self.read_frame = Vec::with_capacity(frame_size);
        self.read_frame.push(TCP_FRAME_PREFIX);
        self.read_frame
            .extend_from_slice(&packet_size.to_ne_bytes());
        self.read_frame.extend(payload);
        self.read_offset = 0;
        Ok(())
    }
}

impl fmt::Debug for ReliableUdpPeerStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpPeerStream")
            .field("peer", &self.peer)
            .field("read_closed", &self.read_closed)
            .field("write_closed", &self.write_closed)
            .finish_non_exhaustive()
    }
}

impl AsyncRead for ReliableUdpPeerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if this.read_offset < this.read_frame.len() {
                let available = &this.read_frame[this.read_offset..];
                let copy_length = available.len().min(output.remaining());
                output.put_slice(&available[..copy_length]);
                this.read_offset += copy_length;
                if this.read_offset == this.read_frame.len() {
                    this.read_frame.clear();
                    this.read_offset = 0;
                }
                return Poll::Ready(Ok(()));
            }
            if this.read_closed {
                return Poll::Ready(Ok(()));
            }
            match this.poll_inbound_capacity_notification(context) {
                // Register the command-capacity wake, but keep draining data
                // already admitted to this peer's mailbox. Once that mailbox
                // empties, both sources share this task's waker and the
                // retained reservation can restore core delivery credit.
                Poll::Pending => {}
                Poll::Ready(Err(error)) => {
                    this.mark_transport_closed();
                    return Poll::Ready(Err(error));
                }
                Poll::Ready(Ok(())) => {}
            }
            match Pin::new(&mut this.inbound).poll_recv(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(PeerInbound::Packet(payload))) => {
                    if let Err(error) = this.install_read_frame(payload) {
                        this.mark_transport_closed();
                        return Poll::Ready(Err(error));
                    }
                    this.request_inbound_capacity();
                }
                Poll::Ready(None) => {
                    let terminal = this.terminal.reason();
                    this.mark_transport_closed();
                    return Poll::Ready(terminal_read_result(terminal));
                }
            }
        }
    }
}

impl AsyncWrite for ReliableUdpPeerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.write_closed || this.terminal.is_closed() {
            this.mark_write_closed();
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP peer stream is closed",
            )));
        }
        if input.is_empty() {
            return Poll::Ready(Ok(0));
        }
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }

        let mut accepted = 0;
        if this.write_buffer.len() < TCP_FRAME_HEADER_SIZE {
            let header_remaining = TCP_FRAME_HEADER_SIZE - this.write_buffer.len();
            let take = header_remaining.min(input.len());
            this.write_buffer.extend_from_slice(&input[..take]);
            accepted += take;
        }
        let frame_size = match this.buffered_frame_size() {
            Ok(Some(frame_size)) => frame_size,
            Ok(None) => return Poll::Ready(Ok(accepted)),
            Err(error) => {
                this.mark_transport_closed();
                return Poll::Ready(Err(error));
            }
        };
        let frame_remaining = frame_size - this.write_buffer.len();
        let take = frame_remaining.min(input.len() - accepted);
        this.write_buffer
            .extend_from_slice(&input[accepted..accepted + take]);
        accepted += take;
        if this.write_buffer.len() == frame_size {
            let mut payload = std::mem::take(&mut this.write_buffer);
            payload.drain(..TCP_FRAME_HEADER_SIZE);
            this.pending_send = Some(payload);
            if let Poll::Ready(Err(error)) = this.poll_pending_send(context) {
                return Poll::Ready(Err(error));
            }
        }
        Poll::Ready(Ok(accepted))
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        if !this.write_buffer.is_empty() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP peer stream has an incomplete TCP frame",
            )));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.write_closed {
            return Poll::Ready(Ok(()));
        }
        match this.poll_pending_send(context) {
            Poll::Pending => return Poll::Pending,
            Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
            Poll::Ready(Ok(())) => {}
        }
        if !this.write_buffer.is_empty() {
            return Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "reliable-UDP peer stream has an incomplete TCP frame",
            )));
        }
        this.read_closed = true;
        this.write_closed = true;
        this.inbound.close();
        this.request_close();
        Poll::Ready(Ok(()))
    }
}

impl Drop for ReliableUdpPeerStream {
    fn drop(&mut self) {
        self.request_close();
    }
}

/// A single peer stream which owns the socket hub that drives it.
///
/// This is the convenient client-side form: it can be moved directly into a
/// `ControlTransport` without retaining a separate hub handle. Field order is
/// intentional—the peer requests a clean Close before the hub's drop path
/// requests task shutdown.
pub struct ReliableUdpOwnedPeerStream {
    peer: ReliableUdpPeerStream,
    hub: ReliableUdpSessionHub,
}

impl ReliableUdpOwnedPeerStream {
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer.peer_addr()
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.hub.local_addr()
    }
}

impl fmt::Debug for ReliableUdpOwnedPeerStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpOwnedPeerStream")
            .field("peer", &self.peer_addr())
            .field("local", &self.local_addr())
            .finish_non_exhaustive()
    }
}

impl AsyncRead for ReliableUdpOwnedPeerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_read(context, output)
    }
}

impl AsyncWrite for ReliableUdpOwnedPeerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        input: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().peer).poll_write(context, input)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().peer).poll_shutdown(context)
    }
}

/// Cloneable command side of a shared reliable-UDP socket. Keeping this
/// separate from the hub owner lets client dial races, reconnects, and the
/// netpuncher all retain the same NAT-visible source port.
#[derive(Clone, Debug)]
pub struct ReliableUdpSessionHandle {
    commands: mpsc::Sender<HubCommand>,
    voice_media: mpsc::Sender<ReliableUdpVoiceDatagram>,
}

impl ReliableUdpSessionHandle {
    pub(crate) fn try_send_voice_media(&self, peer: SocketAddr, payload: Vec<u8>) -> bool {
        self.voice_media
            .try_send(ReliableUdpVoiceDatagram {
                peer: canonical_reliable_udp_peer_address(peer),
                payload,
            })
            .is_ok()
    }

    pub async fn init_puncher(&self, address: SocketAddr, role: NetpuncherRole) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::InitPuncher {
                address,
                role,
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped during puncher initialization",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped during puncher initialization",
            )
        })?
    }

    pub async fn send_puncher_packet(
        &self,
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
    ) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::SendPuncherPacket {
                family,
                packet,
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while sending a puncher packet",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped while sending a puncher packet",
            )
        })?
    }

    pub async fn close_puncher(&self, address: SocketAddr) -> io::Result<()> {
        let (response, completed) = oneshot::channel();
        self.commands
            .send(HubCommand::ClosePuncher { address, response })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped while closing a puncher route",
                )
            })?;
        completed.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped while closing a puncher route",
            )
        })?
    }

    pub async fn connect(&self, peer: SocketAddr) -> io::Result<ReliableUdpPeerStream> {
        let (response, connected) = oneshot::channel();
        self.commands
            .send(HubCommand::Connect {
                peer: canonical_reliable_udp_peer_address(peer),
                response,
            })
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "reliable-UDP session hub stopped",
                )
            })?;
        connected.await.map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped during connect",
            )
        })?
    }
}

/// Shared reliable-UDP socket with stream-like outgoing and incoming peers.
pub struct ReliableUdpSessionHub {
    local_addr: SocketAddr,
    commands: mpsc::Sender<HubCommand>,
    voice_media: mpsc::Sender<ReliableUdpVoiceDatagram>,
    incoming: mpsc::Receiver<io::Result<ReliableUdpPeerStream>>,
    puncher_events: Option<mpsc::Receiver<NetpuncherIoEvent>>,
    voice_media_events: Option<mpsc::Receiver<ReliableUdpVoiceDatagram>>,
    task: Option<JoinHandle<io::Result<()>>>,
    shutdown_requested: bool,
}

impl ReliableUdpSessionHub {
    pub fn bind(bind_address: SocketAddr) -> io::Result<Self> {
        Self::from_driver(ReliableUdpSocketDriver::bind(bind_address)?)
    }

    pub fn bind_with_statistics(
        bind_address: SocketAddr,
        statistics: crate::NetworkIoStatistics,
    ) -> io::Result<Self> {
        Self::from_driver(ReliableUdpSocketDriver::bind_with_statistics(
            bind_address,
            statistics,
        )?)
    }

    pub fn from_driver(driver: ReliableUdpSocketDriver) -> io::Result<Self> {
        let runtime = tokio::runtime::Handle::try_current().map_err(|_| {
            io::Error::other("reliable-UDP session hub requires an entered Tokio runtime")
        })?;
        let local_addr = canonical_reliable_udp_peer_address(driver.local_addr()?);
        let (commands, command_rx) = mpsc::channel(HUB_COMMAND_CAPACITY);
        let (incoming_tx, incoming) = mpsc::channel(INCOMING_PEER_CAPACITY);
        let (puncher_event_tx, puncher_events) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let (voice_media, voice_media_rx) = mpsc::channel(VOICE_MEDIA_CAPACITY);
        let (voice_media_event_tx, voice_media_events) = mpsc::channel(VOICE_MEDIA_CAPACITY);
        let outbox = Arc::new(UdpSharedOutbox::default());
        let task_commands = commands.clone();
        let task = runtime.spawn(run_hub(
            driver,
            task_commands,
            command_rx,
            incoming_tx,
            puncher_event_tx,
            HubVoiceMedia {
                outgoing: voice_media_rx,
                incoming: voice_media_event_tx,
            },
            outbox,
        ));
        Ok(Self {
            local_addr,
            commands,
            voice_media,
            incoming,
            puncher_events: Some(puncher_events),
            voice_media_events: Some(voice_media_events),
            task: Some(task),
            shutdown_requested: false,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn handle(&self) -> ReliableUdpSessionHandle {
        ReliableUdpSessionHandle {
            commands: self.commands.clone(),
            voice_media: self.voice_media.clone(),
        }
    }

    pub async fn init_puncher(&self, address: SocketAddr, role: NetpuncherRole) -> io::Result<()> {
        self.handle().init_puncher(address, role).await
    }

    pub async fn send_puncher_packet(
        &self,
        family: NetpuncherAddressFamily,
        packet: NetpuncherPacket,
    ) -> io::Result<()> {
        self.handle().send_puncher_packet(family, packet).await
    }

    pub async fn close_puncher(&self, address: SocketAddr) -> io::Result<()> {
        self.handle().close_puncher(address).await
    }

    /// Detaches the low-volume callback stream so a session loop can select
    /// it independently from ordinary incoming game peers.
    pub fn take_puncher_event_receiver(&mut self) -> mpsc::Receiver<NetpuncherIoEvent> {
        self.puncher_events
            .take()
            .expect("puncher event receiver already taken")
    }

    pub(crate) fn take_voice_media_receiver(&mut self) -> mpsc::Receiver<ReliableUdpVoiceDatagram> {
        self.voice_media_events
            .take()
            .expect("voice media receiver already taken")
    }

    pub async fn connect(&self, peer: SocketAddr) -> io::Result<ReliableUdpPeerStream> {
        self.handle().connect(peer).await
    }

    /// Connects one peer and transfers ownership of this hub into the returned
    /// stream. Dropping that stream closes the peer and shuts down the hub.
    pub async fn connect_owned(self, peer: SocketAddr) -> io::Result<ReliableUdpOwnedPeerStream> {
        let stream = self.connect(peer).await?;
        Ok(ReliableUdpOwnedPeerStream {
            peer: stream,
            hub: self,
        })
    }

    pub async fn accept(&mut self) -> io::Result<ReliableUdpPeerStream> {
        self.incoming.recv().await.unwrap_or_else(|| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "reliable-UDP session hub stopped",
            ))
        })
    }

    pub async fn shutdown(mut self) -> io::Result<()> {
        let (completion, completed) = oneshot::channel();
        let command_delivered = self
            .commands
            .send(HubCommand::Shutdown {
                completion: Some(completion),
            })
            .await
            .is_ok();
        // If this future is cancelled while the bounded queue is full, leave
        // the flag clear so Drop can retry or abort the socket task.
        self.shutdown_requested = command_delivered;
        if command_delivered {
            let _ = completed.await;
        }
        match self.task.take() {
            Some(task) => task.await.map_err(|error| {
                io::Error::other(format!("reliable-UDP session task failed: {error}"))
            })?,
            None => Ok(()),
        }
    }
}

impl fmt::Debug for ReliableUdpSessionHub {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReliableUdpSessionHub")
            .field("local_addr", &self.local_addr)
            .field("shutdown_requested", &self.shutdown_requested)
            .finish_non_exhaustive()
    }
}

impl Drop for ReliableUdpSessionHub {
    fn drop(&mut self) {
        if !self.shutdown_requested {
            self.shutdown_requested = true;
            if self
                .commands
                .try_send(HubCommand::Shutdown { completion: None })
                .is_err()
            {
                if let Some(task) = self.task.as_ref() {
                    task.abort();
                }
            }
        }
    }
}

async fn run_hub(
    mut driver: ReliableUdpSocketDriver,
    commands: mpsc::Sender<HubCommand>,
    mut command_rx: mpsc::Receiver<HubCommand>,
    incoming: mpsc::Sender<io::Result<ReliableUdpPeerStream>>,
    puncher_events: mpsc::Sender<NetpuncherIoEvent>,
    mut voice_media: HubVoiceMedia,
    outbox: Arc<UdpSharedOutbox>,
) -> io::Result<()> {
    // Task cancellation and the hub owner's full-command-queue abort path do
    // not execute an async shutdown branch. The synchronous guard rejects all
    // surviving senders and releases every drain waiter on every exit.
    let _outbox_guard = UdpOutboxRunGuard(outbox.clone());
    let mut peers = BTreeMap::<SocketAddr, ConnectedPeer>::new();
    let mut pending_connects =
        BTreeMap::<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>::new();
    let mut next_peer_generation = 0_u64;
    let mut abandoned_peer_maintenance = tokio::time::interval_at(
        tokio::time::Instant::now() + ABANDONED_PEER_MAINTENANCE_INTERVAL,
        ABANDONED_PEER_MAINTENANCE_INTERVAL,
    );
    abandoned_peer_maintenance.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = command_rx.recv() => {
                let Some(command) = command else {
                    outbox.fail_all();
                    close_all(&mut driver, &mut peers, &mut pending_connects).await;
                    return Ok(());
                };
                match command {
                    HubCommand::InitPuncher { address, role, response } => {
                        let result = match driver.init_puncher(address, role).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(error),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::SendPuncherPacket { family, packet, response } => {
                        let result = match driver.send_puncher_packet(family, &packet).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(reliable_udp_driver_io_error(error)),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::ClosePuncher { address, response } => {
                        let result = match driver.close_puncher(address).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                                Ok(())
                            }
                            Err(error) => Err(error),
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::Connect { peer, response } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if peers.contains_key(&peer)
                            || pending_connects.contains_key(&peer)
                            || driver.core().peer_status(peer).is_some()
                        {
                            let _ = response.send(Err(io::Error::new(
                                io::ErrorKind::AlreadyExists,
                                format!("reliable-UDP peer {peer} is already connected or connecting"),
                            )));
                            continue;
                        }
                        pending_connects.insert(peer, response);
                        match driver.connect(peer).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_pending_connect(&mut pending_connects, peer, error);
                            }
                        }
                    }
                    HubCommand::BindStatistics {
                        peer,
                        generation,
                        connection_id,
                        response,
                    } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        let result = if peer_generation_matches(&peers, peer, generation) {
                            driver.bind_peer_statistics(peer, connection_id)
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::NotConnected,
                                format!("reliable-UDP peer {peer} is no longer connected"),
                            ))
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::PromoteOutbound {
                        peer,
                        generation,
                        packet_log,
                        response,
                    } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        let result = if peer_generation_matches(&peers, peer, generation) {
                            outbox.register_route(peer, generation, packet_log).ok_or_else(|| {
                                io::Error::new(
                                    io::ErrorKind::AlreadyExists,
                                    format!(
                                        "reliable-UDP peer {peer} generation {generation} already has an outbound route"
                                    ),
                                )
                            })
                        } else {
                            Err(io::Error::new(
                                io::ErrorKind::NotConnected,
                                format!("reliable-UDP peer {peer} is no longer connected"),
                            ))
                        };
                        let _ = response.send(result);
                    }
                    HubCommand::Send {
                        peer,
                        generation,
                        payload,
                    } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if !peer_generation_matches(&peers, peer, generation) {
                            continue;
                        }
                        match driver.send_packet(peer, &payload).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_peer(&mut peers, peer, generation, error.to_string());
                                let _ = driver.close_peer(peer).await;
                            }
                        }
                    }
                    HubCommand::Close { peer, generation } => {
                        let peer = canonical_reliable_udp_peer_address(peer);
                        if !peer_generation_matches(&peers, peer, generation) {
                            continue;
                        }
                        match driver.close_peer(peer).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                fail_peer(&mut peers, peer, generation, error.to_string())
                            }
                        }
                    }
                    HubCommand::InboundCapacity { peer, generation } => {
                        service_peer_inbound(
                            &mut driver,
                            peer,
                            generation,
                            PeerInboundServiceSinks {
                                commands: &commands,
                                incoming: &incoming,
                                puncher_events: &puncher_events,
                            },
                            &mut peers,
                            &mut pending_connects,
                            &mut next_peer_generation,
                        )
                        .await;
                    }
                    HubCommand::Shutdown { completion } => {
                        outbox.fail_all();
                        close_all(&mut driver, &mut peers, &mut pending_connects).await;
                        if let Some(completion) = completion {
                            let _ = completion.send(());
                        }
                        return Ok(());
                    }
                }
            }
            _ = outbox.wait_ready() => {
                let Some(action) = outbox.take_action() else {
                    continue;
                };
                match action {
                    UdpOutboxAction::Send {
                        route,
                        peer,
                        generation,
                        payload,
                    } => {
                        if !peer_generation_matches(&peers, peer, generation) {
                            outbox.fail_route(route);
                            continue;
                        }
                        match driver.send_packet(peer, &payload).await {
                            Ok(events) => {
                                dispatch_events(
                                    &mut driver,
                                    events,
                                    &commands,
                                    &incoming,
                                    &puncher_events,
                                    &mut peers,
                                    &mut pending_connects,
                                    &mut next_peer_generation,
                                )
                                .await;
                            }
                            Err(error) => {
                                // C4Network2IO::Broadcast combines every Send
                                // result with `&=`. Retain this route's accepted
                                // suffix, then let the outbox continue with the
                                // later targets (oracle-src-pinned
                                // src/C4Network2IO.cpp:378-404,1437-1477).
                                outbox.fail_route(route);
                                fail_peer(&mut peers, peer, generation, error.to_string());
                                let _ = driver.close_peer(peer).await;
                            }
                        }
                    }
                    UdpOutboxAction::Failed {
                        route,
                        peer,
                        generation,
                        error,
                    } => {
                        outbox.fail_route(route);
                        if peer_generation_matches(&peers, peer, generation) {
                            fail_peer(&mut peers, peer, generation, error.to_string());
                            let _ = driver.close_peer(peer).await;
                        }
                    }
                    UdpOutboxAction::Retired {
                        route,
                        peer,
                        generation,
                    } => {
                        if peer_generation_matches(&peers, peer, generation) {
                            match driver.close_peer(peer).await {
                                Ok(events) => {
                                    dispatch_events(
                                        &mut driver,
                                        events,
                                        &commands,
                                        &incoming,
                                        &puncher_events,
                                        &mut peers,
                                        &mut pending_connects,
                                        &mut next_peer_generation,
                                    )
                                    .await;
                                }
                                Err(error) => {
                                    fail_peer(&mut peers, peer, generation, error.to_string())
                                }
                            }
                        }
                        outbox.finish_retire(route);
                    }
                    UdpOutboxAction::Close {
                        route,
                        peer,
                        generation,
                        payload,
                    } => {
                        if peer_generation_matches(&peers, peer, generation) {
                            if let Some(payload) = payload {
                                if let Ok(events) = driver.send_packet(peer, &payload).await {
                                    dispatch_events(
                                        &mut driver,
                                        events,
                                        &commands,
                                        &incoming,
                                        &puncher_events,
                                        &mut peers,
                                        &mut pending_connects,
                                        &mut next_peer_generation,
                                    )
                                    .await;
                                }
                            }
                            if peer_generation_matches(&peers, peer, generation) {
                                if let Some(events) = resolve_udp_outbox_close_result(
                                    &mut peers,
                                    peer,
                                    generation,
                                    driver.close_peer(peer).await,
                                ) {
                                    dispatch_events(
                                        &mut driver,
                                        events,
                                        &commands,
                                        &incoming,
                                        &puncher_events,
                                        &mut peers,
                                        &mut pending_connects,
                                        &mut next_peer_generation,
                                    )
                                    .await;
                                }
                            }
                        }
                        outbox.finish_retire(route);
                    }
                }
            }
            ready = driver.wait_ready() => {
                if let ReliableUdpPollReady::Datagram(_, source) = &ready {
                    let source = canonical_reliable_udp_peer_address(*source);
                    let capacity = peers
                        .get(&source)
                        .map(connected_peer_delivery_capacity)
                        .unwrap_or(PEER_INBOUND_PACKET_CAPACITY);
                    driver.set_peer_delivery_credit(source, capacity);
                }
                // Once readiness advances the reliable-UDP core, finish its
                // ACK/event flush outside the cancellable select future.
                let result = driver.process_ready(ready).await;
                match result {
                    Ok(events) => {
                        if let Some((peer, payload)) = driver.take_voice_media() {
                            let peer = canonical_reliable_udp_peer_address(peer);
                            let authenticated = peers
                                .get(&peer)
                                .and_then(|connected| {
                                    outbox.voice_receive_cookie(peer, connected.generation)
                                })
                                .is_some_and(|cookie| {
                                    crate::voice::voice_datagram_has_cookie(&payload, cookie)
                                });
                            if authenticated {
                                let _ = voice_media
                                    .incoming
                                    .try_send(ReliableUdpVoiceDatagram { peer, payload });
                            }
                        }
                        dispatch_events(
                            &mut driver,
                            events,
                            &commands,
                            &incoming,
                            &puncher_events,
                            &mut peers,
                            &mut pending_connects,
                            &mut next_peer_generation,
                        )
                        .await;
                    }
                    Err(error) => {
                        let message = error.to_string();
                        outbox.fail_all();
                        fail_all(&mut peers, &mut pending_connects, &message);
                        let _ = incoming.try_send(Err(io::Error::new(error.kind(), message)));
                        return Err(error);
                    }
                }
            }
            voice = voice_media.outgoing.recv(), if crate::voice::voice_media_may_run(!command_rx.is_empty(), false) => {
                if let Some(voice) = voice {
                    let _ = driver.try_send_voice_media(voice.peer, &voice.payload);
                }
            }
            _ = abandoned_peer_maintenance.tick() => {
                // A stream Drop normally enqueues Close. The bounded command
                // queue may be full, so retain a low-frequency recovery edge
                // without scanning every peer before every hub transition.
                close_abandoned_peers(&mut driver, &mut peers).await;
            }
        }
    }
}

fn drain_staged_peer_inbound(connected: &mut ConnectedPeer) {
    while let Some(item) = connected.staged.pop_front() {
        match connected.inbound.try_send(item) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(item)) => {
                connected.staged.push_front(item);
                break;
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                connected.staged.clear();
                break;
            }
        }
    }
}

fn connected_peer_delivery_capacity(connected: &ConnectedPeer) -> usize {
    if connected.staged.is_empty() {
        connected.inbound.capacity()
    } else {
        0
    }
}

fn queue_peer_inbound(connected: &mut ConnectedPeer, item: PeerInbound) {
    if !connected.staged.is_empty() {
        connected.staged.push_back(item);
        return;
    }
    match connected.inbound.try_send(item) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(item)) => {
            // Delivery credit normally prevents this branch. Retain the
            // already-admitted logical packet if capacity changed between
            // core dispatch and mailbox publication.
            connected.staged.push_back(item);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

fn finish_peer_inbound(mut connected: ConnectedPeer) {
    if connected.staged.is_empty() {
        return;
    }
    tokio::spawn(async move {
        while let Some(item) = connected.staged.pop_front() {
            if connected.inbound.send(item).await.is_err() {
                break;
            }
        }
    });
}

async fn service_peer_inbound(
    driver: &mut ReliableUdpSocketDriver,
    peer: SocketAddr,
    generation: u64,
    sinks: PeerInboundServiceSinks<'_>,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    next_peer_generation: &mut u64,
) {
    let peer = canonical_reliable_udp_peer_address(peer);
    if !peer_generation_matches(peers, peer, generation) {
        return;
    }
    let capacity = {
        let Some(connected) = peers.get_mut(&peer) else {
            return;
        };
        drain_staged_peer_inbound(connected);
        connected_peer_delivery_capacity(connected)
    };
    if capacity == 0 {
        return;
    }
    let Ok(events) = driver.drain_peer_packets(peer, capacity).await else {
        return;
    };
    if events.is_empty() {
        return;
    }
    dispatch_events(
        driver,
        events,
        sinks.commands,
        sinks.incoming,
        sinks.puncher_events,
        peers,
        pending_connects,
        next_peer_generation,
    )
    .await;
}

// Event dispatch mutates the driver, independent delivery channels, peer
// registries, and generation counter in one ordered reducer step.
#[allow(clippy::too_many_arguments)]
async fn dispatch_events(
    driver: &mut ReliableUdpSocketDriver,
    events: Vec<ReliableUdpEvent>,
    commands: &mpsc::Sender<HubCommand>,
    incoming: &mpsc::Sender<io::Result<ReliableUdpPeerStream>>,
    puncher_events: &mpsc::Sender<NetpuncherIoEvent>,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    next_peer_generation: &mut u64,
) {
    for event in events {
        match event {
            ReliableUdpEvent::Connected { peer, .. } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if peers.contains_key(&peer) {
                    continue;
                }
                let generation = *next_peer_generation;
                *next_peer_generation = next_peer_generation.wrapping_add(1);
                // Native UDP stores completed ordered packets in PacketList's
                // effectively-unbounded default and drains them synchronously
                // through the connection callback. The async adapter must
                // likewise retain every delivered packet without awaiting one
                // peer and stalling the shared socket driver
                // (oracle-src-pinned src/C4NetIO.h:543-566;
                // src/C4NetIO.cpp:2648-2652,3175-3199).
                let (inbound, inbound_rx) = mpsc::channel(PEER_INBOUND_PACKET_CAPACITY);
                let terminal = Arc::new(PeerTerminalState::open());
                let stream = ReliableUdpPeerStream::new(
                    peer,
                    generation,
                    commands.clone(),
                    inbound_rx,
                    terminal.clone(),
                );
                let delivered = if let Some(response) = pending_connects.remove(&peer) {
                    response.send(Ok(stream)).is_ok()
                } else {
                    incoming.try_send(Ok(stream)).is_ok()
                };
                if delivered {
                    peers.insert(
                        peer,
                        ConnectedPeer {
                            generation,
                            inbound,
                            staged: VecDeque::new(),
                            terminal,
                        },
                    );
                } else {
                    let _ = driver.close_peer(peer).await;
                }
            }
            ReliableUdpEvent::Packet { peer, payload } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if let Some(connected) = peers.get_mut(&peer) {
                    queue_peer_inbound(connected, PeerInbound::Packet(payload));
                }
            }
            ReliableUdpEvent::Disconnected { peer, reason } => {
                let peer = canonical_reliable_udp_peer_address(peer);
                if let Some(response) = pending_connects.remove(&peer) {
                    let _ = response.send(Err(disconnect_error(peer, reason)));
                }
                if let Some(connected) = peers.remove(&peer) {
                    connected.terminal.close(PeerTerminal::Disconnected(reason));
                    finish_peer_inbound(connected);
                }
            }
            ReliableUdpEvent::Puncher(event) => {
                let puncher_address = event.puncher_address();
                if puncher_events.try_send(event).is_err() {
                    // Puncher input is network-driven. If the bounded callback
                    // queue cannot retain it, close this exact special route
                    // instead of leaking memory or misrouting later packets.
                    let _ = driver.close_puncher(puncher_address).await;
                }
            }
        }
    }
}

fn reliable_udp_driver_io_error(error: ReliableUdpDriverError) -> io::Error {
    match error {
        ReliableUdpDriverError::Io(error) => error,
        ReliableUdpDriverError::Runtime(error) => {
            io::Error::new(io::ErrorKind::InvalidData, error.to_string())
        }
    }
}

fn disconnect_error(peer: SocketAddr, reason: ReliableUdpDisconnectReason) -> io::Error {
    let kind = match reason {
        ReliableUdpDisconnectReason::ConnectionTimeout => io::ErrorKind::TimedOut,
        ReliableUdpDisconnectReason::ConnectionReset | ReliableUdpDisconnectReason::Reconnect => {
            io::ErrorKind::ConnectionReset
        }
        ReliableUdpDisconnectReason::Starvation => io::ErrorKind::ConnectionAborted,
        ReliableUdpDisconnectReason::Closed | ReliableUdpDisconnectReason::ClosedByPeer => {
            io::ErrorKind::ConnectionAborted
        }
    };
    io::Error::new(
        kind,
        format!("reliable-UDP peer {peer} disconnected: {}", reason.as_str()),
    )
}

fn fail_pending_connect(
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    peer: SocketAddr,
    error: io::Error,
) {
    if let Some(response) = pending_connects.remove(&peer) {
        let _ = response.send(Err(error));
    }
}

fn peer_generation_matches(
    peers: &BTreeMap<SocketAddr, ConnectedPeer>,
    peer: SocketAddr,
    generation: u64,
) -> bool {
    peers
        .get(&canonical_reliable_udp_peer_address(peer))
        .is_some_and(|connected| connected.generation == generation)
}

async fn close_abandoned_peers(
    driver: &mut ReliableUdpSocketDriver,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
) {
    let abandoned = peers
        .iter()
        .filter_map(|(peer, connected)| connected.inbound.is_closed().then_some(*peer))
        .collect::<Vec<_>>();
    for peer in abandoned {
        peers.remove(&peer);
        let _ = driver.close_peer(peer).await;
    }
}

fn fail_peer(
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    peer: SocketAddr,
    generation: u64,
    error: String,
) {
    let peer = canonical_reliable_udp_peer_address(peer);
    if !peer_generation_matches(peers, peer, generation) {
        return;
    }
    if let Some(connected) = peers.remove(&peer) {
        connected
            .terminal
            .close(PeerTerminal::Failed(error.clone()));
        finish_peer_inbound(connected);
    }
}

fn resolve_udp_outbox_close_result(
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    peer: SocketAddr,
    generation: u64,
    result: io::Result<Vec<ReliableUdpEvent>>,
) -> Option<Vec<ReliableUdpEvent>> {
    match result {
        Ok(events) => Some(events),
        Err(error) => {
            fail_peer(peers, peer, generation, error.to_string());
            None
        }
    }
}

fn fail_all(
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
    error: &str,
) {
    for (_, connected) in std::mem::take(peers) {
        connected
            .terminal
            .close(PeerTerminal::Failed(error.to_string()));
        finish_peer_inbound(connected);
    }
    for (peer, response) in std::mem::take(pending_connects) {
        let _ = response.send(Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            format!("reliable-UDP peer {peer} failed: {error}"),
        )));
    }
}

async fn close_all(
    driver: &mut ReliableUdpSocketDriver,
    peers: &mut BTreeMap<SocketAddr, ConnectedPeer>,
    pending_connects: &mut BTreeMap<SocketAddr, oneshot::Sender<io::Result<ReliableUdpPeerStream>>>,
) {
    let addresses = peers
        .keys()
        .chain(pending_connects.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    for peer in addresses {
        if let Ok(events) = driver.close_peer(peer).await {
            for event in events {
                if let ReliableUdpEvent::Disconnected { peer, reason } = event {
                    let peer = canonical_reliable_udp_peer_address(peer);
                    if let Some(response) = pending_connects.remove(&peer) {
                        let _ = response.send(Err(disconnect_error(peer, reason)));
                    }
                    if let Some(connected) = peers.remove(&peer) {
                        connected.terminal.close(PeerTerminal::Disconnected(reason));
                        finish_peer_inbound(connected);
                    }
                }
            }
        }
    }
    fail_all(
        peers,
        pending_connects,
        "reliable-UDP session hub shut down",
    );
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UdpSocket;
    use tokio::time::timeout;

    use super::*;
    use crate::{
        decode_reliable_udp_data_fragment, encode_reliable_udp_connect,
        encode_reliable_udp_connect_ok, encode_reliable_udp_data_fragments,
        reliable_udp_packet_kind, ControlDelivery, ControlMessage, ControlTransport, PingPacket,
        ReliableUdpConnect, ReliableUdpConnectOk, ReliableUdpMulticastMode, ReliableUdpPacketKind,
    };

    #[test]
    fn synchronous_bind_reports_a_missing_tokio_runtime_instead_of_panicking() {
        let error = ReliableUdpSessionHub::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .expect_err("binding without an entered runtime must fail");
        assert_eq!(error.kind(), io::ErrorKind::Other);
    }

    fn loopback() -> SocketAddr {
        SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)
    }

    #[test]
    fn udp_outbox_many_retains_unselected_suffix_and_yields_one_ordered_target_per_action() {
        // C++ may call BroadcastMsg from either thread while mt-safe UDP
        // Execute independently services socket input. The single Rust hub
        // preserves that receive-progress seam by scheduling one selected
        // target per turn (oracle-src-pinned
        // src/C4Network2IO.cpp:395-407;
        // src/C4NetIO.cpp:2282-2297,2789-2810).
        let mut outbox = UdpLogicalOutbox::default();
        let routes = (0..3)
            .map(|_| {
                outbox.register_route(Arc::new(Mutex::new(crate::RecoverablePacketLog::default())))
            })
            .collect::<Vec<_>>();
        let payload = Arc::<[u8]>::from([crate::PACKET_LOG_START, 1]);
        assert_eq!(outbox.enqueue_many(routes.iter().copied(), payload), routes);

        for (index, &expected_route) in routes.iter().enumerate() {
            assert!(matches!(
                outbox.next_action(),
                Some(UdpOutboxAction::Send { route, .. }) if route == expected_route
            ));
            let remaining = routes[index + 1..].iter().copied().collect::<VecDeque<_>>();
            assert!(
                matches!(
                    outbox.queue.front(),
                    Some(UdpOutboxWork::Many { routes, .. }) if routes == &remaining
                ) || remaining.is_empty() && outbox.queue.is_empty()
            );
        }
    }

    #[test]
    fn udp_recoverable_fanout_reuses_one_encoded_payload_allocation() {
        // Native Broadcast passes one packet buffer through each selected
        // connection's PacketLog before CreatePostMortem restores its
        // oldest-to-newest sequence (oracle-src-pinned
        // src/C4Network2IO.cpp:395-404,1379-1407,1437-1477).
        let mut outbox = UdpLogicalOutbox::default();
        let logs = (0..3)
            .map(|index| {
                let mut log = crate::RecoverablePacketLog::default();
                assert_eq!(
                    log.record_outbound(vec![crate::PACKET_LOG_START, index]),
                    Some(0)
                );
                let log = Arc::new(Mutex::new(log));
                let route = outbox.register_route(log.clone());
                (route, log)
            })
            .collect::<Vec<_>>();
        let payload = Arc::<[u8]>::from([crate::PACKET_LOG_START, 0xaa, 0xbb]);
        assert_eq!(
            outbox.enqueue_many(logs.iter().map(|(route, _)| *route), payload.clone()),
            logs.iter().map(|(route, _)| *route).collect::<Vec<_>>()
        );

        while outbox.next_action().is_some() {}

        for (index, (_, log)) in logs.into_iter().enumerate() {
            let mut log = log.lock().unwrap();
            assert!(log.newest_packet_shares_storage_with(&payload));
            assert_eq!(
                log.create_post_mortem(40 + index as u32),
                Some(crate::PostMortemPacket {
                    connection_id: 40 + index as u32,
                    packet_counter: 2,
                    packets: vec![vec![crate::PACKET_LOG_START, index as u8], payload.to_vec(),],
                })
            );
        }
    }

    #[tokio::test]
    async fn udp_outbox_ready_wait_does_not_take_many_target_before_the_selected_branch() {
        let outbox = Arc::new(UdpSharedOutbox::default());
        let routes = (1..=2)
            .map(|port| {
                outbox
                    .register_route(
                        SocketAddr::from(([127, 0, 0, 1], port)),
                        7,
                        Arc::new(Mutex::new(crate::RecoverablePacketLog::default())),
                    )
                    .expect("test routes are unique")
            })
            .collect::<Vec<_>>();
        assert_eq!(
            outbox.enqueue_many(
                routes.iter().map(|route| route.lease.route),
                UdpPreparedPayload::Packet(Arc::from([crate::PACKET_LOG_START, 1])),
            ),
            routes
                .iter()
                .map(|route| route.lease.route)
                .collect::<Vec<_>>()
        );

        timeout(Duration::from_secs(1), outbox.wait_ready())
            .await
            .expect("queued batch did not make the outbox ready");
        assert!(matches!(
            outbox.take_action(),
            Some(UdpOutboxAction::Send { route, .. }) if route == routes[0].lease.route
        ));
        assert!(matches!(
            outbox.take_action(),
            Some(UdpOutboxAction::Send { route, .. }) if route == routes[1].lease.route
        ));
    }

    #[tokio::test]
    async fn udp_outbox_many_abort_before_first_send_retains_every_target_suffix() {
        // Send logs each selected connection before native UDP submission;
        // aborting the endpoint task cannot erase an already accepted target
        // from PostMortem recovery (oracle-src-pinned
        // src/C4Network2IO.cpp:395-404,1437-1477;
        // src/C4NetIO.cpp:2789-2810).
        let outbox = Arc::new(UdpSharedOutbox::default());
        let logs = (1..=2)
            .map(|port| {
                let log = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
                let sender = outbox
                    .register_route(SocketAddr::from(([127, 0, 0, 1], port)), 7, log.clone())
                    .expect("test routes are unique");
                (sender, log)
            })
            .collect::<Vec<_>>();
        let payload = Arc::<[u8]>::from([crate::PACKET_LOG_START, 9]);
        assert_eq!(
            outbox.enqueue_many(
                logs.iter().map(|(sender, _)| sender.lease.route),
                UdpPreparedPayload::Packet(payload.clone()),
            ),
            logs.iter()
                .map(|(sender, _)| sender.lease.route)
                .collect::<Vec<_>>()
        );
        outbox.wait_ready().await;
        assert!(matches!(
            outbox.take_action(),
            Some(UdpOutboxAction::Send { route, .. }) if route == logs[0].0.lease.route
        ));

        drop(UdpOutboxRunGuard(outbox.clone()));

        for (sender, log) in logs {
            timeout(Duration::from_secs(1), sender.wait_drained())
                .await
                .expect("hub abort did not drain a batch target");
            assert!(sender
                .try_send(ControlMessage::Ping(PingPacket {
                    sent_at: 1,
                    packet_counter: 2,
                }))
                .is_err());
            assert_eq!(
                log.lock().unwrap().create_post_mortem(31).unwrap().packets,
                vec![payload.to_vec()]
            );
        }
    }

    #[tokio::test]
    async fn udp_outbox_many_abort_after_first_target_retains_unprocessed_suffix_order() {
        // Native Broadcast logs each selected connection before its synchronous
        // UDP submission; task cancellation must retain both the in-flight
        // batch and later accepted packets in per-route order
        // (oracle-src-pinned src/C4Network2IO.cpp:395-404,1437-1477;
        // src/C4NetIO.cpp:2789-2810).
        let outbox = Arc::new(UdpSharedOutbox::default());
        let logs = (1..=3)
            .map(|port| {
                let log = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
                let sender = outbox
                    .register_route(SocketAddr::from(([127, 0, 0, 1], port)), 8, log.clone())
                    .expect("test routes are unique");
                (sender, log)
            })
            .collect::<Vec<_>>();
        let prefix = Arc::<[u8]>::from([crate::PACKET_LOG_START, 1]);
        let batch_payload = Arc::<[u8]>::from([crate::PACKET_LOG_START, 2]);
        let suffix = Arc::<[u8]>::from([crate::PACKET_LOG_START, 3]);
        assert!(outbox
            .enqueue(
                logs[0].0.lease.route,
                UdpPreparedPayload::Packet(prefix.clone()),
            )
            .is_ok());
        assert_eq!(
            outbox.enqueue_many(
                logs.iter().map(|(sender, _)| sender.lease.route),
                UdpPreparedPayload::Packet(batch_payload.clone()),
            ),
            logs.iter()
                .map(|(sender, _)| sender.lease.route)
                .collect::<Vec<_>>()
        );
        assert!(outbox
            .enqueue(
                logs[2].0.lease.route,
                UdpPreparedPayload::Packet(suffix.clone()),
            )
            .is_ok());

        assert!(matches!(
            outbox.take_action(),
            Some(UdpOutboxAction::Send { route, .. }) if route == logs[0].0.lease.route
        ));
        assert!(matches!(
            outbox.take_action(),
            Some(UdpOutboxAction::Send { route, .. }) if route == logs[0].0.lease.route
        ));

        // The selected target is already logged; the outbox still owns every
        // later target and queued suffix when the endpoint task is aborted.
        drop(UdpOutboxRunGuard(outbox.clone()));

        for (sender, _) in &logs {
            timeout(Duration::from_secs(1), sender.wait_drained())
                .await
                .expect("hub abort did not drain a batch target");
        }
        assert_eq!(
            logs[0]
                .1
                .lock()
                .unwrap()
                .create_post_mortem(41)
                .unwrap()
                .packets,
            vec![prefix.to_vec(), batch_payload.to_vec()]
        );
        assert_eq!(
            logs[1]
                .1
                .lock()
                .unwrap()
                .create_post_mortem(42)
                .unwrap()
                .packets,
            vec![batch_payload.to_vec()]
        );
        assert_eq!(
            logs[2]
                .1
                .lock()
                .unwrap()
                .create_post_mortem(43)
                .unwrap()
                .packets,
            vec![batch_payload.to_vec(), suffix.to_vec()]
        );
    }

    #[test]
    fn udp_outbox_many_is_one_wake_and_preserves_per_route_fifo_log_barrier() {
        // C4Network2Client selects the broadcast targets once, then
        // C4Network2IOConnection::Send logs immediately before each target's
        // physical send. Broadcast continues after an individual send fails
        // (oracle-src-pinned src/C4Network2Client.cpp:497-541;
        // src/C4Network2IO.cpp:378-404,1437-1477).
        let mut outbox = UdpLogicalOutbox::default();
        let log_a = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let log_b = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let route_a = outbox.register_route(log_a.clone());
        let route_b = outbox.register_route(log_b.clone());

        let body_1 = Arc::<[u8]>::from([crate::PACKET_LOG_START, 1]);
        let body_2 = Arc::<[u8]>::from([crate::PACKET_LOG_START, 2]);
        let body_3 = Arc::<[u8]>::from([crate::PACKET_LOG_START, 3]);
        let body_4 = Arc::<[u8]>::from([crate::PACKET_LOG_START, 4]);
        assert!(outbox.enqueue_single(route_a, body_1.clone()).is_ok());
        assert_eq!(
            outbox.enqueue_many([route_a, route_b], body_2.clone()),
            vec![route_a, route_b]
        );
        assert!(outbox.enqueue_single(route_a, body_3.clone()).is_ok());
        assert!(outbox.retire(route_a));
        assert_eq!(outbox.enqueue_single(route_a, body_4.clone()), Err(body_4));
        assert_eq!(outbox.wake_count(), 1, "one empty-to-ready wake");

        let mut trace = Vec::new();
        while let Some(action) = outbox.next_action() {
            match action {
                UdpOutboxAction::Send { route, payload, .. } => {
                    trace.push(("send", route, payload[1]));
                }
                UdpOutboxAction::Retired { route, .. } => trace.push(("retire", route, 0)),
                UdpOutboxAction::Failed { .. } => unreachable!("test queues valid packets"),
                UdpOutboxAction::Close { .. } => unreachable!("test does not close a route"),
            }
        }
        assert_eq!(
            trace,
            vec![
                ("send", route_a, 1),
                ("send", route_a, 2),
                ("send", route_b, 2),
                ("send", route_a, 3),
                ("retire", route_a, 0),
            ]
        );
        assert_eq!(
            log_a
                .lock()
                .unwrap()
                .create_post_mortem(11)
                .unwrap()
                .packets,
            vec![body_1.to_vec(), body_2.to_vec(), body_3.to_vec()]
        );
        assert_eq!(
            log_b
                .lock()
                .unwrap()
                .create_post_mortem(12)
                .unwrap()
                .packets,
            vec![body_2.to_vec()]
        );
    }

    #[test]
    fn udp_outbox_kth_failure_retains_its_suffix_and_continues_later_targets() {
        // C4Network2IO::Broadcast combines every per-connection Send result
        // with `&=` instead of short-circuiting, while a closed connection
        // keeps the packets already accepted for PostMortem replay
        // (oracle-src-pinned src/C4Network2IO.cpp:378-404,1437-1477).
        let mut outbox = UdpLogicalOutbox::default();
        let log_a = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let log_b = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let log_c = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let route_a = outbox.register_route(log_a);
        let route_b = outbox.register_route(log_b.clone());
        let route_c = outbox.register_route(log_c);
        let broadcast = Arc::<[u8]>::from([crate::PACKET_LOG_START, 1]);
        let b_suffix = Arc::<[u8]>::from([crate::PACKET_LOG_START, 2]);
        let c_suffix = Arc::<[u8]>::from([crate::PACKET_LOG_START, 3]);

        assert_eq!(
            outbox.enqueue_many([route_a, route_b, route_c], broadcast.clone()),
            vec![route_a, route_b, route_c]
        );
        assert!(outbox.enqueue_single(route_b, b_suffix.clone()).is_ok());
        assert!(outbox.enqueue_single(route_c, c_suffix.clone()).is_ok());

        assert!(matches!(
            outbox.next_action(),
            Some(UdpOutboxAction::Send { route, payload, .. })
                if route == route_a && payload == broadcast
        ));
        assert!(matches!(
            outbox.next_action(),
            Some(UdpOutboxAction::Send { route, payload, .. })
                if route == route_b && payload == broadcast
        ));
        assert!(outbox.fail_route(route_b));
        assert_eq!(
            outbox.enqueue_single(route_b, Arc::from([crate::PACKET_LOG_START, 4])),
            Err(Arc::from([crate::PACKET_LOG_START, 4]))
        );

        let mut remaining = Vec::new();
        while let Some(action) = outbox.next_action() {
            match action {
                UdpOutboxAction::Send { route, payload, .. } => {
                    remaining.push((route, payload[1]));
                }
                UdpOutboxAction::Failed { .. }
                | UdpOutboxAction::Retired { .. }
                | UdpOutboxAction::Close { .. } => {}
            }
        }
        assert_eq!(remaining, vec![(route_c, 1), (route_c, 3)]);
        assert_eq!(
            log_b
                .lock()
                .unwrap()
                .create_post_mortem(22)
                .unwrap()
                .packets,
            vec![broadcast.to_vec(), b_suffix.to_vec()]
        );
    }

    #[test]
    fn udp_outbox_failure_and_retirement_release_route_state_after_log_barrier() {
        // C4Network2IO removes a closed connection after preserving its
        // PostMortem packet suffix; later sends cannot re-enter that route
        // (oracle-src-pinned src/C4Network2IO.cpp:718-738,1274-1281,1437-1477).
        let mut outbox = UdpLogicalOutbox::default();
        let failed =
            outbox.register_route(Arc::new(Mutex::new(crate::RecoverablePacketLog::default())));
        let retired =
            outbox.register_route(Arc::new(Mutex::new(crate::RecoverablePacketLog::default())));
        assert_eq!(outbox.route_count(), 2);

        assert!(outbox.fail_route(failed));
        assert_eq!(outbox.route_count(), 1);
        assert!(outbox.retire(retired));
        assert!(matches!(
            outbox.next_action(),
            Some(UdpOutboxAction::Retired { route, .. }) if route == retired
        ));
        outbox.finish_retire(retired);
        assert_eq!(outbox.route_count(), 0);
    }

    #[test]
    fn udp_outbox_rejects_duplicate_route_promotion_for_one_peer_generation() {
        let outbox = Arc::new(UdpSharedOutbox::default());
        let peer = loopback();
        let first = outbox.register_route(
            peer,
            7,
            Arc::new(Mutex::new(crate::RecoverablePacketLog::default())),
        );
        let duplicate = outbox.register_route(
            peer,
            7,
            Arc::new(Mutex::new(crate::RecoverablePacketLog::default())),
        );

        assert!(first.is_some());
        assert!(duplicate.is_none());
        assert_eq!(outbox.route_count(), 1);
    }

    #[test]
    fn udp_outbox_close_error_fails_the_matching_physical_peer() {
        // C4Network2IO removes a failed connection before a same-address
        // replacement may be admitted (oracle-src-pinned
        // src/C4Network2IO.cpp:718-738,1274-1281).
        let peer = loopback();
        let (inbound, _inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 7,
                inbound,
                staged: VecDeque::new(),
                terminal: terminal.clone(),
            },
        )]);

        let events = resolve_udp_outbox_close_result(
            &mut peers,
            peer,
            7,
            Err(io::Error::other("close failed")),
        );

        assert!(events.is_none());
        assert!(!peers.contains_key(&peer));
        assert!(matches!(
            terminal.reason(),
            Some(PeerTerminal::Failed(error)) if error == "close failed"
        ));
    }

    #[test]
    fn stale_udp_outbox_close_error_does_not_fail_the_replacement_generation() {
        let peer = loopback();
        let (inbound, _inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 8,
                inbound,
                staged: VecDeque::new(),
                terminal: terminal.clone(),
            },
        )]);

        let events = resolve_udp_outbox_close_result(
            &mut peers,
            peer,
            7,
            Err(io::Error::other("stale close failed")),
        );

        assert!(events.is_none());
        assert!(peers.contains_key(&peer));
        assert!(!terminal.is_closed());
    }

    #[tokio::test]
    async fn udp_route_sender_last_drop_retires_and_releases_its_route_lease() {
        // C4Network2IOConnection destruction closes and unlinks its native
        // transport connection; a cancelled Rust dial must not leave a
        // selectable route behind (oracle-src-pinned
        // src/C4Network2IO.cpp:718-738,1274-1281).
        let outbox = Arc::new(UdpSharedOutbox::default());
        let sender = outbox
            .register_route(
                loopback(),
                7,
                Arc::new(Mutex::new(crate::RecoverablePacketLog::default())),
            )
            .expect("test route is unique");
        let route = sender.lease.route;
        let drained = sender.lease.drained.clone();
        let last_owner = sender.clone();
        drop(sender);
        assert_eq!(outbox.route_count(), 1);

        drop(last_owner);
        let action = timeout(Duration::from_secs(1), outbox.next_action())
            .await
            .expect("last sender drop did not queue route retirement");
        assert!(matches!(
            action,
            UdpOutboxAction::Retired { route: retired, .. } if retired == route
        ));
        outbox.finish_retire(route);
        timeout(Duration::from_secs(1), drained.wait())
            .await
            .expect("retirement did not release the route drain waiter");
        assert_eq!(outbox.route_count(), 0);
    }

    #[tokio::test]
    async fn udp_outbox_run_guard_rejects_senders_and_finishes_logs_on_task_abort() {
        // The hub owner may abort a task whose bounded command queue is full.
        // Accepted C4Network2 packets still belong to the connection's
        // PostMortem suffix (oracle-src-pinned src/C4Network2IO.cpp:1437-1477).
        let outbox = Arc::new(UdpSharedOutbox::default());
        let packet_log = Arc::new(Mutex::new(crate::RecoverablePacketLog::default()));
        let sender = outbox
            .register_route(loopback(), 9, packet_log.clone())
            .expect("test route is unique");
        let drained = sender.lease.drained.clone();
        let packet = ControlMessage::Status(crate::NetworkStatus {
            state: crate::NETWORK_STATE_LOBBY,
            control_mode: 1,
            target_tick: 5,
        });
        let expected = crate::transport::encode_complete_message(packet.clone()).unwrap();
        assert!(sender.try_send(packet).is_ok());

        drop(UdpOutboxRunGuard(outbox.clone()));

        timeout(Duration::from_secs(1), drained.wait())
            .await
            .expect("hub task abort did not release the route drain waiter");
        assert!(sender
            .try_send(ControlMessage::Ping(PingPacket {
                sent_at: 6,
                packet_counter: 7,
            }))
            .is_err());
        assert_eq!(outbox.route_count(), 0);
        assert_eq!(
            packet_log
                .lock()
                .unwrap()
                .create_post_mortem(91)
                .unwrap()
                .packets,
            vec![expected]
        );
    }

    #[test]
    fn peer_stream_adapter_accepts_cpp_frames_above_the_old_two_mib_cap() {
        const BODY_SIZE: usize = 2 * 1024 * 1024 + 1;

        let (commands, _commands_rx) = mpsc::channel(1);
        let (_inbound_tx, inbound) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream = ReliableUdpPeerStream::new(
            SocketAddr::from(([127, 0, 0, 1], 11_111)),
            1,
            commands,
            inbound,
            terminal,
        );

        stream.install_read_frame(vec![0x5a; BODY_SIZE]).unwrap();
        assert_eq!(stream.read_frame[0], TCP_FRAME_PREFIX);
        assert_eq!(
            &stream.read_frame[1..TCP_FRAME_HEADER_SIZE],
            &(BODY_SIZE as u32).to_ne_bytes()
        );
        assert_eq!(stream.read_frame.len(), TCP_FRAME_HEADER_SIZE + BODY_SIZE);

        stream.write_buffer.clear();
        stream.write_buffer.push(TCP_FRAME_PREFIX);
        stream
            .write_buffer
            .extend_from_slice(&(BODY_SIZE as u32).to_ne_bytes());
        assert_eq!(
            stream.buffered_frame_size().unwrap(),
            Some(TCP_FRAME_HEADER_SIZE + BODY_SIZE)
        );
    }

    async fn connected_pair() -> (
        ReliableUdpSessionHub,
        ReliableUdpSessionHub,
        ReliableUdpPeerStream,
        ReliableUdpPeerStream,
    ) {
        let outgoing_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let mut incoming_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let (outgoing, incoming) = tokio::join!(
            outgoing_hub.connect(incoming_hub.local_addr()),
            incoming_hub.accept(),
        );
        let outgoing = outgoing.unwrap();
        let incoming = incoming.unwrap();
        assert_eq!(outgoing.peer_addr(), incoming_hub.local_addr());
        assert_eq!(incoming.peer_addr(), outgoing_hub.local_addr());
        (outgoing_hub, incoming_hub, outgoing, incoming)
    }

    #[tokio::test]
    async fn hub_voice_lane_is_bounded_and_bypasses_peer_streams() {
        let (outgoing_hub, mut incoming_hub, _outgoing, incoming) = connected_pair().await;
        let mut voice_media = incoming_hub.take_voice_media_receiver();
        let incoming_route = incoming
            .outbox_registration()
            .promote(Arc::new(Mutex::new(crate::RecoverablePacketLog::default())))
            .await
            .unwrap();
        let expected = crate::voice::VoiceRouteCookie::from_bytes(
            [0x11; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
        );
        let forged = crate::voice::VoiceRouteCookie::from_bytes(
            [0x22; crate::voice::VOICE_ROUTE_COOKIE_BYTES],
        );
        incoming_route.set_voice_receive_cookie(expected);
        let frame = crate::voice::VoiceFrame::outbound(7, 11, 29, vec![0x5a; 164]).unwrap();
        let forged_packet = crate::voice::encode_authenticated_voice_packet(
            forged,
            &crate::voice::VoicePacket::Direct(frame.clone()),
        )
        .unwrap();
        let packet = crate::voice::encode_authenticated_voice_packet(
            expected,
            &crate::voice::VoicePacket::Direct(frame),
        )
        .unwrap();

        assert!(outgoing_hub
            .handle()
            .try_send_voice_media(incoming_hub.local_addr(), forged_packet));
        assert!(timeout(Duration::from_millis(30), voice_media.recv())
            .await
            .is_err());

        assert!(outgoing_hub
            .handle()
            .try_send_voice_media(incoming_hub.local_addr(), packet.clone()));
        assert_eq!(
            timeout(Duration::from_secs(2), voice_media.recv())
                .await
                .unwrap(),
            Some(ReliableUdpVoiceDatagram {
                peer: outgoing_hub.local_addr(),
                payload: packet,
            })
        );
    }

    #[test]
    fn hub_voice_queue_holds_at_most_160_milliseconds() {
        assert!(
            VOICE_MEDIA_CAPACITY * usize::from(crate::VOICE_FRAME_DURATION_MS) <= 160,
            "each bounded hub stage must hold little encoded speech"
        );
    }

    #[tokio::test]
    async fn netpuncher_route_reports_own_address_and_punches_without_a_game_peer() {
        async fn write_payload(stream: &mut ReliableUdpPeerStream, payload: &[u8]) {
            let mut frame = Vec::with_capacity(TCP_FRAME_HEADER_SIZE + payload.len());
            frame.push(TCP_FRAME_PREFIX);
            frame.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
            frame.extend_from_slice(payload);
            stream.write_all(&frame).await.unwrap();
            stream.flush().await.unwrap();
        }

        async fn read_payload(stream: &mut ReliableUdpPeerStream) -> Vec<u8> {
            let mut header = [0_u8; TCP_FRAME_HEADER_SIZE];
            stream.read_exact(&mut header).await.unwrap();
            assert_eq!(header[0], TCP_FRAME_PREFIX);
            let length = u32::from_ne_bytes(header[1..].try_into().unwrap()) as usize;
            let mut payload = vec![0_u8; length];
            stream.read_exact(&mut payload).await.unwrap();
            payload
        }

        let mut subject = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let subject_address = subject.local_addr();
        let subject_handle = subject.handle();
        let mut puncher_event_rx = subject.take_puncher_event_receiver();
        let mut puncher = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let puncher_address = puncher.local_addr();

        subject_handle
            .init_puncher(puncher_address, NetpuncherRole::Host)
            .await
            .unwrap();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(puncher_stream.peer_addr(), subject_address);
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Connected {
                family: NetpuncherAddressFamily::Ipv4,
                puncher_address,
                observed_address: subject_address,
            })
        );
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        subject_handle
            .send_puncher_packet(NetpuncherAddressFamily::Ipv4, NetpuncherPacket::IdRequest)
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), read_payload(&mut puncher_stream))
                .await
                .unwrap(),
            crate::encode_netpuncher_packet(&NetpuncherPacket::IdRequest)
        );

        let target = UdpSocket::bind(loopback()).await.unwrap();
        let target_address = target.local_addr().unwrap();
        write_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&NetpuncherPacket::ClientRequest {
                address: target_address,
            }),
        )
        .await;
        let mut wire = [0_u8; 64];
        let (length, source) = timeout(Duration::from_secs(2), target.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canonical_reliable_udp_peer_address(source), subject_address);
        assert_eq!(length, 9);
        assert_eq!(wire[0], 0x01);
        assert_eq!(&wire[5..9], &0_u32.to_ne_bytes());
        assert!(
            timeout(Duration::from_millis(50), target.recv_from(&mut wire))
                .await
                .is_err()
        );
        assert!(puncher_event_rx.try_recv().is_err());
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        let assigned = NetpuncherPacket::AssignId { id: 0x1122_3344 };
        write_payload(
            &mut puncher_stream,
            &crate::encode_netpuncher_packet(&assigned),
        )
        .await;
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Packet {
                family: NetpuncherAddressFamily::Ipv4,
                puncher_address,
                packet: assigned,
            })
        );

        subject_handle.close_puncher(puncher_address).await.unwrap();
        let mut closed = [0_u8; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_stream.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        drop(puncher_stream);

        subject_handle
            .init_puncher(puncher_address, NetpuncherRole::Host)
            .await
            .unwrap();
        let mut puncher_stream = timeout(Duration::from_secs(2), puncher.accept())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), puncher_event_rx.recv())
                .await
                .unwrap(),
            Some(NetpuncherIoEvent::Connected { .. })
        ));
        write_payload(&mut puncher_stream, &[0x51, 0x02, 0, 0, 0, 0]).await;
        assert_eq!(
            timeout(Duration::from_secs(2), puncher_stream.read(&mut closed))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert!(timeout(Duration::from_millis(50), subject.accept())
            .await
            .is_err());

        drop(puncher_stream);
        subject.shutdown().await.unwrap();
        puncher.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reliable_udp_session_stream_round_trips_control_transport_frames() {
        let (outgoing_hub, incoming_hub, outgoing, incoming) = connected_pair().await;
        let mut outgoing = ControlTransport::new(outgoing);
        let mut incoming = ControlTransport::new(incoming);
        let ping = PingPacket {
            sent_at: 0x1234_5678,
            packet_counter: 9,
        };

        outgoing
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Ping(ping)
        );

        let control = clonk_engine::ControlPacket::Message(clonk_engine::MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: 1,
            to_player: -1,
            message: clonk_engine::LegacyCString::from_bytes(vec![b'x'; 1_600]).unwrap(),
            by_client: 1,
        });
        let data = crate::encode_control_entry_payload(&control).unwrap();
        assert!(data.len() > crate::RELIABLE_UDP_DATA_PAYLOAD_LIMIT * 3);
        outgoing
            .send_message(ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data: data.clone(),
            })
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Packet {
                delivery: ControlDelivery::Direct,
                data,
            }
        );

        incoming
            .send_message(ControlMessage::Pong(ping))
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(2), outgoing.read_message())
                .await
                .unwrap()
                .unwrap(),
            ControlMessage::Pong(ping)
        );

        drop(outgoing);
        drop(incoming);
        outgoing_hub.shutdown().await.unwrap();
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn peer_stream_shutdown_sends_close_and_yields_remote_eof() {
        let (outgoing_hub, incoming_hub, mut outgoing, mut incoming) = connected_pair().await;
        outgoing.shutdown().await.unwrap();

        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(
            incoming
                .write_all(&[TCP_FRAME_PREFIX])
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );

        drop(outgoing);
        drop(incoming);
        outgoing_hub.shutdown().await.unwrap();
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn owned_peer_stream_keeps_its_hub_alive_and_closes_it_on_drop() {
        let outgoing_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let expected_local = outgoing_hub.local_addr();
        let mut incoming_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let incoming_address = incoming_hub.local_addr();
        let (outgoing, incoming) = tokio::join!(
            outgoing_hub.connect_owned(incoming_address),
            incoming_hub.accept(),
        );
        let outgoing = outgoing.unwrap();
        assert_eq!(outgoing.local_addr(), expected_local);
        assert_eq!(outgoing.peer_addr(), incoming_address);
        let mut outgoing = ControlTransport::new(outgoing);
        let mut incoming = ControlTransport::new(incoming.unwrap());
        let ping = PingPacket {
            sent_at: 33,
            packet_counter: 3,
        };

        outgoing
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();
        assert_eq!(
            incoming.read_message().await.unwrap(),
            ControlMessage::Ping(ping)
        );

        drop(outgoing);
        let mut incoming_stream = incoming.into_inner();
        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming_stream.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );

        drop(incoming_stream);
        incoming_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn stale_stream_close_and_encode_failure_do_not_close_a_reconnected_peer_generation() {
        let mut hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let hub_address = hub.local_addr();
        let raw_peer = UdpSocket::bind(loopback()).await.unwrap();
        let raw_peer_address = raw_peer.local_addr().unwrap();
        let mut wire = [0_u8; 512];

        raw_peer
            .send_to(
                &encode_reliable_udp_connect(&ReliableUdpConnect::unicast(0, hub_address)),
                hub_address,
            )
            .await
            .unwrap();
        let (length, source) = timeout(Duration::from_secs(2), raw_peer.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(source, hub_address);
        assert_eq!(
            reliable_udp_packet_kind(&wire[..length]),
            Some(ReliableUdpPacketKind::Connect)
        );
        raw_peer
            .send_to(
                &encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
                    packet_number: 0,
                    multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                    observed_address: hub_address,
                }),
                hub_address,
            )
            .await
            .unwrap();
        let stale_stream = timeout(Duration::from_secs(2), hub.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stale_stream.peer_addr(), raw_peer_address);
        let stale_outbound = stale_stream
            .outbox_registration()
            .promote(Arc::new(Mutex::new(crate::RecoverablePacketLog::default())))
            .await
            .unwrap();

        raw_peer
            .send_to(
                &encode_reliable_udp_connect(&ReliableUdpConnect::unicast(17, hub_address)),
                hub_address,
            )
            .await
            .unwrap();
        let replacement_stream = timeout(Duration::from_secs(2), hub.accept())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(replacement_stream.peer_addr(), raw_peer_address);

        // Queueing an encode failure and dropping the disconnected stream
        // target the old generation after the hub has installed the
        // replacement at the same socket address. Neither stale action may
        // close the replacement generation.
        assert!(stale_outbound
            .try_send(ControlMessage::ExecSync {
                control_tick: (i32::MAX as u32) + 1,
            })
            .is_ok());
        drop(stale_stream);
        timeout(Duration::from_secs(2), stale_outbound.wait_drained())
            .await
            .expect("stale encoding failure was not consumed");
        replacement_stream
            .bind_statistics_connection(31)
            .await
            .expect("stale encoding failure closed the replacement peer generation");
        let ping = PingPacket {
            sent_at: 0x1234_5678,
            packet_counter: 9,
        };
        let mut replacement = ControlTransport::new(replacement_stream);
        replacement
            .send_message(ControlMessage::Ping(ping))
            .await
            .unwrap();

        let mut expected_payload = vec![0x00];
        expected_payload.extend_from_slice(&ping.sent_at.to_ne_bytes());
        expected_payload.extend_from_slice(&ping.packet_counter.to_ne_bytes());
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let (length, source) = tokio::time::timeout_at(deadline, raw_peer.recv_from(&mut wire))
                .await
                .expect("replacement generation did not send its packet")
                .unwrap();
            assert_eq!(source, hub_address);
            match reliable_udp_packet_kind(&wire[..length]) {
                Some(ReliableUdpPacketKind::Data) => {
                    let fragment = decode_reliable_udp_data_fragment(&wire[..length]).unwrap();
                    assert_eq!(fragment.payload, expected_payload);
                    break;
                }
                Some(ReliableUdpPacketKind::Close) => {
                    panic!("stale stream closed the replacement peer generation")
                }
                _ => {}
            }
        }

        drop(replacement);
        hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn saturated_hub_command_queue_backpressures_the_stream_write() {
        let (commands, mut command_rx) = mpsc::channel(1);
        assert!(commands
            .try_send(HubCommand::Shutdown { completion: None })
            .is_ok());
        let (_inbound, inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream = ReliableUdpPeerStream::new(loopback(), 4, commands, inbound_rx, terminal);
        let mut frame = vec![TCP_FRAME_PREFIX];
        frame.extend_from_slice(&1_u32.to_ne_bytes());
        frame.push(0);

        let mut write = tokio::spawn(async move {
            let result = async {
                stream.write_all(&frame).await?;
                stream.flush().await
            }
            .await;
            (stream, result)
        });
        assert!(timeout(Duration::from_millis(20), &mut write)
            .await
            .is_err());
        assert!(matches!(
            command_rx.try_recv(),
            Ok(HubCommand::Shutdown { .. })
        ));
        let (stream, result) = timeout(Duration::from_secs(2), write)
            .await
            .unwrap()
            .unwrap();
        result.unwrap();
        assert!(matches!(
            command_rx.recv().await,
            Some(HubCommand::Send { payload, .. }) if payload == vec![0]
        ));
        drop(stream);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancelling_shutdown_while_the_command_queue_is_full_releases_the_socket() {
        let hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let local_addr = hub.local_addr();
        for generation in 0..HUB_COMMAND_CAPACITY as u64 {
            hub.commands
                .try_send(HubCommand::Close {
                    peer: SocketAddr::from(([127, 0, 0, 1], 30_000)),
                    generation,
                })
                .unwrap();
        }

        let mut shutdown = Box::pin(hub.shutdown());
        let waker = std::task::Waker::noop();
        let mut context = Context::from_waker(waker);
        assert!(matches!(
            shutdown.as_mut().poll(&mut context),
            Poll::Pending
        ));
        drop(shutdown);

        let rebound = timeout(Duration::from_secs(2), async {
            loop {
                match ReliableUdpSessionHub::bind(local_addr) {
                    Ok(hub) => break hub,
                    Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("unexpected UDP rebind failure: {error}"),
                }
            }
        })
        .await
        .expect("cancelled shutdown leaked the UDP task/socket");
        rebound.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn reliable_udp_inbound_delivery_is_lossless_beyond_the_retransmit_window() {
        const PACKET_COUNT: u32 = 10_001;

        // C++ inbound PacketList defaults to ~0u and synchronously drains each
        // complete ordered packet. Its separate outgoing retransmit window is
        // 10,000 packets, so inbound app delivery must remain lossless past it
        // (oracle-src-pinned src/C4NetIO.h:543-566;
        // src/C4NetIO.cpp:1916,2648-2652,3175-3199).
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let sink = UdpSocket::bind(loopback()).await.unwrap();
        let peer = sink.local_addr().unwrap();
        driver.connect(peer).await.unwrap();
        let (commands, _command_rx) = mpsc::channel(1);
        let (incoming, _incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let (inbound, inbound_rx) = mpsc::channel(PACKET_COUNT as usize);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream =
            ReliableUdpPeerStream::new(peer, 7, commands.clone(), inbound_rx, terminal.clone());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 7,
                inbound,
                staged: VecDeque::new(),
                terminal: terminal.clone(),
            },
        )]);
        for index in 0..PACKET_COUNT - 1 {
            peers[&peer]
                .inbound
                .send(PeerInbound::Packet(index.to_ne_bytes().to_vec()))
                .await
                .unwrap();
        }
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 8;
        dispatch_events(
            &mut driver,
            vec![ReliableUdpEvent::Packet {
                peer,
                payload: (PACKET_COUNT - 1).to_ne_bytes().to_vec(),
            }],
            &commands,
            &incoming,
            &puncher_events,
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        assert!(peers.contains_key(&peer));
        assert!(driver.core().peer_status(peer).is_some());
        assert!(!terminal.is_closed());
        for index in 0..PACKET_COUNT {
            let mut frame = [0_u8; TCP_FRAME_HEADER_SIZE + std::mem::size_of::<u32>()];
            stream.read_exact(&mut frame).await.unwrap();
            assert_eq!(frame[0], TCP_FRAME_PREFIX);
            assert_eq!(
                &frame[TCP_FRAME_HEADER_SIZE..],
                index.to_ne_bytes().as_slice()
            );
        }
    }

    #[tokio::test]
    async fn inbound_capacity_services_only_the_notified_peer() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let target_sink = UdpSocket::bind(loopback()).await.unwrap();
        let target = target_sink.local_addr().unwrap();
        let unaffected_sink = UdpSocket::bind(loopback()).await.unwrap();
        let unaffected = unaffected_sink.local_addr().unwrap();
        driver.connect(target).await.unwrap();
        driver.connect(unaffected).await.unwrap();

        let (commands, _command_rx) = mpsc::channel(1);
        let (incoming, _incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let (target_inbound, mut target_rx) = mpsc::channel(1);
        let (unaffected_inbound, mut unaffected_rx) = mpsc::channel(1);
        target_inbound
            .send(PeerInbound::Packet(vec![1]))
            .await
            .unwrap();
        unaffected_inbound
            .send(PeerInbound::Packet(vec![2]))
            .await
            .unwrap();
        assert!(matches!(
            target_rx.recv().await,
            Some(PeerInbound::Packet(payload)) if payload == vec![1]
        ));
        assert!(matches!(
            unaffected_rx.recv().await,
            Some(PeerInbound::Packet(payload)) if payload == vec![2]
        ));

        let target_terminal = Arc::new(PeerTerminalState::open());
        let unaffected_terminal = Arc::new(PeerTerminalState::open());
        let mut peers = BTreeMap::from([
            (
                target,
                ConnectedPeer {
                    generation: 1,
                    inbound: target_inbound,
                    staged: VecDeque::from([PeerInbound::Packet(vec![3])]),
                    terminal: target_terminal,
                },
            ),
            (
                unaffected,
                ConnectedPeer {
                    generation: 2,
                    inbound: unaffected_inbound,
                    staged: VecDeque::from([PeerInbound::Packet(vec![4])]),
                    terminal: unaffected_terminal,
                },
            ),
        ]);
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 3;

        service_peer_inbound(
            &mut driver,
            target,
            0,
            PeerInboundServiceSinks {
                commands: &commands,
                incoming: &incoming,
                puncher_events: &puncher_events,
            },
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;
        assert_eq!(peers[&target].staged.len(), 1);
        assert_eq!(peers[&unaffected].staged.len(), 1);

        service_peer_inbound(
            &mut driver,
            target,
            1,
            PeerInboundServiceSinks {
                commands: &commands,
                incoming: &incoming,
                puncher_events: &puncher_events,
            },
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        assert!(peers[&target].staged.is_empty());
        assert_eq!(peers[&unaffected].staged.len(), 1);
        assert!(matches!(
            target_rx.recv().await,
            Some(PeerInbound::Packet(payload)) if payload == vec![3]
        ));
        assert!(unaffected_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn saturated_command_queue_retains_capacity_wakeup_and_drains_staged_before_core() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let raw_peer = UdpSocket::bind(loopback()).await.unwrap();
        let peer = raw_peer.local_addr().unwrap();
        let driver_address = SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            driver.local_addr().unwrap().port(),
        );
        let mut wire = [0_u8; 512];

        driver.connect(peer).await.unwrap();
        timeout(Duration::from_secs(2), raw_peer.recv_from(&mut wire))
            .await
            .unwrap()
            .unwrap();
        raw_peer
            .send_to(
                &encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
                    packet_number: 0,
                    multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                    observed_address: driver_address,
                }),
                driver_address,
            )
            .await
            .unwrap();
        assert!(matches!(
            timeout(Duration::from_secs(2), driver.poll())
                .await
                .unwrap()
                .unwrap()
                .as_slice(),
            [ReliableUdpEvent::Connected { peer: connected, .. }] if *connected == peer
        ));

        let (commands, mut command_rx) = mpsc::channel(HUB_COMMAND_CAPACITY);
        for generation in 0..HUB_COMMAND_CAPACITY as u64 {
            commands
                .try_send(HubCommand::Close {
                    peer: SocketAddr::from(([127, 0, 0, 1], 30_000)),
                    generation,
                })
                .unwrap();
        }
        let (incoming, _incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        let (inbound, inbound_rx) = mpsc::channel(2);
        inbound.send(PeerInbound::Packet(vec![1])).await.unwrap();
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream =
            ReliableUdpPeerStream::new(peer, 7, commands.clone(), inbound_rx, terminal.clone());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 7,
                inbound,
                staged: VecDeque::from([PeerInbound::Packet(vec![2])]),
                terminal,
            },
        )]);
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 8;

        driver.set_peer_delivery_credit(peer, 0);
        let held_wire = encode_reliable_udp_data_fragments(0, &[3])
            .unwrap()
            .remove(0);
        raw_peer.send_to(&held_wire, driver_address).await.unwrap();
        assert!(timeout(Duration::from_secs(2), driver.poll())
            .await
            .unwrap()
            .unwrap()
            .is_empty());

        let mut first = [0_u8; TCP_FRAME_HEADER_SIZE + 1];
        stream.read_exact(&mut first).await.unwrap();
        assert_eq!(first[TCP_FRAME_HEADER_SIZE], 1);
        assert!(stream.inbound_capacity_pending);

        for _ in 0..HUB_COMMAND_CAPACITY {
            assert!(matches!(
                command_rx.try_recv(),
                Ok(HubCommand::Close { .. })
            ));
        }
        let read = tokio::spawn(async move {
            let mut staged = [0_u8; TCP_FRAME_HEADER_SIZE + 1];
            let mut core = [0_u8; TCP_FRAME_HEADER_SIZE + 1];
            stream.read_exact(&mut staged).await.unwrap();
            stream.read_exact(&mut core).await.unwrap();
            (stream, staged, core)
        });
        let (notified_peer, notified_generation) =
            match timeout(Duration::from_secs(2), command_rx.recv())
                .await
                .unwrap()
                .unwrap()
            {
                HubCommand::InboundCapacity { peer, generation } => (peer, generation),
                _ => panic!("expected retained inbound-capacity wakeup"),
            };
        service_peer_inbound(
            &mut driver,
            notified_peer,
            notified_generation,
            PeerInboundServiceSinks {
                commands: &commands,
                incoming: &incoming,
                puncher_events: &puncher_events,
            },
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        let (stream, staged, core) = timeout(Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(staged[TCP_FRAME_HEADER_SIZE], 2);
        assert_eq!(core[TCP_FRAME_HEADER_SIZE], 3);
        drop(stream);
    }

    #[tokio::test]
    async fn maintenance_closes_an_abandoned_peer_when_its_close_command_was_saturated() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let sink = UdpSocket::bind(loopback()).await.unwrap();
        let peer = sink.local_addr().unwrap();
        driver.connect(peer).await.unwrap();

        let (commands, _command_rx) = mpsc::channel(1);
        commands
            .try_send(HubCommand::Shutdown { completion: None })
            .unwrap();
        let (inbound, inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let stream = ReliableUdpPeerStream::new(peer, 11, commands, inbound_rx, terminal.clone());
        let mut peers = BTreeMap::from([(
            peer,
            ConnectedPeer {
                generation: 11,
                inbound,
                staged: VecDeque::new(),
                terminal,
            },
        )]);

        drop(stream);
        assert!(peers[&peer].inbound.is_closed());
        assert!(driver.core().peer_status(peer).is_some());
        close_abandoned_peers(&mut driver, &mut peers).await;
        assert!(!peers.contains_key(&peer));
        assert!(driver.core().peer_status(peer).is_none());
    }

    #[tokio::test]
    async fn terminal_signal_closes_writes_before_the_read_side_polls() {
        let (commands, _command_rx) = mpsc::channel(1);
        let (_inbound, inbound_rx) = mpsc::channel(1);
        let terminal = Arc::new(PeerTerminalState::open());
        let mut stream =
            ReliableUdpPeerStream::new(loopback(), 9, commands, inbound_rx, terminal.clone());
        terminal.close(PeerTerminal::Failed("terminal failure".to_string()));

        let error = stream.write_all(&[TCP_FRAME_PREFIX]).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn saturated_incoming_peer_queue_rejects_the_new_peer() {
        let mut driver = ReliableUdpSocketDriver::bind(loopback()).unwrap();
        let sink = UdpSocket::bind(loopback()).await.unwrap();
        let peer = sink.local_addr().unwrap();
        driver.connect(peer).await.unwrap();
        let (commands, _command_rx) = mpsc::channel(1);
        let (incoming, mut incoming_rx) = mpsc::channel(1);
        let (puncher_events, _puncher_event_rx) = mpsc::channel(PUNCHER_EVENT_CAPACITY);
        assert!(incoming.try_send(Err(io::Error::other("occupied"))).is_ok());
        let mut peers = BTreeMap::new();
        let mut pending_connects = BTreeMap::new();
        let mut next_peer_generation = 0;

        dispatch_events(
            &mut driver,
            vec![ReliableUdpEvent::Connected {
                peer,
                observed_address: None,
            }],
            &commands,
            &incoming,
            &puncher_events,
            &mut peers,
            &mut pending_connects,
            &mut next_peer_generation,
        )
        .await;

        assert!(peers.is_empty());
        assert!(driver.core().peer_status(peer).is_none());
        assert!(incoming_rx.recv().await.unwrap().is_err());
        assert!(incoming_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn one_hub_accepts_and_keeps_multiple_peer_streams_separate() {
        let first_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let second_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let mut host_hub = ReliableUdpSessionHub::bind(loopback()).unwrap();
        let host_address = host_hub.local_addr();

        let (first, second, accepted) = tokio::join!(
            first_hub.connect(host_address),
            second_hub.connect(host_address),
            async {
                let first = host_hub.accept().await.unwrap();
                let second = host_hub.accept().await.unwrap();
                [first, second]
            },
        );
        let mut first = ControlTransport::new(first.unwrap());
        let mut second = ControlTransport::new(second.unwrap());
        let mut accepted = accepted
            .into_iter()
            .map(|stream| (stream.peer_addr(), ControlTransport::new(stream)))
            .collect::<BTreeMap<_, _>>();
        let first_ping = PingPacket {
            sent_at: 11,
            packet_counter: 1,
        };
        let second_ping = PingPacket {
            sent_at: 22,
            packet_counter: 2,
        };

        first
            .send_message(ControlMessage::Ping(first_ping))
            .await
            .unwrap();
        second
            .send_message(ControlMessage::Ping(second_ping))
            .await
            .unwrap();
        assert_eq!(
            accepted
                .get_mut(&first_hub.local_addr())
                .unwrap()
                .read_message()
                .await
                .unwrap(),
            ControlMessage::Ping(first_ping)
        );
        assert_eq!(
            accepted
                .get_mut(&second_hub.local_addr())
                .unwrap()
                .read_message()
                .await
                .unwrap(),
            ControlMessage::Ping(second_ping)
        );

        drop(first);
        drop(second);
        drop(accepted);
        first_hub.shutdown().await.unwrap();
        second_hub.shutdown().await.unwrap();
        host_hub.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn hub_shutdown_closes_connected_remote_streams() {
        let (outgoing_hub, incoming_hub, outgoing, mut incoming) = connected_pair().await;
        let shutdown = tokio::spawn(outgoing_hub.shutdown());

        let mut byte = [0; 1];
        assert_eq!(
            timeout(Duration::from_secs(2), incoming.read(&mut byte))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        shutdown.await.unwrap().unwrap();

        drop(outgoing);
        drop(incoming);
        incoming_hub.shutdown().await.unwrap();
    }
}
