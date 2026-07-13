//! C++-faithful ping cadence and connection timeout bookkeeping.

use std::cmp;

/// `C4NetTimer` (`src/C4Network2IO.h:34`).
pub const NETWORK_TIMER_INTERVAL_MS: u64 = 500;
/// `C4NetPingFreq` (`src/C4Network2IO.h:35`).
pub const PING_FREQUENCY_MS: u64 = 1_000;
/// `C4NetAcceptTimeout` (`src/C4Network2IO.h:37`).
pub const ACCEPT_TIMEOUT_SECONDS: i64 = 10;
/// `C4NetPingTimeout` (`src/C4Network2IO.h:38`).
pub const PING_TIMEOUT_MS: i64 = 30_000;
/// Packets at or above this ID participate in C++ post-mortem recovery.
pub const PACKET_LOG_START: u8 = 0x04;

const NO_TIMESTAMP: u64 = u64::MAX;
const NO_PING_TIME: i32 = -1;

/// The two clocks used by C++ liveness checks.
///
/// `timeGetTime()` supplies millisecond ping timing, while `time(nullptr)`
/// supplies whole-second acceptance and pre-first-pong timeouts
/// (`src/C4Network2IO.cpp:1155-1177`). Keeping both values explicit prevents a
/// caller from accidentally replacing either C++ boundary with the other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LivenessClock {
    monotonic_ms: u64,
    wall_seconds: i64,
}

impl LivenessClock {
    pub const fn new(monotonic_ms: u64, wall_seconds: i64) -> Self {
        Self {
            monotonic_ms,
            wall_seconds,
        }
    }

    pub const fn monotonic_ms(self) -> u64 {
        self.monotonic_ms
    }

    pub const fn wall_seconds(self) -> i64 {
        self.wall_seconds
    }
}

/// Shared cadence gate corresponding to `C4Network2IO::iLastPing`.
///
/// C++ owns one of these per network-I/O instance, not per connection: when it
/// becomes due, every open connection is pinged (`src/C4Network2IO.cpp:
/// 612-617,1141-1151`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingSchedule {
    last_ping_ms: u64,
}

impl PingSchedule {
    pub const fn new(now_ms: u64) -> Self {
        Self {
            last_ping_ms: now_ms,
        }
    }

    /// Returns and consumes the current due edge.
    ///
    /// This deliberately mirrors the inclusive C++ expression
    /// `!Inside(last, now - C4NetPingFreq, now)`, including unsigned wrapping
    /// and the clock-rollback case (`src/C4Network2IO.cpp:613-616`).
    pub fn take_due(&mut self, now_ms: u64) -> bool {
        let lower_bound = now_ms.wrapping_sub(PING_FREQUENCY_MS);
        let within_current_window = self.last_ping_ms >= lower_bound && self.last_ping_ms <= now_ms;
        if within_current_window {
            false
        } else {
            self.last_ping_ms = now_ms;
            true
        }
    }

    pub const fn last_ping_ms(self) -> u64 {
        self.last_ping_ms
    }
}

/// Wire fields created for each open connection on a ping edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PingProbe {
    pub sent_at: u32,
    pub packet_counter: u32,
}

/// Connection statuses relevant to the C++ timeout decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessPhase {
    Connected,
    HalfAccepted,
    Accepted,
}

/// C++'s two independently reported connection timeout causes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionTimeout {
    Acceptance,
    Ping,
}

/// Per-connection half of the C++ ping/timeout state machine.
///
/// `PingSchedule` is separate because C++ schedules all connections from one
/// global cadence gate. Packet construction and `record_ping_dispatched` are
/// also separate because C++ records the outstanding timestamp after its send
/// attempt, even when that attempt fails (`src/C4Network2IO.cpp:1148-1150`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionLiveness {
    phase: LivenessPhase,
    status_since_wall_seconds: i64,
    ping_time_ms: i32,
    last_ping_ms: u64,
    last_pong_ms: u64,
    inbound_packet_counter: u32,
}

impl ConnectionLiveness {
    pub const fn new_connected(wall_seconds: i64) -> Self {
        Self {
            phase: LivenessPhase::Connected,
            status_since_wall_seconds: wall_seconds,
            ping_time_ms: NO_PING_TIME,
            last_ping_ms: NO_TIMESTAMP,
            last_pong_ms: NO_TIMESTAMP,
            inbound_packet_counter: 0,
        }
    }

    pub const fn phase(&self) -> LivenessPhase {
        self.phase
    }

    /// Mirrors the C++ transition to `CS_HalfAccepted`, which does not refresh
    /// the acceptance timestamp (`src/C4Network2IO.cpp:1343-1354`).
    pub fn mark_half_accepted(&mut self) {
        self.phase = LivenessPhase::HalfAccepted;
    }

    /// Mirrors the transition to `CS_Accepted`, including its timestamp reset.
    pub fn mark_accepted(&mut self, wall_seconds: i64) {
        self.phase = LivenessPhase::Accepted;
        self.status_since_wall_seconds = wall_seconds;
    }

    /// Accounts for a received packet exactly as `OnPacketReceived` does.
    pub fn record_inbound_packet(&mut self, packet_type: u8) {
        if packet_type >= PACKET_LOG_START {
            self.inbound_packet_counter = self.inbound_packet_counter.wrapping_add(1);
        }
    }

    pub const fn inbound_packet_counter(&self) -> u32 {
        self.inbound_packet_counter
    }

    /// Constructs `C4PacketPing`'s wire data without changing liveness state.
    pub const fn make_ping(&self, now_ms: u64) -> PingProbe {
        PingProbe {
            sent_at: now_ms as u32,
            packet_counter: self.inbound_packet_counter,
        }
    }

    /// Records `OnPing` after the caller attempted to dispatch a probe.
    pub fn record_ping_dispatched(&mut self, now_ms: u64) {
        if self.last_pong_ms < self.last_ping_ms {
            return;
        }
        self.last_ping_ms = now_ms;
    }

    /// Records an echoed pong and returns the C++ `iPingTime` value.
    ///
    /// The echoed timestamp is intentionally not matched against the current
    /// outstanding ping: C++ accepts any open connection's `PID_Pong` and uses
    /// its wrapping 32-bit travel-time subtraction (`src/C4Network2IO.cpp:
    /// 1021-1027,1704-1710`).
    pub fn record_pong(&mut self, echoed_sent_at: u32, now_ms: u64) -> i32 {
        let travel_time = (now_ms as u32).wrapping_sub(echoed_sent_at) as i32;
        self.ping_time_ms = travel_time;
        self.last_pong_ms = now_ms;
        travel_time
    }

    pub fn measured_ping_ms(&self) -> Option<i32> {
        (self.ping_time_ms != NO_PING_TIME).then_some(self.ping_time_ms)
    }

    /// Returns the first currently unanswered ping timestamp, if observable.
    pub fn outstanding_ping_since_ms(&self) -> Option<u64> {
        (self.last_ping_ms != NO_TIMESTAMP
            && (self.ping_time_ms == NO_PING_TIME || self.last_ping_ms > self.last_pong_ms))
            .then_some(self.last_ping_ms)
    }

    /// Mirrors `C4Network2IOConnection::getLag`.
    pub fn lag_ms(&self, now_ms: u64) -> Option<i32> {
        if self.ping_time_ms != NO_PING_TIME
            && self.last_ping_ms != NO_TIMESTAMP
            && (self.last_pong_ms == NO_TIMESTAMP || self.last_ping_ms > self.last_pong_ms)
        {
            let unanswered_ms = now_ms.wrapping_sub(self.last_ping_ms) as i32;
            Some(cmp::max(unanswered_ms, self.ping_time_ms))
        } else {
            self.measured_ping_ms()
        }
    }

    /// Applies C++'s strict acceptance or ping timeout boundary.
    pub fn check_timeout(&self, now: LivenessClock) -> Option<ConnectionTimeout> {
        match self.phase {
            LivenessPhase::Connected | LivenessPhase::HalfAccepted => {
                let age_seconds = now
                    .wall_seconds
                    .saturating_sub(self.status_since_wall_seconds);
                (age_seconds > ACCEPT_TIMEOUT_SECONDS).then_some(ConnectionTimeout::Acceptance)
            }
            LivenessPhase::Accepted => {
                let effective_lag_ms = self.lag_ms(now.monotonic_ms).map_or_else(
                    || {
                        now.wall_seconds
                            .saturating_sub(self.status_since_wall_seconds)
                            .saturating_mul(1_000)
                    },
                    i64::from,
                );
                (effective_lag_ms > PING_TIMEOUT_MS).then_some(ConnectionTimeout::Ping)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock(monotonic_ms: u64, wall_seconds: i64) -> LivenessClock {
        LivenessClock::new(monotonic_ms, wall_seconds)
    }

    fn accepted(at: LivenessClock) -> ConnectionLiveness {
        let mut connection = ConnectionLiveness::new_connected(at.wall_seconds());
        connection.mark_accepted(at.wall_seconds());
        connection
    }

    #[test]
    fn ping_cadence_is_strictly_later_than_one_second() {
        // C4Network2IO::Execute uses inclusive Inside(last, now - 1000, now),
        // so equality at 1000 ms is not due (src/C4Network2IO.cpp:612-617;
        // src/C4Math.h:22).
        let mut schedule = PingSchedule::new(10_000);

        assert!(!schedule.take_due(10_999));
        assert!(!schedule.take_due(11_000));
        assert!(schedule.take_due(11_001));
        assert!(!schedule.take_due(12_001));
        assert!(schedule.take_due(12_002));
    }

    #[test]
    fn ping_schedule_treats_a_clock_rollback_as_due() {
        // A last-ping value outside the inclusive [now - frequency, now]
        // range is due in C++ (src/C4Network2IO.cpp:613-616).
        let mut schedule = PingSchedule::new(2_000);

        assert!(schedule.take_due(1_900));
        assert_eq!(schedule.last_ping_ms(), 1_900);
    }

    #[test]
    fn probe_carries_only_recoverable_inbound_packet_count() {
        // OnPacketReceived increments at PID_PacketLogStart (0x04), while the
        // outgoing ping snapshots that counter (src/C4PacketBase.h:101-102;
        // src/C4Network2IO.cpp:1148,1362-1366).
        let mut connection = ConnectionLiveness::new_connected(0);
        connection.record_inbound_packet(0x03);
        connection.record_inbound_packet(0x04);
        connection.record_inbound_packet(0x40);

        assert_eq!(
            connection.make_ping(0x1_0000_0005),
            PingProbe {
                sent_at: 5,
                packet_counter: 2,
            }
        );
    }

    #[test]
    fn packet_counter_wraps_like_cpp_uint32() {
        let mut connection = ConnectionLiveness::new_connected(0);
        connection.inbound_packet_counter = u32::MAX;

        connection.record_inbound_packet(PACKET_LOG_START);

        assert_eq!(connection.inbound_packet_counter(), 0);
    }

    #[test]
    fn before_first_pong_each_ping_replaces_the_timestamp() {
        // The C++ ~0 sentinels make OnPing refresh until the first pong has
        // arrived (src/C4Network2IO.cpp:1259-1267,1326-1333).
        let mut connection = accepted(clock(0, 0));

        connection.record_ping_dispatched(1_000);
        connection.record_ping_dispatched(2_000);

        assert_eq!(connection.outstanding_ping_since_ms(), Some(2_000));
        assert_eq!(connection.lag_ms(40_000), None);
    }

    #[test]
    fn after_first_pong_the_first_unanswered_ping_is_retained() {
        // Once a pong exists, OnPing refuses to move iLastPing while it is
        // newer than iLastPong (src/C4Network2IO.cpp:1326-1333).
        let mut connection = accepted(clock(0, 0));
        connection.record_ping_dispatched(1_000);
        connection.record_pong(1_000, 1_100);

        connection.record_ping_dispatched(2_000);
        connection.record_ping_dispatched(3_000);

        assert_eq!(connection.outstanding_ping_since_ms(), Some(2_000));
        assert_eq!(connection.lag_ms(2_050), Some(100));
        assert_eq!(connection.lag_ms(2_101), Some(101));
    }

    #[test]
    fn pong_rtt_uses_wrapping_wire_timestamp_subtraction() {
        // C4PacketPing::getTravelTime subtracts uint32 timestamps
        // (src/C4Network2IO.cpp:1704-1710).
        let mut connection = accepted(clock(0, 0));

        connection.record_pong(u32::MAX - 15, 0x1_0000_0010);

        assert_eq!(connection.measured_ping_ms(), Some(32));
    }

    #[test]
    fn acceptance_timeout_is_strict_and_half_accept_does_not_reset_it() {
        // HalfAccepted is absent from SetStatus's timestamp-reset list and the
        // accept timeout uses difftime(...) > 10 (src/C4Network2IO.cpp:
        // 1155-1169,1343-1354).
        let mut connection = ConnectionLiveness::new_connected(100);
        connection.mark_half_accepted();

        assert_eq!(connection.check_timeout(clock(0, 110)), None);
        assert_eq!(
            connection.check_timeout(clock(0, 111)),
            Some(ConnectionTimeout::Acceptance)
        );
    }

    #[test]
    fn accepted_without_a_pong_uses_wall_clock_age() {
        // Until iPingTime changes from -1, CheckTimeout falls back to whole
        // wall-clock seconds since acceptance (src/C4Network2IO.cpp:1170-1177;
        // src/C4Network2IO.cpp:1283-1295).
        let connection = accepted(clock(5_000, 100));

        assert_eq!(connection.check_timeout(clock(99_000, 130)), None);
        assert_eq!(
            connection.check_timeout(clock(99_000, 131)),
            Some(ConnectionTimeout::Ping)
        );
    }

    #[test]
    fn measured_ping_timeout_is_strictly_greater_than_thirty_seconds() {
        // getLag grows from the first unanswered ping and CheckTimeout uses
        // > C4NetPingTimeout, not >= (src/C4Network2IO.cpp:1170-1177,
        // 1283-1295).
        let mut connection = accepted(clock(0, 100));
        connection.record_ping_dispatched(1_000);
        connection.record_pong(1_000, 1_100);
        connection.record_ping_dispatched(2_000);
        connection.record_ping_dispatched(3_000);

        assert_eq!(connection.check_timeout(clock(32_000, 101)), None);
        assert_eq!(
            connection.check_timeout(clock(32_001, 101)),
            Some(ConnectionTimeout::Ping)
        );
    }

    #[test]
    fn non_pong_traffic_does_not_keep_a_connection_alive() {
        // Only PID_Pong updates iLastPong/iPingTime; general packet receipt
        // merely advances the recovery counter (src/C4Network2IO.cpp:
        // 594-597,1021-1027,1362-1366).
        let mut connection = accepted(clock(0, 100));
        connection.record_ping_dispatched(1_000);
        connection.record_pong(1_000, 1_100);
        connection.record_ping_dispatched(2_000);
        connection.record_inbound_packet(0x40);

        assert_eq!(
            connection.check_timeout(clock(32_001, 101)),
            Some(ConnectionTimeout::Ping)
        );
    }
}
