use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{ClientId, Tick};

/// Error cases that can occur when coordinating control packets.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("client {0} already registered")]
    ClientAlreadyRegistered(ClientId),
    #[error("client {0} not registered")]
    UnknownClient(ClientId),
}

/// A control packet that mirrors the information exchanged in the legacy
/// `C4GameControlPacket`.
///
/// `payload` is the serialized `C4Control` list, including its final
/// `PID_None` byte. `client_id` and `tick` are the packet's outer fields and
/// must not be repeated in the payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPacket {
    client_id: ClientId,
    tick: Tick,
    timestamp_ms: u64,
    payload: Vec<u8>,
}

impl ControlPacket {
    pub fn builder(client_id: ClientId, tick: Tick) -> ControlPacketBuilder {
        ControlPacketBuilder {
            client_id,
            tick,
            timestamp_ms: 0,
        }
    }

    pub fn client_id(&self) -> ClientId {
        self.client_id
    }

    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn timestamp_ms(&self) -> u64 {
        self.timestamp_ms
    }

    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

/// Builder for [`ControlPacket`]. Keeps construction ergonomic while enforcing
/// that a payload is supplied.
#[derive(Debug, Clone)]
pub struct ControlPacketBuilder {
    client_id: ClientId,
    tick: Tick,
    timestamp_ms: u64,
}

impl ControlPacketBuilder {
    pub fn timestamp_ms(mut self, timestamp_ms: u64) -> Self {
        self.timestamp_ms = timestamp_ms;
        self
    }

    pub fn payload<T: Into<Vec<u8>>>(self, payload: T) -> ControlPacket {
        ControlPacket {
            client_id: self.client_id,
            tick: self.tick,
            timestamp_ms: self.timestamp_ms,
            payload: payload.into(),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct ClientState {
    pending: BTreeMap<Tick, ControlPacket>,
    highest_tick_seen: Option<Tick>,
}

impl ClientState {
    fn register_packet(&mut self, packet: ControlPacket) -> InsertStatus {
        let tick = packet.tick;
        match self.pending.entry(tick) {
            std::collections::btree_map::Entry::Occupied(_) => InsertStatus::Duplicate,
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(packet);
                self.highest_tick_seen =
                    Some(self.highest_tick_seen.map_or(tick, |prev| prev.max(tick)));
                InsertStatus::Stored
            }
        }
    }
}

/// Coordinates control packets from multiple clients while keeping deterministic
/// ordering and tracking gaps that require re-requests.
#[derive(Debug, Clone)]
pub struct ControlCoordinator {
    backlog_limit: usize,
    current_tick: Tick,
    clients: BTreeMap<ClientId, ClientState>,
}

impl ControlCoordinator {
    pub fn new(backlog_limit: usize) -> Self {
        Self::with_start_tick(backlog_limit, 0)
    }

    pub fn with_start_tick(backlog_limit: usize, start_tick: Tick) -> Self {
        Self {
            backlog_limit,
            current_tick: start_tick,
            clients: BTreeMap::new(),
        }
    }

    pub fn register_client(&mut self, client_id: ClientId) -> Result<(), ControlError> {
        if self.clients.contains_key(&client_id) {
            return Err(ControlError::ClientAlreadyRegistered(client_id));
        }
        self.clients.insert(client_id, ClientState::default());
        Ok(())
    }

    pub fn remove_client(&mut self, client_id: ClientId) -> Result<Vec<ReadyBatch>, ControlError> {
        if self.clients.remove(&client_id).is_none() {
            return Err(ControlError::UnknownClient(client_id));
        }
        Ok(self.collect_ready())
    }

    pub fn ingest(&mut self, packet: ControlPacket) -> Result<ControlOutcome, ControlError> {
        let client_id = packet.client_id();
        let tick = packet.tick();
        let state = self
            .clients
            .get_mut(&client_id)
            .ok_or(ControlError::UnknownClient(client_id))?;

        if tick < self.current_tick {
            return Ok(ControlOutcome::stale());
        }

        let status = state.register_packet(packet);
        let missing = self.compute_missing(client_id);
        let ready = self.collect_ready();
        self.enforce_backlog();

        Ok(ControlOutcome {
            status,
            ready,
            missing,
        })
    }

    pub fn current_tick(&self) -> Tick {
        self.current_tick
    }

    /// Moves the ready cursor to the first control tick that has not executed.
    /// Runtime mode changes use the final status-barrier tick to discard
    /// join-era gaps before decentralized packing resumes.
    pub fn advance_to(&mut self, next_tick: Tick) -> Vec<ReadyBatch> {
        if next_tick <= self.current_tick {
            return Vec::new();
        }
        self.skip_to(next_tick);
        let ready = self.collect_ready();
        self.enforce_backlog();
        ready
    }

    /// Moves the cursor without collecting buffered contributions. Central
    /// mode transitions discard old gaps but must wait for an actual complete
    /// packet instead of locally packing per-client controls.
    pub fn skip_to(&mut self, next_tick: Tick) {
        if next_tick <= self.current_tick {
            return;
        }
        self.current_tick = next_tick;
        for state in self.clients.values_mut() {
            state.pending.retain(|tick, _| *tick >= next_tick);
        }
        self.enforce_backlog();
    }

    /// Packs the current tick with whichever registered clients have
    /// contributed, then resumes ordinary all-client packing for successors.
    /// C++ uses this only after the host's `CNM_Async` wait budget expires.
    pub fn force_current_tick(&mut self) -> Vec<ReadyBatch> {
        let tick = self.current_tick;
        let mut packets = Vec::with_capacity(self.clients.len());
        for state in self.clients.values_mut() {
            if let Some(packet) = state.pending.remove(&tick) {
                packets.push(packet);
            }
        }
        self.current_tick = self.current_tick.saturating_add(1);
        let mut ready = vec![ReadyBatch { tick, packets }];
        ready.extend(self.collect_ready());
        self.enforce_backlog();
        ready
    }

    /// Registered clients that have not contributed `tick` yet.
    ///
    /// The host needs this to tell a peer that hiccuped from one that has
    /// stopped keeping up: the async deadline bounds the wait for the first, but
    /// paying it every tick for the second costs every other participant.
    pub fn clients_missing(&self, tick: Tick) -> Vec<ClientId> {
        self.clients
            .iter()
            .filter(|(_, state)| !state.pending.contains_key(&tick))
            .map(|(client_id, _)| *client_id)
            .collect()
    }

    pub fn backlog_limit(&self) -> usize {
        self.backlog_limit
    }

    pub fn client_ids(&self) -> impl Iterator<Item = ClientId> + '_ {
        self.clients.keys().copied()
    }

    /// Returns the current missing control ranges for all registered clients.
    pub fn missing_ranges(&self) -> Vec<MissingRange> {
        let mut missing = Vec::new();
        let target_tick = self
            .clients
            .values()
            .filter_map(|state| state.highest_tick_seen)
            .max()
            .unwrap_or(self.current_tick);

        for (&client_id, state) in &self.clients {
            let mut expected = self.current_tick;
            for &tick in state.pending.keys() {
                if tick < expected {
                    continue;
                }
                if tick > expected {
                    missing.push(MissingRange::new(client_id, expected, tick));
                }
                expected = tick.saturating_add(1);
            }

            let effective_highest = state.highest_tick_seen.unwrap_or(target_tick);
            if effective_highest >= expected {
                missing.push(MissingRange::new(
                    client_id,
                    expected,
                    effective_highest.saturating_add(1),
                ));
            }
        }

        if self.backlog_limit > 0 {
            let min_allowed = self.current_tick.saturating_sub(self.backlog_limit as Tick);
            for range in &mut missing {
                range.from = range.from.max(min_allowed);
                if range.to <= range.from {
                    range.to = range.from;
                }
            }
            missing.retain(|range| !range.is_empty());
        }

        merge_adjacent_ranges(missing)
    }

    fn collect_ready(&mut self) -> Vec<ReadyBatch> {
        let mut ready = Vec::new();
        loop {
            if self.clients.is_empty() {
                break;
            }

            let tick = self.current_tick;
            if !self
                .clients
                .values()
                .all(|state| state.pending.contains_key(&tick))
            {
                break;
            }

            let mut packets = Vec::with_capacity(self.clients.len());
            for (client_id, state) in self.clients.iter_mut() {
                if let Some(packet) = state.pending.remove(&tick) {
                    packets.push(packet);
                } else {
                    // Defensive: all() above already ensured availability.
                    panic!("missing packet for client {client_id} at tick {tick}");
                }
            }

            ready.push(ReadyBatch { tick, packets });
            self.current_tick = self.current_tick.saturating_add(1);
        }
        ready
    }

    fn enforce_backlog(&mut self) {
        if self.backlog_limit == 0 {
            return;
        }
        let threshold = self.current_tick.saturating_sub(self.backlog_limit as Tick);
        for state in self.clients.values_mut() {
            state.pending.retain(|&tick, _| tick >= threshold);
        }
    }

    fn compute_missing(&self, client_id: ClientId) -> Vec<MissingRange> {
        let mut missing = Vec::new();
        let Some(state) = self.clients.get(&client_id) else {
            return missing;
        };

        let mut expected = self.current_tick;
        for &tick in state.pending.keys() {
            if tick < expected {
                continue;
            }
            if tick > expected {
                missing.push(MissingRange::new(client_id, expected, tick));
            }
            expected = tick.saturating_add(1);
        }

        if let Some(highest) = state.highest_tick_seen {
            if highest >= expected {
                missing.push(MissingRange::new(
                    client_id,
                    expected,
                    highest.saturating_add(1),
                ));
            }
        }

        if self.backlog_limit > 0 {
            let min_allowed = self.current_tick.saturating_sub(self.backlog_limit as Tick);
            for range in &mut missing {
                range.from = range.from.max(min_allowed);
                if range.to <= range.from {
                    range.to = range.from;
                }
            }
            missing.retain(|range| !range.is_empty());
        }

        merge_adjacent_ranges(missing)
    }
}

fn merge_adjacent_ranges(mut ranges: Vec<MissingRange>) -> Vec<MissingRange> {
    if ranges.len() <= 1 {
        return ranges;
    }
    ranges.sort_by_key(|r| (r.client_id, r.from, r.to));
    let mut merged = Vec::with_capacity(ranges.len());
    let mut current = ranges[0].clone();
    for range in ranges.into_iter().skip(1) {
        if range.client_id == current.client_id && range.from <= current.to {
            current.to = current.to.max(range.to);
        } else {
            if !current.is_empty() {
                merged.push(current);
            }
            current = range;
        }
    }
    if !current.is_empty() {
        merged.push(current);
    }
    merged
}

/// Outcome of calling [`ControlCoordinator::ingest`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlOutcome {
    pub status: InsertStatus,
    pub ready: Vec<ReadyBatch>,
    pub missing: Vec<MissingRange>,
}

impl ControlOutcome {
    pub fn stale() -> Self {
        Self {
            status: InsertStatus::Stale,
            ready: Vec::new(),
            missing: Vec::new(),
        }
    }
}

/// Insert status for a control packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertStatus {
    Stored,
    Duplicate,
    Stale,
}

/// Batch of synchronized control packets for a given tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadyBatch {
    tick: Tick,
    packets: Vec<ControlPacket>,
}

impl ReadyBatch {
    pub fn tick(&self) -> Tick {
        self.tick
    }

    pub fn packets(&self) -> &[ControlPacket] {
        &self.packets
    }

    pub fn into_packets(self) -> Vec<ControlPacket> {
        self.packets
    }
}

/// Missing control range that should be requested again from a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingRange {
    client_id: ClientId,
    from: Tick,
    to: Tick,
}

impl MissingRange {
    pub fn new(client_id: ClientId, from: Tick, to: Tick) -> Self {
        Self {
            client_id,
            from,
            to,
        }
    }

    pub fn client_id(&self) -> ClientId {
        self.client_id
    }

    pub fn from(&self) -> Tick {
        self.from
    }

    pub fn to(&self) -> Tick {
        self.to
    }

    pub fn len(&self) -> Tick {
        self.to.saturating_sub(self.from)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(client: ClientId, tick: Tick, payload: &[u8]) -> ControlPacket {
        let mut control_list = payload.to_vec();
        control_list.push(0xff);
        ControlPacket::builder(client, tick)
            .timestamp_ms(42)
            .payload(control_list)
    }

    #[test]
    fn collects_ready_batches_once_all_clients_present() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        let out = coord.ingest(packet(1, 0, b"A")).unwrap();
        assert!(out.ready.is_empty());

        let out = coord.ingest(packet(2, 0, b"B")).unwrap();
        assert_eq!(out.ready.len(), 1);
        let batch = &out.ready[0];
        assert_eq!(batch.tick(), 0);
        assert_eq!(batch.packets().len(), 2);
        assert_eq!(coord.current_tick(), 1);
    }

    #[test]
    fn advancing_to_live_tick_releases_buffered_contributions() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();
        coord.ingest(packet(1, 12, b"old")).unwrap();
        coord.ingest(packet(1, 137, b"a")).unwrap();
        let outcome = coord.ingest(packet(2, 137, b"b")).unwrap();
        assert!(outcome.ready.is_empty());

        let ready = coord.advance_to(137);

        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].tick(), 137);
        assert_eq!(coord.current_tick(), 138);
        assert!(!coord.clients[&1].pending.contains_key(&12));
    }

    /// The invariant that makes `CNM_Async` determinism-safe: once the host has
    /// packed a tick without a straggler, that straggler's control for the tick
    /// must be discarded, never applied later. Executing it on a subsequent
    /// tick would put the late client's simulation ahead of everyone else's.
    #[test]
    fn control_arriving_after_its_tick_was_forced_is_stale_and_never_executes() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        // Only client 1 makes the deadline; the host forces the tick.
        coord.ingest(packet(1, 0, b"prompt")).unwrap();
        let forced = coord.force_current_tick();
        assert_eq!(forced.len(), 1);
        assert_eq!(forced[0].tick(), 0);
        assert_eq!(forced[0].packets().len(), 1, "the slow client is omitted");
        assert_eq!(coord.current_tick(), 1);

        // Client 2's control for tick 0 finally arrives.
        let late = coord.ingest(packet(2, 0, b"late")).unwrap();
        assert_eq!(late.status, InsertStatus::Stale);
        assert!(late.ready.is_empty(), "a stale packet publishes nothing");

        // It must not be retained and re-emitted against any later tick.
        assert!(!coord.clients[&2].pending.contains_key(&0));
        coord.ingest(packet(1, 1, b"next")).unwrap();
        let ready = coord.ingest(packet(2, 1, b"next")).unwrap().ready;
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].tick(), 1);
        for control in ready[0].packets() {
            assert!(
                control.payload().starts_with(b"next"),
                "tick 1 must carry only tick 1's control, got {:?}",
                control.payload()
            );
        }
    }

    #[test]
    fn forcing_empty_tick_advances_without_registered_clients() {
        let mut coord = ControlCoordinator::new(100);

        let ready = coord.force_current_tick();

        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].tick(), 0);
        assert!(ready[0].packets().is_empty());
        assert_eq!(coord.current_tick(), 1);
    }

    #[test]
    fn detects_missing_ranges_for_out_of_order_messages() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(7).unwrap();
        coord.register_client(9).unwrap();

        let out = coord.ingest(packet(7, 0, b"foo")).unwrap();
        assert!(out.missing.is_empty());

        let out = coord.ingest(packet(9, 2, b"bar")).unwrap();
        assert_eq!(out.missing.len(), 1);
        let miss = &out.missing[0];
        assert_eq!(miss.client_id(), 9);
        assert_eq!(miss.from(), 0);
        assert_eq!(miss.to(), 2);
    }

    #[test]
    fn missing_range_clears_after_receiving_gap() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        coord.ingest(packet(1, 0, b"a")).unwrap();
        let out = coord.ingest(packet(2, 2, b"b")).unwrap();
        assert_eq!(out.missing.len(), 1);

        let out = coord.ingest(packet(2, 0, b"c")).unwrap();
        assert!(out
            .missing
            .iter()
            .any(|range| range.from() == 1 && range.to() == 2));
        assert!(out.ready.iter().any(|batch| batch.tick() == 0));

        let out = coord.ingest(packet(2, 1, b"d")).unwrap();
        assert!(out.missing.is_empty());

        let out = coord.ingest(packet(1, 1, b"sync")).unwrap();
        assert!(out.ready.iter().any(|batch| batch.tick() == 1));
    }

    #[test]
    fn duplicate_packet_for_same_tick_keeps_first_control() {
        // C++ oracle: C4GameControlNetwork::HandleControl returns immediately
        // when getCtrl(client, tick) already finds a packet
        // (src/C4GameControlNetwork.cpp:517-523). Retransmission must never
        // replace synchronized input that other peers may already have seen.
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        let out = coord.ingest(packet(1, 0, b"old")).unwrap();
        assert_eq!(out.status, InsertStatus::Stored);

        let out = coord.ingest(packet(1, 0, b"new")).unwrap();
        assert_eq!(out.status, InsertStatus::Duplicate);
        assert!(out.ready.is_empty());

        let out = coord.ingest(packet(2, 0, b"peer")).unwrap();
        assert_eq!(out.ready.len(), 1);
        assert_eq!(out.ready[0].packets()[0].payload(), b"old\xff");
    }

    #[test]
    fn stale_packets_are_ignored() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        coord.ingest(packet(1, 0, b"foo")).unwrap();
        coord.ingest(packet(2, 0, b"bar")).unwrap();
        assert_eq!(coord.current_tick(), 1);

        let out = coord.ingest(packet(1, 0, b"late")).unwrap();
        assert_eq!(out.status, InsertStatus::Stale);
        assert!(out.ready.is_empty());
    }

    #[test]
    fn removal_of_client_unlocks_waiting_batches() {
        let mut coord = ControlCoordinator::new(100);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        coord.ingest(packet(1, 0, b"a")).unwrap();
        coord.ingest(packet(1, 1, b"b")).unwrap();
        coord.ingest(packet(2, 1, b"c")).unwrap();

        let ready = coord.remove_client(2).unwrap();
        assert_eq!(ready.len(), 2);
        assert_eq!(ready[0].tick(), 0);
        assert_eq!(ready[1].tick(), 1);
        assert_eq!(coord.current_tick(), 2);
    }

    #[test]
    fn backlog_limit_drops_ancient_packets() {
        let mut coord = ControlCoordinator::with_start_tick(2, 50);
        coord.register_client(1).unwrap();

        for tick in 50..70 {
            coord.ingest(packet(1, tick, b"x")).unwrap();
        }

        assert!(coord
            .clients
            .get(&1)
            .unwrap()
            .pending
            .keys()
            .all(|&tick| tick >= 48));
    }
}
