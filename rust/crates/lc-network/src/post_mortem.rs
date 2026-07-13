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

/// Per-connection copy of C++'s recoverable outbound packet backlog.
#[derive(Debug, Default)]
pub struct RecoverablePacketLog {
    next_packet_counter: u32,
    packets: Vec<(u32, Vec<u8>)>,
}

impl RecoverablePacketLog {
    pub fn record_outbound(&mut self, packet: Vec<u8>) -> Option<u32> {
        if packet
            .first()
            .is_none_or(|packet_type| *packet_type < crate::PACKET_LOG_START)
        {
            return None;
        }
        let number = self.next_packet_counter;
        self.next_packet_counter = self.next_packet_counter.wrapping_add(1);
        self.packets.insert(0, (number, packet));
        Some(number)
    }

    pub const fn next_packet_counter(&self) -> u32 {
        self.next_packet_counter
    }

    pub fn logged_packet_count(&self) -> usize {
        self.packets.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_log_numbers_only_cpp_recoverable_packet_ids() {
        // C4Network2IOConnection::Send logs and numbers packets starting at
        // PID_PacketLogStart (0x04); lower IDs bypass the log entirely
        // (src/C4PacketBase.h:98-102; src/C4Network2IO.cpp:1426-1442).
        let mut log = RecoverablePacketLog::default();

        assert_eq!(log.record_outbound(Vec::new()), None);
        assert_eq!(log.record_outbound(vec![0x03, 0xaa]), None);
        assert_eq!(log.record_outbound(vec![0x04, 0xbb]), Some(0));
        assert_eq!(log.record_outbound(vec![0x40, 0xcc]), Some(1));
        assert_eq!(log.next_packet_counter(), 2);
        assert_eq!(log.logged_packet_count(), 2);
    }
}
