//! C++-compatible per-connection and per-protocol network I/O rates.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::NetworkProtocol;

/// `C4NetStatisticsFreq` (`src/C4Network2IO.h:36`).
pub const NETWORK_STATISTICS_INTERVAL_MS: u64 = 1_000;
/// Header allowance charged for every successful TCP send/receive call.
pub const TCP_STATISTICS_HEADER_BYTES: u64 = 52;
/// Header allowance charged for every reliable-UDP datagram.
pub const UDP_STATISTICS_HEADER_BYTES: u64 = 32;

/// Stable identity of one C4Network2IO connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ConnectionStatisticsKey {
    pub connection_id: u32,
    pub protocol: NetworkProtocol,
}

impl ConnectionStatisticsKey {
    pub const fn new(connection_id: u32, protocol: NetworkProtocol) -> Self {
        Self {
            connection_id,
            protocol,
        }
    }
}

/// Cached result of the most recent statistics interval for one connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConnectionRateStatistics {
    pub input_rate: u64,
    pub output_rate: u64,
    pub packet_loss: u32,
    pub ping_ms: i32,
    pub lag_ms: i32,
}

impl Default for ConnectionRateStatistics {
    fn default() -> Self {
        Self {
            input_rate: 0,
            output_rate: 0,
            packet_loss: 0,
            ping_ms: -1,
            lag_ms: -1,
        }
    }
}

/// Cached result of the most recent statistics interval for one protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProtocolRateStatistics {
    pub input_rate: u64,
    pub output_rate: u64,
    pub broadcast_rate: u64,
}

/// Immutable view consumed by the network statistics/graph layer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NetworkIoStatisticsSnapshot {
    pub tcp: ProtocolRateStatistics,
    pub udp: ProtocolRateStatistics,
    pub connections: Vec<(ConnectionStatisticsKey, ConnectionRateStatistics)>,
}

impl NetworkIoStatisticsSnapshot {
    /// Mirrors C++'s accessors: every non-TCP value selects the UDP bucket.
    pub const fn protocol(&self, protocol: NetworkProtocol) -> ProtocolRateStatistics {
        match protocol {
            NetworkProtocol::Tcp => self.tcp,
            NetworkProtocol::Udp | NetworkProtocol::Unknown(_) => self.udp,
        }
    }

    pub fn connection(&self, key: ConnectionStatisticsKey) -> Option<ConnectionRateStatistics> {
        self.connections
            .iter()
            .find_map(|(candidate, statistics)| (*candidate == key).then_some(*statistics))
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct RawConnectionStatistics {
    input_bytes: u64,
    output_bytes: u64,
    packet_loss: u32,
}

#[derive(Clone, Copy, Debug)]
struct ConnectionStatisticsState {
    generation: u64,
    open: bool,
    raw: RawConnectionStatistics,
    cached: ConnectionRateStatistics,
}

impl Default for ConnectionStatisticsState {
    fn default() -> Self {
        Self {
            generation: 0,
            open: true,
            raw: RawConnectionStatistics::default(),
            cached: ConnectionRateStatistics::default(),
        }
    }
}

#[derive(Debug)]
struct NetworkIoStatisticsState {
    last_statistics_ms: u64,
    next_connection_generation: u64,
    connections: HashMap<ConnectionStatisticsKey, ConnectionStatisticsState>,
    tcp_broadcast_bytes: u64,
    udp_broadcast_bytes: u64,
    tcp: ProtocolRateStatistics,
    udp: ProtocolRateStatistics,
}

/// Shared accounting source corresponding to `C4Network2IO::GenerateStatistics`.
///
/// Byte-recording methods collect low-level I/O until `generate_statistics`
/// consumes a due edge. Cached rates remain available until the next edge.
#[derive(Clone, Debug)]
pub struct NetworkIoStatistics {
    state: Arc<Mutex<NetworkIoStatisticsState>>,
}

impl Default for NetworkIoStatistics {
    fn default() -> Self {
        Self::new(0)
    }
}

impl NetworkIoStatistics {
    pub fn new(now_ms: u64) -> Self {
        Self {
            state: Arc::new(Mutex::new(NetworkIoStatisticsState {
                last_statistics_ms: now_ms,
                next_connection_generation: 1,
                connections: HashMap::new(),
                tcp_broadcast_bytes: 0,
                udp_broadcast_bytes: 0,
                tcp: ProtocolRateStatistics::default(),
                udp: ProtocolRateStatistics::default(),
            })),
        }
    }

    /// Opens (or reopens) one connection and returns its cheap shared recorder.
    /// Reopening the same key invalidates every recorder from the old route.
    pub fn open_connection(
        &self,
        connection_id: u32,
        protocol: NetworkProtocol,
    ) -> ConnectionStatisticsRecorder {
        self.open_connection_with_raw_if_current(connection_id, protocol, 0, 0, 0)
            .0
    }

    /// Opens a route and transfers pre-route bytes only if they still belong
    /// to the current sample. The epoch check and transfer are atomic with
    /// `generate_statistics`.
    pub(crate) fn open_connection_with_raw_if_current(
        &self,
        connection_id: u32,
        protocol: NetworkProtocol,
        pending_sample_ms: u64,
        pending_input_bytes: u64,
        pending_output_bytes: u64,
    ) -> (ConnectionStatisticsRecorder, u64) {
        let key = ConnectionStatisticsKey::new(connection_id, protocol);
        let mut state = self.state.lock().expect("network statistics lock poisoned");
        let sampled_at_ms = state.last_statistics_ms;
        let generation = state.next_connection_generation;
        state.next_connection_generation = state.next_connection_generation.wrapping_add(1);
        let connection = state.connections.entry(key).or_default();
        connection.generation = generation;
        connection.open = true;
        connection.raw = if pending_sample_ms == sampled_at_ms {
            RawConnectionStatistics {
                input_bytes: pending_input_bytes,
                output_bytes: pending_output_bytes,
                packet_loss: 0,
            }
        } else {
            RawConnectionStatistics::default()
        };
        connection.cached = ConnectionRateStatistics::default();
        drop(state);
        (
            ConnectionStatisticsRecorder {
                statistics: self.clone(),
                key,
                generation,
            },
            sampled_at_ms,
        )
    }

    /// Stops a connection contributing to later protocol totals.
    pub fn close_connection(&self, key: ConnectionStatisticsKey) {
        if let Some(connection) = self
            .state
            .lock()
            .expect("network statistics lock poisoned")
            .connections
            .get_mut(&key)
        {
            connection.open = false;
            connection.raw = RawConnectionStatistics::default();
            connection.cached = ConnectionRateStatistics::default();
        }
    }

    /// Records low-level multicast traffic. Ordinary high-level broadcasts
    /// are recorded as output on each selected connection instead.
    pub fn record_broadcast_datagram(&self, protocol: NetworkProtocol, payload_bytes: usize) {
        let bytes = accounted_bytes(protocol, payload_bytes);
        let mut state = self.state.lock().expect("network statistics lock poisoned");
        match protocol_bucket(protocol) {
            ProtocolBucket::Tcp => {
                // C4NetIOTCP::GetStatistic always reports zero.
            }
            ProtocolBucket::Udp => {
                state.udp_broadcast_bytes = state.udp_broadcast_bytes.saturating_add(bytes);
            }
        }
    }

    /// Consumes a statistics edge when the previous sample lies strictly
    /// outside C++'s inclusive one-second window.
    pub fn generate_statistics(&self, now_ms: u64) -> bool {
        let mut state = self.state.lock().expect("network statistics lock poisoned");
        let lower_bound = now_ms.wrapping_sub(NETWORK_STATISTICS_INTERVAL_MS);
        if state.last_statistics_ms >= lower_bound && state.last_statistics_ms <= now_ms {
            return false;
        }

        let interval_ms = now_ms.wrapping_sub(state.last_statistics_ms);
        state.last_statistics_ms = now_ms;
        let mut tcp_input_sum = 0_u64;
        let mut tcp_output_sum = 0_u64;
        let mut udp_input_sum = 0_u64;
        let mut udp_output_sum = 0_u64;

        for (key, connection) in &mut state.connections {
            if !connection.open {
                continue;
            }
            let raw = std::mem::take(&mut connection.raw);
            connection.cached.input_rate = normalize_rate(raw.input_bytes, interval_ms);
            connection.cached.output_rate = normalize_rate(raw.output_bytes, interval_ms);
            connection.cached.packet_loss = raw.packet_loss;
            match protocol_bucket(key.protocol) {
                ProtocolBucket::Tcp => {
                    tcp_input_sum = tcp_input_sum.saturating_add(connection.cached.input_rate);
                    tcp_output_sum = tcp_output_sum.saturating_add(connection.cached.output_rate);
                }
                ProtocolBucket::Udp => {
                    udp_input_sum = udp_input_sum.saturating_add(connection.cached.input_rate);
                    udp_output_sum = udp_output_sum.saturating_add(connection.cached.output_rate);
                }
            }
        }

        // Native DoStatistics normalizes each peer before adding it to these
        // sums, then GenerateStatistics normalizes the sums a second time.
        state.tcp.input_rate = normalize_rate(tcp_input_sum, interval_ms);
        state.tcp.output_rate = normalize_rate(tcp_output_sum, interval_ms);
        state.udp.input_rate = normalize_rate(udp_input_sum, interval_ms);
        state.udp.output_rate = normalize_rate(udp_output_sum, interval_ms);
        state.tcp.broadcast_rate = normalize_rate(state.tcp_broadcast_bytes, interval_ms);
        state.udp.broadcast_rate = normalize_rate(state.udp_broadcast_bytes, interval_ms);
        state.tcp_broadcast_bytes = 0;
        state.udp_broadcast_bytes = 0;
        true
    }

    pub fn protocol_statistics(&self, protocol: NetworkProtocol) -> ProtocolRateStatistics {
        let state = self.state.lock().expect("network statistics lock poisoned");
        match protocol_bucket(protocol) {
            ProtocolBucket::Tcp => state.tcp,
            ProtocolBucket::Udp => state.udp,
        }
    }

    pub fn connection_statistics(
        &self,
        key: ConnectionStatisticsKey,
    ) -> Option<ConnectionRateStatistics> {
        self.state
            .lock()
            .expect("network statistics lock poisoned")
            .connections
            .get(&key)
            .filter(|connection| connection.open)
            .map(|connection| connection.cached)
    }

    pub fn snapshot(&self) -> NetworkIoStatisticsSnapshot {
        let state = self.state.lock().expect("network statistics lock poisoned");
        let mut connections = state
            .connections
            .iter()
            .filter_map(|(key, connection)| connection.open.then_some((*key, connection.cached)))
            .collect::<Vec<_>>();
        connections.sort_unstable_by_key(|(key, _)| (key.protocol.to_wire(), key.connection_id));
        NetworkIoStatisticsSnapshot {
            tcp: state.tcp,
            udp: state.udp,
            connections,
        }
    }

    pub fn last_statistics_ms(&self) -> u64 {
        self.state
            .lock()
            .expect("network statistics lock poisoned")
            .last_statistics_ms
    }
}

/// Per-connection writer shared by transport and liveness owners.
#[derive(Clone, Debug)]
pub struct ConnectionStatisticsRecorder {
    statistics: NetworkIoStatistics,
    key: ConnectionStatisticsKey,
    generation: u64,
}

impl ConnectionStatisticsRecorder {
    pub const fn key(&self) -> ConnectionStatisticsKey {
        self.key
    }

    /// Records one successful socket receive/datagram, including C++'s fixed
    /// IP+transport header allowance.
    pub fn record_input(&self, payload_bytes: usize) {
        let _ = self.record_input_at_current_sample(payload_bytes);
    }

    pub(crate) fn record_input_at_current_sample(&self, payload_bytes: usize) -> Option<u64> {
        let bytes = accounted_bytes(self.key.protocol, payload_bytes);
        self.with_open_connection_at_current_sample(|connection| {
            connection.raw.input_bytes = connection.raw.input_bytes.saturating_add(bytes);
        })
    }

    /// Records one successful socket send/datagram, including C++'s fixed
    /// IP+transport header allowance.
    pub fn record_output(&self, payload_bytes: usize) {
        let _ = self.record_output_at_current_sample(payload_bytes);
    }

    pub(crate) fn record_output_at_current_sample(&self, payload_bytes: usize) -> Option<u64> {
        let bytes = accounted_bytes(self.key.protocol, payload_bytes);
        self.with_open_connection_at_current_sample(|connection| {
            connection.raw.output_bytes = connection.raw.output_bytes.saturating_add(bytes);
        })
    }

    /// Records bytes that already include any low-level header allowance.
    pub fn record_raw_input_bytes(&self, bytes: u64) {
        self.with_open_connection(|connection| {
            connection.raw.input_bytes = connection.raw.input_bytes.saturating_add(bytes);
        });
    }

    /// Records bytes that already include any low-level header allowance.
    pub fn record_raw_output_bytes(&self, bytes: u64) {
        self.with_open_connection(|connection| {
            connection.raw.output_bytes = connection.raw.output_bytes.saturating_add(bytes);
        });
    }

    pub fn set_packet_loss(&self, packet_loss: u32) {
        self.with_open_connection(|connection| connection.raw.packet_loss = packet_loss);
    }

    pub fn set_ping(&self, ping_ms: i32, lag_ms: i32) {
        self.with_open_connection(|connection| {
            connection.cached.ping_ms = ping_ms;
            connection.cached.lag_ms = lag_ms;
        });
    }

    pub fn close(&self) {
        let mut state = self
            .statistics
            .state
            .lock()
            .expect("network statistics lock poisoned");
        if let Some(connection) = state.connections.get_mut(&self.key) {
            if connection.generation == self.generation {
                connection.open = false;
                connection.raw = RawConnectionStatistics::default();
                connection.cached = ConnectionRateStatistics::default();
            }
        }
    }

    fn with_open_connection(&self, update: impl FnOnce(&mut ConnectionStatisticsState)) {
        let _ = self.with_open_connection_at_current_sample(update);
    }

    fn with_open_connection_at_current_sample(
        &self,
        update: impl FnOnce(&mut ConnectionStatisticsState),
    ) -> Option<u64> {
        let mut state = self
            .statistics
            .state
            .lock()
            .expect("network statistics lock poisoned");
        let sampled_at_ms = state.last_statistics_ms;
        if let Some(connection) = state.connections.get_mut(&self.key) {
            if connection.open && connection.generation == self.generation {
                update(connection);
                return Some(sampled_at_ms);
            }
        }
        None
    }
}

#[derive(Clone, Copy)]
enum ProtocolBucket {
    Tcp,
    Udp,
}

const fn protocol_bucket(protocol: NetworkProtocol) -> ProtocolBucket {
    match protocol {
        NetworkProtocol::Tcp => ProtocolBucket::Tcp,
        NetworkProtocol::Udp | NetworkProtocol::Unknown(_) => ProtocolBucket::Udp,
    }
}

fn accounted_bytes(protocol: NetworkProtocol, payload_bytes: usize) -> u64 {
    let header = match protocol_bucket(protocol) {
        ProtocolBucket::Tcp => TCP_STATISTICS_HEADER_BYTES,
        ProtocolBucket::Udp => UDP_STATISTICS_HEADER_BYTES,
    };
    u64::try_from(payload_bytes)
        .unwrap_or(u64::MAX)
        .saturating_add(header)
}

fn normalize_rate(bytes: u64, interval_ms: u64) -> u64 {
    if interval_ms == 0 {
        return 0;
    }
    bytes.saturating_mul(NETWORK_STATISTICS_INTERVAL_MS) / interval_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_cadence_uses_the_inclusive_cpp_window() {
        let statistics = NetworkIoStatistics::new(10_000);
        let connection = statistics.open_connection(7, NetworkProtocol::Tcp);
        connection.record_raw_input_bytes(2_000);

        assert!(!statistics.generate_statistics(10_500));
        assert!(!statistics.generate_statistics(11_000));
        assert_eq!(statistics.last_statistics_ms(), 10_000);
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Tcp),
            ProtocolRateStatistics::default()
        );

        assert!(statistics.generate_statistics(11_001));
        assert_eq!(statistics.last_statistics_ms(), 11_001);
        assert_ne!(
            statistics.protocol_statistics(NetworkProtocol::Tcp),
            ProtocolRateStatistics::default()
        );
    }

    #[test]
    fn statistics_cadence_preserves_the_native_initial_underflow_edge() {
        // On Unix the first timeGetTime call can be below one second. Native
        // unsigned subtraction then puts the prior timestamp outside Inside,
        // so the first Execute samples immediately (C4Chrono.cpp:23-31;
        // C4Network2IO.cpp:99-102,619-624).
        let statistics = NetworkIoStatistics::new(0);
        let connection = statistics.open_connection(1, NetworkProtocol::Tcp);
        connection.record_raw_input_bytes(500);

        assert!(statistics.generate_statistics(500));
        assert_eq!(statistics.last_statistics_ms(), 500);
    }

    #[test]
    fn generate_statistics_preserves_double_normalization_and_single_broadcast_pass() {
        let statistics = NetworkIoStatistics::new(0);
        let connection = statistics.open_connection(1, NetworkProtocol::Tcp);
        connection.record_raw_input_bytes(1_500);
        connection.record_raw_output_bytes(3_000);
        statistics.record_broadcast_datagram(NetworkProtocol::Udp, 1_468);

        assert!(statistics.generate_statistics(1_500));
        assert_eq!(
            statistics.connection_statistics(connection.key()),
            Some(ConnectionRateStatistics {
                input_rate: 1_000,
                output_rate: 2_000,
                ..ConnectionRateStatistics::default()
            })
        );
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Tcp),
            ProtocolRateStatistics {
                input_rate: 666,
                output_rate: 1_333,
                broadcast_rate: 0,
            }
        );
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Udp),
            ProtocolRateStatistics {
                input_rate: 0,
                output_rate: 0,
                broadcast_rate: 1_000,
            }
        );
    }

    #[test]
    fn generation_clears_interval_counters_but_keeps_cached_values_until_due() {
        let statistics = NetworkIoStatistics::new(0);
        let connection = statistics.open_connection(3, NetworkProtocol::Udp);
        connection.record_raw_input_bytes(2_002);
        connection.set_packet_loss(9);
        connection.set_ping(44, 51);
        statistics.record_broadcast_datagram(NetworkProtocol::Udp, 970);

        assert!(statistics.generate_statistics(1_001));
        let first = statistics.snapshot();
        assert!(!statistics.generate_statistics(2_001));
        assert_eq!(statistics.snapshot(), first);

        assert!(statistics.generate_statistics(2_002));
        assert_eq!(
            statistics.connection_statistics(connection.key()),
            Some(ConnectionRateStatistics {
                input_rate: 0,
                output_rate: 0,
                packet_loss: 0,
                ping_ms: 44,
                lag_ms: 51,
            })
        );
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Udp),
            ProtocolRateStatistics::default()
        );
    }

    #[test]
    fn connection_rounding_happens_before_protocol_aggregation() {
        let statistics = NetworkIoStatistics::new(0);
        let first = statistics.open_connection(1, NetworkProtocol::Udp);
        let second = statistics.open_connection(2, NetworkProtocol::Udp);
        first.record_raw_input_bytes(1);
        second.record_raw_input_bytes(1);

        assert!(statistics.generate_statistics(1_500));
        assert_eq!(
            statistics
                .protocol_statistics(NetworkProtocol::Udp)
                .input_rate,
            0
        );
    }

    #[test]
    fn operation_recorders_apply_native_header_allowances() {
        let statistics = NetworkIoStatistics::new(0);
        let tcp = statistics.open_connection(1, NetworkProtocol::Tcp);
        let udp = statistics.open_connection(2, NetworkProtocol::Udp);
        tcp.record_input(100);
        tcp.record_output(100);
        udp.record_input(100);
        udp.record_output(100);
        statistics.record_broadcast_datagram(NetworkProtocol::Udp, 100);
        statistics.record_broadcast_datagram(NetworkProtocol::Tcp, 100);

        assert!(statistics.generate_statistics(1_001));
        assert_eq!(
            statistics.connection_statistics(tcp.key()),
            Some(ConnectionRateStatistics {
                input_rate: 151,
                output_rate: 151,
                ..ConnectionRateStatistics::default()
            })
        );
        assert_eq!(
            statistics.connection_statistics(udp.key()),
            Some(ConnectionRateStatistics {
                input_rate: 131,
                output_rate: 131,
                ..ConnectionRateStatistics::default()
            })
        );
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Tcp),
            ProtocolRateStatistics {
                input_rate: 150,
                output_rate: 150,
                broadcast_rate: 0,
            }
        );
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Udp),
            ProtocolRateStatistics {
                input_rate: 130,
                output_rate: 130,
                broadcast_rate: 131,
            }
        );
    }

    #[test]
    fn non_tcp_protocols_share_the_cpp_udp_accessor_bucket() {
        let statistics = NetworkIoStatistics::new(0);
        let unknown = statistics.open_connection(9, NetworkProtocol::Unknown(17));
        unknown.record_raw_output_bytes(2_002);

        assert!(statistics.generate_statistics(1_001));
        assert_eq!(
            statistics.protocol_statistics(NetworkProtocol::Unknown(99)),
            statistics.protocol_statistics(NetworkProtocol::Udp)
        );
        unknown.close();
        assert_eq!(statistics.connection_statistics(unknown.key()), None);
    }

    #[test]
    fn reopening_a_key_invalidates_stale_recorders() {
        let statistics = NetworkIoStatistics::new(10_000);
        let stale = statistics.open_connection(4, NetworkProtocol::Tcp);
        stale.record_raw_input_bytes(10_000);
        let current = statistics.open_connection(4, NetworkProtocol::Tcp);

        stale.record_raw_input_bytes(20_000);
        stale.close();
        current.record_raw_input_bytes(1_001);

        assert!(statistics.generate_statistics(11_001));
        assert_eq!(
            statistics.connection_statistics(current.key()),
            Some(ConnectionRateStatistics {
                input_rate: 1_000,
                ..ConnectionRateStatistics::default()
            })
        );
    }

    #[test]
    fn pending_route_transfer_is_conditioned_on_the_atomic_sample_epoch() {
        let statistics = NetworkIoStatistics::new(0);
        let (first, sampled_at_ms) = statistics.open_connection_with_raw_if_current(
            8,
            NetworkProtocol::Udp,
            0,
            1_001,
            2_002,
        );
        assert_eq!(sampled_at_ms, 0);
        assert!(statistics.generate_statistics(1_001));
        assert_eq!(
            statistics.connection_statistics(first.key()),
            Some(ConnectionRateStatistics {
                input_rate: 1_000,
                output_rate: 2_000,
                ..ConnectionRateStatistics::default()
            })
        );

        let (current, sampled_at_ms) = statistics.open_connection_with_raw_if_current(
            8,
            NetworkProtocol::Udp,
            0,
            10_000,
            20_000,
        );
        assert_eq!(sampled_at_ms, 1_001);
        assert!(statistics.generate_statistics(2_002));
        assert_eq!(
            statistics.connection_statistics(current.key()),
            Some(ConnectionRateStatistics::default())
        );

        assert_eq!(current.record_input_at_current_sample(969), Some(2_002));
        assert!(statistics.generate_statistics(3_003));
        assert_eq!(
            statistics
                .connection_statistics(current.key())
                .unwrap()
                .input_rate,
            1_000
        );
    }
}
