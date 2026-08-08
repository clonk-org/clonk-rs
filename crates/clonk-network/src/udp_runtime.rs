//! Packet-oriented C++ compatible reliable-UDP runtime.

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    net::{SocketAddr, SocketAddrV6},
    ops::Deref,
    time::Duration,
};

use socket2::{Protocol, Type};
use thiserror::Error;
use tokio::{net::UdpSocket, time::Instant};

use crate::puncher::{
    decode_netpuncher_packet, encode_netpuncher_packet, encode_netpuncher_punch,
    NetpuncherAddressFamily, NetpuncherIoEvent, NetpuncherPacket, NetpuncherRole,
};
use crate::udp::{
    decode_reliable_udp_add_address, decode_reliable_udp_check, decode_reliable_udp_close,
    decode_reliable_udp_connect, decode_reliable_udp_connect_ok, decode_reliable_udp_data_fragment,
    encode_reliable_udp_add_address, encode_reliable_udp_check, encode_reliable_udp_close,
    encode_reliable_udp_connect, encode_reliable_udp_connect_ok,
    encode_reliable_udp_data_fragments, encode_reliable_udp_ping_response,
    reliable_udp_packet_kind, ReliableUdpAddAddress, ReliableUdpChannel, ReliableUdpClose,
    ReliableUdpConnect, ReliableUdpConnectOk, ReliableUdpEncodeError, ReliableUdpMulticastMode,
    ReliableUdpPacketKind, ReliableUdpReassembledPacket, ReliableUdpReceiveWindow,
};

pub const RELIABLE_UDP_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const RELIABLE_UDP_CONNECT_RETRIES: u8 = 5;
pub const RELIABLE_UDP_CHECK_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(test)]
thread_local! {
    static NEXT_DEADLINE_PEER_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_next_deadline_peer_visits() {
    NEXT_DEADLINE_PEER_VISITS.set(0);
}

#[cfg(test)]
fn next_deadline_peer_visits() -> usize {
    NEXT_DEADLINE_PEER_VISITS.get()
}

pub const RELIABLE_UDP_OUTGOING_PACKET_CAPACITY: usize = 10_000;

/// Maximum wait for the driver's one-time Tokio writable-interest setup.
///
/// The interest remains registered for the socket's lifetime, so later sends
/// never recreate this timeout: they make one immediate `try_send_to` attempt
/// and drop on `WouldBlock`, matching C++ C4NetIO.cpp:1772-1790.
pub const RELIABLE_UDP_SEND_BUDGET: Duration = Duration::from_millis(2);

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
    Puncher(NetpuncherIoEvent),
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
    alternate_address: Option<SocketAddr>,
    observed_address: Option<SocketAddr>,
    status: ReliableUdpPeerStatus,
    outgoing_packet_number: u32,
    outgoing_packets: VecDeque<ReliableUdpStoredPacket>,
    receive_window: ReliableUdpReceiveWindow,
    pending_packets: VecDeque<ReliableUdpReassembledPacket>,
    delivery_credit: usize,
    connect_deadline: Option<Duration>,
    connect_retries_remaining: u8,
    notify_connect_failure: bool,
}

impl ReliableUdpPeer {
    fn connecting(address: SocketAddr, now: Duration, notify_connect_failure: bool) -> Self {
        Self {
            address,
            alternate_address: None,
            observed_address: None,
            status: ReliableUdpPeerStatus::Connecting,
            outgoing_packet_number: 0,
            outgoing_packets: VecDeque::new(),
            receive_window: ReliableUdpReceiveWindow::new(0, 0),
            pending_packets: VecDeque::new(),
            delivery_credit: usize::MAX,
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
        let reserved_capacity = self
            .delivery_credit
            .saturating_add(self.pending_packets.len());
        step.append(self.drain_deliverable(reserved_capacity));
        step
    }

    fn set_delivery_credit(&mut self, capacity: usize) {
        self.delivery_credit = capacity.saturating_sub(self.pending_packets.len());
    }

    fn drain_deliverable(&mut self, capacity: usize) -> ReliableUdpStep {
        self.set_delivery_credit(capacity);
        let mut step = ReliableUdpStep::default();
        if self.status != ReliableUdpPeerStatus::Working {
            return step;
        }
        let pending_limit = capacity.min(self.pending_packets.len());
        for _ in 0..pending_limit {
            let Some(packet) = self.pending_packets.pop_front() else {
                break;
            };
            step.events.push(ReliableUdpEvent::Packet {
                peer: self.address,
                payload: packet.payload,
            });
        }
        let packets = self
            .receive_window
            .take_complete_direct_packets(self.delivery_credit);
        self.delivery_credit = self.delivery_credit.saturating_sub(packets.len());
        step.events
            .extend(packets.into_iter().map(|packet| ReliableUdpEvent::Packet {
                peer: self.address,
                payload: packet.payload,
            }));
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

    fn plan_check(&mut self, force: bool, now: Duration) -> ReliableUdpStep {
        let Some(check) =
            self.receive_window
                .plan_check_at(self.outgoing_packet_number, now, force)
        else {
            return ReliableUdpStep::default();
        };
        let mut step = ReliableUdpStep::default();
        if let Ok(payload) = encode_reliable_udp_check(&check) {
            step.datagrams.push(self.datagram(payload));
        }
        step
    }

    fn receive(&mut self, wire: &[u8], now: Duration) -> ReliableUdpStep {
        let Some(kind) = reliable_udp_packet_kind(wire) else {
            return ReliableUdpStep::default();
        };
        // This runtime always negotiates MCM_NoMC. Ignore stray multicast
        // payload/check traffic before it can mutate either receive window or
        // apply multicast-group acknowledgements to this peer's direct queue.
        if wire[0] & 0x80 != 0
            && matches!(
                kind,
                ReliableUdpPacketKind::Data | ReliableUdpPacketKind::Check
            )
        {
            return ReliableUdpStep::default();
        }
        let mut step = ReliableUdpStep::default();
        if wire.len() >= 5 {
            let packet_number = u32::from_ne_bytes(
                wire[1..5]
                    .try_into()
                    .expect("five-byte reliable UDP header was length-checked"),
            );
            let channel = if wire[0] & 0x80 == 0 {
                ReliableUdpChannel::Direct
            } else {
                ReliableUdpChannel::Multicast
            };
            self.receive_window
                .observe_packet_header(channel, packet_number);
            if self.status == ReliableUdpPeerStatus::Working {
                step.append(self.plan_check(false, now));
            }
        }
        let handled = match kind {
            ReliableUdpPacketKind::Ping
            | ReliableUdpPacketKind::Test
            | ReliableUdpPacketKind::AddAddress
            | ReliableUdpPacketKind::Other(_) => ReliableUdpStep::default(),
            ReliableUdpPacketKind::Connect => self.receive_connect(wire),
            ReliableUdpPacketKind::ConnectOk => self.receive_connect_ok(wire),
            ReliableUdpPacketKind::Data => self.receive_data(wire),
            ReliableUdpPacketKind::Check => self.receive_check(wire),
            ReliableUdpPacketKind::Close => self.receive_close(wire),
        };
        step.append(handled);
        step
    }

    fn receive_connect(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(Some(connection)) = decode_reliable_udp_connect(wire) else {
            return ReliableUdpStep::default();
        };
        if let Some(observed_address) = self.observed_address.filter(|observed_address| {
            canonical_reliable_udp_peer_address(*observed_address)
                != canonical_reliable_udp_peer_address(connection.address)
        }) {
            if wire[0] & 0x80 != 0 {
                return ReliableUdpStep::default();
            }
            let packet = ReliableUdpAddAddress {
                packet_number: self.outgoing_packet_number,
                address: observed_address,
                new_address: connection.address,
            };
            return ReliableUdpStep {
                datagrams: vec![self.datagram(encode_reliable_udp_add_address(&packet))],
                events: Vec::new(),
            };
        }
        let next_expected_packet_number = self.receive_window.next_expected_packet_number();
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
        self.mark_working(Some(connection.observed_address))
    }

    fn receive_data(&mut self, wire: &[u8]) -> ReliableUdpStep {
        let Ok(fragment) = decode_reliable_udp_data_fragment(wire) else {
            return ReliableUdpStep::default();
        };
        let mut step = ReliableUdpStep::default();
        let Ok(packets) = self
            .receive_window
            .receive_direct_data_fragment_with_limit(fragment, self.delivery_credit)
        else {
            return step;
        };
        self.delivery_credit = self.delivery_credit.saturating_sub(packets.len());
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
        let mut step = ReliableUdpStep::default();
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
    next_connect_deadline: Option<Duration>,
    topology_epoch: u64,
}

impl ReliableUdpEndpointCore {
    /// Extra physical sends for `wire` beyond the original.
    pub fn redundant_copies_for(&self, _peer: SocketAddr, wire: &[u8]) -> usize {
        crate::udp::reliable_udp_redundant_copies(wire)
    }

    pub fn new_at(now: Duration) -> Self {
        Self {
            peers: BTreeMap::new(),
            next_check_at: now + RELIABLE_UDP_CHECK_INTERVAL,
            next_connect_deadline: None,
            topology_epoch: 0,
        }
    }

    fn insert_peer(&mut self, address: SocketAddr, peer: ReliableUdpPeer) {
        let connect_deadline = peer.connect_deadline;
        if self.peers.insert(address, peer).is_none() {
            self.topology_epoch = self.topology_epoch.wrapping_add(1);
            if let Some(deadline) = connect_deadline {
                self.next_connect_deadline = Some(
                    self.next_connect_deadline
                        .map_or(deadline, |current| current.min(deadline)),
                );
            }
        } else {
            self.refresh_next_connect_deadline();
        }
    }

    fn remove_peer(&mut self, address: &SocketAddr) -> Option<ReliableUdpPeer> {
        let removed = self.peers.remove(address);
        if let Some(peer) = &removed {
            self.topology_epoch = self.topology_epoch.wrapping_add(1);
            if peer
                .connect_deadline
                .is_some_and(|deadline| Some(deadline) == self.next_connect_deadline)
            {
                self.refresh_next_connect_deadline();
            }
        }
        removed
    }

    fn refresh_next_connect_deadline(&mut self) {
        self.next_connect_deadline = self
            .peers
            .values()
            .inspect(|_| {
                #[cfg(test)]
                NEXT_DEADLINE_PEER_VISITS.set(NEXT_DEADLINE_PEER_VISITS.get() + 1);
            })
            .filter_map(|peer| peer.connect_deadline)
            .min();
    }

    fn topology_epoch(&self) -> u64 {
        self.topology_epoch
    }

    pub fn connect_at(&mut self, peer: SocketAddr, now: Duration) -> ReliableUdpStep {
        let peer = canonical_reliable_udp_peer_address(peer);
        if self.peer_key(peer).is_some() {
            return ReliableUdpStep::default();
        }
        let connection = ReliableUdpPeer::connecting(peer, now, true);
        let datagram = connection.connect_datagram();
        self.insert_peer(peer, connection);
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
        let peer_key = self
            .peer_key(peer)
            .ok_or(ReliableUdpRuntimeError::UnknownPeer(peer))?;
        self.peers
            .get_mut(&peer_key)
            .expect("resolved reliable-UDP peer exists")
            .send_packet(payload)
            .map_err(Into::into)
    }

    pub(crate) fn set_peer_delivery_credit(&mut self, peer: SocketAddr, capacity: usize) {
        let peer = canonical_reliable_udp_peer_address(peer);
        let Some(peer_key) = self.peer_key(peer) else {
            return;
        };
        self.peers
            .get_mut(&peer_key)
            .expect("resolved reliable-UDP peer exists")
            .set_delivery_credit(capacity);
    }

    pub(crate) fn drain_peer_packets(
        &mut self,
        peer: SocketAddr,
        capacity: usize,
    ) -> ReliableUdpStep {
        let peer = canonical_reliable_udp_peer_address(peer);
        let Some(peer_key) = self.peer_key(peer) else {
            return ReliableUdpStep::default();
        };
        self.peers
            .get_mut(&peer_key)
            .expect("resolved reliable-UDP peer exists")
            .drain_deliverable(capacity)
    }

    pub fn receive_at(
        &mut self,
        source: SocketAddr,
        wire: &[u8],
        now: Duration,
    ) -> ReliableUdpStep {
        let source = canonical_reliable_udp_peer_address(source);
        let Some(kind) = reliable_udp_packet_kind(wire) else {
            return ReliableUdpStep::default();
        };
        // C++ filters loopback-test and hole-punch datagrams by their masked
        // status byte before looking up or mutating any peer.
        if kind == ReliableUdpPacketKind::Test {
            return ReliableUdpStep::default();
        }
        if let Some(peer_key) = self.peer_key(source) {
            // AddAddr is intercepted at the endpoint because it may merge two
            // peer objects. The multicast-bit form is not an AddAddr packet in
            // C++ even though generic kind inspection masks that bit.
            if kind == ReliableUdpPacketKind::AddAddress && wire[0] & 0x80 == 0 {
                return self.receive_add_address(source, wire);
            }
            let (step, deadline_changed, closed) = {
                let peer = self
                    .peers
                    .get_mut(&peer_key)
                    .expect("resolved reliable-UDP peer exists");
                let previous_deadline = peer.connect_deadline;
                let step = peer.receive(wire, now);
                (
                    step,
                    peer.connect_deadline != previous_deadline,
                    peer.status == ReliableUdpPeerStatus::Closed,
                )
            };
            if deadline_changed {
                self.refresh_next_connect_deadline();
            }
            if closed {
                self.remove_peer(&peer_key);
            }
            return step;
        }
        if kind == ReliableUdpPacketKind::Ping {
            // Native sends a flagged response to its multicast endpoint. This
            // unicast-only runtime retains the exact header while replying to
            // the source endpoint through the same dual-stack socket.
            return ReliableUdpStep {
                datagrams: vec![ReliableUdpDatagram {
                    destination: reliable_udp_send_address(source),
                    payload: encode_reliable_udp_ping_response(wire[0]),
                }],
                events: Vec::new(),
            };
        }
        if wire.first() != Some(&0x02) {
            return ReliableUdpStep::default();
        }
        // C++ creates a reciprocal connecting peer for an unknown Conn and
        // deliberately does not forward that first datagram into Peer::OnRecv.
        let connection = ReliableUdpPeer::connecting(source, now, false);
        let datagram = connection.connect_datagram();
        self.insert_peer(source, connection);
        ReliableUdpStep {
            datagrams: vec![datagram],
            events: Vec::new(),
        }
    }

    fn receive_add_address(&mut self, source: SocketAddr, wire: &[u8]) -> ReliableUdpStep {
        let Ok(packet) = decode_reliable_udp_add_address(wire) else {
            return ReliableUdpStep::default();
        };
        let address = canonical_reliable_udp_peer_address(packet.address);
        let new_address = canonical_reliable_udp_peer_address(packet.new_address);
        if source != address && source != new_address {
            return ReliableUdpStep::default();
        }
        let Some(peer_key) = self.peer_key(address) else {
            return ReliableUdpStep::default();
        };
        let Some(duplicate_key) = self.peer_key(new_address) else {
            return ReliableUdpStep::default();
        };
        if peer_key == duplicate_key {
            return ReliableUdpStep::default();
        }
        self.peers
            .get_mut(&peer_key)
            .expect("resolved reliable-UDP peer exists")
            .alternate_address = Some(new_address);
        self.remove_peer(&duplicate_key)
            .map(|mut duplicate| duplicate.close(ReliableUdpDisconnectReason::Closed))
            .unwrap_or_default()
    }

    pub fn timer_at(&mut self, now: Duration) -> ReliableUdpStep {
        let check_due = now >= self.next_check_at;
        let connect_due = self
            .next_connect_deadline
            .is_some_and(|deadline| now >= deadline);
        if !check_due && !connect_due {
            return ReliableUdpStep::default();
        }

        let mut step = ReliableUdpStep::default();
        if check_due {
            for peer in self.peers.values_mut() {
                if peer.status == ReliableUdpPeerStatus::Working {
                    step.append(peer.plan_check(true, now));
                }
            }
            self.next_check_at = now + RELIABLE_UDP_CHECK_INTERVAL;
        }
        let due_peers = if connect_due {
            self.peers
                .iter()
                .filter_map(|(address, peer)| {
                    (peer.status == ReliableUdpPeerStatus::Connecting
                        && peer
                            .connect_deadline
                            .is_some_and(|deadline| now >= deadline))
                    .then_some(*address)
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        for address in due_peers {
            let Some(peer) = self.peers.get_mut(&address) else {
                continue;
            };
            if peer.connect_retries_remaining != 0 {
                peer.connect_retries_remaining -= 1;
                peer.connect_deadline = Some(now + RELIABLE_UDP_CONNECT_TIMEOUT);
                step.datagrams.push(peer.connect_datagram());
            } else {
                step.append(peer.close(ReliableUdpDisconnectReason::ConnectionTimeout));
            }
            if peer.status == ReliableUdpPeerStatus::Closed {
                self.remove_peer(&address);
            }
        }
        if connect_due {
            self.refresh_next_connect_deadline();
        }
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
        let Some(peer_key) = self.peer_key(peer) else {
            return ReliableUdpStep::default();
        };
        self.remove_peer(&peer_key)
            .map(|mut peer| peer.close(reason))
            .unwrap_or_default()
    }

    pub fn peer_status(&self, peer: SocketAddr) -> Option<ReliableUdpPeerStatus> {
        let peer_key = self.peer_key(peer)?;
        self.peers.get(&peer_key).map(|peer| peer.status)
    }

    pub fn outgoing_packet_count(&self, peer: SocketAddr) -> Option<usize> {
        let peer_key = self.peer_key(peer)?;
        self.peers
            .get(&peer_key)
            .map(|peer| peer.outgoing_packets.len())
    }

    fn peer_key(&self, address: SocketAddr) -> Option<SocketAddr> {
        let address = canonical_reliable_udp_peer_address(address);
        if self
            .peers
            .get(&address)
            .is_some_and(|peer| peer.status != ReliableUdpPeerStatus::Closed)
        {
            return Some(address);
        }
        self.peers.iter().find_map(|(peer_key, peer)| {
            (peer.status != ReliableUdpPeerStatus::Closed
                && peer.alternate_address == Some(address))
            .then_some(*peer_key)
        })
    }

    pub fn next_deadline(&self) -> Duration {
        self.next_connect_deadline
            .map_or(self.next_check_at, |deadline| {
                deadline.min(self.next_check_at)
            })
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
    /// What the bound socket can actually address. A host without an IPv6
    /// stack degrades to IPv4 rather than losing the UDP transport entirely.
    family: crate::dual_stack::SocketFamily,
    core: ReliableUdpEndpointCore,
    punchers: ReliableUdpPuncherRoutes,
    statistics: Option<crate::NetworkIoStatistics>,
    peer_statistics: BTreeMap<SocketAddr, ReliableUdpPeerStatistics>,
    statistics_topology_epoch: u64,
    started_at: Instant,
    receive_buffer: Vec<u8>,
    protocol_timer: Option<std::pin::Pin<Box<tokio::time::Sleep>>>,
    last_send: Option<ReliableUdpLastSend>,
    socket_writability_established: bool,
    #[cfg(test)]
    protocol_timer_arms: usize,
    #[cfg(test)]
    socket_writability_establishments: usize,
    #[cfg(test)]
    force_next_planned_send_would_block: bool,
}

#[derive(Debug)]
struct AttachedUdpConnectionStatistics(crate::ConnectionStatisticsRecorder);

impl Deref for AttachedUdpConnectionStatistics {
    type Target = crate::ConnectionStatisticsRecorder;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for AttachedUdpConnectionStatistics {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[derive(Debug)]
struct ReliableUdpPeerStatistics {
    sampled_at_ms: u64,
    pending_input_bytes: u64,
    pending_output_bytes: u64,
    recorder: Option<AttachedUdpConnectionStatistics>,
}

impl ReliableUdpPeerStatistics {
    fn new(sampled_at_ms: u64) -> Self {
        Self {
            sampled_at_ms,
            pending_input_bytes: 0,
            pending_output_bytes: 0,
            recorder: None,
        }
    }

    fn bind(&mut self, recorder: crate::ConnectionStatisticsRecorder, sampled_at_ms: u64) {
        self.advance_sample(sampled_at_ms);
        self.recorder = Some(AttachedUdpConnectionStatistics(recorder));
    }

    fn record_input(&mut self, payload_bytes: usize, sampled_at_ms: u64) {
        let sampled_at_ms = match &self.recorder {
            Some(recorder) => {
                let Some(sampled_at_ms) = recorder.record_input_at_current_sample(payload_bytes)
                else {
                    return;
                };
                sampled_at_ms
            }
            None => sampled_at_ms,
        };
        self.advance_sample(sampled_at_ms);
        self.pending_input_bytes = self
            .pending_input_bytes
            .saturating_add(udp_accounted_bytes(payload_bytes));
    }

    fn record_output(&mut self, payload_bytes: usize, sampled_at_ms: u64) {
        let sampled_at_ms = match &self.recorder {
            Some(recorder) => {
                let Some(sampled_at_ms) = recorder.record_output_at_current_sample(payload_bytes)
                else {
                    return;
                };
                sampled_at_ms
            }
            None => sampled_at_ms,
        };
        self.advance_sample(sampled_at_ms);
        self.pending_output_bytes = self
            .pending_output_bytes
            .saturating_add(udp_accounted_bytes(payload_bytes));
    }

    fn unbind(&mut self) {
        self.recorder = None;
    }

    fn advance_sample(&mut self, sampled_at_ms: u64) {
        if self.sampled_at_ms != sampled_at_ms {
            self.sampled_at_ms = sampled_at_ms;
            self.pending_input_bytes = 0;
            self.pending_output_bytes = 0;
        }
    }
}

fn udp_accounted_bytes(payload_bytes: usize) -> u64 {
    u64::try_from(payload_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(crate::UDP_STATISTICS_HEADER_BYTES)
}

#[derive(Clone, Copy, Debug)]
struct ReliableUdpPuncherRoute {
    address: SocketAddr,
    role: NetpuncherRole,
}

#[derive(Clone, Copy, Debug, Default)]
struct ReliableUdpPuncherRoutes {
    ipv4: Option<ReliableUdpPuncherRoute>,
    ipv6: Option<ReliableUdpPuncherRoute>,
}

impl ReliableUdpPuncherRoutes {
    fn route(self, family: NetpuncherAddressFamily) -> Option<ReliableUdpPuncherRoute> {
        match family {
            NetpuncherAddressFamily::Ipv4 => self.ipv4,
            NetpuncherAddressFamily::Ipv6 => self.ipv6,
        }
    }

    fn set(&mut self, family: NetpuncherAddressFamily, route: ReliableUdpPuncherRoute) {
        match family {
            NetpuncherAddressFamily::Ipv4 => self.ipv4 = Some(route),
            NetpuncherAddressFamily::Ipv6 => self.ipv6 = Some(route),
        }
    }

    fn find(
        self,
        address: SocketAddr,
    ) -> Option<(NetpuncherAddressFamily, ReliableUdpPuncherRoute)> {
        let address = canonical_reliable_udp_peer_address(address);
        [
            (NetpuncherAddressFamily::Ipv4, self.ipv4),
            (NetpuncherAddressFamily::Ipv6, self.ipv6),
        ]
        .into_iter()
        .find_map(|(family, route)| {
            route
                .filter(|route| route.address == address)
                .map(|route| (family, route))
        })
    }

    fn clear_if(&mut self, family: NetpuncherAddressFamily, address: SocketAddr) {
        let slot = match family {
            NetpuncherAddressFamily::Ipv4 => &mut self.ipv4,
            NetpuncherAddressFamily::Ipv6 => &mut self.ipv6,
        };
        if slot.is_some_and(|route| route.address == address) {
            *slot = None;
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ReliableUdpLastSend {
    Peer(SocketAddr),
    BestEffort,
}

pub(crate) enum ReliableUdpPollReady {
    Datagram(usize, SocketAddr),
    Timer,
    SocketError(io::Error),
}

impl ReliableUdpSocketDriver {
    pub fn bind(bind_address: SocketAddr) -> io::Result<Self> {
        Self::bind_with_socket_constructor(bind_address, &crate::dual_stack::new_socket)
    }

    pub(crate) fn bind_with_socket_constructor(
        bind_address: SocketAddr,
        constructor: crate::dual_stack::SocketConstructor<'_>,
    ) -> io::Result<Self> {
        tokio::runtime::Handle::try_current().map_err(|_| {
            io::Error::other("reliable-UDP driver requires an entered Tokio runtime")
        })?;
        let (socket, address) = crate::dual_stack::create_bound_socket_with(
            bind_address,
            Type::DGRAM,
            Some(Protocol::UDP),
            constructor,
        )?;
        socket.set_nonblocking(true)?;
        socket.bind(&address.into())?;
        let socket = UdpSocket::from_std(socket.into())?;
        let family = crate::dual_stack::bound_socket_family(socket.local_addr()?);
        let started_at = Instant::now();
        Ok(Self {
            socket,
            family,
            core: ReliableUdpEndpointCore::new_at(Duration::ZERO),
            punchers: ReliableUdpPuncherRoutes::default(),
            statistics: None,
            peer_statistics: BTreeMap::new(),
            statistics_topology_epoch: 0,
            started_at,
            receive_buffer: vec![0; u16::MAX as usize + 1],
            protocol_timer: None,
            last_send: None,
            socket_writability_established: false,
            #[cfg(test)]
            protocol_timer_arms: 0,
            #[cfg(test)]
            socket_writability_establishments: 0,
            #[cfg(test)]
            force_next_planned_send_would_block: false,
        })
    }

    /// Form `destination` must take to leave this socket.
    ///
    /// An IPv4-only socket has no route to an IPv6 peer at all. Reporting that
    /// as unreachable rather than letting the kernel answer `EAFNOSUPPORT`
    /// keeps the failure scoped to that one peer: the reliable layer closes it,
    /// where a raw socket error takes down the whole hub.
    fn socket_destination(&self, destination: SocketAddr) -> io::Result<SocketAddr> {
        match self.family {
            crate::dual_stack::SocketFamily::DualStack => {
                Ok(reliable_udp_send_address(destination))
            }
            crate::dual_stack::SocketFamily::MappedIpv4 => {
                match canonical_reliable_udp_peer_address(destination) {
                    destination @ SocketAddr::V4(_) => Ok(reliable_udp_send_address(destination)),
                    destination => Err(io::Error::new(
                        io::ErrorKind::NetworkUnreachable,
                        format!("this host has no IPv6 route to reliable-UDP peer {destination}"),
                    )),
                }
            }
            crate::dual_stack::SocketFamily::Ipv4Only => {
                match canonical_reliable_udp_peer_address(destination) {
                    destination @ SocketAddr::V4(_) => Ok(destination),
                    destination => Err(io::Error::new(
                        io::ErrorKind::NetworkUnreachable,
                        format!("this host has no IPv6 route to reliable-UDP peer {destination}"),
                    )),
                }
            }
        }
    }

    /// Binds a driver whose statistics are measured at the physical UDP
    /// socket. This is intentionally below `ReliableUdpPeerStream`, whose
    /// synthetic TCP-style frames never appear on the wire.
    pub fn bind_with_statistics(
        bind_address: SocketAddr,
        statistics: crate::NetworkIoStatistics,
    ) -> io::Result<Self> {
        let mut driver = Self::bind(bind_address)?;
        driver.statistics = Some(statistics);
        Ok(driver)
    }

    /// Associates one live low-level peer with its high-level Network2
    /// connection. Bytes collected during this peer's handshake are retained
    /// until the next statistics sample and transferred into the real route.
    pub fn bind_peer_statistics(&mut self, peer: SocketAddr, connection_id: u32) -> io::Result<()> {
        let peer = canonical_reliable_udp_peer_address(peer);
        let peer_key = self.core.peer_key(peer).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotConnected,
                format!("reliable-UDP peer {peer} is no longer connected"),
            )
        })?;
        let Some(statistics) = self.statistics.clone() else {
            return Ok(());
        };
        let sampled_at_ms = statistics.last_statistics_ms();
        let peer_statistics = self
            .peer_statistics
            .entry(peer_key)
            .or_insert_with(|| ReliableUdpPeerStatistics::new(sampled_at_ms));
        if let Some(recorder) = &peer_statistics.recorder {
            if recorder.key().connection_id == connection_id {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("reliable-UDP peer {peer} already has a statistics connection"),
            ));
        }
        let (recorder, sampled_at_ms) = statistics.open_connection_with_raw_if_current(
            connection_id,
            crate::NetworkProtocol::Udp,
            peer_statistics.sampled_at_ms,
            peer_statistics.pending_input_bytes,
            peer_statistics.pending_output_bytes,
        );
        peer_statistics.bind(recorder, sampled_at_ms);
        Ok(())
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    pub fn core(&self) -> &ReliableUdpEndpointCore {
        &self.core
    }

    pub(crate) fn set_peer_delivery_credit(&mut self, peer: SocketAddr, capacity: usize) {
        self.core.set_peer_delivery_credit(peer, capacity);
    }

    pub(crate) async fn drain_peer_packets(
        &mut self,
        peer: SocketAddr,
        capacity: usize,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.drain_peer_packets(peer, capacity);
        if step.datagrams.is_empty() && step.events.is_empty() {
            Ok(Vec::new())
        } else {
            self.finish_step(step).await
        }
    }

    pub fn puncher_address(&self, family: NetpuncherAddressFamily) -> Option<SocketAddr> {
        self.punchers.route(family).map(|route| route.address)
    }

    /// Stores one puncher endpoint per address family before starting the
    /// reliable-UDP association, matching C4Network2IO::InitPuncher.
    pub async fn init_puncher(
        &mut self,
        puncher_address: SocketAddr,
        role: NetpuncherRole,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let puncher_address = canonical_reliable_udp_peer_address(puncher_address);
        let family = match puncher_address {
            SocketAddr::V4(_) => NetpuncherAddressFamily::Ipv4,
            SocketAddr::V6(_) => NetpuncherAddressFamily::Ipv6,
        };
        // Reserving a route this socket cannot address only defers the same
        // refusal to the first datagram, where it arrives as a bare socket
        // error and takes the hub's other peers with it.
        self.socket_destination(puncher_address).map_err(|_| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                format!("this host cannot reach netpuncher {puncher_address}"),
            )
        })?;
        if self
            .punchers
            .route(family)
            .is_some_and(|route| route.address != puncher_address || route.role != role)
            || (self.core.peer_status(puncher_address).is_some()
                && self.punchers.find(puncher_address).is_none())
        {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("reliable-UDP endpoint {puncher_address} is already in use"),
            ));
        }
        self.punchers.set(
            family,
            ReliableUdpPuncherRoute {
                address: puncher_address,
                role,
            },
        );
        let step = self.core.connect_at(puncher_address, self.elapsed());
        self.finish_step(step).await
    }

    /// Sends one netpuncher control packet through the configured family's
    /// reliable route. A missing route is a no-op, as in C4Network2IO.
    pub async fn send_puncher_packet(
        &mut self,
        family: NetpuncherAddressFamily,
        packet: &NetpuncherPacket,
    ) -> Result<Vec<ReliableUdpEvent>, ReliableUdpDriverError> {
        let Some(route) = self.punchers.route(family) else {
            return Ok(Vec::new());
        };
        let wire = encode_netpuncher_packet(packet);
        let step = self.core.send_packet(route.address, &wire)?;
        Ok(self.finish_step(step).await?)
    }

    /// Closes one exact reserved puncher route after a higher layer rejects a
    /// valid-but-inapplicable packet. Ordinary peers are never affected.
    pub async fn close_puncher(
        &mut self,
        puncher_address: SocketAddr,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let puncher_address = canonical_reliable_udp_peer_address(puncher_address);
        let Some((_, route)) = self.punchers.find(puncher_address) else {
            return Ok(Vec::new());
        };
        let step = self.core.close_peer(route.address);
        self.finish_step(step).await
    }

    /// Sends C4Network2IO::Punch's raw application-level Pong without adding
    /// a reliable-UDP peer or data envelope.
    pub async fn punch(&mut self, punchee_address: SocketAddr) -> io::Result<()> {
        let destination = self.socket_destination(punchee_address)?;
        let sent_at_ms = self.elapsed().as_millis() as u32;
        let wire = encode_netpuncher_punch(sent_at_ms);
        self.last_send = Some(ReliableUdpLastSend::BestEffort);
        self.socket.send_to(&wire, destination).await?;
        Ok(())
    }

    pub async fn connect(&mut self, peer: SocketAddr) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.connect_at(peer, self.elapsed());
        self.finish_step(step).await
    }

    pub async fn send_packet(
        &mut self,
        peer: SocketAddr,
        payload: &[u8],
    ) -> Result<Vec<ReliableUdpEvent>, ReliableUdpDriverError> {
        let step = self.core.send_packet(peer, payload)?;
        Ok(self.finish_step(step).await?)
    }

    /// Waits without mutating protocol state. This half of `poll` is safe to
    /// cancel from an outer `select!`: Tokio's UDP receive leaves the datagram
    /// queued unless it completes and this future returns it to the caller.
    pub(crate) async fn wait_ready(&mut self) -> ReliableUdpPollReady {
        let deadline = self.started_at + self.core.next_deadline();
        let needs_timer_arm = self
            .protocol_timer
            .as_ref()
            .is_none_or(|timer| timer.deadline() != deadline);
        #[cfg(test)]
        if needs_timer_arm {
            self.protocol_timer_arms += 1;
        }
        let protocol_timer = self
            .protocol_timer
            .get_or_insert_with(|| Box::pin(tokio::time::sleep_until(deadline)));
        if needs_timer_arm && protocol_timer.deadline() != deadline {
            protocol_timer.as_mut().reset(deadline);
        }
        tokio::select! {
            result = self.socket.recv_from(&mut self.receive_buffer) => {
                match result {
                    Ok((length, source)) => ReliableUdpPollReady::Datagram(length, source),
                    Err(error) => ReliableUdpPollReady::SocketError(error),
                }
            }
            _ = protocol_timer.as_mut() => ReliableUdpPollReady::Timer,
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
                if reliable_udp_peer_input_is_accounted(&self.receive_buffer[..length]) {
                    self.record_peer_input(source, length);
                }
                self.core
                    .receive_at(source, &self.receive_buffer[..length], now)
            }
            ReliableUdpPollReady::Timer => self.core.timer_at(now),
            ReliableUdpPollReady::SocketError(error) => {
                if reliable_udp_unreachable_error(&error) {
                    match self.last_send {
                        Some(ReliableUdpLastSend::Peer(peer)) => {
                            let step = self.core.report_unreachable(peer);
                            if !step.events.is_empty() {
                                return self.finish_step(step).await;
                            }
                        }
                        // C++ ignores failure to send Ping replies and the
                        // best-effort Close/AddAddr controls; it must not tear
                        // down an unrelated or surviving peer.
                        Some(ReliableUdpLastSend::BestEffort) => return Ok(Vec::new()),
                        None => {}
                    }
                }
                return Err(error);
            }
        };
        if received_datagram {
            step.append(self.core.timer_at(now));
        }
        self.finish_step(step).await
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
        self.finish_step(step).await
    }

    /// Sends one best-effort Close datagram before reporting local teardown.
    pub async fn close_peer(&mut self, peer: SocketAddr) -> io::Result<Vec<ReliableUdpEvent>> {
        let step = self.core.close_peer(peer);
        self.finish_step(step).await
    }

    fn elapsed(&self) -> Duration {
        Instant::now().saturating_duration_since(self.started_at)
    }

    fn record_peer_input(&mut self, peer: SocketAddr, payload_bytes: usize) {
        let Some(statistics) = &self.statistics else {
            return;
        };
        let Some(peer_key) = self.core.peer_key(peer) else {
            return;
        };
        let sampled_at_ms = statistics.last_statistics_ms();
        self.peer_statistics
            .entry(peer_key)
            .or_insert_with(|| ReliableUdpPeerStatistics::new(sampled_at_ms))
            .record_input(payload_bytes, sampled_at_ms);
    }

    fn record_peer_output(&mut self, peer: SocketAddr, payload_bytes: usize) {
        let Some(statistics) = &self.statistics else {
            return;
        };
        let peer = canonical_reliable_udp_peer_address(peer);
        let peer_key = self
            .core
            .peer_key(peer)
            .or_else(|| self.peer_statistics.contains_key(&peer).then_some(peer));
        let Some(peer_key) = peer_key else {
            return;
        };
        let sampled_at_ms = statistics.last_statistics_ms();
        self.peer_statistics
            .entry(peer_key)
            .or_insert_with(|| ReliableUdpPeerStatistics::new(sampled_at_ms))
            .record_output(payload_bytes, sampled_at_ms);
    }

    fn close_absent_peer_statistics_if_topology_changed(&mut self) {
        let topology_epoch = self.core.topology_epoch();
        if self.statistics_topology_epoch == topology_epoch {
            return;
        }
        self.peer_statistics.retain(|peer, _| {
            self.core
                .peers
                .get(peer)
                .is_some_and(|peer| peer.status != ReliableUdpPeerStatus::Closed)
        });
        self.statistics_topology_epoch = topology_epoch;
    }

    fn unbind_reconnected_peer_statistics(&mut self, events: &[ReliableUdpEvent]) {
        for event in events {
            let ReliableUdpEvent::Disconnected {
                peer,
                reason: ReliableUdpDisconnectReason::Reconnect,
            } = event
            else {
                continue;
            };
            let peer = canonical_reliable_udp_peer_address(*peer);
            if let Some(peer_statistics) = self.peer_statistics.get_mut(&peer) {
                peer_statistics.unbind();
            }
        }
    }

    async fn finish_step(&mut self, step: ReliableUdpStep) -> io::Result<Vec<ReliableUdpEvent>> {
        let events = self.flush_step(step).await?;
        self.route_puncher_events(events).await
    }

    async fn route_puncher_events(
        &mut self,
        events: Vec<ReliableUdpEvent>,
    ) -> io::Result<Vec<ReliableUdpEvent>> {
        let mut pending = VecDeque::from(events);
        let mut routed = Vec::new();
        while let Some(event) = pending.pop_front() {
            match event {
                ReliableUdpEvent::Connected {
                    peer,
                    observed_address,
                } => {
                    let Some((_route_family, route)) = self.punchers.find(peer) else {
                        routed.push(ReliableUdpEvent::Connected {
                            peer,
                            observed_address,
                        });
                        continue;
                    };
                    if let Some(observed_address) = observed_address {
                        let observed_address =
                            canonical_reliable_udp_peer_address(observed_address);
                        // OnPuncherConnect derives request-family policy from
                        // the normalized observed address. Later AssID packets
                        // remain tagged by their source puncher route.
                        let observed_family = match observed_address {
                            SocketAddr::V4(_) => NetpuncherAddressFamily::Ipv4,
                            SocketAddr::V6(_) => NetpuncherAddressFamily::Ipv6,
                        };
                        routed.push(ReliableUdpEvent::Puncher(NetpuncherIoEvent::Connected {
                            family: observed_family,
                            puncher_address: route.address,
                            observed_address,
                        }));
                    }
                }
                ReliableUdpEvent::Packet { peer, payload } => {
                    let Some((family, route)) = self.punchers.find(peer) else {
                        routed.push(ReliableUdpEvent::Packet { peer, payload });
                        continue;
                    };
                    match decode_netpuncher_packet(&payload) {
                        Ok(NetpuncherPacket::ClientRequest { address })
                            if route.role == NetpuncherRole::Host =>
                        {
                            // Punch is deliberately best effort. Its ICMP
                            // result must not tear down the puncher or a game
                            // peer which happens to use the same endpoint.
                            let _ = self.punch(address).await;
                        }
                        Ok(packet) => {
                            routed.push(ReliableUdpEvent::Puncher(NetpuncherIoEvent::Packet {
                                family,
                                puncher_address: route.address,
                                packet,
                            }))
                        }
                        Err(_) => {
                            // Construct failure makes HandlePuncherPacket
                            // close exactly this reliable address.
                            let close = self.core.close_peer(route.address);
                            pending.extend(self.flush_step(close).await?);
                        }
                    }
                }
                ReliableUdpEvent::Disconnected { peer, reason } => {
                    let Some((family, route)) = self.punchers.find(peer) else {
                        routed.push(ReliableUdpEvent::Disconnected { peer, reason });
                        continue;
                    };
                    self.punchers.clear_if(family, route.address);
                }
                ReliableUdpEvent::Puncher(event) => {
                    routed.push(ReliableUdpEvent::Puncher(event));
                }
            }
        }
        Ok(routed)
    }

    async fn send_planned_datagram(
        &mut self,
        datagram: &ReliableUdpDatagram,
    ) -> (SocketAddr, bool, io::Result<usize>) {
        let peer = canonical_reliable_udp_peer_address(datagram.destination);
        let peer_backed = !reliable_udp_send_is_best_effort(&datagram.payload)
            && self.core.peer_status(peer).is_some();
        self.last_send = Some(if peer_backed {
            ReliableUdpLastSend::Peer(peer)
        } else {
            ReliableUdpLastSend::BestEffort
        });
        // Native charges both buckets before sendto: every peer-originated
        // datagram hits that peer, and flagged multicast hits broadcast too.
        self.record_peer_output(peer, datagram.payload.len());
        if datagram
            .payload
            .first()
            .is_some_and(|status| status & 0x80 != 0)
        {
            if let Some(statistics) = &self.statistics {
                statistics
                    .record_broadcast_datagram(crate::NetworkProtocol::Udp, datagram.payload.len());
            }
        }
        // C++ `C4NetIOSimpleUDP::Send` issues one non-blocking `sendto` and on
        // EWOULDBLOCK resets the error and reports success — it drops the
        // datagram and lets the reliable layer repair it for that one peer
        // (oracle-src-pinned src/C4NetIO.cpp:1772-1790, :211-214).
        //
        // Awaiting writability unconditionally is not equivalent, and the
        // difference is not academic: one hub task owns this socket for every
        // peer, so suspending here holds up control delivery to all of them
        // behind whichever peer is congested, turning one bad uplink into a
        // stall for the whole session.
        //
        // Establish Tokio's lifetime WRITABLE interest once, then preserve the
        // native single-attempt behavior with immediate sends. PollEvented
        // keeps that reactor interest after cached readiness is consumed.
        let result = match self.socket_destination(datagram.destination) {
            Ok(destination) => self
                .establish_socket_writability()
                .await
                .and_then(|()| self.try_send_planned_payload(&datagram.payload, destination)),
            Err(error) => Err(error),
        };
        (peer, peer_backed, result)
    }

    fn try_send_planned_payload(
        &mut self,
        payload: &[u8],
        destination: SocketAddr,
    ) -> io::Result<usize> {
        #[cfg(test)]
        let result = if std::mem::take(&mut self.force_next_planned_send_would_block) {
            Err(io::ErrorKind::WouldBlock.into())
        } else {
            self.socket.try_send_to(payload, destination)
        };
        #[cfg(not(test))]
        let result = self.socket.try_send_to(payload, destination);
        match result {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(payload.len()),
            result => result,
        }
    }

    async fn establish_socket_writability(&mut self) -> io::Result<()> {
        if self.socket_writability_established {
            return Ok(());
        }
        #[cfg(test)]
        {
            self.socket_writability_establishments += 1;
        }
        let readiness =
            tokio::time::timeout(RELIABLE_UDP_SEND_BUDGET, self.socket.writable()).await;
        // Polling `writable` once installs lifetime WRITABLE interest in
        // PollEvented. Even when this small first-send budget expires, the
        // reactor can repopulate cached readiness for a later immediate send.
        self.socket_writability_established = true;
        readiness.unwrap_or(Ok(()))
    }

    #[cfg(test)]
    fn protocol_timer_arms(&self) -> usize {
        self.protocol_timer_arms
    }

    #[cfg(test)]
    fn socket_writability_establishments(&self) -> usize {
        self.socket_writability_establishments
    }

    #[cfg(test)]
    fn force_next_planned_send_would_block(&mut self) {
        self.force_next_planned_send_would_block = true;
    }

    async fn flush_step(&mut self, mut step: ReliableUdpStep) -> io::Result<Vec<ReliableUdpEvent>> {
        let mut first_send_error = None;
        for datagram in step.datagrams {
            let (peer, peer_backed, result) = self.send_planned_datagram(&datagram).await;
            match result {
                Ok(_) => {}
                Err(error) => {
                    if peer_backed {
                        first_send_error.get_or_insert((error, peer));
                    }
                }
            }
        }
        // Native control sends are best effort: a failed Check/ConnOK/Close
        // never suppresses a packet or lifecycle callback already produced by
        // the same receive transition.
        if let Some((error, peer)) = first_send_error {
            if reliable_udp_unreachable_error(&error) {
                let ReliableUdpStep { datagrams, events } = self.core.report_unreachable(peer);
                for close in datagrams {
                    let _ = self.send_planned_datagram(&close).await;
                }
                step.events.extend(events);
            }
            if step.events.is_empty() {
                self.close_absent_peer_statistics_if_topology_changed();
                return Err(error);
            }
        }
        self.unbind_reconnected_peer_statistics(&step.events);
        self.close_absent_peer_statistics_if_topology_changed();
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

fn reliable_udp_unreachable_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::HostUnreachable
            | io::ErrorKind::NetworkUnreachable
    )
}

fn reliable_udp_send_is_best_effort(payload: &[u8]) -> bool {
    matches!(
        reliable_udp_packet_kind(payload),
        Some(
            ReliableUdpPacketKind::Ping
                | ReliableUdpPacketKind::Test
                | ReliableUdpPacketKind::Close
                | ReliableUdpPacketKind::AddAddress
        )
    )
}

fn reliable_udp_peer_input_is_accounted(payload: &[u8]) -> bool {
    match reliable_udp_packet_kind(payload) {
        None | Some(ReliableUdpPacketKind::Test) => false,
        Some(ReliableUdpPacketKind::AddAddress) => {
            payload.first().is_some_and(|status| status & 0x80 != 0)
        }
        Some(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        net::{Ipv4Addr, Ipv6Addr},
    };

    use socket2::{Domain, Socket};

    use super::*;
    use crate::udp::{
        decode_reliable_udp_check, decode_reliable_udp_connect, decode_reliable_udp_data_fragment,
        encode_reliable_udp_check, ReliableUdpCheck,
    };

    fn address(last: u8, port: u16) -> SocketAddr {
        SocketAddr::new(Ipv4Addr::new(192, 0, 2, last).into(), port)
    }

    async fn next_driver_events(driver: &mut ReliableUdpSocketDriver) -> Vec<ReliableUdpEvent> {
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

    async fn next_driver_datagram(driver: &mut ReliableUdpSocketDriver) -> Vec<ReliableUdpEvent> {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let ready = driver.wait_ready().await;
                let received = matches!(ready, ReliableUdpPollReady::Datagram(_, _));
                let events = driver.process_ready(ready).await.unwrap();
                if received {
                    return events;
                }
            }
        })
        .await
        .unwrap()
    }

    async fn poll_driver_ready_once(driver: &mut ReliableUdpSocketDriver) {
        let mut ready = Box::pin(driver.wait_ready());
        std::future::poll_fn(|context| {
            assert!(std::future::Future::poll(ready.as_mut(), context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
    }

    async fn recv_spy_kind(
        spy: &UdpSocket,
        buffer: &mut [u8],
        expected: ReliableUdpPacketKind,
    ) -> (usize, SocketAddr) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let received = spy.recv_from(buffer).await.unwrap();
                if reliable_udp_packet_kind(&buffer[..received.0]) == Some(expected) {
                    return received;
                }
            }
        })
        .await
        .unwrap()
    }

    async fn connect_spy(driver: &mut ReliableUdpSocketDriver, spy: &UdpSocket) -> SocketAddr {
        let spy_address = spy.local_addr().unwrap();
        assert!(driver.connect(spy_address).await.unwrap().is_empty());
        let mut buffer = [0_u8; 512];
        let (_, driver_address) =
            recv_spy_kind(spy, &mut buffer, ReliableUdpPacketKind::Connect).await;
        let connect_ok = encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
            packet_number: 0,
            multicast_mode: ReliableUdpMulticastMode::NoMulticast,
            observed_address: canonical_reliable_udp_peer_address(driver_address),
        });
        spy.send_to(&connect_ok, driver_address).await.unwrap();
        assert!(matches!(
            next_driver_events(driver).await.as_slice(),
            [ReliableUdpEvent::Connected { peer, .. }] if *peer == spy_address
        ));
        canonical_reliable_udp_peer_address(driver_address)
    }

    #[test]
    fn puncher_routes_keep_ipv4_and_ipv6_slots_independent() {
        let ipv4 = ReliableUdpPuncherRoute {
            address: address(1, 11_115),
            role: NetpuncherRole::Host,
        };
        let ipv6 = ReliableUdpPuncherRoute {
            address: SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 11_115, 0, 0)),
            role: NetpuncherRole::Client,
        };
        let mut routes = ReliableUdpPuncherRoutes::default();
        routes.set(NetpuncherAddressFamily::Ipv4, ipv4);
        routes.set(NetpuncherAddressFamily::Ipv6, ipv6);

        assert_eq!(
            routes.route(NetpuncherAddressFamily::Ipv4).unwrap().address,
            ipv4.address
        );
        assert_eq!(
            routes.route(NetpuncherAddressFamily::Ipv6).unwrap().address,
            ipv6.address
        );
        routes.clear_if(NetpuncherAddressFamily::Ipv4, ipv4.address);
        assert!(routes.route(NetpuncherAddressFamily::Ipv4).is_none());
        assert_eq!(
            routes.route(NetpuncherAddressFamily::Ipv6).unwrap().address,
            ipv6.address
        );
    }

    #[tokio::test]
    async fn ipv4_wildcard_bind_still_reaches_an_ipv6_netpuncher() {
        // A dual-stack socket pinned to `::ffff:0.0.0.0` is IPv4 as far as the
        // kernel is concerned, and Linux answers EAFNOSUPPORT for every IPv6
        // destination sent over it. A host whose netpuncher resolved from an
        // AAAA record could then not start a game at all.
        let puncher = ReliableUdpSocketDriver::bind(SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::LOCALHOST,
            0,
            0,
            0,
        )))
        .unwrap();
        let mut driver =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0))
                .unwrap();
        driver
            .init_puncher(puncher.local_addr().unwrap(), NetpuncherRole::Host)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn named_ipv4_bind_uses_a_mapped_ipv6_destination() {
        // C++ creates an AF_INET6 socket and sends each peer through
        // addr.AsIPv6() (oracle-src-pinned src/C4NetIO.cpp:1514-1525,
        // 3136-3144; src/C4Network2Address.cpp:137-179). Rust's named-interface
        // extension must retain that sockaddr shape: macOS rejects an AF_INET
        // sockaddr passed to this mapped AF_INET6 socket.
        let driver =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)).unwrap();
        let peer = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 11_112);

        assert_eq!(driver.family, crate::dual_stack::SocketFamily::MappedIpv4);
        assert_eq!(
            driver.socket_destination(peer).unwrap(),
            reliable_udp_send_address(peer)
        );
        assert_eq!(
            driver
                .socket_destination(SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::LOCALHOST,
                    11_112,
                    0,
                    0,
                )))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NetworkUnreachable
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_waits_reuse_the_original_protocol_timer() {
        let mut driver =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)).unwrap();

        poll_driver_ready_once(&mut driver).await;
        tokio::time::advance(RELIABLE_UDP_CHECK_INTERVAL / 2).await;
        poll_driver_ready_once(&mut driver).await;

        assert_eq!(driver.protocol_timer_arms(), 1);

        tokio::time::advance(RELIABLE_UDP_CHECK_INTERVAL / 2).await;
        assert!(matches!(
            driver.wait_ready().await,
            ReliableUdpPollReady::Timer
        ));
    }

    #[tokio::test]
    async fn lossless_datagram_sends_reuse_socket_writability() {
        // C++ performs one non-blocking send attempt per datagram without
        // waiting between them (oracle-src-pinned src/C4NetIO.cpp:1772-1790).
        let mut driver =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)).unwrap();
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let destination = spy.local_addr().unwrap();
        let payloads = [b"first".as_slice(), b"second".as_slice()];

        for payload in payloads {
            let datagram = ReliableUdpDatagram {
                destination,
                payload: payload.to_vec(),
            };
            assert_eq!(
                driver.send_planned_datagram(&datagram).await.2.unwrap(),
                payload.len()
            );
        }

        let mut buffer = [0; 16];
        for payload in payloads {
            let (length, _) =
                tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&buffer[..length], payload);
        }
        assert_eq!(driver.socket_writability_establishments(), 1);
    }

    #[tokio::test]
    async fn would_block_drops_without_rewaiting_and_later_sends_resume_in_order() {
        // Native reports EWOULDBLOCK as success and immediately continues with
        // later datagrams (oracle-src-pinned src/C4NetIO.cpp:1772-1790).
        let mut driver =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)).unwrap();
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let destination = spy.local_addr().unwrap();
        let datagram = |payload: &[u8]| ReliableUdpDatagram {
            destination,
            payload: payload.to_vec(),
        };

        let before = datagram(b"before");
        assert_eq!(
            driver.send_planned_datagram(&before).await.2.unwrap(),
            before.payload.len()
        );
        driver.force_next_planned_send_would_block();
        let dropped = datagram(b"dropped");
        assert_eq!(
            driver.send_planned_datagram(&dropped).await.2.unwrap(),
            dropped.payload.len()
        );
        for payload in [b"after-one".as_slice(), b"after-two".as_slice()] {
            let later = datagram(payload);
            assert_eq!(
                driver.send_planned_datagram(&later).await.2.unwrap(),
                later.payload.len()
            );
        }

        let mut buffer = [0; 16];
        for payload in [
            b"before".as_slice(),
            b"after-one".as_slice(),
            b"after-two".as_slice(),
        ] {
            let (length, _) =
                tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(&buffer[..length], payload);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(50), spy.recv_from(&mut buffer))
                .await
                .is_err()
        );
        assert_eq!(driver.socket_writability_establishments(), 1);
    }

    #[tokio::test]
    async fn a_host_without_ipv6_binds_ipv4_and_refuses_the_ipv6_puncher() {
        // A kernel booted with `ipv6.disable=1` fails `socket(AF_INET6, ...)`
        // itself, which used to take down the host's whole UDP transport.
        let without_ipv6 = |domain: Domain, kind: Type, protocol: Option<Protocol>| {
            (domain == Domain::IPV6)
                .then(crate::dual_stack::ipv6_unavailable_error)
                .map_or_else(|| Socket::new(domain, kind, protocol), Err)
        };
        let mut driver = ReliableUdpSocketDriver::bind_with_socket_constructor(
            SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0),
            &without_ipv6,
        )
        .unwrap();
        assert_eq!(
            driver.local_addr().unwrap().ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        let ipv4_peer = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 11_115);
        assert_eq!(driver.socket_destination(ipv4_peer).unwrap(), ipv4_peer);

        // Nothing may be sent to the IPv6 puncher afterwards: trading the bind
        // failure for an EAFNOSUPPORT on the first datagram would only move the
        // same failure later, where it kills the hub instead of one route.
        let error = driver
            .init_puncher(
                SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 11_115, 0, 0)),
                NetpuncherRole::Host,
            )
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(driver
            .puncher_address(NetpuncherAddressFamily::Ipv6)
            .is_none());

        // The IPv4 half of the same netpuncher still works.
        let ipv4_puncher =
            ReliableUdpSocketDriver::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0)).unwrap();
        let ipv4_address = canonical_reliable_udp_peer_address(ipv4_puncher.local_addr().unwrap());
        driver
            .init_puncher(ipv4_address, NetpuncherRole::Host)
            .await
            .unwrap();
        assert_eq!(
            driver.puncher_address(NetpuncherAddressFamily::Ipv4),
            Some(ipv4_address)
        );
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
    fn delivery_credit_counts_pending_packets_once() {
        let (_, b_address, mut a, _) = handshake_pair();
        let peer = a
            .peers
            .get_mut(&b_address)
            .expect("handshake installs the peer");
        peer.pending_packets
            .push_back(ReliableUdpReassembledPacket {
                first_packet_number: 0,
                payload: b"first".to_vec(),
            });
        peer.pending_packets
            .push_back(ReliableUdpReassembledPacket {
                first_packet_number: 1,
                payload: b"second".to_vec(),
            });

        let drained = a.drain_peer_packets(b_address, 2);

        assert_eq!(
            drained.events,
            vec![
                ReliableUdpEvent::Packet {
                    peer: b_address,
                    payload: b"first".to_vec(),
                },
                ReliableUdpEvent::Packet {
                    peer: b_address,
                    payload: b"second".to_vec(),
                },
            ],
            "two available mailbox slots must admit two already-retained packets"
        );
    }

    #[test]
    fn exhausted_delivery_credit_withholds_ack_until_capacity_returns() {
        let (a_address, b_address, mut a, mut b) = handshake_pair();
        a.set_peer_delivery_credit(b_address, 0);
        let outbound = b.send_packet(a_address, b"retained").unwrap();
        let mut received = ReliableUdpStep::default();
        for datagram in outbound.datagrams {
            received.append(a.receive_at(b_address, &datagram.payload, Duration::ZERO));
        }

        assert!(
            received.events.is_empty(),
            "a full consumer mailbox must retain, not publish, the packet"
        );
        let blocked_check = a.timer_at(Duration::from_secs(1));
        let blocked_ack = decode_reliable_udp_check(&blocked_check.datagrams[0].payload).unwrap();
        assert_eq!(
            blocked_ack.next_expected_packet_number, 0,
            "retained data must not be acknowledged before the consumer owns it"
        );

        let resumed = a.drain_peer_packets(b_address, 1);
        assert_eq!(
            resumed.events,
            vec![ReliableUdpEvent::Packet {
                peer: b_address,
                payload: b"retained".to_vec(),
            }]
        );
        let resumed_check = a.timer_at(Duration::from_secs(2));
        let resumed_ack = decode_reliable_udp_check(&resumed_check.datagrams[0].payload).unwrap();
        assert_eq!(resumed_ack.next_expected_packet_number, 1);
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
    fn multicast_offer_gets_one_no_multicast_conn_ok() {
        let local_address = address(1, 11_111);
        let peer_address = address(2, 22_222);
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);

        let initial = endpoint.connect_at(peer_address, Duration::ZERO);
        assert_eq!(initial.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_connect(&initial.datagrams[0].payload)
                .unwrap()
                .expect("outbound Conn is valid")
                .multicast_address,
            None
        );

        let mut offer = ReliableUdpConnect::unicast(0, local_address);
        offer.multicast_address = Some(SocketAddr::V6(SocketAddrV6::new(
            "ff3e:40:2001:db8::1234".parse().unwrap(),
            11_113,
            0,
            0,
        )));
        let response = endpoint.receive_at(
            peer_address,
            &encode_reliable_udp_connect(&offer),
            Duration::ZERO,
        );

        assert_eq!(response.datagrams.len(), 1);
        assert_eq!(response.datagrams[0].payload[0], 0x03);
        assert_eq!(
            decode_reliable_udp_connect_ok(&response.datagrams[0].payload)
                .unwrap()
                .multicast_mode,
            ReliableUdpMulticastMode::NoMulticast
        );
        assert!(matches!(
            response.events.as_slice(),
            [ReliableUdpEvent::Connected { peer, .. }] if *peer == peer_address
        ));
        assert_eq!(
            endpoint.peer_status(peer_address),
            Some(ReliableUdpPeerStatus::Working)
        );
    }

    #[test]
    fn no_multicast_peer_ignores_flagged_data_and_check_without_mutating_direct_stream() {
        let (_, peer_address, mut endpoint, _) = handshake_pair();
        endpoint
            .send_packet(peer_address, b"queued direct packet")
            .unwrap();
        assert_eq!(endpoint.outgoing_packet_count(peer_address), Some(1));

        let mut flagged_data = encode_reliable_udp_data_fragments(0, b"stray multicast packet")
            .unwrap()
            .remove(0);
        flagged_data[0] |= 0x80;
        assert_eq!(
            endpoint.receive_at(peer_address, &flagged_data, Duration::ZERO),
            ReliableUdpStep::default()
        );

        let mut flagged_check = encode_reliable_udp_check(&ReliableUdpCheck {
            packet_number: 7,
            next_expected_packet_number: 1,
            next_expected_multicast_packet_number: 0,
            missing_packet_numbers: vec![0],
            missing_multicast_packet_numbers: Vec::new(),
        })
        .unwrap();
        flagged_check[0] |= 0x80;
        assert_eq!(
            endpoint.receive_at(peer_address, &flagged_check, Duration::ZERO),
            ReliableUdpStep::default()
        );
        assert_eq!(endpoint.outgoing_packet_count(peer_address), Some(1));
        assert_eq!(
            endpoint.peer_status(peer_address),
            Some(ReliableUdpPeerStatus::Working)
        );

        let direct_data = encode_reliable_udp_data_fragments(0, b"direct still starts at zero")
            .unwrap()
            .remove(0);
        assert_eq!(
            endpoint.receive_at(peer_address, &direct_data, Duration::ZERO),
            ReliableUdpStep {
                datagrams: Vec::new(),
                events: vec![ReliableUdpEvent::Packet {
                    peer: peer_address,
                    payload: b"direct still starts at zero".to_vec(),
                }],
            }
        );
    }

    #[test]
    fn test_datagrams_are_dropped_and_unknown_ping_is_answered_without_a_peer() {
        let source = address(9, 19_999);
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);

        for status in [0x01, 0x81] {
            for length in [1, 5, 37] {
                let mut wire = vec![0xaa; length];
                wire[0] = status;
                assert_eq!(
                    endpoint.receive_at(source, &wire, Duration::ZERO),
                    ReliableUdpStep::default(),
                    "test status 0x{status:02x}, length {length}"
                );
            }
        }
        assert_eq!(endpoint.peer_status(source), None);

        for status in [0x00, 0x80] {
            let step = endpoint.receive_at(source, &[status], Duration::ZERO);
            assert!(step.events.is_empty());
            assert_eq!(
                step.datagrams,
                vec![ReliableUdpDatagram {
                    destination: reliable_udp_send_address(source),
                    payload: vec![status, 0, 0, 0, 0],
                }]
            );
            assert_eq!(endpoint.peer_status(source), None);
        }

        let (_, known_peer, mut connected, _) = handshake_pair();
        for status in [0x00, 0x80, 0x01, 0x81] {
            assert_eq!(
                connected.receive_at(known_peer, &[status], Duration::ZERO),
                ReliableUdpStep::default()
            );
        }
        for status in [0x01, 0x81] {
            let mut wire = vec![status];
            wire.extend_from_slice(&99_u32.to_ne_bytes());
            wire.extend_from_slice(&[0xaa; 13]);
            assert_eq!(
                connected.receive_at(known_peer, &wire, Duration::ZERO),
                ReliableUdpStep::default()
            );
        }
        let check = connected.timer_at(Duration::from_secs(1));
        assert_eq!(check.datagrams.len(), 1);
        let check = decode_reliable_udp_check(&check.datagrams[0].payload).unwrap();
        assert_eq!(check.next_expected_packet_number, 0);
        assert!(check.missing_packet_numbers.is_empty());
        assert_eq!(
            connected.peer_status(known_peer),
            Some(ReliableUdpPeerStatus::Working)
        );
    }

    #[test]
    fn immediate_checks_damp_unknown_header_reasks_until_the_original_deadline() {
        let (_, known_peer, mut endpoint, _) = handshake_pair();
        let header = |packet_number: u32| {
            let mut wire = vec![0x7f];
            wire.extend_from_slice(&packet_number.to_ne_bytes());
            wire
        };

        let first = endpoint.receive_at(known_peer, &header(3), Duration::ZERO);
        assert_eq!(first.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_check(&first.datagrams[0].payload)
                .unwrap()
                .missing_packet_numbers,
            vec![0, 1, 2]
        );
        assert!(endpoint
            .receive_at(known_peer, &header(3), Duration::from_millis(125))
            .datagrams
            .is_empty());

        let continuation = endpoint.receive_at(known_peer, &header(5), Duration::from_millis(188));
        assert_eq!(continuation.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_check(&continuation.datagrams[0].payload)
                .unwrap()
                .missing_packet_numbers,
            vec![3, 4]
        );

        let expired =
            endpoint.receive_at(known_peer, &header(5), crate::RELIABLE_UDP_RECHECK_INTERVAL);
        assert_eq!(expired.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_check(&expired.datagrams[0].payload)
                .unwrap()
                .missing_packet_numbers,
            vec![0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn changed_unicast_conn_emits_add_address_without_resetting_the_peer() {
        let (local_address, peer_address, mut endpoint, _) = handshake_pair();
        let new_local_address = address(8, 18_888);
        let changed_conn =
            encode_reliable_udp_connect(&ReliableUdpConnect::unicast(0, new_local_address));

        let mut multicast_conn = changed_conn.clone();
        multicast_conn[0] |= 0x80;
        assert_eq!(
            endpoint.receive_at(peer_address, &multicast_conn, Duration::ZERO),
            ReliableUdpStep::default(),
            "only a changed unicast Conn emits AddAddr"
        );

        let step = endpoint.receive_at(peer_address, &changed_conn, Duration::ZERO);

        assert!(step.events.is_empty());
        assert_eq!(step.datagrams.len(), 1);
        assert_eq!(
            step.datagrams[0].destination,
            reliable_udp_send_address(peer_address)
        );
        assert_eq!(
            decode_reliable_udp_add_address(&step.datagrams[0].payload),
            Ok(ReliableUdpAddAddress {
                packet_number: 0,
                address: local_address,
                new_address: new_local_address,
            })
        );
        assert_eq!(
            endpoint.peer_status(peer_address),
            Some(ReliableUdpPeerStatus::Working)
        );
    }

    #[test]
    fn add_address_rejects_spoof_then_merges_duplicate_and_routes_the_alias() {
        fn establish(endpoint: &mut ReliableUdpEndpointCore, peer: SocketAddr) {
            assert_eq!(endpoint.connect_at(peer, Duration::ZERO).datagrams.len(), 1);
            let connect_ok = encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
                packet_number: 0,
                multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                observed_address: peer,
            });
            assert!(matches!(
                endpoint
                    .receive_at(peer, &connect_ok, Duration::ZERO)
                    .events
                    .as_slice(),
                [ReliableUdpEvent::Connected { peer: connected, .. }] if *connected == peer
            ));
        }

        let old_address = address(2, 22_222);
        let new_address = address(3, 33_333);
        let attacker = address(4, 44_444);
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);
        for peer in [old_address, new_address, attacker] {
            establish(&mut endpoint, peer);
        }
        let packet = ReliableUdpAddAddress {
            packet_number: 7,
            address: old_address,
            new_address,
        };
        let wire = encode_reliable_udp_add_address(&packet);

        assert_eq!(
            endpoint.receive_at(attacker, &wire, Duration::ZERO),
            ReliableUdpStep::default(),
            "a known third peer may not merge two carried addresses"
        );
        let mut multicast_flagged = wire.clone();
        multicast_flagged[0] |= 0x80;
        let flagged = endpoint.receive_at(new_address, &multicast_flagged, Duration::ZERO);
        assert!(flagged.events.is_empty());
        assert_eq!(flagged.datagrams.len(), 1);
        assert_eq!(
            decode_reliable_udp_check(&flagged.datagrams[0].payload)
                .unwrap()
                .missing_multicast_packet_numbers,
            (0..7).collect::<Vec<_>>(),
            "C++ checks the generic flagged header before ignoring its AddAddr body"
        );

        let merged = endpoint.receive_at(new_address, &wire, Duration::ZERO);
        assert_eq!(merged.datagrams.len(), 1);
        assert!(reliable_udp_send_is_best_effort(
            &merged.datagrams[0].payload
        ));
        assert_eq!(
            decode_reliable_udp_close(&merged.datagrams[0].payload),
            Ok(ReliableUdpClose {
                packet_number: 0,
                address: new_address,
            })
        );
        assert_eq!(
            merged.events,
            vec![ReliableUdpEvent::Disconnected {
                peer: new_address,
                reason: ReliableUdpDisconnectReason::Closed,
            }]
        );
        assert_eq!(
            endpoint.peer_status(old_address),
            Some(ReliableUdpPeerStatus::Working)
        );
        assert_eq!(
            endpoint.peer_status(new_address),
            Some(ReliableUdpPeerStatus::Working),
            "the duplicate address now resolves to the surviving peer"
        );
        assert!(endpoint
            .connect_at(new_address, Duration::ZERO)
            .datagrams
            .is_empty());

        let data = encode_reliable_udp_data_fragments(0, b"via alternate")
            .unwrap()
            .remove(0);
        assert_eq!(
            endpoint.receive_at(new_address, &data, Duration::ZERO),
            ReliableUdpStep {
                datagrams: Vec::new(),
                events: vec![ReliableUdpEvent::Packet {
                    peer: old_address,
                    payload: b"via alternate".to_vec(),
                }],
            }
        );
        let outbound = endpoint.send_packet(new_address, b"same peer").unwrap();
        assert_eq!(outbound.datagrams.len(), 1);
        assert!(!reliable_udp_send_is_best_effort(
            &outbound.datagrams[0].payload
        ));
        assert_eq!(
            outbound.datagrams[0].destination,
            reliable_udp_send_address(old_address)
        );
    }

    #[test]
    fn changed_incoming_baseline_reconnects_symmetrically() {
        let (a_address, b_address, mut a, _) = handshake_pair();
        let restarted_conn =
            encode_reliable_udp_connect(&ReliableUdpConnect::unicast(7, a_address));

        let step = a.receive_at(b_address, &restarted_conn, Duration::ZERO);
        assert_eq!(step.datagrams.len(), 3);
        assert_eq!(
            reliable_udp_packet_kind(&step.datagrams[0].payload),
            Some(ReliableUdpPacketKind::Check)
        );
        assert_eq!(
            decode_reliable_udp_check(&step.datagrams[0].payload)
                .unwrap()
                .missing_packet_numbers,
            (0..7).collect::<Vec<_>>()
        );
        assert_eq!(
            reliable_udp_packet_kind(&step.datagrams[1].payload),
            Some(ReliableUdpPacketKind::Connect)
        );
        assert_eq!(
            reliable_udp_packet_kind(&step.datagrams[2].payload),
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
    fn steady_deadline_reads_do_not_rescan_connected_peers() {
        let mut endpoint = ReliableUdpEndpointCore::new_at(Duration::ZERO);
        for last in 1..=24 {
            endpoint.connect_at(address(last, 11_111), Duration::ZERO);
        }
        reset_next_deadline_peer_visits();

        assert_eq!(endpoint.next_deadline(), Duration::from_secs(1));
        assert_eq!(endpoint.next_deadline(), Duration::from_secs(1));

        assert_eq!(next_deadline_peer_visits(), 0);
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
                if !packet_number.is_multiple_of(37) && packet_number != 599 {
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
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut a = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let mut b = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let a_address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), a.local_addr().unwrap().port());
        let b_address = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), b.local_addr().unwrap().port());

        assert!(a.connect(b_address).await.unwrap().is_empty());
        assert!(b.poll().await.unwrap().is_empty());
        assert!(matches!(
            next_driver_events(&mut a).await.as_slice(),
            [ReliableUdpEvent::Connected { .. }]
        ));
        assert!(matches!(
            next_driver_events(&mut b).await.as_slice(),
            [ReliableUdpEvent::Connected { .. }]
        ));

        a.send_packet(b_address, b"hello over reliable udp")
            .await
            .unwrap();
        assert!(matches!(
            next_driver_events(&mut b).await.as_slice(),
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
            next_driver_events(&mut b).await,
            vec![ReliableUdpEvent::Disconnected {
                peer: a_address,
                reason: ReliableUdpDisconnectReason::ClosedByPeer,
            }]
        );
        assert_eq!(b.core().peer_status(a_address), None);
    }

    #[tokio::test]
    async fn socket_driver_answers_connectionless_ping_and_silently_filters_test() {
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut driver = ReliableUdpSocketDriver::bind(wildcard).unwrap();
        let driver_address = SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            driver.local_addr().unwrap().port(),
        );
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let spy_address = spy.local_addr().unwrap();
        let mut buffer = [0_u8; 64];

        for status in [0x00, 0x80] {
            spy.send_to(&[status], driver_address).await.unwrap();
            assert!(driver.poll().await.unwrap().is_empty());
            let (length, source) =
                tokio::time::timeout(Duration::from_secs(2), spy.recv_from(&mut buffer))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(canonical_reliable_udp_peer_address(source), driver_address);
            assert_eq!(&buffer[..length], &[status, 0, 0, 0, 0]);
            assert_eq!(driver.core().peer_status(spy_address), None);
        }

        for status in [0x01, 0x81] {
            let mut wire = vec![status];
            wire.extend_from_slice(&99_u32.to_ne_bytes());
            wire.extend_from_slice(&[0xaa; 7]);
            spy.send_to(&wire, driver_address).await.unwrap();
            assert!(driver.poll().await.unwrap().is_empty());
            assert!(
                tokio::time::timeout(Duration::from_millis(50), spy.recv_from(&mut buffer))
                    .await
                    .is_err()
            );
            assert_eq!(driver.core().peer_status(spy_address), None);
        }
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
            next_driver_events(&mut driver).await.as_slice(),
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

    #[tokio::test]
    async fn socket_driver_drop_closes_statistics_route() {
        let statistics = crate::NetworkIoStatistics::new(0);
        let key = crate::ConnectionStatisticsKey::new(17, crate::NetworkProtocol::Udp);
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut driver =
            ReliableUdpSocketDriver::bind_with_statistics(wildcard, statistics.clone()).unwrap();
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();

        connect_spy(&mut driver, &spy).await;
        driver
            .bind_peer_statistics(spy.local_addr().unwrap(), key.connection_id)
            .unwrap();
        assert!(statistics.connection_statistics(key).is_some());
        drop(driver);

        assert_eq!(statistics.connection_statistics(key), None);
        assert!(statistics.snapshot().connections.is_empty());
    }

    #[tokio::test]
    async fn socket_driver_statistics_count_physical_udp_datagrams() {
        let statistics = crate::NetworkIoStatistics::new(0);
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut driver =
            ReliableUdpSocketDriver::bind_with_statistics(wildcard, statistics.clone()).unwrap();
        let spy = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let spy_address = spy.local_addr().unwrap();
        let driver_address = connect_spy(&mut driver, &spy).await;
        driver.bind_peer_statistics(spy_address, 23).unwrap();
        assert!(statistics.generate_statistics(1_001));

        driver
            .send_packet(spy_address, b"physical UDP")
            .await
            .unwrap();
        let mut buffer = [0; 64];
        let _ = recv_spy_kind(&spy, &mut buffer, ReliableUdpPacketKind::Data).await;
        spy.send_to(&[0x7f, 0x42], driver_address).await.unwrap();
        next_driver_datagram(&mut driver).await;

        assert!(statistics.generate_statistics(2_002));
        let udp = statistics.protocol_statistics(crate::NetworkProtocol::Udp);
        assert!(udp.input_rate > 0);
        assert!(udp.output_rate > 0);
    }

    #[tokio::test]
    async fn udp_statistics_are_attributed_to_each_peer_route() {
        fn normalize(bytes: u64) -> u64 {
            bytes.saturating_mul(crate::NETWORK_STATISTICS_INTERVAL_MS) / 1_001
        }

        let statistics = crate::NetworkIoStatistics::new(0);
        let wildcard = SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0));
        let mut driver =
            ReliableUdpSocketDriver::bind_with_statistics(wildcard, statistics.clone()).unwrap();
        let first = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let second = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        let first_address = first.local_addr().unwrap();
        let second_address = second.local_addr().unwrap();
        let driver_address = connect_spy(&mut driver, &first).await;
        assert_eq!(connect_spy(&mut driver, &second).await, driver_address);
        let first_key = crate::ConnectionStatisticsKey::new(7, crate::NetworkProtocol::Udp);
        let second_key = crate::ConnectionStatisticsKey::new(9, crate::NetworkProtocol::Udp);
        driver
            .bind_peer_statistics(first_address, first_key.connection_id)
            .unwrap();
        driver
            .bind_peer_statistics(second_address, second_key.connection_id)
            .unwrap();

        // Handshake traffic accumulated before the real IDs were available is
        // transferred into those routes, never a synthetic socket key.
        assert!(statistics.generate_statistics(1_001));
        for key in [first_key, second_key] {
            let route = statistics.connection_statistics(key).unwrap();
            assert!(route.input_rate > 0);
            assert!(route.output_rate > 0);
        }
        assert!(statistics
            .snapshot()
            .connections
            .iter()
            .all(|(key, _)| key.connection_id != u32::MAX));

        // Two peers sharing this one socket retain independent output buckets
        // and aggregate exactly once into the UDP protocol total.
        driver.send_packet(first_address, b"one").await.unwrap();
        driver
            .send_packet(second_address, b"a larger second payload")
            .await
            .unwrap();
        let mut first_wire = [0_u8; 128];
        let mut second_wire = [0_u8; 128];
        let (first_length, _) =
            recv_spy_kind(&first, &mut first_wire, ReliableUdpPacketKind::Data).await;
        let (second_length, _) =
            recv_spy_kind(&second, &mut second_wire, ReliableUdpPacketKind::Data).await;
        assert!(statistics.generate_statistics(2_002));
        // C++ sends each reliable fragment once; the physical accounting must
        // therefore charge each route exactly once too
        // (oracle-src-pinned src/C4NetIO.cpp:2789-2809,3128).
        let first_output = normalize(udp_accounted_bytes(first_length));
        let second_output = normalize(udp_accounted_bytes(second_length));
        assert_eq!(
            statistics
                .connection_statistics(first_key)
                .unwrap()
                .output_rate,
            first_output
        );
        assert_eq!(
            statistics
                .connection_statistics(second_key)
                .unwrap()
                .output_rate,
            second_output
        );
        assert_ne!(first_output, second_output);
        assert_eq!(
            statistics
                .protocol_statistics(crate::NetworkProtocol::Udp)
                .output_rate,
            normalize(first_output.saturating_add(second_output))
        );

        // Known-peer datagrams count before validation. Broadcast remains a
        // separate low-level bucket and never inflates either route.
        first.send_to(&[0x7f, 0x11], driver_address).await.unwrap();
        assert!(next_driver_datagram(&mut driver).await.is_empty());
        second
            .send_to(&[0x7e, 0x22, 0x33], driver_address)
            .await
            .unwrap();
        assert!(next_driver_datagram(&mut driver).await.is_empty());
        let connectionless = UdpSocket::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
            .await
            .unwrap();
        connectionless
            .send_to(&[0x80], driver_address)
            .await
            .unwrap();
        assert!(next_driver_datagram(&mut driver).await.is_empty());
        let mut broadcast_wire = [0_u8; 16];
        let (broadcast_length, _) = recv_spy_kind(
            &connectionless,
            &mut broadcast_wire,
            ReliableUdpPacketKind::Ping,
        )
        .await;
        assert!(statistics.generate_statistics(3_003));
        let first_input = normalize(udp_accounted_bytes(2));
        let second_input = normalize(udp_accounted_bytes(3));
        assert_eq!(
            statistics
                .connection_statistics(first_key)
                .unwrap()
                .input_rate,
            first_input
        );
        assert_eq!(
            statistics
                .connection_statistics(second_key)
                .unwrap()
                .input_rate,
            second_input
        );
        assert_eq!(
            statistics
                .protocol_statistics(crate::NetworkProtocol::Udp)
                .input_rate,
            normalize(first_input.saturating_add(second_input))
        );
        assert_eq!(
            statistics
                .protocol_statistics(crate::NetworkProtocol::Udp)
                .broadcast_rate,
            normalize(udp_accounted_bytes(broadcast_length))
        );

        // Closing removes only that route. Reopening the same endpoint may
        // safely reuse its real key without disturbing the surviving peer.
        driver.close_peer(first_address).await.unwrap();
        let _ = recv_spy_kind(&first, &mut first_wire, ReliableUdpPacketKind::Close).await;
        assert_eq!(statistics.connection_statistics(first_key), None);
        assert!(statistics.connection_statistics(second_key).is_some());
        connect_spy(&mut driver, &first).await;
        driver
            .bind_peer_statistics(first_address, first_key.connection_id)
            .unwrap();
        assert!(statistics.connection_statistics(first_key).is_some());
        assert!(statistics.generate_statistics(4_004));
        first
            .send_to(&[0x7d, 0x44, 0x55, 0x66], driver_address)
            .await
            .unwrap();
        assert!(next_driver_datagram(&mut driver).await.is_empty());
        assert!(statistics.generate_statistics(5_005));
        assert_eq!(
            statistics
                .connection_statistics(first_key)
                .unwrap()
                .input_rate,
            normalize(udp_accounted_bytes(4))
        );
        assert_eq!(
            statistics
                .connection_statistics(second_key)
                .unwrap()
                .input_rate,
            0
        );
    }
}
