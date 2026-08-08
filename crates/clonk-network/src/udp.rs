//! C++ reliable-UDP wire model.

use std::{
    collections::{BTreeMap, BTreeSet},
    mem::size_of,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    time::Duration,
};

use thiserror::Error;

const IPID_PING: u8 = 0x00;
const IPID_TEST: u8 = 0x01;
const IPID_CONN: u8 = 0x02;
const IPID_CONN_OK: u8 = 0x03;
const IPID_DATA: u8 = 0x04;
const IPID_CHECK: u8 = 0x05;
const IPID_CLOSE: u8 = 0x06;
const IPID_ADD_ADDRESS: u8 = 0x07;
const INTERNAL_PACKET_TYPE_MASK: u8 = 0x7f;
const BIN_ADDR_SIZE: usize = 19;
const PACKET_HEADER_SIZE: usize = 5;
const CONNECT_PACKET_SIZE: usize = 47;
const CONNECT_OK_PACKET_SIZE: usize = 28;
const DATA_PACKET_HEADER_SIZE: usize = 13;
const CHECK_PACKET_HEADER_SIZE: usize = 21;
const CLOSE_PACKET_SIZE: usize = 24;
const ADD_ADDRESS_PACKET_SIZE: usize = 43;
const MAX_DATAGRAM_SIZE: usize = 512;
const MAX_CHECK_ASK_COUNT: usize = 10;

/// `C4NetIOUDP::iVersion` carried by every reliable-UDP connection request.
pub const RELIABLE_UDP_PROTOCOL_VERSION: u32 = 2;

/// Maximum inner-packet bytes carried by one C++ reliable-UDP data fragment.
pub const RELIABLE_UDP_DATA_PAYLOAD_LIMIT: usize = MAX_DATAGRAM_SIZE - DATA_PACKET_HEADER_SIZE;

/// Re-ask damping for a repair request that went unanswered.
///
/// **Deliberate divergence from the oracle.** C++ uses one second
/// (`C4NetIOUDP::Peer::iReCheckInterval`, oracle-src-pinned
/// src/C4NetIO.cpp:1914). The first repair request is immediate on both sides,
/// so this interval only governs the case where a repair request is *itself*
/// lost — and there one second is a lockstep freeze for every participant, not
/// just the peer that dropped a datagram.
///
/// Measured with `cargo run -p clonk-network --example link_impairment` at
/// 60 ms RTT and +0..20 ms jitter, 400 control packets:
///
/// | loss | interval | mean     | p95   | p99     | max     |
/// |------|----------|----------|-------|---------|---------|
/// | 2%   | 1 s      | 44.50ms  | 55ms  | 171ms   | 188ms   |
/// | 2%   | 250 ms   | 44.50ms  | 55ms  | 171ms   | 188ms   |
/// | 5%   | 1 s      | 90.96ms  | 423ms | 1.009s  | 1.229s  |
/// | 5%   | 250 ms   | 55.61ms  | 169ms | 352ms   | 462ms   |
///
/// Below the loss rate at which a repair request is itself lost the two are
/// identical, so this costs nothing on a healthy link; on a lossy one it cuts
/// the p99 stall by 65% for about 7% more datagrams.
///
/// This changes only *when* a repair is re-requested. The delivered packet
/// stream, its ordering and the wire format are untouched, so simulation state
/// cannot observe it, and a C++ peer answers the extra asks unchanged.
/// Recorded in PORT_STATUS.md.
///
/// Do not lower this below roughly 2x a transatlantic round trip: the point is
/// to re-ask after a lost repair, not to duplicate repairs on exactly the
/// congested links where loss happens.
pub const RELIABLE_UDP_RECHECK_INTERVAL: Duration = Duration::from_millis(250);

/// Fields emitted by a unicast `C4NetIOUDP::ConnPacket`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableUdpConnect {
    pub packet_number: u32,
    pub protocol_version: u32,
    pub address: SocketAddr,
    pub multicast_address: Option<SocketAddr>,
}

impl ReliableUdpConnect {
    /// Builds the unicast-only request used when multicast is unavailable.
    pub fn unicast(packet_number: u32, address: SocketAddr) -> Self {
        Self {
            packet_number,
            protocol_version: RELIABLE_UDP_PROTOCOL_VERSION,
            address,
            multicast_address: None,
        }
    }
}

/// C++ `ConnOKPacket::MCMode` values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliableUdpMulticastMode {
    NoMulticast,
    Multicast,
    MulticastOk,
}

/// Fields decoded from a `C4NetIOUDP::ConnOKPacket`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableUdpConnectOk {
    pub packet_number: u32,
    pub multicast_mode: ReliableUdpMulticastMode,
    /// Source endpoint observed by the peer for the bound UDP socket.
    pub observed_address: SocketAddr,
}

/// Fields carried by the packed C++ close datagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableUdpClose {
    pub packet_number: u32,
    pub address: SocketAddr,
}

/// Fields carried by C++'s packed alternate-address notification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableUdpAddAddress {
    pub packet_number: u32,
    pub address: SocketAddr,
    pub new_address: SocketAddr,
}

/// Internal reliable-UDP packet kind after masking the multicast bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliableUdpPacketKind {
    Ping,
    Test,
    Connect,
    ConnectOk,
    Data,
    Check,
    Close,
    AddAddress,
    Other(u8),
}

/// One decoded `C4NetIOUDP::DataPacketHdr` and its fragment payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliableUdpDataFragment {
    pub packet_number: u32,
    pub first_packet_number: u32,
    pub total_size: u32,
    pub payload: Vec<u8>,
}

/// Fields carried by a unicast `C4NetIOUDP::CheckPacketHdr` and its ask list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliableUdpCheck {
    pub packet_number: u32,
    pub next_expected_packet_number: u32,
    pub next_expected_multicast_packet_number: u32,
    pub missing_packet_numbers: Vec<u32>,
    pub missing_multicast_packet_numbers: Vec<u32>,
}

/// Packet-number space selected by the C++ reliable-UDP broadcast flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReliableUdpChannel {
    Direct,
    Multicast,
}

/// Receive-side packet-number windows used to construct C++ Check packets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliableUdpReceiveWindow {
    direct: ReliableUdpReceiveChannel,
    multicast: ReliableUdpReceiveChannel,
    direct_packets: BTreeMap<u32, ReliableUdpPartialPacket>,
    next_recheck_at: Option<Duration>,
    last_packet_asked: u32,
    last_multicast_packet_asked: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReliableUdpReceiveChannel {
    next_expected_packet_number: u32,
    high_water_packet_number: u32,
    present_packet_numbers: BTreeSet<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReliableUdpPartialPacket {
    total_size: u32,
    fragment_count: u32,
    range_end: u32,
    fragments: BTreeMap<u32, Vec<u8>>,
}

/// One complete inner packet reconstructed from reliable-UDP fragments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReliableUdpReassembledPacket {
    pub first_packet_number: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReliableUdpReassemblyError {
    #[error(
        "reliable UDP packet number {packet_number} precedes first packet number {first_packet_number}"
    )]
    PacketBeforeFirst {
        packet_number: u32,
        first_packet_number: u32,
    },
    #[error(
        "reliable UDP packet {packet_number} is outside the fragment range beginning at {first_packet_number}"
    )]
    PacketOutsideRange {
        packet_number: u32,
        first_packet_number: u32,
    },
    #[error(
        "reliable UDP fragment range beginning at {first_packet_number} overflows the packet number space"
    )]
    PacketRangeOverflow { first_packet_number: u32 },
    #[error(
        "reliable UDP fragment {packet_number} has payload length {actual}; expected {expected}"
    )]
    InvalidFragmentPayloadSize {
        packet_number: u32,
        expected: usize,
        actual: usize,
    },
    #[error(
        "reliable UDP packet {first_packet_number} changed total size from {expected} to {actual}"
    )]
    InconsistentTotalSize {
        first_packet_number: u32,
        expected: u32,
        actual: u32,
    },
    #[error(
        "reliable UDP packet {first_packet_number} overlaps retained packet {retained_first_packet_number}"
    )]
    OverlappingPacketRange {
        first_packet_number: u32,
        retained_first_packet_number: u32,
    },
    #[error("reliable UDP fragment {packet_number} conflicts with its retained bytes")]
    ConflictingDuplicate { packet_number: u32 },
}

impl ReliableUdpReceiveWindow {
    pub fn new(
        next_expected_packet_number: u32,
        next_expected_multicast_packet_number: u32,
    ) -> Self {
        Self {
            direct: ReliableUdpReceiveChannel::new(next_expected_packet_number),
            multicast: ReliableUdpReceiveChannel::new(next_expected_multicast_packet_number),
            direct_packets: BTreeMap::new(),
            next_recheck_at: None,
            last_packet_asked: 0,
            last_multicast_packet_asked: 0,
        }
    }

    /// Records the packet number from any reliable-UDP header.
    pub fn observe_packet_header(&mut self, channel: ReliableUdpChannel, packet_number: u32) {
        self.channel_mut(channel).observe_header(packet_number);
    }

    /// Records a validated data fragment together with its header packet number.
    pub fn observe_data_fragment(&mut self, channel: ReliableUdpChannel, packet_number: u32) {
        let channel = self.channel_mut(channel);
        channel.observe_header(packet_number);
        channel.observe_data_fragment(packet_number);
    }

    /// Retains one validated unicast fragment and returns newly deliverable packets.
    pub fn receive_direct_data_fragment(
        &mut self,
        fragment: ReliableUdpDataFragment,
    ) -> Result<Vec<ReliableUdpReassembledPacket>, ReliableUdpReassemblyError> {
        self.receive_direct_data_fragment_with_limit(fragment, usize::MAX)
    }

    pub(crate) fn receive_direct_data_fragment_with_limit(
        &mut self,
        fragment: ReliableUdpDataFragment,
        delivery_limit: usize,
    ) -> Result<Vec<ReliableUdpReassembledPacket>, ReliableUdpReassemblyError> {
        self.direct.observe_header(fragment.packet_number);
        if fragment.packet_number < self.direct.next_expected_packet_number {
            return Ok(Vec::new());
        }

        let metadata = ReliableUdpFragmentMetadata::from_fragment(&fragment)?;
        if let Some(packet) = self.direct_packets.get_mut(&fragment.first_packet_number) {
            if packet.total_size != fragment.total_size {
                return Err(ReliableUdpReassemblyError::InconsistentTotalSize {
                    first_packet_number: fragment.first_packet_number,
                    expected: packet.total_size,
                    actual: fragment.total_size,
                });
            }
            packet.add_fragment(
                metadata.fragment_index,
                fragment.packet_number,
                fragment.payload,
            )?;
        } else {
            if let Some((&retained_first_packet_number, _)) =
                self.direct_packets
                    .iter()
                    .find(|(first_packet_number, packet)| {
                        fragment.first_packet_number < packet.range_end
                            && **first_packet_number < metadata.range_end
                    })
            {
                return Err(ReliableUdpReassemblyError::OverlappingPacketRange {
                    first_packet_number: fragment.first_packet_number,
                    retained_first_packet_number,
                });
            }
            let mut packet = ReliableUdpPartialPacket::new(
                fragment.total_size,
                metadata.fragment_count,
                metadata.range_end,
            );
            packet.add_fragment(
                metadata.fragment_index,
                fragment.packet_number,
                fragment.payload,
            )?;
            self.direct_packets
                .insert(fragment.first_packet_number, packet);
        }
        self.direct.observe_data_fragment(fragment.packet_number);
        Ok(self.take_complete_direct_packets(delivery_limit))
    }

    /// Constructs the C++ acknowledgment counters and bounded missing list.
    pub fn plan_check(&self, outgoing_packet_number: u32) -> ReliableUdpCheck {
        self.plan_check_from(
            outgoing_packet_number,
            self.direct.next_expected_packet_number,
            self.multicast.next_expected_packet_number,
        )
    }

    /// Applies re-ask damping and C++'s direct-first continuation. The damping
    /// shape is C++'s; only its interval diverges, see
    /// [`RELIABLE_UDP_RECHECK_INTERVAL`].
    /// `force` affects emission only; it never bypasses the active cursors.
    pub fn plan_check_at(
        &mut self,
        outgoing_packet_number: u32,
        now: Duration,
        force: bool,
    ) -> Option<ReliableUdpCheck> {
        let damping_active = self.next_recheck_at.is_some_and(|deadline| deadline > now);
        if !damping_active {
            self.last_packet_asked = 0;
            self.last_multicast_packet_asked = 0;
        }
        let direct_start = if damping_active {
            self.last_packet_asked
                .wrapping_add(1)
                .max(self.direct.next_expected_packet_number)
        } else {
            self.direct.next_expected_packet_number
        };
        let multicast_start = if damping_active {
            self.last_multicast_packet_asked
                .wrapping_add(1)
                .max(self.multicast.next_expected_packet_number)
        } else {
            self.multicast.next_expected_packet_number
        };
        let check = self.plan_check_from(outgoing_packet_number, direct_start, multicast_start);
        if let Some(packet_number) = check.missing_packet_numbers.last() {
            self.last_packet_asked = *packet_number;
        }
        if let Some(packet_number) = check.missing_multicast_packet_numbers.last() {
            self.last_multicast_packet_asked = *packet_number;
        }
        let has_asks = !check.missing_packet_numbers.is_empty()
            || !check.missing_multicast_packet_numbers.is_empty();
        if !damping_active {
            self.next_recheck_at =
                has_asks.then(|| now.saturating_add(RELIABLE_UDP_RECHECK_INTERVAL));
        }
        (has_asks || force).then_some(check)
    }

    pub fn next_expected_packet_number(&self) -> u32 {
        self.direct.next_expected_packet_number
    }

    fn plan_check_from(
        &self,
        outgoing_packet_number: u32,
        direct_start: u32,
        multicast_start: u32,
    ) -> ReliableUdpCheck {
        let missing_packet_numbers = self
            .direct
            .missing_packet_numbers_from(direct_start, MAX_CHECK_ASK_COUNT);
        let missing_multicast_packet_numbers = self.multicast.missing_packet_numbers_from(
            multicast_start,
            MAX_CHECK_ASK_COUNT - missing_packet_numbers.len(),
        );
        ReliableUdpCheck {
            packet_number: outgoing_packet_number,
            next_expected_packet_number: self.direct.next_expected_packet_number,
            next_expected_multicast_packet_number: self.multicast.next_expected_packet_number,
            missing_packet_numbers,
            missing_multicast_packet_numbers,
        }
    }

    fn channel_mut(&mut self, channel: ReliableUdpChannel) -> &mut ReliableUdpReceiveChannel {
        match channel {
            ReliableUdpChannel::Direct => &mut self.direct,
            ReliableUdpChannel::Multicast => &mut self.multicast,
        }
    }

    pub(crate) fn take_complete_direct_packets(
        &mut self,
        delivery_limit: usize,
    ) -> Vec<ReliableUdpReassembledPacket> {
        let mut complete_packets = Vec::new();
        while complete_packets.len() < delivery_limit {
            let first_packet_number = self.direct.next_expected_packet_number;
            let is_complete = self
                .direct_packets
                .get(&first_packet_number)
                .is_some_and(ReliableUdpPartialPacket::is_complete);
            if !is_complete {
                break;
            }
            let Some(packet) = self.direct_packets.remove(&first_packet_number) else {
                break;
            };
            for packet_number in first_packet_number..packet.range_end {
                self.direct.present_packet_numbers.remove(&packet_number);
            }
            self.direct.next_expected_packet_number = packet.range_end;
            complete_packets.push(packet.reassemble(first_packet_number));
        }
        complete_packets
    }
}

struct ReliableUdpFragmentMetadata {
    fragment_index: u32,
    fragment_count: u32,
    range_end: u32,
}

impl ReliableUdpFragmentMetadata {
    fn from_fragment(
        fragment: &ReliableUdpDataFragment,
    ) -> Result<Self, ReliableUdpReassemblyError> {
        if fragment.packet_number < fragment.first_packet_number {
            return Err(ReliableUdpReassemblyError::PacketBeforeFirst {
                packet_number: fragment.packet_number,
                first_packet_number: fragment.first_packet_number,
            });
        }
        let fragment_payload_limit = RELIABLE_UDP_DATA_PAYLOAD_LIMIT as u32;
        let fragment_count = fragment.total_size.saturating_sub(1) / fragment_payload_limit + 1;
        let range_end = fragment
            .first_packet_number
            .checked_add(fragment_count)
            .ok_or(ReliableUdpReassemblyError::PacketRangeOverflow {
                first_packet_number: fragment.first_packet_number,
            })?;
        if fragment.packet_number >= range_end {
            return Err(ReliableUdpReassemblyError::PacketOutsideRange {
                packet_number: fragment.packet_number,
                first_packet_number: fragment.first_packet_number,
            });
        }
        let fragment_index = fragment.packet_number - fragment.first_packet_number;
        let payload_offset = u64::from(fragment_index) * u64::from(fragment_payload_limit);
        let expected_payload_size = u64::from(fragment.total_size)
            .saturating_sub(payload_offset)
            .min(u64::from(fragment_payload_limit)) as usize;
        if fragment.payload.len() != expected_payload_size {
            return Err(ReliableUdpReassemblyError::InvalidFragmentPayloadSize {
                packet_number: fragment.packet_number,
                expected: expected_payload_size,
                actual: fragment.payload.len(),
            });
        }
        Ok(Self {
            fragment_index,
            fragment_count,
            range_end,
        })
    }
}

impl ReliableUdpPartialPacket {
    fn new(total_size: u32, fragment_count: u32, range_end: u32) -> Self {
        Self {
            total_size,
            fragment_count,
            range_end,
            fragments: BTreeMap::new(),
        }
    }

    fn add_fragment(
        &mut self,
        fragment_index: u32,
        packet_number: u32,
        payload: Vec<u8>,
    ) -> Result<(), ReliableUdpReassemblyError> {
        if let Some(retained_payload) = self.fragments.get(&fragment_index) {
            if retained_payload != &payload {
                return Err(ReliableUdpReassemblyError::ConflictingDuplicate { packet_number });
            }
        } else {
            self.fragments.insert(fragment_index, payload);
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.fragments.len() == self.fragment_count as usize
    }

    fn reassemble(self, first_packet_number: u32) -> ReliableUdpReassembledPacket {
        let mut payload = Vec::new();
        for fragment in self.fragments.into_values() {
            payload.extend_from_slice(&fragment);
        }
        ReliableUdpReassembledPacket {
            first_packet_number,
            payload,
        }
    }
}

impl ReliableUdpReceiveChannel {
    fn new(next_expected_packet_number: u32) -> Self {
        Self {
            next_expected_packet_number,
            high_water_packet_number: next_expected_packet_number,
            present_packet_numbers: BTreeSet::new(),
        }
    }

    fn observe_header(&mut self, packet_number: u32) {
        self.high_water_packet_number = self.high_water_packet_number.max(packet_number);
    }

    fn observe_data_fragment(&mut self, packet_number: u32) {
        if packet_number >= self.next_expected_packet_number {
            self.present_packet_numbers.insert(packet_number);
        }
    }

    fn missing_packet_numbers_from(&self, start: u32, limit: usize) -> Vec<u32> {
        let start = start.max(self.next_expected_packet_number);
        if limit == 0 || start >= self.high_water_packet_number {
            return Vec::new();
        }

        let mut missing_packet_numbers = Vec::with_capacity(limit);
        let mut candidate = start;
        for present_packet_number in self
            .present_packet_numbers
            .range(start..self.high_water_packet_number)
            .copied()
        {
            while candidate < present_packet_number && missing_packet_numbers.len() < limit {
                missing_packet_numbers.push(candidate);
                candidate = candidate.saturating_add(1);
            }
            if missing_packet_numbers.len() == limit {
                return missing_packet_numbers;
            }
            candidate = present_packet_number.saturating_add(1);
        }
        while candidate < self.high_water_packet_number && missing_packet_numbers.len() < limit {
            missing_packet_numbers.push(candidate);
            candidate = candidate.saturating_add(1);
        }
        missing_packet_numbers
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReliableUdpEncodeError {
    #[error("reliable UDP payload length {0} exceeds the C++ uint32 size field")]
    PayloadTooLarge(usize),
    #[error("reliable UDP missing-fragment count {0} exceeds the C++ uint32 count field")]
    MissingFragmentCountTooLarge(usize),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReliableUdpDecodeError {
    #[error("reliable UDP packet has length {actual}; expected {expected}")]
    InvalidLength { expected: usize, actual: usize },
    #[error("unexpected reliable UDP packet type 0x{0:02x}")]
    UnexpectedType(u8),
    #[error("unsupported reliable UDP address type {0}")]
    UnsupportedAddressType(u8),
    #[error("unsupported reliable UDP multicast mode {0}")]
    UnsupportedMulticastMode(i32),
    #[error("reliable UDP check missing-fragment counts overflow the packet length")]
    InvalidCheckCounts,
}

/// Builds the connectionless five-byte reply used for an unknown-peer Ping.
/// Only the multicast flag survives from the incoming status byte; C++ resets
/// the packet number to zero.
pub fn encode_reliable_udp_ping_response(incoming_status: u8) -> Vec<u8> {
    let mut wire = Vec::with_capacity(PACKET_HEADER_SIZE);
    wire.push(IPID_PING | (incoming_status & !INTERNAL_PACKET_TYPE_MASK));
    wire.extend_from_slice(&0_u32.to_ne_bytes());
    wire
}

/// Encodes the packed native-endian connection request used by `C4NetIOUDP`.
pub fn encode_reliable_udp_connect(connection: &ReliableUdpConnect) -> Vec<u8> {
    let mut wire = Vec::with_capacity(CONNECT_PACKET_SIZE);
    wire.push(IPID_CONN);
    wire.extend_from_slice(&connection.packet_number.to_ne_bytes());
    wire.extend_from_slice(&connection.protocol_version.to_ne_bytes());
    encode_bin_address(connection.address, &mut wire);
    encode_optional_bin_address(connection.multicast_address, &mut wire);
    wire
}

/// Decodes a packed C++ connection request. Size and version mismatches are
/// silently ignored by C++ and therefore return `Ok(None)` here.
pub fn decode_reliable_udp_connect(
    wire: &[u8],
) -> Result<Option<ReliableUdpConnect>, ReliableUdpDecodeError> {
    if wire.len() != CONNECT_PACKET_SIZE {
        return Ok(None);
    }
    if wire[0] & INTERNAL_PACKET_TYPE_MASK != IPID_CONN {
        return Err(ReliableUdpDecodeError::UnexpectedType(wire[0]));
    }
    let packet_number = u32::from_ne_bytes(wire[1..5].try_into().expect("checked packet length"));
    let protocol_version =
        u32::from_ne_bytes(wire[5..9].try_into().expect("checked packet length"));
    if protocol_version != RELIABLE_UDP_PROTOCOL_VERSION {
        return Ok(None);
    }
    Ok(Some(ReliableUdpConnect {
        packet_number,
        protocol_version,
        address: decode_bin_address(&wire[9..28])?,
        multicast_address: decode_optional_bin_address(&wire[28..])?,
    }))
}

/// Encodes the packed response that reports the observed source endpoint.
pub fn encode_reliable_udp_connect_ok(connection: &ReliableUdpConnectOk) -> Vec<u8> {
    let mut wire = Vec::with_capacity(CONNECT_OK_PACKET_SIZE);
    wire.push(IPID_CONN_OK);
    wire.extend_from_slice(&connection.packet_number.to_ne_bytes());
    let multicast_mode = match connection.multicast_mode {
        ReliableUdpMulticastMode::NoMulticast => 0_i32,
        ReliableUdpMulticastMode::Multicast => 1,
        ReliableUdpMulticastMode::MulticastOk => 2,
    };
    wire.extend_from_slice(&multicast_mode.to_ne_bytes());
    encode_bin_address(connection.observed_address, &mut wire);
    wire
}

/// Decodes the packed response that reports this socket's peer-observed endpoint.
pub fn decode_reliable_udp_connect_ok(
    wire: &[u8],
) -> Result<ReliableUdpConnectOk, ReliableUdpDecodeError> {
    if wire.len() != CONNECT_OK_PACKET_SIZE {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: CONNECT_OK_PACKET_SIZE,
            actual: wire.len(),
        });
    }
    if wire[0] & INTERNAL_PACKET_TYPE_MASK != IPID_CONN_OK {
        return Err(ReliableUdpDecodeError::UnexpectedType(wire[0]));
    }
    let packet_number = u32::from_ne_bytes(wire[1..5].try_into().expect("checked packet length"));
    let multicast_mode =
        match i32::from_ne_bytes(wire[5..9].try_into().expect("checked packet length")) {
            0 => ReliableUdpMulticastMode::NoMulticast,
            1 => ReliableUdpMulticastMode::Multicast,
            2 => ReliableUdpMulticastMode::MulticastOk,
            mode => return Err(ReliableUdpDecodeError::UnsupportedMulticastMode(mode)),
        };
    let observed_address = decode_bin_address(&wire[9..])?;
    Ok(ReliableUdpConnectOk {
        packet_number,
        multicast_mode,
        observed_address,
    })
}

/// Encodes C++'s best-effort close notification.
pub fn encode_reliable_udp_close(close: &ReliableUdpClose) -> Vec<u8> {
    let mut wire = Vec::with_capacity(CLOSE_PACKET_SIZE);
    wire.push(IPID_CLOSE);
    wire.extend_from_slice(&close.packet_number.to_ne_bytes());
    encode_bin_address(close.address, &mut wire);
    wire
}

/// Decodes C++'s packed close notification.
pub fn decode_reliable_udp_close(wire: &[u8]) -> Result<ReliableUdpClose, ReliableUdpDecodeError> {
    if wire.len() < CLOSE_PACKET_SIZE {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: CLOSE_PACKET_SIZE,
            actual: wire.len(),
        });
    }
    if wire[0] & INTERNAL_PACKET_TYPE_MASK != IPID_CLOSE {
        return Err(ReliableUdpDecodeError::UnexpectedType(wire[0]));
    }
    Ok(ReliableUdpClose {
        packet_number: decode_native_u32(wire, 1).expect("checked close packet length"),
        address: decode_bin_address(&wire[5..CLOSE_PACKET_SIZE])?,
    })
}

/// Encodes C++'s packed notification that two endpoints address one peer.
pub fn encode_reliable_udp_add_address(packet: &ReliableUdpAddAddress) -> Vec<u8> {
    let mut wire = Vec::with_capacity(ADD_ADDRESS_PACKET_SIZE);
    wire.push(IPID_ADD_ADDRESS);
    wire.extend_from_slice(&packet.packet_number.to_ne_bytes());
    encode_bin_address(packet.address, &mut wire);
    encode_bin_address(packet.new_address, &mut wire);
    wire
}

/// Decodes C++'s packed alternate-address notification. Native reads the
/// first complete structure and does not reject trailing datagram bytes.
pub fn decode_reliable_udp_add_address(
    wire: &[u8],
) -> Result<ReliableUdpAddAddress, ReliableUdpDecodeError> {
    if wire.len() < ADD_ADDRESS_PACKET_SIZE {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: ADD_ADDRESS_PACKET_SIZE,
            actual: wire.len(),
        });
    }
    if wire[0] != IPID_ADD_ADDRESS {
        return Err(ReliableUdpDecodeError::UnexpectedType(wire[0]));
    }
    Ok(ReliableUdpAddAddress {
        packet_number: decode_native_u32(wire, 1).expect("checked alternate-address packet length"),
        address: decode_bin_address(&wire[5..24])?,
        new_address: decode_bin_address(&wire[24..ADD_ADDRESS_PACKET_SIZE])?,
    })
}

/// Reads the five-byte common header kind without decoding a packet body.
pub fn reliable_udp_packet_kind(wire: &[u8]) -> Option<ReliableUdpPacketKind> {
    let packet_type = *wire.first()? & INTERNAL_PACKET_TYPE_MASK;
    Some(match packet_type {
        IPID_PING => ReliableUdpPacketKind::Ping,
        IPID_TEST => ReliableUdpPacketKind::Test,
        IPID_CONN => ReliableUdpPacketKind::Connect,
        IPID_CONN_OK => ReliableUdpPacketKind::ConnectOk,
        IPID_DATA => ReliableUdpPacketKind::Data,
        IPID_CHECK => ReliableUdpPacketKind::Check,
        IPID_CLOSE => ReliableUdpPacketKind::Close,
        IPID_ADD_ADDRESS => ReliableUdpPacketKind::AddAddress,
        other => ReliableUdpPacketKind::Other(other),
    })
}

/// Extra copies of a data packet to put on the wire beyond the original.
///
/// C++ sends each fragment once and repairs a loss after a `Check`. Immediate
/// copies can mask loss on a fast link, but on a narrow shared uplink their UDP
/// and IP framing can saturate the link and delay the original packets that
/// drive lockstep. Keep the native one-send policy for every packet size.
pub(crate) const REDUNDANT_DATA_PACKET_COPIES: usize = 0;

/// Inner-packet size at or below which a data packet is sent redundantly.
///
/// Control packets are tens of bytes; a resource chunk is orders of magnitude
/// larger and fragments into full 499-byte datagrams. Keying on the inner
/// packet's total size therefore separates the latency-critical stream from the
/// bulk one without needing the traffic class down here, and a control packet
/// that somehow exceeded this simply falls back to today's repair behavior.
const REDUNDANT_DATA_PACKET_LIMIT: u32 = 256;

/// Returns the number of extra physical sends for an outgoing datagram.
///
/// The native policy is one physical send followed by reliable repair, so this
/// returns zero. The packet classification remains here because callers use the
/// function as the stable policy seam.
pub fn reliable_udp_redundant_copies(wire: &[u8]) -> usize {
    if wire
        .first()
        .map(|status| status & INTERNAL_PACKET_TYPE_MASK)
        != Some(IPID_DATA)
    {
        return 0;
    }
    wire.get(9..13)
        .and_then(|size| size.try_into().ok())
        .filter(|size| u32::from_ne_bytes(*size) <= REDUNDANT_DATA_PACKET_LIMIT)
        .map_or(0, |_| REDUNDANT_DATA_PACKET_COPIES)
}

/// Splits one inner packet into packed C++ reliable-UDP data datagrams.
pub fn encode_reliable_udp_data_fragments(
    first_packet_number: u32,
    payload: &[u8],
) -> Result<Vec<Vec<u8>>, ReliableUdpEncodeError> {
    let total_size = u32::try_from(payload.len())
        .map_err(|_| ReliableUdpEncodeError::PayloadTooLarge(payload.len()))?;
    let fragment_count = payload.len().saturating_sub(1) / RELIABLE_UDP_DATA_PAYLOAD_LIMIT + 1;
    let mut fragments = Vec::with_capacity(fragment_count);
    for fragment_index in 0..fragment_count {
        let start = fragment_index * RELIABLE_UDP_DATA_PAYLOAD_LIMIT;
        let end = payload
            .len()
            .min(start.saturating_add(RELIABLE_UDP_DATA_PAYLOAD_LIMIT));
        let fragment_payload = &payload[start..end];
        let packet_number = first_packet_number.wrapping_add(fragment_index as u32);
        let mut wire = Vec::with_capacity(DATA_PACKET_HEADER_SIZE + fragment_payload.len());
        wire.push(IPID_DATA);
        wire.extend_from_slice(&packet_number.to_ne_bytes());
        wire.extend_from_slice(&first_packet_number.to_ne_bytes());
        wire.extend_from_slice(&total_size.to_ne_bytes());
        wire.extend_from_slice(fragment_payload);
        fragments.push(wire);
    }
    Ok(fragments)
}

/// Decodes one packed reliable-UDP data fragment without reassembly state.
pub fn decode_reliable_udp_data_fragment(
    wire: &[u8],
) -> Result<ReliableUdpDataFragment, ReliableUdpDecodeError> {
    if wire.len() < DATA_PACKET_HEADER_SIZE {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: DATA_PACKET_HEADER_SIZE,
            actual: wire.len(),
        });
    }
    if wire[0] & INTERNAL_PACKET_TYPE_MASK != IPID_DATA {
        return Err(ReliableUdpDecodeError::UnexpectedType(wire[0]));
    }
    Ok(ReliableUdpDataFragment {
        packet_number: u32::from_ne_bytes(
            wire[1..5].try_into().expect("checked data header length"),
        ),
        first_packet_number: u32::from_ne_bytes(
            wire[5..9].try_into().expect("checked data header length"),
        ),
        total_size: u32::from_ne_bytes(wire[9..13].try_into().expect("checked data header length")),
        payload: wire[DATA_PACKET_HEADER_SIZE..].to_vec(),
    })
}

/// Encodes the packed acknowledgment and missing-fragment request sent by C++.
pub fn encode_reliable_udp_check(
    check: &ReliableUdpCheck,
) -> Result<Vec<u8>, ReliableUdpEncodeError> {
    let missing_count = u32::try_from(check.missing_packet_numbers.len()).map_err(|_| {
        ReliableUdpEncodeError::MissingFragmentCountTooLarge(check.missing_packet_numbers.len())
    })?;
    let missing_multicast_count = u32::try_from(check.missing_multicast_packet_numbers.len())
        .map_err(|_| {
            ReliableUdpEncodeError::MissingFragmentCountTooLarge(
                check.missing_multicast_packet_numbers.len(),
            )
        })?;
    let mut wire = Vec::new();
    wire.push(IPID_CHECK);
    wire.extend_from_slice(&check.packet_number.to_ne_bytes());
    wire.extend_from_slice(&check.next_expected_packet_number.to_ne_bytes());
    wire.extend_from_slice(&check.next_expected_multicast_packet_number.to_ne_bytes());
    wire.extend_from_slice(&missing_count.to_ne_bytes());
    wire.extend_from_slice(&missing_multicast_count.to_ne_bytes());
    for packet_number in check
        .missing_packet_numbers
        .iter()
        .chain(&check.missing_multicast_packet_numbers)
    {
        wire.extend_from_slice(&packet_number.to_ne_bytes());
    }
    Ok(wire)
}

/// Decodes a packed C++ acknowledgment and missing-fragment request.
pub fn decode_reliable_udp_check(wire: &[u8]) -> Result<ReliableUdpCheck, ReliableUdpDecodeError> {
    if wire.len() < CHECK_PACKET_HEADER_SIZE {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        });
    }
    let packet_type = wire
        .first()
        .copied()
        .ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })?;
    if packet_type & INTERNAL_PACKET_TYPE_MASK != IPID_CHECK {
        return Err(ReliableUdpDecodeError::UnexpectedType(packet_type));
    }
    let packet_number =
        decode_native_u32(wire, 1).ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })?;
    let next_expected_packet_number =
        decode_native_u32(wire, 5).ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })?;
    let next_expected_multicast_packet_number =
        decode_native_u32(wire, 9).ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })?;
    let missing_count =
        decode_native_u32(wire, 13).ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })? as usize;
    let missing_multicast_count =
        decode_native_u32(wire, 17).ok_or(ReliableUdpDecodeError::InvalidLength {
            expected: CHECK_PACKET_HEADER_SIZE,
            actual: wire.len(),
        })? as usize;
    let total_missing_count = missing_count
        .checked_add(missing_multicast_count)
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;
    let ask_list_size = total_missing_count
        .checked_mul(size_of::<u32>())
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;
    let expected_size = CHECK_PACKET_HEADER_SIZE
        .checked_add(ask_list_size)
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;
    if wire.len() < expected_size {
        return Err(ReliableUdpDecodeError::InvalidLength {
            expected: expected_size,
            actual: wire.len(),
        });
    }
    let direct_ask_size = missing_count
        .checked_mul(size_of::<u32>())
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;
    let ask_list = wire.get(CHECK_PACKET_HEADER_SIZE..expected_size).ok_or(
        ReliableUdpDecodeError::InvalidLength {
            expected: expected_size,
            actual: wire.len(),
        },
    )?;
    let direct_asks = ask_list
        .get(..direct_ask_size)
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;
    let multicast_asks = ask_list
        .get(direct_ask_size..)
        .ok_or(ReliableUdpDecodeError::InvalidCheckCounts)?;

    Ok(ReliableUdpCheck {
        packet_number,
        next_expected_packet_number,
        next_expected_multicast_packet_number,
        missing_packet_numbers: decode_native_u32_list(direct_asks),
        missing_multicast_packet_numbers: decode_native_u32_list(multicast_asks),
    })
}

fn decode_native_u32(wire: &[u8], offset: usize) -> Option<u32> {
    let bytes = <[u8; 4]>::try_from(wire.get(offset..offset.checked_add(4)?)?).ok()?;
    Some(u32::from_ne_bytes(bytes))
}

fn decode_native_u32_list(wire: &[u8]) -> Vec<u32> {
    wire.chunks_exact(size_of::<u32>())
        .filter_map(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_ne_bytes)
        .collect()
}

fn encode_bin_address(address: SocketAddr, wire: &mut Vec<u8>) {
    wire.extend_from_slice(&address.port().to_ne_bytes());
    match address {
        SocketAddr::V4(address) => {
            wire.push(1);
            wire.extend_from_slice(&address.ip().octets());
            wire.extend_from_slice(&[0; 12]);
        }
        SocketAddr::V6(address) => {
            wire.push(2);
            wire.extend_from_slice(&address.ip().octets());
        }
    }
}

fn encode_optional_bin_address(address: Option<SocketAddr>, wire: &mut Vec<u8>) {
    encode_bin_address(
        address
            .unwrap_or_else(|| SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0))),
        wire,
    );
}

fn decode_bin_address(wire: &[u8]) -> Result<SocketAddr, ReliableUdpDecodeError> {
    debug_assert_eq!(wire.len(), BIN_ADDR_SIZE);
    let port = u16::from_ne_bytes(wire[..2].try_into().expect("checked BinAddr length"));
    match wire[2] {
        1 => Ok(SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(wire[3], wire[4], wire[5], wire[6]),
            port,
        ))),
        2 => Ok(SocketAddr::V6(SocketAddrV6::new(
            Ipv6Addr::from(
                <[u8; 16]>::try_from(&wire[3..]).expect("checked BinAddr address length"),
            ),
            port,
            0,
            0,
        ))),
        address_type => Err(ReliableUdpDecodeError::UnsupportedAddressType(address_type)),
    }
}

fn decode_optional_bin_address(wire: &[u8]) -> Result<Option<SocketAddr>, ReliableUdpDecodeError> {
    debug_assert_eq!(wire.len(), BIN_ADDR_SIZE);
    match wire[2] {
        1 | 2 => decode_bin_address(wire).map(|address| {
            (!address.ip().is_unspecified() || address.port() != 0).then_some(address)
        }),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

    use super::*;

    #[test]
    fn cpp_conn_codec_preserves_the_addresses_and_native_layout() {
        // C4NetIOUDP uses packed native-endian headers. Conn carries protocol
        // version 2, destination BinAddr, then multicast BinAddr; ConnOK carries
        // the endpoint that the peer observed for this same UDP socket
        // (pristine 9ffa0a5d src/C4NetIO.cpp:1921-2047, 2861-2968).
        let connection = ReliableUdpConnect {
            packet_number: 0x1122_3344,
            protocol_version: RELIABLE_UDP_PROTOCOL_VERSION,
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 11_115)),
            multicast_address: Some(SocketAddr::V6(SocketAddrV6::new(
                "ff3e:40:2001:db8::1234".parse::<Ipv6Addr>().unwrap(),
                11_113,
                0,
                0,
            ))),
        };
        let mut expected_connection = vec![0x02];
        expected_connection.extend_from_slice(&0x1122_3344_u32.to_ne_bytes());
        expected_connection.extend_from_slice(&2_u32.to_ne_bytes());
        expected_connection.extend_from_slice(&11_115_u16.to_ne_bytes());
        expected_connection.push(1);
        expected_connection.extend_from_slice(&[203, 0, 113, 7]);
        expected_connection.extend_from_slice(&[0; 12]);
        expected_connection.extend_from_slice(&11_113_u16.to_ne_bytes());
        expected_connection.push(2);
        expected_connection.extend_from_slice(
            &"ff3e:40:2001:db8::1234"
                .parse::<Ipv6Addr>()
                .unwrap()
                .octets(),
        );

        assert_eq!(RELIABLE_UDP_PROTOCOL_VERSION, 2);
        assert_eq!(
            encode_reliable_udp_connect(&connection),
            expected_connection
        );
        assert_eq!(
            decode_reliable_udp_connect(&expected_connection),
            Ok(Some(connection))
        );

        let observed_address =
            SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(198, 51, 100, 23), 62_000));
        let mut connection_ok = vec![0x03];
        connection_ok.extend_from_slice(&0x5566_7788_u32.to_ne_bytes());
        connection_ok.extend_from_slice(&0_i32.to_ne_bytes());
        connection_ok.extend_from_slice(&62_000_u16.to_ne_bytes());
        connection_ok.push(1);
        connection_ok.extend_from_slice(&[198, 51, 100, 23]);
        connection_ok.extend_from_slice(&[0; 12]);

        assert_eq!(
            encode_reliable_udp_connect_ok(&ReliableUdpConnectOk {
                packet_number: 0x5566_7788,
                multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                observed_address,
            }),
            connection_ok
        );
        assert_eq!(
            decode_reliable_udp_connect_ok(&connection_ok).unwrap(),
            ReliableUdpConnectOk {
                packet_number: 0x5566_7788,
                multicast_mode: ReliableUdpMulticastMode::NoMulticast,
                observed_address,
            }
        );
    }

    #[test]
    fn cpp_conn_decoder_normalizes_null_and_unknown_multicast_types() {
        let connection = ReliableUdpConnect {
            packet_number: 7,
            protocol_version: RELIABLE_UDP_PROTOCOL_VERSION,
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(192, 0, 2, 9), 11_111)),
            multicast_address: None,
        };
        let canonical = encode_reliable_udp_connect(&connection);
        assert_eq!(canonical.len(), CONNECT_PACKET_SIZE);
        assert_eq!(canonical[30], 2, "C++ null address is IPv6 unspecified");

        for unknown_type in [0x00, 0xcd] {
            let mut fixture = canonical.clone();
            fixture[30] = unknown_type;
            let decoded = decode_reliable_udp_connect(&fixture)
                .unwrap()
                .expect("valid Conn is not ignored");
            assert_eq!(decoded.multicast_address, None);
            assert_eq!(encode_reliable_udp_connect(&decoded), canonical);
        }
    }

    #[test]
    fn cpp_conn_decoder_ignores_size_and_protocol_mismatches() {
        let connection = ReliableUdpConnect {
            packet_number: 9,
            protocol_version: RELIABLE_UDP_PROTOCOL_VERSION,
            address: SocketAddr::V6(SocketAddrV6::new(Ipv6Addr::LOCALHOST, 11_111, 0, 0)),
            multicast_address: None,
        };
        let fixture = encode_reliable_udp_connect(&connection);
        assert_eq!(
            decode_reliable_udp_connect(&fixture[..fixture.len() - 1]),
            Ok(None)
        );
        let mut oversized = fixture.clone();
        oversized.push(0);
        assert_eq!(decode_reliable_udp_connect(&oversized), Ok(None));

        let mut wrong_version = fixture;
        wrong_version[5..9].copy_from_slice(&3_u32.to_ne_bytes());
        assert_eq!(decode_reliable_udp_connect(&wrong_version), Ok(None));
    }

    #[test]
    fn cpp_ping_response_and_add_address_codec_preserve_the_packed_layout() {
        assert_eq!(
            encode_reliable_udp_ping_response(0x00),
            vec![0x00, 0, 0, 0, 0]
        );
        assert_eq!(
            encode_reliable_udp_ping_response(0x80),
            vec![0x80, 0, 0, 0, 0]
        );
        assert_eq!(
            reliable_udp_packet_kind(&[0x00]),
            Some(ReliableUdpPacketKind::Ping)
        );
        assert_eq!(
            reliable_udp_packet_kind(&[0x81, 0xaa]),
            Some(ReliableUdpPacketKind::Test)
        );

        let packet = ReliableUdpAddAddress {
            packet_number: 0x1122_3344,
            address: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, 7), 11_115)),
            new_address: SocketAddr::V6(SocketAddrV6::new(
                "2001:db8::1234".parse::<Ipv6Addr>().unwrap(),
                11_113,
                0,
                0,
            )),
        };
        let mut expected = vec![0x07];
        expected.extend_from_slice(&packet.packet_number.to_ne_bytes());
        expected.extend_from_slice(&packet.address.port().to_ne_bytes());
        expected.push(1);
        expected.extend_from_slice(&[203, 0, 113, 7]);
        expected.extend_from_slice(&[0; 12]);
        expected.extend_from_slice(&packet.new_address.port().to_ne_bytes());
        expected.push(2);
        expected.extend_from_slice(&"2001:db8::1234".parse::<Ipv6Addr>().unwrap().octets());

        assert_eq!(expected.len(), ADD_ADDRESS_PACKET_SIZE);
        assert_eq!(encode_reliable_udp_add_address(&packet), expected);
        assert_eq!(decode_reliable_udp_add_address(&expected), Ok(packet));

        let mut oversized = expected.clone();
        oversized.push(0xaa);
        assert_eq!(decode_reliable_udp_add_address(&oversized), Ok(packet));
        assert_eq!(
            decode_reliable_udp_add_address(&expected[..expected.len() - 1]),
            Err(ReliableUdpDecodeError::InvalidLength {
                expected: ADD_ADDRESS_PACKET_SIZE,
                actual: ADD_ADDRESS_PACKET_SIZE - 1,
            })
        );
        let mut multicast_flagged = expected;
        multicast_flagged[0] = 0x87;
        assert_eq!(
            decode_reliable_udp_add_address(&multicast_flagged),
            Err(ReliableUdpDecodeError::UnexpectedType(0x87))
        );
    }

    #[test]
    fn cpp_close_codec_is_the_packed_24_byte_ipv4_and_ipv6_layout() {
        // ClosePacket is PacketHdr followed by BinAddr. C++ accepts any
        // datagram containing at least the complete packed structure.
        let cases = [
            (
                ReliableUdpClose {
                    packet_number: 0,
                    address: SocketAddr::V4(SocketAddrV4::new(
                        Ipv4Addr::new(203, 0, 113, 7),
                        11_115,
                    )),
                },
                [203, 0, 113, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
                1,
            ),
            (
                ReliableUdpClose {
                    packet_number: 0,
                    address: SocketAddr::V6(SocketAddrV6::new(
                        "2001:db8::1234".parse::<Ipv6Addr>().unwrap(),
                        11_113,
                        0,
                        0,
                    )),
                },
                "2001:db8::1234".parse::<Ipv6Addr>().unwrap().octets(),
                2,
            ),
        ];

        for &(close, address_octets, address_type) in &cases {
            let mut fixture = vec![0x06];
            fixture.extend_from_slice(&close.packet_number.to_ne_bytes());
            fixture.extend_from_slice(&close.address.port().to_ne_bytes());
            fixture.push(address_type);
            fixture.extend_from_slice(&address_octets);

            assert_eq!(fixture.len(), CLOSE_PACKET_SIZE);
            assert_eq!(encode_reliable_udp_close(&close), fixture);
            assert_eq!(decode_reliable_udp_close(&fixture), Ok(close));

            let mut multicast_flagged = fixture.clone();
            multicast_flagged[0] |= 0x80;
            assert_eq!(decode_reliable_udp_close(&multicast_flagged), Ok(close));

            let mut oversized = fixture;
            oversized.push(0xaa);
            assert_eq!(decode_reliable_udp_close(&oversized), Ok(close));
        }

        let fixture = encode_reliable_udp_close(&cases[0].0);
        for length in 0..CLOSE_PACKET_SIZE {
            assert_eq!(
                decode_reliable_udp_close(&fixture[..length]),
                Err(ReliableUdpDecodeError::InvalidLength {
                    expected: CLOSE_PACKET_SIZE,
                    actual: length,
                })
            );
        }
    }

    #[test]
    fn cpp_data_fragments_cross_the_499_byte_payload_boundary_with_shared_metadata() {
        // C4NetIOUDP packs Status/Nr/FNr/Size into a 13-byte header inside a
        // 512-byte datagram. The first 500-byte payload therefore becomes a
        // 499-byte fragment plus one byte, both sharing FNr and total Size
        // (pristine 9ffa0a5d src/C4NetIO.cpp:1921-2047, 2545-2569,
        // 2973-2996, 3175-3214).
        let first_packet_number = 0x1122_3344;
        for (payload_len, expected_fragment_lengths) in
            [(499_usize, &[499_usize][..]), (500, &[499, 1][..])]
        {
            let payload = (0..payload_len)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>();
            let encoded = encode_reliable_udp_data_fragments(first_packet_number, &payload)
                .expect("test payload fits C++ uint32 size");

            assert_eq!(RELIABLE_UDP_DATA_PAYLOAD_LIMIT, 499);
            assert_eq!(encoded.len(), expected_fragment_lengths.len());
            let mut payload_offset = 0;
            for (fragment_index, (&fragment_len, wire)) in
                expected_fragment_lengths.iter().zip(&encoded).enumerate()
            {
                let packet_number = first_packet_number.wrapping_add(fragment_index as u32);
                let fragment_payload = &payload[payload_offset..payload_offset + fragment_len];
                let mut expected = vec![0x04];
                expected.extend_from_slice(&packet_number.to_ne_bytes());
                expected.extend_from_slice(&first_packet_number.to_ne_bytes());
                expected.extend_from_slice(&(payload_len as u32).to_ne_bytes());
                expected.extend_from_slice(fragment_payload);

                assert_eq!(wire, &expected, "payload length {payload_len}");
                assert_eq!(
                    decode_reliable_udp_data_fragment(wire).unwrap(),
                    ReliableUdpDataFragment {
                        packet_number,
                        first_packet_number,
                        total_size: payload_len as u32,
                        payload: fragment_payload.to_vec(),
                    },
                    "payload length {payload_len}"
                );
                payload_offset += fragment_len;
            }
        }
    }

    #[test]
    fn cpp_check_codec_orders_direct_then_multicast_missing_fragments() {
        // C4NetIOUDP emits a packed native-endian Check header followed by
        // direct missing fragment numbers and then multicast missing fragment
        // numbers (pristine 9ffa0a5d src/C4NetIO.cpp:1921-2047,
        // 2812-2840, 2999-3031, 3100-3121).
        let check = ReliableUdpCheck {
            packet_number: 13,
            next_expected_packet_number: 8,
            next_expected_multicast_packet_number: 4,
            missing_packet_numbers: vec![8, 10],
            missing_multicast_packet_numbers: vec![5],
        };
        let mut expected = vec![0x05];
        expected.extend_from_slice(&13_u32.to_ne_bytes());
        expected.extend_from_slice(&8_u32.to_ne_bytes());
        expected.extend_from_slice(&4_u32.to_ne_bytes());
        expected.extend_from_slice(&2_u32.to_ne_bytes());
        expected.extend_from_slice(&1_u32.to_ne_bytes());
        expected.extend_from_slice(&8_u32.to_ne_bytes());
        expected.extend_from_slice(&10_u32.to_ne_bytes());
        expected.extend_from_slice(&5_u32.to_ne_bytes());

        assert_eq!(encode_reliable_udp_check(&check).unwrap(), expected);
        assert_eq!(decode_reliable_udp_check(&expected).unwrap(), check);
    }

    #[test]
    fn cpp_receive_window_requests_direct_holes_before_multicast_up_to_ten() {
        // Every header monotonically raises the receive high-water marker.
        // Missing selection excludes that marker, preserves ascending packet
        // order, and spends the ten-entry budget on direct traffic first
        // (pristine 9ffa0a5d src/C4NetIO.cpp:2757-2758, 2812-2857,
        // 3175-3214).
        let mut window = ReliableUdpReceiveWindow::new(8, 4);
        window.observe_data_fragment(ReliableUdpChannel::Direct, 9);
        window.observe_data_fragment(ReliableUdpChannel::Direct, 12);
        window.observe_data_fragment(ReliableUdpChannel::Direct, 17);
        window.observe_packet_header(ReliableUdpChannel::Direct, 15);
        window.observe_data_fragment(ReliableUdpChannel::Multicast, 5);
        window.observe_packet_header(ReliableUdpChannel::Multicast, 10);
        window.observe_packet_header(ReliableUdpChannel::Multicast, 7);

        assert_eq!(
            window.plan_check(23),
            ReliableUdpCheck {
                packet_number: 23,
                next_expected_packet_number: 8,
                next_expected_multicast_packet_number: 4,
                missing_packet_numbers: vec![8, 10, 11, 13, 14, 15, 16],
                missing_multicast_packet_numbers: vec![4, 6, 7],
            }
        );
    }

    #[test]
    fn recheck_damps_continues_new_holes_and_expires_strictly() {
        // The damping shape is C++'s (oracle-src-pinned src/C4NetIO.cpp:3090-3119):
        // unchanged holes stay quiet inside the window, strictly higher holes
        // continue immediately, and the deadline set by the first ask survives
        // those continuations rather than being pushed back. Only the interval
        // itself diverges, from C++'s one second; see
        // RELIABLE_UDP_RECHECK_INTERVAL for the measurement and the reason.
        let mut window = ReliableUdpReceiveWindow::new(8, 0);
        window.observe_packet_header(ReliableUdpChannel::Direct, 10);

        let first = window
            .plan_check_at(23, Duration::ZERO, false)
            .expect("fresh holes must emit a Check");
        assert_eq!(first.missing_packet_numbers, vec![8, 9]);
        assert_eq!(
            window.plan_check_at(23, Duration::from_millis(125), false),
            None,
            "unchanged holes are damped inside the recheck window"
        );

        window.observe_packet_header(ReliableUdpChannel::Direct, 13);
        let continuation = window
            .plan_check_at(23, Duration::from_millis(188), false)
            .expect("new higher holes must be asked immediately");
        assert_eq!(continuation.missing_packet_numbers, vec![10, 11, 12]);
        assert_eq!(
            window.plan_check_at(23, Duration::from_millis(249), false),
            None,
            "a continuation does not push the original deadline back"
        );

        let expired = window
            .plan_check_at(23, RELIABLE_UDP_RECHECK_INTERVAL, false)
            .expect("the original deadline expires at exact equality");
        assert_eq!(expired.missing_packet_numbers, vec![8, 9, 10, 11, 12]);
    }

    #[test]
    fn recheck_shares_direct_first_budget_and_force_only_changes_emission() {
        // Ask-list budget and direct-first ordering are C++'s
        // (oracle-src-pinned src/C4NetIO.cpp:3090-3119); only the window length
        // diverges, see RELIABLE_UDP_RECHECK_INTERVAL.
        let mut window = ReliableUdpReceiveWindow::new(4, 20);
        window.observe_packet_header(ReliableUdpChannel::Direct, 8);
        window.observe_packet_header(ReliableUdpChannel::Multicast, 28);

        let first = window
            .plan_check_at(90, Duration::ZERO, false)
            .expect("fresh direct and multicast holes must emit");
        assert_eq!(first.missing_packet_numbers, vec![4, 5, 6, 7]);
        assert_eq!(
            first.missing_multicast_packet_numbers,
            vec![20, 21, 22, 23, 24, 25]
        );
        let second = window
            .plan_check_at(90, Duration::from_millis(125), false)
            .expect("the unasked multicast tail continues inside the window");
        assert!(second.missing_packet_numbers.is_empty());
        assert_eq!(second.missing_multicast_packet_numbers, vec![26, 27]);
        assert_eq!(
            window.plan_check_at(90, Duration::from_millis(125), false),
            None
        );

        let forced = window
            .plan_check_at(90, Duration::from_millis(125), true)
            .expect("forced cadence emits an empty Check");
        assert!(forced.missing_packet_numbers.is_empty());
        assert!(forced.missing_multicast_packet_numbers.is_empty());
    }

    #[test]
    fn cpp_unicast_reassembly_retains_out_of_order_fragments_until_next_is_complete() {
        // C4NetIOUDP retains valid fragments by first packet number, rejects
        // inconsistent fragment metadata, and delivers only a complete packet
        // beginning at the next expected number before advancing by its exact
        // fragment count (pristine 9ffa0a5d src/C4NetIO.cpp:2586-2637,
        // 2970-2993, 3175-3214).
        let payload = (0..500)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let fragments = encode_reliable_udp_data_fragments(40, &payload)
            .unwrap()
            .into_iter()
            .map(|wire| decode_reliable_udp_data_fragment(&wire).unwrap())
            .collect::<Vec<_>>();
        let mut window = ReliableUdpReceiveWindow::new(40, 0);

        assert_eq!(
            window.receive_direct_data_fragment(fragments[1].clone()),
            Ok(Vec::new())
        );
        assert_eq!(
            window.plan_check(90),
            ReliableUdpCheck {
                packet_number: 90,
                next_expected_packet_number: 40,
                next_expected_multicast_packet_number: 0,
                missing_packet_numbers: vec![40],
                missing_multicast_packet_numbers: Vec::new(),
            }
        );

        let mut inconsistent_first = fragments[0].clone();
        inconsistent_first.first_packet_number = 41;
        assert_eq!(
            window.receive_direct_data_fragment(inconsistent_first),
            Err(ReliableUdpReassemblyError::PacketBeforeFirst {
                packet_number: 40,
                first_packet_number: 41,
            })
        );
        let mut inconsistent_size = fragments[0].clone();
        inconsistent_size.total_size = 501;
        assert_eq!(
            window.receive_direct_data_fragment(inconsistent_size),
            Err(ReliableUdpReassemblyError::InconsistentTotalSize {
                first_packet_number: 40,
                expected: 500,
                actual: 501,
            })
        );

        assert_eq!(
            window.receive_direct_data_fragment(fragments[0].clone()),
            Ok(vec![ReliableUdpReassembledPacket {
                first_packet_number: 40,
                payload,
            }])
        );
        assert_eq!(
            window.plan_check(90),
            ReliableUdpCheck {
                packet_number: 90,
                next_expected_packet_number: 42,
                next_expected_multicast_packet_number: 0,
                missing_packet_numbers: Vec::new(),
                missing_multicast_packet_numbers: Vec::new(),
            }
        );
    }

    /// C++ sends each reliable-UDP fragment once and repairs losses on request.
    #[test]
    fn reliable_udp_data_uses_the_cpp_single_send_policy() {
        // C4NetIOUDP::Peer::Send calls SendDirect once for each fragment
        // (oracle-src-pinned src/C4NetIO.cpp:2789-2809,3128).
        let control = encode_reliable_udp_data_fragments(7, &[0_u8; 40]).expect("encode control");
        assert_eq!(control.len(), 1);
        assert_eq!(reliable_udp_redundant_copies(&control[0]), 0);

        // A resource chunk: many full fragments, none of them re-sent.
        let bulk = encode_reliable_udp_data_fragments(7, &[0_u8; 8_192]).expect("encode bulk");
        assert!(bulk.len() > 1);
        for fragment in &bulk {
            assert_eq!(
                reliable_udp_redundant_copies(fragment),
                0,
                "bulk transfer must not pay for redundancy"
            );
        }

        // Only data packets qualify; a Check carries its own retry damping.
        assert_eq!(reliable_udp_redundant_copies(&[IPID_CHECK, 0, 0, 0, 0]), 0);
        assert_eq!(reliable_udp_redundant_copies(&[]), 0);
    }

    /// The copy has to be byte-identical, or it would occupy a fresh packet
    /// number and the receiver would deliver the control twice.
    #[test]
    fn redundant_copy_is_the_same_bytes_and_is_discarded_on_arrival() {
        let fragments = encode_reliable_udp_data_fragments(3, b"tick").expect("encode");
        let wire = &fragments[0];
        let first = decode_reliable_udp_data_fragment(wire).expect("decode original");
        let copy = decode_reliable_udp_data_fragment(wire).expect("decode duplicate");
        assert_eq!(first.packet_number, copy.packet_number);

        let mut window = ReliableUdpReceiveWindow::new(3, 0);
        let delivered = window
            .receive_direct_data_fragment(first)
            .expect("original delivers");
        assert_eq!(delivered.len(), 1);
        let repeated = window
            .receive_direct_data_fragment(copy)
            .expect("a duplicate is accepted, not an error");
        assert!(
            repeated.is_empty(),
            "the duplicate must not deliver the control a second time"
        );
    }
}
