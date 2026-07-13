//! C++ reliable-UDP wire model.

use std::{
    collections::BTreeSet,
    mem::size_of,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
};

use thiserror::Error;

const IPID_CONN: u8 = 0x02;
const IPID_CONN_OK: u8 = 0x03;
const IPID_DATA: u8 = 0x04;
const IPID_CHECK: u8 = 0x05;
const INTERNAL_PACKET_TYPE_MASK: u8 = 0x7f;
const BIN_ADDR_SIZE: usize = 19;
const CONNECT_PACKET_SIZE: usize = 47;
const CONNECT_OK_PACKET_SIZE: usize = 28;
const DATA_PACKET_HEADER_SIZE: usize = 13;
const CHECK_PACKET_HEADER_SIZE: usize = 21;
const MAX_DATAGRAM_SIZE: usize = 512;
const MAX_CHECK_ASK_COUNT: usize = 10;

/// `C4NetIOUDP::iVersion` carried by every reliable-UDP connection request.
pub const RELIABLE_UDP_PROTOCOL_VERSION: u32 = 2;

/// Maximum inner-packet bytes carried by one C++ reliable-UDP data fragment.
pub const RELIABLE_UDP_DATA_PAYLOAD_LIMIT: usize =
    MAX_DATAGRAM_SIZE - DATA_PACKET_HEADER_SIZE;

/// Fields emitted by a unicast `C4NetIOUDP::ConnPacket`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReliableUdpConnect {
    pub packet_number: u32,
    pub address: SocketAddr,
    pub multicast_address: SocketAddr,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ReliableUdpReceiveChannel {
    next_expected_packet_number: u32,
    high_water_packet_number: u32,
    present_packet_numbers: BTreeSet<u32>,
}

impl ReliableUdpReceiveWindow {
    pub fn new(
        next_expected_packet_number: u32,
        next_expected_multicast_packet_number: u32,
    ) -> Self {
        Self {
            direct: ReliableUdpReceiveChannel::new(next_expected_packet_number),
            multicast: ReliableUdpReceiveChannel::new(next_expected_multicast_packet_number),
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

    /// Constructs the C++ acknowledgment counters and bounded missing list.
    pub fn plan_check(&self, outgoing_packet_number: u32) -> ReliableUdpCheck {
        let missing_packet_numbers = self.direct.missing_packet_numbers(MAX_CHECK_ASK_COUNT);
        let missing_multicast_packet_numbers = self
            .multicast
            .missing_packet_numbers(MAX_CHECK_ASK_COUNT - missing_packet_numbers.len());
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

    fn missing_packet_numbers(&self, limit: usize) -> Vec<u32> {
        if limit == 0 || self.next_expected_packet_number >= self.high_water_packet_number {
            return Vec::new();
        }

        let mut missing_packet_numbers = Vec::with_capacity(limit);
        let mut candidate = self.next_expected_packet_number;
        for present_packet_number in self
            .present_packet_numbers
            .range(self.next_expected_packet_number..self.high_water_packet_number)
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

/// Encodes the packed native-endian connection request used by `C4NetIOUDP`.
pub fn encode_reliable_udp_connect(connection: &ReliableUdpConnect) -> Vec<u8> {
    let mut wire = Vec::with_capacity(CONNECT_PACKET_SIZE);
    wire.push(IPID_CONN);
    wire.extend_from_slice(&connection.packet_number.to_ne_bytes());
    wire.extend_from_slice(&RELIABLE_UDP_PROTOCOL_VERSION.to_ne_bytes());
    encode_bin_address(connection.address, &mut wire);
    encode_bin_address(connection.multicast_address, &mut wire);
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
    let multicast_mode = match i32::from_ne_bytes(
        wire[5..9].try_into().expect("checked packet length"),
    ) {
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
        total_size: u32::from_ne_bytes(
            wire[9..13]
                .try_into()
                .expect("checked data header length"),
        ),
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
        address_type => Err(ReliableUdpDecodeError::UnsupportedAddressType(
            address_type,
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

    use super::*;

    #[test]
    fn cpp_conn_encoding_and_conn_ok_decoding_preserve_the_observed_endpoint() {
        // C4NetIOUDP uses packed native-endian headers. Conn carries protocol
        // version 2, destination BinAddr, then multicast BinAddr; ConnOK carries
        // the endpoint that the peer observed for this same UDP socket
        // (pristine 9ffa0a5d src/C4NetIO.cpp:1921-2047, 2861-2968).
        let connection = ReliableUdpConnect {
            packet_number: 0x1122_3344,
            address: SocketAddr::V4(SocketAddrV4::new(
                Ipv4Addr::new(203, 0, 113, 7),
                11_115,
            )),
            multicast_address: SocketAddr::V6(SocketAddrV6::new(
                "ff3e:40:2001:db8::1234".parse::<Ipv6Addr>().unwrap(),
                11_113,
                0,
                0,
            )),
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
        assert_eq!(encode_reliable_udp_connect(&connection), expected_connection);

        let observed_address = SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::new(198, 51, 100, 23),
            62_000,
        ));
        let mut connection_ok = vec![0x03];
        connection_ok.extend_from_slice(&0x5566_7788_u32.to_ne_bytes());
        connection_ok.extend_from_slice(&0_i32.to_ne_bytes());
        connection_ok.extend_from_slice(&62_000_u16.to_ne_bytes());
        connection_ok.push(1);
        connection_ok.extend_from_slice(&[198, 51, 100, 23]);
        connection_ok.extend_from_slice(&[0; 12]);

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
            for (fragment_index, (&fragment_len, wire)) in expected_fragment_lengths
                .iter()
                .zip(&encoded)
                .enumerate()
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
}
