//! C++ netpuncher packet domain model.

use std::net::SocketAddr;

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
