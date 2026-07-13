//! C++ netpuncher packet domain model.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};

use thiserror::Error;

const PID_ASSIGN_ID: u8 = 0x51;
const PID_SERVER_REQUEST: u8 = 0x52;
const PID_CLIENT_REQUEST: u8 = 0x53;
const PID_ID_REQUEST: u8 = 0x54;

/// Version byte following every `C4NetpuncherPacketType`.
pub const NETPUNCHER_PROTOCOL_VERSION: u8 = 1;

/// Payload variants exchanged with the UDP netpuncher.
///
/// Mirrors `C4NetpuncherPacketAssID`, `SReq`, `CReq`, and `IDReq`
/// (`src/C4PuncherPacket.h:24-31, 57-107`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetpuncherPacket {
    AssignId { id: u32 },
    ServerRequest { id: u32 },
    ClientRequest { address: SocketAddr },
    IdRequest,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum NetpuncherPacketDecodeError {
    #[error("netpuncher packet is truncated")]
    Truncated,
    #[error("unsupported netpuncher packet type 0x{0:02x}")]
    UnsupportedType(u8),
    #[error("unsupported netpuncher protocol version {0}")]
    UnsupportedVersion(u8),
}

pub fn encode_netpuncher_packet(packet: &NetpuncherPacket) -> Vec<u8> {
    let mut wire = Vec::with_capacity(20);
    match packet {
        NetpuncherPacket::AssignId { id } => {
            wire.extend_from_slice(&[PID_ASSIGN_ID, NETPUNCHER_PROTOCOL_VERSION]);
            wire.extend_from_slice(&id.to_ne_bytes());
        }
        NetpuncherPacket::ServerRequest { id } => {
            wire.extend_from_slice(&[PID_SERVER_REQUEST, NETPUNCHER_PROTOCOL_VERSION]);
            wire.extend_from_slice(&id.to_ne_bytes());
        }
        NetpuncherPacket::ClientRequest { address } => {
            wire.extend_from_slice(&[PID_CLIENT_REQUEST, NETPUNCHER_PROTOCOL_VERSION]);
            wire.extend_from_slice(&address.port().to_ne_bytes());
            let ip = match address {
                SocketAddr::V4(address) => address.ip().to_ipv6_mapped(),
                SocketAddr::V6(address) => *address.ip(),
            };
            wire.extend_from_slice(&ip.octets());
        }
        NetpuncherPacket::IdRequest => {
            wire.extend_from_slice(&[PID_ID_REQUEST, NETPUNCHER_PROTOCOL_VERSION]);
        }
    }
    wire
}

pub fn decode_netpuncher_packet(
    wire: &[u8],
) -> Result<NetpuncherPacket, NetpuncherPacketDecodeError> {
    let packet_type = *wire
        .first()
        .ok_or(NetpuncherPacketDecodeError::Truncated)?;
    let version = *wire.get(1).ok_or(NetpuncherPacketDecodeError::Truncated)?;
    if version != NETPUNCHER_PROTOCOL_VERSION {
        return Err(NetpuncherPacketDecodeError::UnsupportedVersion(version));
    }
    let payload = &wire[2..];
    match packet_type {
        PID_ASSIGN_ID | PID_SERVER_REQUEST => {
            let bytes: [u8; 4] = payload
                .get(..4)
                .ok_or(NetpuncherPacketDecodeError::Truncated)?
                .try_into()
                .map_err(|_| NetpuncherPacketDecodeError::Truncated)?;
            let id = u32::from_ne_bytes(bytes);
            Ok(if packet_type == PID_ASSIGN_ID {
                NetpuncherPacket::AssignId { id }
            } else {
                NetpuncherPacket::ServerRequest { id }
            })
        }
        PID_CLIENT_REQUEST => {
            let port_bytes: [u8; 2] = payload
                .get(..2)
                .ok_or(NetpuncherPacketDecodeError::Truncated)?
                .try_into()
                .map_err(|_| NetpuncherPacketDecodeError::Truncated)?;
            let ip_bytes: [u8; 16] = payload
                .get(2..18)
                .ok_or(NetpuncherPacketDecodeError::Truncated)?
                .try_into()
                .map_err(|_| NetpuncherPacketDecodeError::Truncated)?;
            Ok(NetpuncherPacket::ClientRequest {
                address: SocketAddr::V6(SocketAddrV6::new(
                    Ipv6Addr::from(ip_bytes),
                    u16::from_ne_bytes(port_bytes),
                    0,
                    0,
                )),
            })
        }
        PID_ID_REQUEST => Ok(NetpuncherPacket::IdRequest),
        packet_type => Err(NetpuncherPacketDecodeError::UnsupportedType(packet_type)),
    }
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV6};

    use super::*;

    #[test]
    fn cpp_request_and_response_vectors_use_type_version_and_native_fields() {
        // PackTo writes type 0x51..0x54, version 1, then native uint32 IDs or
        // native uint16 port + the 16-byte IPv6/mapped address
        // (pristine 9ffa0a5d src/C4PuncherPacket.h:24-31;
        // src/C4PuncherPacket.cpp:25-27, 59-72, 75-111).
        let id = 0x1122_3344_u32;
        let client_address = SocketAddr::V6(SocketAddrV6::new(
            Ipv4Addr::new(192, 0, 2, 1).to_ipv6_mapped(),
            11_115,
            0,
            0,
        ));
        let mut assign_id = vec![0x51, NETPUNCHER_PROTOCOL_VERSION];
        assign_id.extend_from_slice(&id.to_ne_bytes());
        let mut server_request = vec![0x52, NETPUNCHER_PROTOCOL_VERSION];
        server_request.extend_from_slice(&id.to_ne_bytes());
        let mut client_request = vec![0x53, NETPUNCHER_PROTOCOL_VERSION];
        client_request.extend_from_slice(&11_115_u16.to_ne_bytes());
        let client_ip = match client_address {
            SocketAddr::V6(address) => *address.ip(),
            SocketAddr::V4(_) => unreachable!(),
        };
        client_request.extend_from_slice(&client_ip.octets());

        for (packet, wire) in [
            (NetpuncherPacket::AssignId { id }, assign_id),
            (NetpuncherPacket::ServerRequest { id }, server_request),
            (
                NetpuncherPacket::ClientRequest {
                    address: client_address,
                },
                client_request,
            ),
            (
                NetpuncherPacket::IdRequest,
                vec![0x54, NETPUNCHER_PROTOCOL_VERSION],
            ),
        ] {
            assert_eq!(encode_netpuncher_packet(&packet), wire);
            assert_eq!(decode_netpuncher_packet(&wire).unwrap(), packet);
        }
    }
}
