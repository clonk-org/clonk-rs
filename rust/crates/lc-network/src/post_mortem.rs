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
    post_mortem_sent: bool,
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

    pub fn acknowledge_received(&mut self, next_inbound_packet: u32) {
        if let Some(first_older) = self
            .packets
            .iter()
            .position(|(number, _)| *number < next_inbound_packet)
        {
            self.packets.truncate(first_older);
        }
    }

    pub fn create_post_mortem(&mut self, connection_id: u32) -> Option<PostMortemPacket> {
        if self.packets.is_empty() || self.post_mortem_sent {
            return None;
        }
        self.post_mortem_sent = true;
        Some(PostMortemPacket {
            connection_id,
            packet_counter: self.next_packet_counter,
            packets: self
                .packets
                .iter()
                .rev()
                .map(|(_, packet)| packet.clone())
                .collect(),
        })
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

    #[test]
    fn ping_ack_prunes_only_packets_strictly_below_its_counter() {
        // PID_Ping carries the receiver's next packet counter. ClearPacketLog
        // keeps that numbered packet and newer entries, deleting the first
        // lower number and every older list node (src/C4Network2IO.cpp:
        // 1000-1007,1358-1377).
        let mut log = RecoverablePacketLog::default();
        log.record_outbound(vec![0x10, 0xaa]);
        log.record_outbound(vec![0x11, 0xbb]);
        log.record_outbound(vec![0x12, 0xcc]);

        log.acknowledge_received(0);
        assert_eq!(log.logged_packet_count(), 3);
        log.acknowledge_received(2);
        assert_eq!(log.packets, vec![(2, vec![0x12, 0xcc])]);
        assert_eq!(log.next_packet_counter(), 3);
    }

    #[test]
    fn recovery_packet_preserves_oldest_to_newest_numbering() {
        // CreatePostMortem publishes the dead connection's remote ID and next
        // output counter, reversing its newest-first log back into packet-number
        // order (src/C4Network2IO.cpp:1379-1395,1497-1544).
        let mut log = RecoverablePacketLog::default();
        log.record_outbound(vec![0x10, 0xaa]);
        log.record_outbound(vec![0x40, 0xbb]);

        assert_eq!(
            log.create_post_mortem(0x1122_3344),
            Some(PostMortemPacket {
                connection_id: 0x1122_3344,
                packet_counter: 2,
                packets: vec![vec![0x10, 0xaa], vec![0x40, 0xbb]],
            })
        );
    }

    #[test]
    fn recovery_creation_requires_a_backlog_and_is_one_shot() {
        // CreatePostMortem refuses an empty log and the fPostMortemSent guard
        // prevents a second recovery envelope for the same dead connection
        // (src/C4Network2IO.cpp:1379-1396).
        let mut log = RecoverablePacketLog::default();
        assert_eq!(log.create_post_mortem(7), None);

        log.record_outbound(vec![0x10, 0xaa]);
        assert!(log.create_post_mortem(7).is_some());
        assert_eq!(log.create_post_mortem(7), None);
    }
}
