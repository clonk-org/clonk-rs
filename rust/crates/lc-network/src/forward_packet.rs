//! C++-faithful packet-forwarding wire model.

/// `C4PacketType::PID_FwdReq` (`src/C4PacketBase.h:95`).
pub const PID_FORWARD_REQUEST: u8 = 0x04;
/// `C4PacketType::PID_Fwd` (`src/C4PacketBase.h:96`).
pub const PID_FORWARD: u8 = 0x05;
/// Capacity of `C4PacketFwd::iClients` (`src/C4Network2IO.h:41,397`).
pub const MAX_FORWARD_CLIENTS: usize = 256;

/// Shared body of `PID_FwdReq` and `PID_Fwd`.
///
/// `nested_packet` is the complete forwarded `C4NetIOPacket`, starting with
/// its packet ID. It deliberately remains opaque at this transport boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardPacket {
    pub negative_list: bool,
    pub clients: Vec<i32>,
    pub nested_packet: Vec<u8>,
}
