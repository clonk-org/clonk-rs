//! Packet-oriented C++ compatible reliable-UDP runtime.

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    net::{SocketAddr, SocketAddrV6},
    time::Duration,
};

use socket2::{Domain, Protocol, Socket, Type};
use thiserror::Error;
use tokio::{net::UdpSocket, time::Instant};

use crate::udp::{
    decode_reliable_udp_check, decode_reliable_udp_close, decode_reliable_udp_connect,
    decode_reliable_udp_connect_ok, decode_reliable_udp_data_fragment, encode_reliable_udp_check,
    encode_reliable_udp_close, encode_reliable_udp_connect, encode_reliable_udp_connect_ok,
    encode_reliable_udp_data_fragments, reliable_udp_packet_kind, ReliableUdpChannel,
    ReliableUdpClose, ReliableUdpConnect, ReliableUdpConnectOk, ReliableUdpEncodeError,
    ReliableUdpMulticastMode, ReliableUdpPacketKind, ReliableUdpReassembledPacket,
    ReliableUdpReceiveWindow,
};

pub const RELIABLE_UDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const RELIABLE_UDP_CONNECT_RETRIES: u8 = 5;
pub const RELIABLE_UDP_CHECK_INTERVAL: Duration = Duration::from_secs(1);
pub const RELIABLE_UDP_OUTGOING_PACKET_CAPACITY: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliableUdpPeerStatus {
    Connecting,
    Working,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliableUdpDisconnectReason {
    Closed,
    ConnectionTimeout,
    Starvation,
    ConnectionReset,
    ClosedByPeer,
    Reconnect,
}

impl ReliableUdpDisconnectReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::ConnectionTimeout => "connection timeout",
            Self::Starvation => "starvation",
            Self::ConnectionReset => "connection reset",
            Self::ClosedByPeer => "connection closed by peer",
            Self::Reconnect => "reconnect",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliableUdpDatagram {
    pub destination: SocketAddr,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReliableUdpEvent {
    Connected {
        peer: SocketAddr,
        observed_address: Option<SocketAddr>,
    },
    Packet {
        peer: SocketAddr,
        payload: Vec<u8>,
    },
    Disconnected {
        peer: SocketAddr,
        reason: ReliableUdpDisconnectReason,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReliableUdpStep {
    pub datagrams: Vec<ReliableUdpDatagram>,
    pub events: Vec<ReliableUdpEvent>,
}

impl ReliableUdpStep {
    fn append(&mut self, mut other: Self) {
        self.datagrams.append(&mut other.datagrams);
        self.events.append(&mut other.events);
    }
}

#[derive(Debug, Error)]
pub enum ReliableUdpRuntimeError {
    #[error("unknown reliable UDP peer {0}")]
    UnknownPeer(SocketAddr),
    #[error(transparent)]
    Encode(#[from] ReliableUdpEncodeError),
}

#[derive(Clone, Debug)]
struct ReliableUdpStoredPacket {
    first_packet_number: u32,
    fragments: Vec<Vec<u8>>,
}

impl ReliableUdpStoredPacket {
    fn fragment(&self, packet_number: u32) -> Option<&[u8]> {
        let offset = packet_number.checked_sub(self.first_packet_number)? as usize;
        self.fragments.get(offset).map(Vec::as_slice)
    }
}

#[derive(Clone, Debug)]
struct ReliableUdpPeer {
    address: SocketAddr,
    observed_address: Option<SocketAddr>,
    status: ReliableUdpPeerStatus,
    outgoing_packet_number: u32,
    outgoing_packets: VecDeque<ReliableUdpStoredPacket>,
    receive_window: ReliableUdpReceiveWindow,
    pending_packets: VecDeque<ReliableUdpReassembledPacket>,
    connect_deadline: Option<Duration>,
    connect_retries_remaining: u8,
    notify_connect_failure: bool,
}

impl ReliableUdpPeer {
    fn connecting(address: SocketAddr, now: Duration, notify_connect_failure: bool) -> Self {
        Self {
            address,
            observed_address: None,
            status: ReliableUdpPeerStatus::Connecting,
            outgoing_packet_number: 0,
            outgoing_packets: VecDeque::new(),
            receive_window: ReliableUdpReceiveWindow::new(0, 0),
            pending_packets: VecDeque::new(),
            connect_deadline: Some(now + RELIABLE_UDP_CONNECT_TIMEOUT),
            connect_retries_remaining: RELIABLE_UDP_CONNECT_RETRIES,
            notify_connect_failure,
        }
    }

    fn connect_datagram(&self) -> ReliableUdpDatagram {
        self.datagram(encode_reliable_udp_connect(&ReliableUdpConnect::unicast(
            self.outgoing_packet_number,
            self.address,
        )))
    }

    fn datagram(&self, payload: Vec<u8>) -> ReliableUdpDatagram {
        ReliableUdpDatagram {
            destination: reliable_udp_send_address(self.address),
            payload,
        }
    }

    fn mark_working(&mut self, observed_address: Option<SocketAddr>) -> ReliableUdpStep {
        let mut step = ReliableUdpStep::default();
        if let Some(observed_address) = observed_address {
            self.observed_address = Some(observed_address);
        }
        let was_working = self.status == ReliableUdpPeerStatus::Working;
        self.status = ReliableUdpPeerStatus::Working;
        self.connect_deadline = None;
        if !was_working {
            step.events.push(ReliableUdpEvent::Connected {
                peer: self.address,
                observed_address: self.observed_address,
            });
        }
        while let Some(packet) = self.pending_packets.pop_front() {
            step.events.push(ReliableUdpEvent::Packet {
                peer: self.address,
                payload: packet.payload,
            });
        }
        step
    }

    fn send_packet(&mut self, payload: &[u8]) -> Result<ReliableUdpStep, ReliableUdpEncodeError> {
        let fragments = encode_reliable_udp_data_fragments(self.outgoing_packet_number, payload)?;
        let first_packet_number = self.outgoing_packet_number;
        self.outgoing_packet_number = self
            .outgoing_packet_number
            .wrapping_add(fragments.len() as u32);
        self.outgoing_packets.push_back(ReliableUdpStoredPacket {
            first_packet_number,
            fragments: fragments.clone(),
        });
        while self.outgoing_packets.len() > RELIABLE_UDP_OUTGOING_PACKET_CAPACITY {
            self.outgoing_packets.pop_front();
        }
        let mut step = ReliableUdpStep::default();
        if self.status == ReliableUdpPeerStatus::Working {
            step.datagrams.extend(
                fragments
                    .into_iter()
                    .map(|fragment| self.datagram(fragment)),
            );
        }
        Ok(step)
    }

    fn plan_check(&self, force: bool) -> ReliableUdpStep {
        let check = self.receive_window.plan_check(self.outgoing_packet_number);
        if !force
            && check.missing_packet_numbers.is_empty()
            && check.missing_multicast_packet_numbers.is_empty()
        {
            return ReliableUdpStep::default();
        }
        let mut step = ReliableUdpStep::default();
        if let Ok(payload) = encode_reliable_udp_check(&check) {
            step.datagrams.push(self.datagram(payload));
        }
        step
    }

    fn receive(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Some(kind) = reliable_udp_packet_kind(wire) else {
            return ReliableUdpStep::default();
        };
        match kind {
            ReliableUdpPacketKind::Connect => self.receive_connect(wire),
            ReliableUdpPacketKind::ConnectOk => self.receive_connect_ok(wire),
            ReliableUdpPacketKind::Data => self.receive_data(wire),
            ReliableUdpPacketKind::Check => self.receive_check(wire),
            ReliableUdpPacketKind::Close => self.receive_close(wire),
            ReliableUdpPacketKind::Other(_) => ReliableUdpStep::default(),
        }
    }

    fn receive_connect(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(Some(connection)) = decode_reliable_udp_connect(wire) else {
            return ReliableUdpStep::default();
        };
        if self.observed_address.is_some_and(|observed_address| {
            canonical_reliable_udp_peer_address(observed_address)
                != canonical_reliable_udp_peer_address(connection.address)
        }) {
            return ReliableUdpStep::default();
        }
        let next_expected_packet_number = self
            .receive_window
            .plan_check(self.outgoing_packet_number)
            .next_expected_packet_number;
        let reconnect = self.status == ReliableUdpPeerStatus::Working
            && next_expected_packet_number != connection.packet_number;
        let mut step = ReliableUdpStep::default();
        if reconnect {
            step.append(self.disconnect(ReliableUdpDisconnectReason::Reconnect));
            self.status = ReliableUdpPeerStatus::Connecting;
            self.notify_connect_failure = false;
            step.datagrams.push(self.connect_datagram());
        }
        self.receive_window = ReliableUdpReceiveWindow::new(connection.packet_number, 0);
        self.pending_packets.clear();
        let response = ReliableUdpConnectOk {
            packet_number: self.outgoing_packet_number,
            multicast_mode: ReliableUdpMulticastMode::NoMulticast,
            observed_address: self.address,
        };
        step.datagrams
            .push(self.datagram(encode_reliable_udp_connect_ok(&response)));
        step.append(self.mark_working(Some(connection.address)));
        step
    }

    fn receive_connect_ok(&mut self, wire: &[u8]) -> ReliableUdpStep {
        if self.status != ReliableUdpPeerStatus::Connecting {
            return ReliableUdpStep::default();
        }
        let Ok(connection) = decode_reliable_udp_connect_ok(wire) else {
            return ReliableUdpStep::default();
        };
        self.receive_window
            .observe_packet_header(ReliableUdpChannel::Direct, connection.packet_number);
        self.mark_working(Some(connection.observed_address))
    }

    fn receive_data(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(fragment) = decode_reliable_udp_data_fragment(wire) else {
            return ReliableUdpStep::default();
        };
        self.receive_window
            .observe_packet_header(ReliableUdpChannel::Direct, fragment.packet_number);
        let mut step = if self.status == ReliableUdpPeerStatus::Working {
            self.plan_check(false)
        } else {
            ReliableUdpStep::default()
        };
        let Ok(packets) = self.receive_window.receive_direct_data_fragment(fragment) else {
            return step;
        };
        for packet in packets {
            if self.status == ReliableUdpPeerStatus::Working {
                step.events.push(ReliableUdpEvent::Packet {
                    peer: self.address,
                    payload: packet.payload,
                });
            } else {
                self.pending_packets.push_back(packet);
            }
        }
        step
    }

    fn receive_check(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(check) = decode_reliable_udp_check(wire) else {
            return ReliableUdpStep::default();
        };
        self.receive_window
            .observe_packet_header(ReliableUdpChannel::Direct, check.packet_number);
        let mut step = if self.status == ReliableUdpPeerStatus::Working {
            self.plan_check(false)
        } else {
            ReliableUdpStep::default()
        };
        while self
            .outgoing_packets
            .front()
            .is_some_and(|packet| packet.first_packet_number < check.next_expected_packet_number)
        {
            self.outgoing_packets.pop_front();
        }
        for packet_number in check.missing_packet_numbers {
            let fragment = self
                .outgoing_packets
                .iter()
                .find_map(|packet| packet.fragment(packet_number))
                .map(<[u8]>::to_vec);
            let Some(fragment) = fragment else {
                step.append(self.close(ReliableUdpDisconnectReason::Starvation));
                break;
            };
            step.datagrams.push(self.datagram(fragment));
        }
        step
    }

    fn receive_close(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(close) = decode_reliable_udp_close(wire) else {
            return ReliableUdpStep::default();
        };
        if self.observed_address.is_some_and(|observed_address| {
            canonical_reliable_udp_peer_address(observed_address)
                != canonical_reliable_udp_peer_address(close.address)
        }) {
            return ReliableUdpStep::default();
        }
        self.disconnect(ReliableUdpDisconnectReason::ClosedByPeer)
    }

    fn close(&mut self, reason: ReliableUdpDisconnectReason) -> ReliableUdpStep {
        if self.status == ReliableUdpPeerStatus::Closed {
            return ReliableUdpStep::default();
        }
        let mut step = ReliableUdpStep::default();
        step.datagrams
            .push(self.datagram(encode_reliable_udp_close(&ReliableUdpClose {
                packet_number: 0,
                address: self.address,
            })));
        step.append(self.disconnect(reason));
        step
    }

    fn disconnect(&mut self, reason: ReliableUdpDisconnectReason) -> ReliableUdpStep {
        if self.status == ReliableUdpPeerStatus::Closed {
            return ReliableUdpStep::default();
        }
        let notify = self.status == ReliableUdpPeerStatus::Working
            || (self.status == ReliableUdpPeerStatus::Connecting && self.notify_connect_failure);
        self.status = ReliableUdpPeerStatus::Closed;
        self.connect_deadline = None;
        ReliableUdpStep {
            datagrams: Vec::new(),
            events: notify
                .then_some(ReliableUdpEvent::Disconnected {
                    peer: self.address,
                    reason,
                })
                .into_iter()
                .collect(),
        }
    }
}

/// Deterministic reliable-UDP endpoint. Callers inject monotonic elapsed time
/// and execute the returned datagram effects on their socket of choice.
#[derive(Clone, Debug)]
pub struct ReliableUdpEndpointCore {
    peers: BTreeMap<SocketAddr, ReliableUdpPeer>,
    next_check_at: Duration,
}

impl ReliableUdpEndpointCore {
    pub fn new_at(now: Duration) -> Self {
        Self {
            peers: BTreeMap::new(),
            next_check_at: now + RELIABLE_UDP_CHECK_INTERVAL,
        }
    }

    pub fn connect_at(&mut self, peer: SocketAddr, now: Duration) -> ReliableUdpStep {
        let peer = canonical_reliable_udp_peer_address(peer);
        if self
            .peers
            .get(&peer)
            .is_some_and(|peer| peer.status != ReliableUdpPeerStatus::Closed)
        {
            return ReliableUdpStep::default();
        }
        let connection = ReliableUdpPeer::connecting(peer, now, true);
        let datagram = connection.connect_datagram();
        self.peers.insert(peer, connection);
        ReliableUdpStep {
            datagrams: vec![datagram],
            events: Vec::new(),
        }
    }

    pub fn send_packet(
        &mut self,
        peer: SocketAddr,
        payload: &[u8],
    ) -> Result<ReliableUdpStep, ReliableUdpRuntimeError> {
        let peer = canonical_reliable_udp_peer_address(peer);
        self.peers
            .get_mut(&peer)
            .ok_or(ReliableUdpRuntimeError::UnknownPeer(peer))?
            .send_packet(payload)
            .map_err(Into::into)
    }

    pub fn receive_at(
        &mut self,
        source: SocketAddr,
        wire: &[u8],
        now: Duration,
    ) -> ReliableUdpStep {
        let source = canonical_reliable_udp_peer_address(source);
        if self
            .peers
            .get(&source)
            .is_some_and(|peer| peer.status == ReliableUdpPeerStatus::Closed)
        {
            self.peers.remove(&source);
        }
        if let Some(peer) = self.peers.get_mut(&source) {
            let step = peer.receive(wire);
            if peer.status == ReliableUdpPeerStatus::Closed {
                self.peers.remove(&source);
            }
            return step;
        }
        if wire.first() != Some(&0x02) {
            return ReliableUdpStep::default();
        }
        // C++ creates a reciprocal connecting peer for an unknown Conn and
        // deliberately does not forward that first datagram into Peer::OnRecv.
        let connection = ReliableUdpPeer::connecting(source, now, false);
        let datagram = connection.connect_datagram();
        self.peers.insert(source, connection);
        ReliableUdpStep {
            datagrams: vec![datagram],
            events: Vec::new(),
        }
    }

    pub fn timer_at(&mut self, now: Duration) -> ReliableUdpStep {
        let mut step = ReliableUdpStep::default();
        if now >= self.next_check_at {
            for peer in self.peers.values() {
                if peer.status == ReliableUdpPeerStatus::Working {
                    step.append(peer.plan_check(true));
                }
            }
            self.next_check_at = now + RELIABLE_UDP_CHECK_INTERVAL;
        }
        let peers = self.peers.keys().copied().collect::<Vec<_>>();
        for address in peers {
            let Some(peer) = self.peers.get_mut(&address) else {
                continue;
            };
            if peer.status != ReliableUdpPeerStatus::Connecting
                || !peer
                    .connect_deadline
                    .is_some_and(|deadline| now >= deadline)
            {
                continue;
            }
            if peer.connect_retries_remaining != 0 {
                peer.connect_retries_remaining -= 1;
                peer.connect_deadline = Some(now + RELIABLE_UDP_CONNECT_TIMEOUT);
                step.datagrams.push(peer.connect_datagram());
            } else {
                step.append(peer.close(ReliableUdpDisconnectReason::ConnectionTimeout));
            }
        }
        self.peers
            .retain(|_, peer| peer.status != ReliableUdpPeerStatus::Closed);
        step
    }

    pub fn report_unreachable(&mut self, peer: SocketAddr) -> ReliableUdpStep {
        self.close_peer_with_reason(peer, ReliableUdpDisconnectReason::ConnectionReset)
    }

    /// Plans C++'s best-effort Close datagram and removes the local peer.
    pub fn close_peer(&mut self, peer: SocketAddr) -> ReliableUdpStep {
        self.close_peer_with_reason(peer, ReliableUdpDisconnectReason::Closed)
    }

    fn close_peer_with_reason(
        &mut self,
        peer: SocketAddr,
        reason: ReliableUdpDisconnectReason,
    ) -> ReliableUdpStep {
        let peer = canonical_reliable_udp_peer_address(peer);
        self.peers
            .remove(&peer)
            .map(|mut peer| peer.close(reason))
            .unwrap_or_default()
    }

    pub fn peer_status(&self, peer: SocketAddr) -> Option<ReliableUdpPeerStatus> {
        self.peers
            .get(&canonical_reliable_udp_peer_address(peer))
            .map(|peer| peer.status)
    }

    pub fn outgoing_packet_count(&self, peer: SocketAddr) -> Option<usize> {
        self.peers
            .get(&canonical_reliable_udp_peer_address(peer))
            .map(|peer| peer.outgoing_packets.len())
    }

    pub fn next_deadline(&self) -> Duration {
        self.peers
            .values()
            .filter_map(|peer| peer.connect_deadline)
            .fold(self.next_check_at, Duration::min)
    }
}

/// Normalizes mapped IPv4 sources so one logical peer cannot be duplicated.
pub fn canonical_reliable_udp_peer_address(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V6(address) => address
            .ip()
            .to_ipv4_mapped()
            .map(|ip| SocketAddr::new(ip.into(), address.port()))
            .unwrap_or(SocketAddr::V6(address)),
        address => address,
    }
}

/// C++ sends every unicast datagram through its dual-stack IPv6 socket.
pub fn reliable_udp_send_address(address: SocketAddr) -> SocketAddr {
    match canonical_reliable_udp_peer_address(address) {
        SocketAddr::V4(address) => SocketAddr::V6(SocketAddrV6::new(
            address.ip().to_ipv6_mapped(),
            address.port(),
            0,
            0,
        )),
        address => address,
    }
}

/// Tokio socket owner for `ReliableUdpEndpointCore`. `poll` serializes one
/// receive or timer transition; callers can keep polling in their own task.
pub struct ReliableUdpSocketDriver {
    socket: UdpSocket,
    core: ReliableUdpEndpointCore,
    started_at: Instant,
    receive_buffer: Vec<u8>,
    last_send_peer: Option<SocketAddr>,
}

pub(crate) enum ReliableUdpPollReady {
    Datagram(usize, SocketAddr),
    Timer,
    SocketError(io::Error),
}

impl ReliableUdpSocketDriver {
    pub fn bind(bind_address: SocketAddr) -> io::Result<Self> {
        tokio::runtime::Handle::try_current().map_err(|_| {
            io::Error::new(
                io::ErrorKind::Other,
                "reliable-UDP driver requires an entered Tokio runtime",
            )
        })?;
        let socket = Socket::new(Domain::IPV6, Type::DGRAM, Some(Protocol::UDP))?;
        socket.set_only_v6(false)?;
        socket.set_nonblocking(true)?;
        socket.bind(&reliable_udp_bind_address(bind_address).into())?;
        let socket = UdpSocket::from_std(socket.into())?;
        let started_at = Instant::now();
        Ok(Self {
            socket,
            core: ReliableUdpEndpointCore::new_at(Duration::ZERO),
            started_at,
            receive_buffer: vec![0; u16::MAX as usize + 1],
            last_send_peer: None,
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn core(&self) -> &ReliableUdpEndpointCore {
        &self.core
    }

    pub async fn connect(&mut self, peer: SocketAddr) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.connect_at(peer, self.elapsed());
        self.flush_step(step).await
    }

    pub async fn send_packet(
        &mut self,
        peer: SocketAddr,
        payload: &[u8],
    ) -> Result<Vec<ReliableUdpEvent>, ReliableUdpDriverError> {
        let step = self.core.send_packet(peer, payload)?;
        Ok(self.flush_step(step).await?)
    }

    /// Waits without mutating protocol state. This half of `poll` is safe to
    /// cancel from an outer `select!`: Tokio's UDP receive leaves the datagram
    /// queued unless it completes and this future returns it to the caller.
    pub(crate) async fn wait_ready(&mut self) -> ReliableUdpPollReady {
        let deadline = self.started_at + self.core.next_deadline();
        tokio::select! {
            result = self.socket.recv_from(&mut self.receive_buffer) => {
                match result {
                    Ok((length, source)) => ReliableUdpPollReady::Datagram(length, source),
                    Err(error) => ReliableUdpPollReady::SocketError(error),
                }
            }
            _ = tokio::time::sleep_until(deadline) => ReliableUdpPollReady::Timer,
        }
    }

    /// Applies one readiness transition and flushes every datagram/event it
    /// generated. Callers must not cancel this half after protocol state has
    /// advanced.
    pub(crate) async fn process_ready(
        &mut self,
        ready: ReliableUdpPollReady,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let now = self.elapsed();
        let received_datagram = matches!(ready, ReliableUdpPollReady::Datagram(_, _));
        let mut step = match ready {
            ReliableUdpPollReady::Datagram(length, source) => {
                self.core
                    .receive_at(source, &self.receive_buffer[..length], now)
            }
            ReliableUdpPollReady::Timer => self.core.timer_at(now),
            ReliableUdpPollReady::SocketError(error) => {
                if reliable_udp_unreachable_error(&error) {
                    if let Some(peer) = self.last_send_peer {
                        let step = self.core.report_unreachable(peer);
                        if !step.events.is_empty() {
                            return self.flush_step(step).await;
                        }
                    }
                }
                return Err(error);
            }
        };
        if received_datagram {
            step.append(self.core.timer_at(now));
        }
        self.flush_step(step).await
    }

    pub async fn poll(&mut self) -> io::Result<Vec<ReliableUdpEvent>> {
        let ready = self.wait_ready().await;
        self.process_ready(ready).await
    }

    pub async fn report_unreachable(
        &mut self,
        peer: SocketAddr,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.report_unreachable(peer);
        self.flush_step(step).await
    }

    /// Sends one best-effort Close datagram before reporting local teardown.
    pub async fn close_peer(&mut self, peer: SocketAddr) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.close_peer(peer);
        self.flush_step(step).await
    }

    fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.started_at)
    }

    async fn flush_step(&mut self, mut step: ReliableUdpStep) -> io::Result<Vec<ReliableUdpEvent>> {
        let mut first_send_error = None;
        for datagram in step.datagrams {
            let peer = canonical_reliable_udp_peer_address(datagram.destination);
            self.last_send_peer = Some(peer);
            if let Err(error) = self
                .socket
                .send_to(&datagram.payload, datagram.destination)
                .await
            {
                first_send_error.get_or_insert((error, peer));
            }
        }
        // Native control sends are best effort: a failed Check/ConnOK/Close
        // never suppresses a packet or lifecycle callback already produced by
        // the same receive transition.
        if let Some((error, peer)) = first_send_error {
            if reliable_udp_unreachable_error(&error) {
                let disconnected = self.core.report_unreachable(peer);
                step.events.extend(disconnected.events);
            }
            if step.events.is_empty() {
                return Err(error);
            }
        }
        Ok(step.events)
    }
}

#[derive(Debug, Error)]
pub enum ReliableUdpDriverError {
    #[error(transparent)]
    Runtime(#[from] ReliableUdpRuntimeError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

fn reliable_udp_bind_address(address: SocketAddr) -> SocketAddr {
    match address {
        SocketAddr::V4(address) => SocketAddr::V6(SocketAddrV6::new(
            address.ip().to_ipv6_mapped(),
            address.port(),
            0,
            0,
        )),
        SocketAddr::V6(address) => SocketAddr::V6(address),
    }
}

fn reliable_udp_unreachable_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
    )
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        net::{Ipv4Addr, Ipv6Addr},
    };

    use super::*;
    use crate::udp::{
        decode_reliable_udp_check, decode_reliable_udp_connect, decode_reliable_udp_data_fragment,
        encode_reliable_udp_check, ReliableUdpCheck,
    };

    fn address(last: u8, port: u16) -> SocketAddr {
        SocketAddr::new(Ipv4Addr::new(192, 0, 2, last).into(), port)
    }

    fn handshake_pair() -> (
        SocketAddr,
        SocketAddr,
        ReliableUdpEndpointCore,
        ReliableUdpEndpointCore,
    ) {
        let a_address = address(1, 11_111);
        let b_address = address(2, 22_222);
        let mut a = ReliableUdpEndpointCore::new_at(Duration::ZERO);
        let mut b = ReliableUdpEndpointCore::new_at(Duration::ZERO);

        let a_conn = a.connect_at(b_address, Duration::ZERO);
        assert_eq!(a_conn.datagrams.len(), 1);
        let b_conn = b.receive_at(a_address, &a_conn.datagrams[0].payload, Duration::ZERO);
        assert_eq!(
            b_conn.datagrams.len(),
            1,
            "unknown Conn sends reciprocal Conn only"
        );
        assert!(b_conn.events.is_empty());

        let a_connected = a.receive_at(b_address, &b_conn.datagrams[0].payload, Duration::ZERO);
        assert_eq!(a_connected.datagrams.len(), 1, "known Conn sends ConnOK");
        assert!(matches!(
            a_connected.events.as_slice(),
            [ReliableUdpEvent::Connected { peer, .. }] if *peer == b_address
        ));

        let b_connected =
            b.receive_at(a_address, &a_connected.datagrams[0].payload, Duration::ZERO);
        assert!(b_connected.datagrams.is_empty());
        assert!(matches!(
            b_connected.events.as_slice(),
            [ReliableUdpEvent::Connected { peer, .. }] if *peer == a_address
        ));
        assert_eq!(
            a.peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );
        assert_eq!(
            b.peer_status(a_address),
            Some(ReliableUdpPeerStatus::Working)
        );
        (a_address, b_address, a, b)
    }

    #[test]
    fn symmetric_conn_handshake_reaches_working_on_both_sides() {
        let (a_address, b_address, _, _) = handshake_pair();
        let connection = ReliableUdpConnect::unicast(17, b_address);
        let wire = encode_reliable_udp_connect(&connection);
        assert_eq!(decode_reliable_udp_connect(&wire), Ok(Some(connection)));

        let mut cpp_no_multicast_wire = wire.clone();
        cpp_no_multicast_wire[30] = 0xcd;
        assert_eq!(
            decode_reliable_udp_connect(&cpp_no_multicast_wire)
                .unwrap()
                .expect("valid Conn is not ignored")
                .multicast_address,
            None
        );
        assert_eq!(
            reliable_udp_send_address(b_address),
            SocketAddr::V6(SocketAddrV6::new(
                Ipv4Addr::new(192, 0, 2, 2).to_ipv6_mapped(),
                22_222,
                0,
                0,
            ))
        );
        assert_eq!(
            canonical_reliable_udp_peer_address(reliable_udp_send_address(a_address)),
            a_address
        );
    }

    #[test]
    fn changed_incoming_baseline_reconnects_symmetrically() {
        let (a_address, b_address, mut a, _) = handshake_pair();
        let restarted_conn =
            encode_reliable_udp_connect(&ReliableUdpConnect::unicast(7, a_address));

        let step = a.receive_at(b_address, &restarted_conn, Duration::ZERO);
        assert_eq!(step.datagrams.len(), 2);
        assert_eq!(
            reliable_udp_packet_kind(&step.datagrams[0].payload),
            Some(ReliableUdpPacketKind::Connect)
        );
        assert_eq!(
            reliable_udp_packet_kind(&step.datagrams[1].payload),
            Some(ReliableUdpPacketKind::ConnectOk)
        );
        assert_eq!(
            step.events,
            vec![
                ReliableUdpEvent::Disconnected {
                    peer: b_address,
                    reason: ReliableUdpDisconnectReason::Reconnect,
                },
                ReliableUdpEvent::Connected {
                    peer: b_address,
                    observed_address: Some(a_address),
                },
            ]
        );
        assert_eq!(
            a.peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );
    }

    #[test]
    fn connect_sends_five_retries_then_times_out() {
        let peer = address(2, 22_222);
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);
        let initial = endpoint.connect_at(peer, Duration::ZERO);
        assert_eq!(initial.datagrams.len(), 1);
        assert_eq!(
            reliable_udp_packet_kind(&initial.datagrams[0].payload),
            Some(ReliableUdpPacketKind::Connect)
        );

        for second in 1..=5 {
            let retry = endpoint.timer_at(Duration::from_secs(second));
            assert_eq!(retry.datagrams.len(), 1, "retry {second}");
            assert_eq!(
                reliable_udp_packet_kind(&retry.datagrams[0].payload),
                Some(ReliableUdpPacketKind::Connect)
            );
            assert!(retry.events.is_empty());
        }
        let timeout = endpoint.timer_at(Duration::from_secs(6));
        assert_eq!(
            reliable_udp_packet_kind(&timeout.datagrams[0].payload),
            Some(ReliableUdpPacketKind::Close)
        );
        assert_eq!(
            timeout.events,
            vec![ReliableUdpEvent::Disconnected {
                peer,
                reason: ReliableUdpDisconnectReason::ConnectionTimeout,
            }]
        );
        assert_eq!(endpoint.peer_status(peer), None);
    }

    #[test]
    fn reciprocal_connect_timeout_is_silent_and_closed_peer_can_reconnect() {
        let local = address(1, 11_111);
        let peer = address(2, 22_222);
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);
        let wire = encode_reliable_udp_connect(&ReliableUdpConnect::unicast(0, local));

        assert_eq!(
            endpoint
                .receive_at(peer, &wire, Duration::ZERO)
                .datagrams
                .len(),
            1
        );
        for second in 1..=5 {
            assert_eq!(
                endpoint
                    .timer_at(Duration::from_secs(second))
                    .datagrams
                    .len(),
                1
            );
        }
        let timeout = endpoint.timer_at(Duration::from_secs(6));
        assert_eq!(timeout.datagrams.len(), 1);
        assert!(timeout.events.is_empty());
        assert_eq!(endpoint.peer_status(peer), None);

        let reconnect = endpoint.connect_at(peer, Duration::from_secs(7));
        assert_eq!(reconnect.datagrams.len(), 1);
        assert_eq!(
            endpoint.peer_status(peer),
            Some(ReliableUdpPeerStatus::Connecting)
        );
    }

    #[test]
    fn working_peer_forces_one_empty_check_per_second() {
        let (_, b_address, mut a, _) = handshake_pair();
        assert!(a.timer_at(Duration::from_millis(999)).datagrams.is_empty());
        let first = a.timer_at(Duration::from_secs(1));
        assert_eq!(first.datagrams.len(), 1);
        let check = decode_reliable_udp_check(&first.datagrams[0].payload).unwrap();
        assert!(check.missing_packet_numbers.is_empty());
        assert_eq!(check.next_expected_packet_number, 0);
        assert!(a.timer_at(Duration::from_secs(1)).datagrams.is_empty());
        assert_eq!(a.timer_at(Duration::from_secs(2)).datagrams.len(), 1);
        assert_eq!(
            a.peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );
    }

    #[test]
    fn check_ack_clears_before_resend_and_missing_fragment_starves() {
        let (a_address, b_address, mut a, _) = handshake_pair();
        for byte in 0..3_u8 {
            a.send_packet(b_address, &[byte]).unwrap();
        }
        assert_eq!(a.outgoing_packet_count(b_address), Some(3));
        let ask_two = encode_reliable_udp_check(&ReliableUdpCheck {
            packet_number: 0,
            next_expected_packet_number: 2,
            next_expected_multicast_packet_number: 0,
            missing_packet_numbers: vec![2],
            missing_multicast_packet_numbers: Vec::new(),
        })
        .unwrap();
        let resend = a.receive_at(b_address, &ask_two, Duration::ZERO);
        assert_eq!(a.outgoing_packet_count(b_address), Some(1));
        assert_eq!(resend.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_data_fragment(&resend.datagrams[0].payload)
                .unwrap()
                .packet_number,
            2
        );

        let ask_cleared = encode_reliable_udp_check(&ReliableUdpCheck {
            packet_number: 0,
            next_expected_packet_number: 2,
            next_expected_multicast_packet_number: 0,
            missing_packet_numbers: vec![1],
            missing_multicast_packet_numbers: Vec::new(),
        })
        .unwrap();
        let starvation = a.receive_at(b_address, &ask_cleared, Duration::ZERO);
        assert_eq!(
            reliable_udp_packet_kind(&starvation.datagrams[0].payload),
            Some(ReliableUdpPacketKind::Close)
        );
        assert_eq!(
            starvation.events,
            vec![ReliableUdpEvent::Disconnected {
                peer: b_address,
                reason: ReliableUdpDisconnectReason::Starvation,
            }]
        );
        assert_eq!(
            canonical_reliable_udp_peer_address(reliable_udp_send_address(a_address)),
            a_address
        );
    }

    #[test]
    fn unreachable_callback_disconnects_and_close_for_another_address_is_ignored() {
        let (a_address, b_address, mut a, _) = handshake_pair();
        let mismatched_close = encode_reliable_udp_close(&ReliableUdpClose {
            packet_number: 0,
            address: address(3, 33_333),
        });
        assert_eq!(
            a.receive_at(b_address, &mismatched_close, Duration::ZERO),
            ReliableUdpStep::default()
        );
        assert_eq!(
            a.peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );

        let disconnected = a.report_unreachable(b_address);
        assert_eq!(disconnected.datagrams.len(), 1);
        assert_eq!(
            disconnected.events,
            vec![ReliableUdpEvent::Disconnected {
                peer: b_address,
                reason: ReliableUdpDisconnectReason::ConnectionReset,
            }]
        );
        assert_eq!(
            decode_reliable_udp_close(&disconnected.datagrams[0].payload)
                .unwrap()
                .address,
            b_address
        );
        assert_eq!(
            canonical_reliable_udp_peer_address(reliable_udp_send_address(a_address)),
            a_address
        );
    }

    #[test]
    fn explicit_close_emits_once_and_matching_peer_close_does_not_reply() {
        let (a_address, b_address, mut a, mut b) = handshake_pair();
        let incoming_close = encode_reliable_udp_close(&ReliableUdpClose {
            packet_number: 0,
            address: a_address,
        });
        for length in 0..incoming_close.len() {
            assert_eq!(
                a.receive_at(b_address, &incoming_close[..length], Duration::ZERO),
                ReliableUdpStep::default(),
                "short Close length {length}"
            );
        }
        assert_eq!(
            a.peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );

        let local_close = a.close_peer(b_address);
        assert_eq!(local_close.datagrams.len(), 1);
        assert_eq!(
            local_close.events,
            vec![ReliableUdpEvent::Disconnected {
                peer: b_address,
                reason: ReliableUdpDisconnectReason::Closed,
            }]
        );
        assert_eq!(a.peer_status(b_address), None);
        assert_eq!(
            decode_reliable_udp_close(&local_close.datagrams[0].payload).unwrap(),
            ReliableUdpClose {
                packet_number: 0,
                address: b_address,
            }
        );
        assert_eq!(a.close_peer(b_address), ReliableUdpStep::default());

        let remote_close =
            b.receive_at(a_address, &local_close.datagrams[0].payload, Duration::ZERO);
        assert!(remote_close.datagrams.is_empty());
        assert_eq!(
            remote_close.events,
            vec![ReliableUdpEvent::Disconnected {
                peer: a_address,
                reason: ReliableUdpDisconnectReason::ClosedByPeer,
            }]
        );
        assert_eq!(b.peer_status(a_address), None);
        let replay = b.receive_at(a_address, &local_close.datagrams[0].payload, Duration::ZERO);
        assert_eq!(replay, ReliableUdpStep::default());
    }

    #[test]
    fn evicted_backlog_request_closes_for_starvation() {
        let (_, b_address, mut a, _) = handshake_pair();
        for value in 0..=RELIABLE_UDP_OUTGOING_PACKET_CAPACITY {
            a.send_packet(b_address, &[value as u8]).unwrap();
        }
        assert_eq!(
            a.outgoing_packet_count(b_address),
            Some(RELIABLE_UDP_OUTGOING_PACKET_CAPACITY)
        );
        let ask_evicted = encode_reliable_udp_check(&ReliableUdpCheck {
            packet_number: 0,
            next_expected_packet_number: 0,
            next_expected_multicast_packet_number: 0,
            missing_packet_numbers: vec![0],
            missing_multicast_packet_numbers: Vec::new(),
        })
        .unwrap();
        let starvation = a.receive_at(b_address, &ask_evicted, Duration::ZERO);
        assert!(matches!(
            starvation.events.as_slice(),
            [ReliableUdpEvent::Disconnected {
                reason: ReliableUdpDisconnectReason::Starvation,
                ..
            }]
        ));
    }

    #[test]
    fn lossy_six_hundred_packet_exchange_recovers_in_order() {
        let (a_address, b_address, mut a, mut b) = handshake_pair();
        let mut network = VecDeque::new();
        let mut delivered = Vec::new();

        for number in 0..600_u32 {
            let step = a.send_packet(b_address, &number.to_ne_bytes()).unwrap();
            for datagram in step.datagrams {
                let packet_number = decode_reliable_udp_data_fragment(&datagram.payload)
                    .unwrap()
                    .packet_number;
                if packet_number % 37 != 0 && packet_number != 599 {
                    network.push_back((a_address, datagram));
                }
            }
            pump_network(
                a_address,
                b_address,
                &mut a,
                &mut b,
                &mut network,
                &mut delivered,
                Duration::ZERO,
            );
        }

        for datagram in a.timer_at(Duration::from_secs(1)).datagrams {
            network.push_back((a_address, datagram));
        }
        pump_network(
            a_address,
            b_address,
            &mut a,
            &mut b,
            &mut network,
            &mut delivered,
            Duration::from_secs(1),
        );
        for datagram in b.timer_at(Duration::from_secs(1)).datagrams {
            network.push_back((b_address, datagram));
        }
        pump_network(
            a_address,
            b_address,
            &mut a,
            &mut b,
            &mut network,
            &mut delivered,
            Duration::from_secs(1),
        );

        assert_eq!(
            delivered,
            (0..600_u32)
                .map(|value| value.to_ne_bytes().to_vec())
                .collect::<Vec<_>>()
        );
        assert_eq!(a.outgoing_packet_count(b_address), Some(0));
    }

    #[allow(clippy::too_many_arguments)]
    fn pump_network(
        a_address: SocketAddr,
        b_address: SocketAddr,
        a: &mut ReliableUdpEndpointCore,
        b: &mut ReliableUdpEndpointCore,
        network: &mut VecDeque<(SocketAddr, ReliableUdpDatagram)>,
        delivered: &mut Vec<Vec<u8>>,
        now: Duration,
    ) {
        while let Some((source, datagram)) = network.pop_front() {
            let destination = canonical_reliable_udp_peer_address(datagram.destination);
            let step = if destination == a_address {
                a.receive_at(source, &datagram.payload, now)
            } else {
                assert_eq!(destination, b_address);
                b.receive_at(source, &datagram.payload, now)
            };
            for event in step.events {
                if let ReliableUdpEvent::Packet { payload, .. } = event {
                    delivered.push(payload);
                }
            }
            let next_source = destination;
            network.extend(
                step.datagrams
                    .into_iter()
                    .map(|datagram| (next_source, datagram)),
            );
        }
    }

    #[tokio::test]
    async fn dual_stack_socket_driver_uses_mapped_ipv4_and_delivers_one_packet() {
        async fn next_events(driver: &mut ReliableUdpSocketDriver) -> Vec<ReliableUdpEvent> {
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let events = driver.poll().await.unwrap();
                    if !events.is_empty() {
                        return events;
                    }
                }
            })
            .await
            .unwrap()
        }

        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut a = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let mut b = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let a_address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), a.local_addr().unwrap().port());
        let b_address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), b.local_addr().unwrap().port());

        assert!(a.connect(b_address).await.unwrap().is_empty());
        assert!(b.poll().await.unwrap().is_empty());
        assert!(matches!(
            next_events(&mut a).await.as_slice(),
            [ReliableUdpEvent::Connected { .. }]
        ));
        assert!(matches!(
            next_events(&mut b).await.as_slice(),
            [ReliableUdpEvent::Connected { .. }]
        ));

        a.send_packet(b_address, b"hello over reliable udp")
            .await
            .unwrap();
        assert!(matches!(
            next_events(&mut b).await.as_slice(),
            [ReliableUdpEvent::Packet { payload, .. }] if payload == b"hello over reliable udp"
        ));
        assert_eq!(
            a.core().peer_status(b_address),
            Some(ReliableUdpPeerStatus::Working)
        );
        assert_eq!(
            b.core().peer_status(a_address),
            Some(ReliableUdpPeerStatus::Working)
        );

        assert_eq!(
            a.close_peer(b_address).await.unwrap(),
            vec![ReliableUdpEvent::Disconnected {
                peer: b_address,
                reason: ReliableUdpDisconnectReason::Closed,
            }]
        );
        assert_eq!(a.core().peer_status(b_address), None);
        assert!(a.close_peer(b_address).await.unwrap().is_empty());
        assert_eq!(
            next_events(&mut b).await,
            vec![ReliableUdpEvent::Disconnected {
                peer: a_address,
                reason: ReliableUdpDisconnectReason::ClosedByPeer,
            }]
        );
        assert_eq!(b.core().peer_status(a_address), None);
    }

    #[tokio::test]
    async fn socket_driver_close_emits_exactly_one_close_datagram() {
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut driver = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let spy_address = spy.local_addr().unwrap();
        let mut buffer = [0; 512];

        assert!(driver.connect(spy_address).await.unwrap().is_empty());
        let (connect_length, driver_address) =
            tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            reliable_udp_packet_kind(&buffer[..connect_length]),
            Some(ReliableUdpPacketKind::Connect)
        );

        let reciprocal_connect = encode_reliable_udp_connect(&ReliableUdpConnect::unicast(
            0,
            canonical_reliable_udp_peer_address(driver_address),
        ));
        spy.send_to(&reciprocal_connect, driver_address)
            .await
            .unwrap();
        assert!(matches!(
            driver.poll().await.unwrap().as_slice(),
            [ReliableUdpEvent::Connected { peer, .. }] if *peer == spy_address
        ));
        let (connect_ok_length, _) =
            tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            reliable_udp_packet_kind(&buffer[..connect_ok_length]),
            Some(ReliableUdpPacketKind::ConnectOk)
        );

        assert_eq!(
            driver.close_peer(spy_address).await.unwrap(),
            vec![ReliableUdpEvent::Disconnected {
                peer: spy_address,
                reason: ReliableUdpDisconnectReason::Closed,
            }]
        );
        assert_eq!(driver.core().peer_status(spy_address), None);
        let (close_length, _) =
            tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            &buffer[..close_length],
            encode_reliable_udp_close(&ReliableUdpClose {
                packet_number: 0,
                address: spy_address,
            })
        );

        assert!(driver.close_peer(spy_address).await.unwrap().is_empty());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), spy.recv_from(&mut buffer))
                .await
                .is_err(),
            "a repeated close must not emit another datagram"
        );
    }
}
