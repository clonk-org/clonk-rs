//! C++ reliable-UDP wire model.

use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6};

use thiserror::Error;

const IPID_CONN: u8 = 0x02;
const IPID_CONN_OK: u8 = 0x03;
const IPID_DATA: u8 = 0x04;
const INTERNAL_PACKET_TYPE_MASK: u8 = 0x7f;
const BIN_ADDR_SIZE: usize = 19;
const CONNECT_PACKET_SIZE: usize = 47;
const CONNECT_OK_PACKET_SIZE: usize = 28;
const DATA_PACKET_HEADER_SIZE: usize = 13;
const MAX_DATAGRAM_SIZE: usize = 512;

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

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ReliableUdpEncodeError {
    #[error("reliable UDP payload length {0} exceeds the C++ uint32 size field")]
    PayloadTooLarge(usize),
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
}
