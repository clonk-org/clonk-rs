//! C++ netpuncher packet domain model.

use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};

use thiserror::Error;

const PID_ASSIGN_ID: u8 = 0x51;
const PID_SERVER_REQUEST: u8 = 0x52;
const PID_CLIENT_REQUEST: u8 = 0x53;
const PID_ID_REQUEST: u8 = 0x54;
const PID_PONG: u8 = 0x01;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetpuncherRole {
    Host,
    Client,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetpuncherRuntimeState {
    Initializing,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetpuncherAddressFamily {
    Ipv4,
    Ipv6,
}

/// Events which cross C4Network2IO's puncher-address boundary without being
/// mistaken for ordinary game-peer traffic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NetpuncherIoEvent {
    Connected {
        family: NetpuncherAddressFamily,
        puncher_address: SocketAddr,
        observed_address: SocketAddr,
    },
    Packet {
        family: NetpuncherAddressFamily,
        puncher_address: SocketAddr,
        packet: NetpuncherPacket,
    },
}

impl NetpuncherIoEvent {
    pub const fn family(&self) -> NetpuncherAddressFamily {
        match self {
            Self::Connected { family, .. } | Self::Packet { family, .. } => *family,
        }
    }

    pub const fn puncher_address(&self) -> SocketAddr {
        match self {
            Self::Connected {
                puncher_address, ..
            }
            | Self::Packet {
                puncher_address, ..
            } => *puncher_address,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NetpuncherGameIds {
    pub ipv4: u32,
    pub ipv6: u32,
}

/// Chooses the packet sent when a network runtime connects to the netpuncher.
///
/// Mirrors the role, game-state, and address-family branches in
/// `C4Network2::OnPuncherConnect` (`src/C4Network2.cpp:1057-1082`).
pub fn reduce_puncher_connect(
    role: NetpuncherRole,
    state: NetpuncherRuntimeState,
    family: NetpuncherAddressFamily,
    game_ids: NetpuncherGameIds,
) -> Option<NetpuncherPacket> {
    match role {
        NetpuncherRole::Host => Some(NetpuncherPacket::IdRequest),
        NetpuncherRole::Client if state == NetpuncherRuntimeState::Initializing => {
            let id = match family {
                NetpuncherAddressFamily::Ipv4 => game_ids.ipv4,
                NetpuncherAddressFamily::Ipv6 => game_ids.ipv6,
            };
            (id != 0).then_some(NetpuncherPacket::ServerRequest { id })
        }
        NetpuncherRole::Client => None,
    }
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

/// Builds the raw application-level `PID_Pong + C4PacketPing` datagram used
/// for NAT hole punching. This deliberately bypasses reliable-UDP framing;
/// its leading `0x01` is consequently filtered as `IPID_Test` by the remote
/// reliable layer after it has opened the NAT mapping.
pub fn encode_netpuncher_punch(sent_at_ms: u32) -> [u8; 9] {
    let mut wire = [0_u8; 9];
    wire[0] = PID_PONG;
    wire[1..5].copy_from_slice(&sent_at_ms.to_ne_bytes());
    wire[5..9].copy_from_slice(&0_u32.to_ne_bytes());
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

    #[test]
    fn cpp_puncher_connect_role_state_and_family_choose_the_outbound_request() {
        // C4Network2::OnPuncherConnect always requests an ID for hosts. Clients
        // request server registration only during GS_Init and only when the
        // connected address family's game ID is nonzero
        // (pristine 9ffa0a5d src/C4Network2.cpp:1057-1082).
        for (case, role, state, family, game_ids, expected) in [
            (
                "host ignores state and missing IDs",
                NetpuncherRole::Host,
                NetpuncherRuntimeState::Other,
                NetpuncherAddressFamily::Ipv4,
                NetpuncherGameIds { ipv4: 0, ipv6: 0 },
                Some(NetpuncherPacket::IdRequest),
            ),
            (
                "initializing IPv4 client registers its IPv4 ID",
                NetpuncherRole::Client,
                NetpuncherRuntimeState::Initializing,
                NetpuncherAddressFamily::Ipv4,
                NetpuncherGameIds {
                    ipv4: 0x1122_3344,
                    ipv6: 0x5566_7788,
                },
                Some(NetpuncherPacket::ServerRequest { id: 0x1122_3344 }),
            ),
            (
                "initializing IPv6 client registers its IPv6 ID",
                NetpuncherRole::Client,
                NetpuncherRuntimeState::Initializing,
                NetpuncherAddressFamily::Ipv6,
                NetpuncherGameIds {
                    ipv4: 0x1122_3344,
                    ipv6: 0x5566_7788,
                },
                Some(NetpuncherPacket::ServerRequest { id: 0x5566_7788 }),
            ),
            (
                "client does not substitute the other family's ID",
                NetpuncherRole::Client,
                NetpuncherRuntimeState::Initializing,
                NetpuncherAddressFamily::Ipv4,
                NetpuncherGameIds {
                    ipv4: 0,
                    ipv6: 0x5566_7788,
                },
                None,
            ),
            (
                "client outside initialization emits nothing",
                NetpuncherRole::Client,
                NetpuncherRuntimeState::Other,
                NetpuncherAddressFamily::Ipv6,
                NetpuncherGameIds {
                    ipv4: 0x1122_3344,
                    ipv6: 0x5566_7788,
                },
                None,
            ),
        ] {
            assert_eq!(
                reduce_puncher_connect(role, state, family, game_ids),
                expected,
                "{case}"
            );
        }
    }

    #[test]
    fn cpp_punch_is_one_raw_pong_with_a_default_packet_counter() {
        let wire = encode_netpuncher_punch(0x0102_0304);
        assert_eq!(wire[0], 0x01);
        assert_eq!(&wire[1..5], &0x0102_0304_u32.to_ne_bytes());
        assert_eq!(&wire[5..9], &0_u32.to_ne_bytes());
    }
}
