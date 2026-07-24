use std::collections::{BTreeMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ClientId;

fn wall_clock_seconds() -> i64 {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    i64::try_from(seconds).unwrap_or(i64::MAX)
}

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

impl PostMortemPacket {
    pub fn packets_from(&self, next_inbound_packet: u32) -> &[Vec<u8>] {
        let Ok(packet_count) = u32::try_from(self.packets.len()) else {
            return &[];
        };
        let first = self.packet_counter.wrapping_sub(packet_count);
        let last = self.packet_counter.wrapping_sub(1);
        if next_inbound_packet < first || next_inbound_packet > last {
            return &[];
        }
        let offset = next_inbound_packet
            .wrapping_add(packet_count)
            .wrapping_sub(self.packet_counter) as usize;
        self.packets.get(offset..).unwrap_or(&[])
    }
}

pub(crate) fn retain_post_failure_packet(
    post_mortem: &mut Option<PostMortemPacket>,
    connection_id: u32,
    next_packet_counter: &mut u32,
    packet: Vec<u8>,
) {
    if packet
        .first()
        .is_none_or(|packet_type| *packet_type < crate::PACKET_LOG_START)
    {
        return;
    }
    let recovery = post_mortem.get_or_insert_with(|| PostMortemPacket {
        connection_id,
        packet_counter: *next_packet_counter,
        packets: Vec::new(),
    });
    debug_assert_eq!(recovery.connection_id, connection_id);
    recovery.packets.push(packet);
    *next_packet_counter = next_packet_counter.wrapping_add(1);
    recovery.packet_counter = *next_packet_counter;
}

#[derive(Debug, Clone, PartialEq, Eq)]
// The session router consumes this in the next multi-route integration slice.
#[allow(dead_code)]
pub(crate) struct PostMortemReplay {
    pub connection_id: u32,
    pub client_id: ClientId,
    pub packets: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
struct ClosedConnection {
    client_id: ClientId,
    next_inbound_packet: u32,
    retained_at_seconds: i64,
}

#[derive(Debug, Default)]
#[allow(dead_code)]
pub(crate) struct ClosedConnectionRouter {
    connections: BTreeMap<u32, ClosedConnection>,
}

#[allow(dead_code)]
impl ClosedConnectionRouter {
    pub fn retain(
        &mut self,
        local_connection_id: u32,
        client_id: ClientId,
        next_inbound_packet: u32,
    ) {
        self.retain_at(
            local_connection_id,
            client_id,
            next_inbound_packet,
            wall_clock_seconds(),
        );
    }

    fn retain_at(
        &mut self,
        local_connection_id: u32,
        client_id: ClientId,
        next_inbound_packet: u32,
        retained_at_seconds: i64,
    ) {
        self.connections.insert(
            local_connection_id,
            ClosedConnection {
                client_id,
                next_inbound_packet,
                retained_at_seconds,
            },
        );
    }

    pub fn expire(&mut self) {
        self.expire_at(wall_clock_seconds());
    }

    fn expire_at(&mut self, now_seconds: i64) {
        self.connections.retain(|_, connection| {
            now_seconds.saturating_sub(connection.retained_at_seconds)
                <= crate::ACCEPT_TIMEOUT_SECONDS
        });
    }

    pub fn contains(&self, local_connection_id: u32) -> bool {
        self.connections.contains_key(&local_connection_id)
    }

    pub fn client_id(&self, local_connection_id: u32) -> Option<ClientId> {
        self.connections
            .get(&local_connection_id)
            .map(|connection| connection.client_id)
    }

    pub fn remove_client(&mut self, client_id: ClientId) {
        self.connections
            .retain(|_, connection| connection.client_id != client_id);
    }

    pub fn recover(&mut self, packet: &PostMortemPacket) -> Option<PostMortemReplay> {
        self.recover_at(packet, wall_clock_seconds())
    }

    fn recover_at(
        &mut self,
        packet: &PostMortemPacket,
        now_seconds: i64,
    ) -> Option<PostMortemReplay> {
        self.expire_at(now_seconds);
        self.connections
            .remove(&packet.connection_id)
            .map(|connection| PostMortemReplay {
                connection_id: packet.connection_id,
                client_id: connection.client_id,
                packets: packet.packets_from(connection.next_inbound_packet).to_vec(),
            })
    }
}

/// Per-connection copy of C++'s recoverable outbound packet backlog.
#[derive(Debug, Default)]
pub struct RecoverablePacketLog {
    next_packet_counter: u32,
    packets: VecDeque<(u32, Vec<u8>)>,
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
        // Native PacketLogEntry prepends one linked-list node in O(1)
        // (oracle-src-pinned src/C4Network2IO.h:251-260;
        // src/C4Network2IO.cpp:1470-1476).
        self.packets.push_front((number, packet));
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
        assert_eq!(log.packets, VecDeque::from([(2, vec![0x12, 0xcc])]));
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

    #[test]
    fn packet_log_prepends_without_relocating_the_retained_backlog() {
        const PACKET_COUNT: usize = 4_096;

        // Native PacketLogEntry prepends one linked-list node in O(1);
        // draining a large accepted route queue must not repeatedly relocate
        // the complete retained suffix (oracle-src-pinned
        // src/C4Network2IO.h:251-260;
        // src/C4Network2IO.cpp:1383-1400,1470-1476).
        let mut log = RecoverablePacketLog::default();
        log.packets.reserve_exact(PACKET_COUNT);
        log.record_outbound(vec![crate::PACKET_LOG_START, 0]);
        let oldest_address = std::ptr::from_ref(&log.packets[0]);
        let reserved_capacity = log.packets.capacity();

        for value in 1..PACKET_COUNT {
            log.record_outbound(vec![crate::PACKET_LOG_START, value as u8]);
        }

        assert_eq!(log.packets.capacity(), reserved_capacity);
        assert!(std::ptr::eq(
            oldest_address,
            std::ptr::from_ref(&log.packets[PACKET_COUNT - 1])
        ));
    }

    #[test]
    fn recovery_starts_at_the_dead_connections_next_inbound_counter() {
        // PID_PostMortem handling asks getPacket(iInPacketCounter) and walks
        // consecutive numbers until the envelope no longer contains one
        // (src/C4Network2IO.cpp:1036-1055,1516-1529).
        let recovery = PostMortemPacket {
            connection_id: 77,
            packet_counter: 7,
            packets: vec![vec![0x10, 0x04], vec![0x10, 0x05], vec![0x10, 0x06]],
        };

        assert_eq!(
            recovery.packets_from(5),
            [vec![0x10, 0x05], vec![0x10, 0x06]]
        );
        assert!(recovery.packets_from(3).is_empty());
        assert!(recovery.packets_from(7).is_empty());
    }

    #[test]
    fn recovery_router_uses_and_removes_the_matching_closed_connection() {
        // PID_PostMortem looks up the retained dead connection by the sender's
        // local ConnID, replays only from that connection's iInPacketCounter,
        // and then removes exactly that connection from the IO list
        // (src/C4Network2IO.cpp:520-570,1036-1055).
        let mut router = ClosedConnectionRouter::default();
        router.retain_at(7, 3, 5, 100);
        router.retain_at(8, 4, 0, 100);
        let recovery = PostMortemPacket {
            connection_id: 7,
            packet_counter: 7,
            packets: vec![vec![0x10, 0x04], vec![0x10, 0x05], vec![0x10, 0x06]],
        };

        assert_eq!(
            router.recover_at(&recovery, 110),
            Some(PostMortemReplay {
                connection_id: 7,
                client_id: 3,
                packets: vec![vec![0x10, 0x05], vec![0x10, 0x06]],
            })
        );
        assert!(!router.contains(7));
        assert!(router.contains(8));
        assert_eq!(router.recover_at(&recovery, 110), None);
    }

    #[test]
    fn recovery_router_expires_only_after_ten_whole_seconds() {
        // CheckTimeout removes a closed connection only when the whole-second
        // difftime is strictly greater than C4NetAcceptTimeout (10 seconds).
        // A late PID_PostMortem then misses GetConnectionByID and is ignored
        // (src/C4Network2IO.cpp:1047-1052,1177-1181).
        let mut router = ClosedConnectionRouter::default();
        router.retain_at(7, 3, 5, 100);
        let recovery = PostMortemPacket {
            connection_id: 7,
            packet_counter: 7,
            packets: vec![vec![0x10, 0x05], vec![0x10, 0x06]],
        };

        router.expire_at(110);
        assert!(router.contains(7));
        assert_eq!(router.recover_at(&recovery, 111), None);
        assert!(!router.contains(7));
    }

    #[test]
    fn recovery_router_removes_every_connection_for_a_removed_client() {
        let mut router = ClosedConnectionRouter::default();
        router.retain(7, 3, 5);
        router.retain(8, 4, 0);
        router.retain(9, 3, 2);

        router.remove_client(3);

        assert!(!router.contains(7));
        assert!(router.contains(8));
        assert!(!router.contains(9));
    }
}
