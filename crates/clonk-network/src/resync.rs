use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::{ClientId, ControlPacket, MissingRange, ReadyBatch, Tick, BROADCAST_CLIENT_ID};

/// Keeps a backlog of control packets so that missing ticks can be replayed on
/// demand to clients that fell behind.
///
/// The backlog is stored per tick with deterministic ordering by `ClientId`
/// which mirrors the legacy `C4GameControlNetwork` control stack semantics.
#[derive(Debug, Clone)]
pub struct ControlBacklog {
    limit: usize,
    entries: BTreeMap<Tick, BTreeMap<ClientId, ControlPacket>>,
}

impl ControlBacklog {
    /// Creates a new backlog that retains up to `limit` ticks. A `limit` of
    /// `0` keeps all ticks.
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: BTreeMap::new(),
        }
    }

    /// Records a control packet for potential replay. The packet is cloned so
    /// callers can hand it to other systems afterwards.
    pub fn record_packet(&mut self, packet: &ControlPacket) {
        let tick = packet.tick();
        let entry = self.entries.entry(tick).or_default();
        entry
            .entry(packet.client_id())
            .or_insert_with(|| packet.clone());
        self.trim();
    }

    /// Records all control packets contained in the ready batch.
    pub fn record_ready_batch(&mut self, batch: &ReadyBatch) {
        for packet in batch.packets() {
            self.record_packet(packet);
        }
    }

    /// Records a collection of ready batches.
    pub fn record_ready_batches<'a, I>(&mut self, batches: I)
    where
        I: IntoIterator<Item = &'a ReadyBatch>,
    {
        for batch in batches {
            self.record_ready_batch(batch);
        }
    }

    /// Returns control packets grouped by tick beginning at `from_tick`. Empty
    /// ticks are skipped.
    pub fn packets_from(&self, from_tick: Tick) -> Vec<(Tick, Vec<ControlPacket>)> {
        self.entries
            .range(from_tick..)
            .filter_map(|(&tick, per_client)| {
                if per_client.is_empty() {
                    None
                } else {
                    Some((tick, per_client.values().cloned().collect()))
                }
            })
            .collect()
    }

    /// Reports whether an exact per-client packet is retained for `tick`.
    pub fn contains_packet(&self, client_id: ClientId, tick: Tick) -> bool {
        self.entries
            .get(&tick)
            .is_some_and(|per_client| per_client.contains_key(&client_id))
    }

    /// Returns control packets for all ticks beginning at `from_tick` until a
    /// gap is encountered, following the legacy host resync behaviour.
    pub fn fulfill_request(&self, from_tick: Tick) -> Vec<ControlPacket> {
        let mut resend = Vec::new();
        let mut tick = from_tick;
        loop {
            match self.entries.get(&tick) {
                Some(per_client) if !per_client.is_empty() => {
                    if let Some(complete) = per_client.get(&BROADCAST_CLIENT_ID) {
                        resend.push(complete.clone());
                    } else {
                        for packet in per_client.values() {
                            resend.push(packet.clone());
                        }
                    }
                    tick = tick.saturating_add(1);
                }
                _ => break,
            }
        }
        resend
    }

    /// Removes all packets associated with the specified client.
    pub fn remove_client(&mut self, client_id: ClientId) {
        self.entries.values_mut().for_each(|per_client| {
            per_client.remove(&client_id);
        });
        self.entries.retain(|_, per_client| !per_client.is_empty());
    }

    /// Clears the backlog completely.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn trim(&mut self) {
        if self.limit == 0 {
            return;
        }
        while self.entries.len() > self.limit {
            if let Some((&first_tick, _)) = self.entries.first_key_value() {
                self.entries.remove(&first_tick);
            } else {
                break;
            }
        }
    }
}

/// Request that should be sent to a client asking it to retransmit control
/// packets beginning at `from_tick`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResyncRequest {
    pub client_id: ClientId,
    pub from_tick: Tick,
}

impl ResyncRequest {
    pub fn new(client_id: ClientId, from_tick: Tick) -> Self {
        Self {
            client_id,
            from_tick,
        }
    }
}

/// Schedules resync requests for clients that have missing control packets.
///
/// Matching the legacy implementation, requests are throttled so the same tick
/// is only requested again after the configured interval has elapsed, unless a
/// new (earlier) gap is observed.
#[derive(Debug, Clone)]
pub struct ResyncScheduler {
    min_interval: Duration,
    entries: BTreeMap<ClientId, (Tick, Instant)>,
}

impl ResyncScheduler {
    /// Creates a scheduler that enforces a minimum interval between repeated
    /// requests for the same client and tick.
    pub fn new(min_interval: Duration) -> Self {
        Self {
            min_interval,
            entries: BTreeMap::new(),
        }
    }

    /// Generates resync requests for the provided missing ranges.
    pub fn schedule<'a, I>(&mut self, missing: I, now: Instant) -> Vec<ResyncRequest>
    where
        I: IntoIterator<Item = &'a MissingRange>,
    {
        let mut requests = Vec::new();
        for range in missing {
            if range.is_empty() {
                continue;
            }
            let client_id = range.client_id();
            let from_tick = range.from();
            let initial_time = now.checked_sub(self.min_interval).unwrap_or(now);
            let entry = self
                .entries
                .entry(client_id)
                .or_insert((from_tick, initial_time));
            let (last_tick, last_sent) = entry;
            let mut should_send = false;
            if from_tick != *last_tick {
                *last_tick = from_tick;
                should_send = true;
            } else if now.duration_since(*last_sent) >= self.min_interval {
                should_send = true;
            }
            if should_send {
                *last_sent = now;
                requests.push(ResyncRequest::new(client_id, from_tick));
            }
        }
        requests
    }

    /// Drops any scheduling state related to the given client.
    pub fn remove_client(&mut self, client_id: ClientId) {
        self.entries.remove(&client_id);
    }

    /// Clears the scheduler state for all clients.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ControlCoordinator, InsertStatus};

    fn packet(client: ClientId, tick: Tick, payload: &[u8]) -> ControlPacket {
        let mut control_list = payload.to_vec();
        control_list.push(0xff);
        ControlPacket::builder(client, tick)
            .timestamp_ms(100)
            .payload(control_list)
    }

    #[test]
    fn backlog_replays_in_tick_order() {
        let mut backlog = ControlBacklog::new(8);

        backlog.record_packet(&packet(2, 5, b"b"));
        backlog.record_packet(&packet(1, 5, b"a"));
        backlog.record_packet(&packet(1, 6, b"c"));
        backlog.record_packet(&packet(2, 6, b"d"));

        let replay = backlog.fulfill_request(5);
        let expected = vec![
            packet(1, 5, b"a"),
            packet(2, 5, b"b"),
            packet(1, 6, b"c"),
            packet(2, 6, b"d"),
        ];
        assert_eq!(replay, expected);
    }

    #[test]
    fn backlog_duplicate_keeps_the_first_control() {
        let mut backlog = ControlBacklog::new(8);
        backlog.record_packet(&packet(1, 5, b"first"));
        backlog.record_packet(&packet(1, 5, b"replacement"));

        assert_eq!(backlog.fulfill_request(5), vec![packet(1, 5, b"first")]);
    }

    #[test]
    fn packets_from_groups_by_tick() {
        let mut backlog = ControlBacklog::new(8);
        backlog.record_packet(&packet(2, 5, b"b"));
        backlog.record_packet(&packet(1, 5, b"a"));
        backlog.record_packet(&packet(3, 6, b"c"));

        let groups = backlog.packets_from(5);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, 5);
        assert_eq!(groups[0].1.len(), 2);
        assert_eq!(groups[0].1[0], packet(1, 5, b"a"));
        assert_eq!(groups[0].1[1], packet(2, 5, b"b"));
        assert_eq!(groups[1].0, 6);
        assert_eq!(groups[1].1, vec![packet(3, 6, b"c")]);
    }

    #[test]
    fn backlog_stops_on_gap() {
        let mut backlog = ControlBacklog::new(8);
        backlog.record_packet(&packet(1, 10, b"x"));
        backlog.record_packet(&packet(1, 12, b"z"));

        let replay = backlog.fulfill_request(10);
        assert_eq!(replay, vec![packet(1, 10, b"x")]);
    }

    #[test]
    fn backlog_prefers_complete_then_falls_back_to_partials() {
        let mut backlog = ControlBacklog::new(8);
        let complete = packet(BROADCAST_CLIENT_ID, 5, b"complete");
        backlog.record_packet(&packet(2, 5, b"partial-b"));
        backlog.record_packet(&complete);
        backlog.record_packet(&packet(1, 5, b"partial-a"));
        backlog.record_packet(&packet(2, 6, b"next-b"));
        backlog.record_packet(&packet(1, 6, b"next-a"));
        backlog.record_packet(&packet(BROADCAST_CLIENT_ID, 8, b"after-gap"));

        assert_eq!(
            backlog.fulfill_request(5),
            vec![complete, packet(1, 6, b"next-a"), packet(2, 6, b"next-b"),]
        );
    }

    #[test]
    fn backlog_enforces_tick_limit() {
        let mut backlog = ControlBacklog::new(2);
        backlog.record_packet(&packet(1, 1, b"x"));
        backlog.record_packet(&packet(1, 2, b"y"));
        backlog.record_packet(&packet(1, 3, b"z"));

        // tick 1 should be trimmed
        let replay = backlog.fulfill_request(2);
        let expected = vec![packet(1, 2, b"y"), packet(1, 3, b"z")];
        assert_eq!(replay, expected);
    }

    #[test]
    fn backlog_removes_client_packets() {
        let mut backlog = ControlBacklog::new(4);
        backlog.record_packet(&packet(1, 1, b"a"));
        backlog.record_packet(&packet(2, 1, b"b"));
        backlog.remove_client(1);

        let replay = backlog.fulfill_request(1);
        assert_eq!(replay, vec![packet(2, 1, b"b")]);
    }

    #[test]
    fn scheduler_emits_initial_request() {
        let mut scheduler = ResyncScheduler::new(Duration::from_millis(2000));
        let now = Instant::now();
        let missing = [MissingRange::new(5, 12, 14)];
        let requests = scheduler.schedule(missing.iter(), now);
        assert_eq!(requests, vec![ResyncRequest::new(5, 12)]);
    }

    #[test]
    fn scheduler_throttles_repeated_requests() {
        let mut scheduler = ResyncScheduler::new(Duration::from_millis(2000));
        let now = Instant::now();
        let missing = [MissingRange::new(7, 20, 25)];

        let initial = scheduler.schedule(missing.iter(), now);
        assert_eq!(initial.len(), 1);

        // Immediate re-check should not produce another request.
        let immediate = scheduler.schedule(missing.iter(), now);
        assert!(immediate.is_empty());

        // After the interval a new request is allowed.
        let later = now + Duration::from_millis(2000);
        let repeated = scheduler.schedule(missing.iter(), later);
        assert_eq!(repeated, vec![ResyncRequest::new(7, 20)]);
    }

    #[test]
    fn scheduler_sends_when_gap_moves() {
        let mut scheduler = ResyncScheduler::new(Duration::from_millis(5000));
        let now = Instant::now();

        let first = [MissingRange::new(9, 30, 31)];
        let initial = scheduler.schedule(first.iter(), now);
        assert_eq!(initial, vec![ResyncRequest::new(9, 30)]);

        // New range starting earlier should trigger immediately even before the interval.
        let earlier = [MissingRange::new(9, 29, 30)];
        let sooner = scheduler.schedule(earlier.iter(), now + Duration::from_millis(100));
        assert_eq!(sooner, vec![ResyncRequest::new(9, 29)]);
    }

    #[test]
    fn scheduler_clears_state_on_remove() {
        let mut scheduler = ResyncScheduler::new(Duration::from_millis(1000));
        let now = Instant::now();
        let missing = [MissingRange::new(3, 5, 6)];
        scheduler.schedule(missing.iter(), now);
        scheduler.remove_client(3);
        let again = scheduler.schedule(missing.iter(), now);
        assert_eq!(again, vec![ResyncRequest::new(3, 5)]);
    }

    #[test]
    fn backlog_and_scheduler_work_with_coordinator() {
        let mut coord = ControlCoordinator::new(8);
        coord.register_client(1).unwrap();
        coord.register_client(2).unwrap();

        let mut backlog = ControlBacklog::new(4);
        let mut scheduler = ResyncScheduler::new(Duration::from_millis(1000));

        let now = Instant::now();

        // Ingest packets for tick 0 only from client 1, causing a missing range for client 2.
        let outcome = coord.ingest(packet(1, 0, b"a")).unwrap();
        backlog.record_packet(&packet(1, 0, b"a"));

        assert_eq!(outcome.status, InsertStatus::Stored);
        assert!(outcome.ready.is_empty());
        let missing = coord.missing_ranges();
        assert_eq!(
            scheduler.schedule(missing.iter(), now),
            vec![ResyncRequest::new(2, 0)]
        );

        // When client 2 sends tick 0 the batch is ready and stored in the backlog.
        let outcome = coord.ingest(packet(2, 0, b"b")).unwrap();
        backlog.record_packet(&packet(2, 0, b"b"));
        backlog.record_ready_batches(outcome.ready.iter());

        assert_eq!(coord.current_tick(), 1);

        // A client requesting tick 0 should receive both packets.
        let replay = backlog.fulfill_request(0);
        assert_eq!(replay, vec![packet(1, 0, b"a"), packet(2, 0, b"b")]);
    }
}
