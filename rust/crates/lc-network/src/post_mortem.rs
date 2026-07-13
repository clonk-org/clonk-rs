/// Recoverable packets rerouted after one connection to a peer closes.
///
/// `packet_counter` is the dead connection's next outbound packet number, and
/// `packets` is ordered from `packet_counter - packets.len()` through
/// `packet_counter - 1`. Each entry retains the complete `C4NetIOPacket`,
/// including its packet-ID byte (`src/C4Network2IO.cpp:1379-1395,1497-1586`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostMortemPacket {
    pub connection_id: u32,
    pub packet_counter: u32,
    pub packets: Vec<Vec<u8>>,
}
