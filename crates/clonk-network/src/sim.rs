//! Deterministic link simulation for lockstep control delivery.
//!
//! This drives real [`ReliableUdpEndpointCore`] endpoints across a simulated
//! link at the control cadence a live game uses (ControlRate 2 at 36 FPS => one
//! control packet every 55 ms) and reports how long each control packet took to
//! become deliverable on the far side. That is exactly the quantity a lockstep
//! stall is made of: every client blocks until the control for its next tick
//! arrives.
//!
//! Time is virtual and every impairment draw comes from a seeded LCG, so a given
//! (seed, conditions) is byte-for-byte reproducible and two builds can be
//! compared directly.
//!
//! This measures the transport only. It does not run the simulation, so the
//! numbers are a floor on stall duration, not a whole-frame budget.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use crate::control_latency::ControlLatencyEstimator;
use crate::udp_runtime::{ReliableUdpEndpointCore, ReliableUdpEvent, ReliableUdpStep};

/// C++ `C4GameControlNetwork` pacing: ControlRate 2 at the 38 FPS default
/// target is one control packet every other frame.
pub const CONTROL_PERIOD: Duration = Duration::from_millis(55);

/// Virtual-clock granularity.
pub const STEP: Duration = Duration::from_millis(1);

/// Deterministic loss/jitter source.
///
/// This is presentation-free test tooling, so it deliberately does not touch the
/// synchronized `Random()` stream.
#[derive(Debug, Clone)]
pub struct SimRng(u64);

impl SimRng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u32
    }

    pub fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            0
        } else {
            self.next_u32() % bound
        }
    }
}

/// One datagram travelling across the simulated link.
#[derive(Debug, Clone)]
pub struct InFlight {
    pub deliver_at: Duration,
    pub to_host: bool,
    pub payload: Vec<u8>,
    /// Competing bulk traffic. It occupies the link exactly like real traffic
    /// but is never handed to an endpoint.
    pub filler: bool,
}

/// Size of one competing-flow datagram, matching the engine's own
/// `MAX_DATAGRAM_SIZE` so bulk transfer is modelled at its real granularity.
pub const CROSS_TRAFFIC_DATAGRAM_BYTES: usize = 512;

/// The impairments applied to a simulated link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkConditions {
    /// Round trip; one-way delay is half of this.
    pub rtt_ms: u64,
    /// Uniform draw over `0..=2*jitter_ms` added to the one-way delay.
    pub jitter_ms: u64,
    pub loss_permille: u32,
    /// Mean length of a correlated loss episode, in milliseconds. Zero gives
    /// independent Bernoulli loss. Real links drop in bursts — a queue
    /// overflows, a radio fades — and a burst is exactly the case a redundant
    /// copy sent in the same breath as the original cannot survive.
    pub burst_ms: u64,
    /// Host-to-client capacity in bits per second. Zero is unmetered.
    ///
    /// Below roughly a megabit this dominates everything else: a 512-byte
    /// datagram is 122 ms of pure serialization on a 33.6 kbit/s link, and on a
    /// narrow link the per-datagram IP/UDP/PPP overhead (~35 B) means packet
    /// *rate* costs more than payload.
    pub downlink_bps: u64,
    /// Client-to-host capacity in bits per second. Zero is unmetered. This is
    /// the direction that starves first on a real consumer link.
    pub uplink_bps: u64,
    /// Drop-tail queue depth in bytes, per direction. Zero is unbounded.
    ///
    /// This is the bufferbloat knob, and it is the one that reproduces
    /// multi-second hotel-wifi lag spikes. A 64 kB buffer in front of a
    /// 33.6 kbit/s link is ~15 s of standing queue; an unbounded queue grows
    /// latency without ever dropping, which is exactly the failure a
    /// loss-and-delay-only model cannot show.
    pub queue_bytes: u64,
    /// Offered load from a competing bulk flow on the host-to-client direction,
    /// in bits per second. Zero disables it.
    ///
    /// Bufferbloat is a *contention* phenomenon: with nothing else on the link
    /// the queue never fills and a profile silently understates the real
    /// problem. That is the flaw in every shipping preset (Unity's network
    /// simulator, Apple's Network Link Conditioner) that models only delay and
    /// loss. In this engine the competing flow is usually a resource transfer —
    /// 100 KiB chunks with no rate limit — or another device on the same wifi.
    pub cross_traffic_down_bps: u64,
    /// The same competing load on the client-to-host direction.
    pub cross_traffic_up_bps: u64,
}

impl Default for LinkConditions {
    fn default() -> Self {
        Self {
            rtt_ms: 60,
            jitter_ms: 10,
            loss_permille: 10,
            burst_ms: 0,
            downlink_bps: 0,
            uplink_bps: 0,
            queue_bytes: 0,
            cross_traffic_down_bps: 0,
            cross_traffic_up_bps: 0,
        }
    }
}

impl LinkConditions {
    /// A link with no impairment at all, for control arms and coverage checks.
    pub fn perfect() -> Self {
        Self {
            rtt_ms: 0,
            jitter_ms: 0,
            loss_permille: 0,
            burst_ms: 0,
            downlink_bps: 0,
            uplink_bps: 0,
            queue_bytes: 0,
            cross_traffic_down_bps: 0,
            cross_traffic_up_bps: 0,
        }
    }

    pub fn one_way(&self) -> Duration {
        Duration::from_millis(self.rtt_ms / 2)
    }

    fn rate_bps(&self, to_host: bool) -> u64 {
        if to_host {
            self.uplink_bps
        } else {
            self.downlink_bps
        }
    }

    fn cross_traffic_bps(&self, to_host: bool) -> u64 {
        if to_host {
            self.cross_traffic_up_bps
        } else {
            self.cross_traffic_down_bps
        }
    }
}

/// Time to clock `bytes` onto a link of `bps` bits per second.
///
/// Integer nanoseconds throughout: the rig's whole value is that a seed
/// reproduces byte-for-byte, and float rounding would vary that across targets.
fn serialization_time(bytes: usize, bps: u64) -> Duration {
    if bps == 0 {
        return Duration::ZERO;
    }
    let nanos = (bytes as u128)
        .saturating_mul(8)
        .saturating_mul(1_000_000_000)
        / u128::from(bps);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Bytes still waiting to be clocked out at `now`, given a transmission cursor.
fn standing_queue_bytes(busy_until: Duration, now: Duration, bps: u64) -> u64 {
    let backlog = busy_until.saturating_sub(now);
    u64::try_from(
        u128::from(backlog.as_nanos() as u64).saturating_mul(u128::from(bps)) / 8 / 1_000_000_000,
    )
    .unwrap_or(u64::MAX)
}

/// A simulated link carrying datagrams between two endpoints.
#[derive(Debug)]
pub struct Link {
    conditions: LinkConditions,
    bad_until: Duration,
    next_episode_at: Duration,
    rng: SimRng,
    queue: Vec<InFlight>,
    dropped: usize,
    sent: usize,
    /// When the client-to-host direction finishes clocking out everything
    /// already accepted. The backlog behind it *is* the standing queue.
    uplink_busy_until: Duration,
    /// The same cursor for host-to-client.
    downlink_busy_until: Duration,
    /// Datagrams refused because the drop-tail queue was full, counting the
    /// competing flow as well as the game's own traffic.
    queue_drops: usize,
    /// Fractional bulk-flow debt, in bit-nanoseconds, so an offered load that is
    /// not a whole number of datagrams per step stays exact and reproducible.
    cross_owed: [u128; 2],
    filler_sent: usize,
    filler_dropped: usize,
}

impl Link {
    pub fn new(conditions: LinkConditions, seed: u64) -> Self {
        Self {
            conditions,
            bad_until: Duration::ZERO,
            next_episode_at: Duration::ZERO,
            rng: SimRng::new(seed),
            queue: Vec::new(),
            dropped: 0,
            sent: 0,
            uplink_busy_until: Duration::ZERO,
            downlink_busy_until: Duration::ZERO,
            queue_drops: 0,
            cross_owed: [0; 2],
            filler_sent: 0,
            filler_dropped: 0,
        }
    }

    pub fn filler_sent(&self) -> usize {
        self.filler_sent
    }

    pub fn filler_dropped(&self) -> usize {
        self.filler_dropped
    }

    /// Offers the competing bulk flow its share of `elapsed`.
    ///
    /// Modelled as a greedy sender: it hands the link whole datagrams as fast as
    /// its offered rate allows and does not back off, which is what an
    /// unthrottled resource transfer does.
    pub fn pump_cross_traffic(&mut self, now: Duration, elapsed: Duration) {
        for to_host in [false, true] {
            let bps = self.conditions.cross_traffic_bps(to_host);
            if bps == 0 {
                continue;
            }
            let slot = usize::from(to_host);
            self.cross_owed[slot] += u128::from(bps) * elapsed.as_nanos();
            let per_datagram = (CROSS_TRAFFIC_DATAGRAM_BYTES as u128) * 8 * 1_000_000_000;
            while self.cross_owed[slot] >= per_datagram {
                self.cross_owed[slot] -= per_datagram;
                let _ = self.admit(now, to_host, vec![0u8; CROSS_TRAFFIC_DATAGRAM_BYTES], true);
            }
        }
    }

    /// Of [`Link::dropped`], how many were refused by a full queue rather than
    /// lost on the wire. Congestion and radio loss want different responses, so
    /// the rig reports them apart.
    pub fn queue_drops(&self) -> usize {
        self.queue_drops
    }

    /// Bytes currently waiting to be clocked out in one direction.
    pub fn standing_queue(&self, now: Duration, to_host: bool) -> u64 {
        let bps = self.conditions.rate_bps(to_host);
        if bps == 0 {
            return 0;
        }
        let cursor = if to_host {
            self.uplink_busy_until
        } else {
            self.downlink_busy_until
        };
        standing_queue_bytes(cursor, now, bps)
    }

    pub fn conditions(&self) -> LinkConditions {
        self.conditions
    }

    pub fn sent(&self) -> usize {
        self.sent
    }

    pub fn dropped(&self) -> usize {
        self.dropped
    }

    fn drops(&mut self, now: Duration) -> bool {
        if self.conditions.loss_permille == 0 {
            return false;
        }
        if self.conditions.burst_ms > 0 {
            // Episodes are scheduled in TIME, not drawn per datagram. Drawing
            // per datagram would make an extra copy of a 50-byte control packet
            // as likely to trigger a radio fade or a queue overflow as the
            // original, so any redundant configuration would manufacture its
            // own extra loss and measure as worse for a reason the physical
            // link does not share. With an absolute schedule the fraction of
            // datagrams landing inside a bad window converges to loss_permille
            // however many are sent, which is what makes the comparison fair.
            while now >= self.next_episode_at {
                self.bad_until =
                    self.next_episode_at + Duration::from_millis(self.conditions.burst_ms);
                let mean_period_ms = (self.conditions.burst_ms * 1000
                    / u64::from(self.conditions.loss_permille).max(1))
                .max(self.conditions.burst_ms + 1);
                let spread = u32::try_from(mean_period_ms).unwrap_or(u32::MAX).max(1);
                let period_ms = mean_period_ms / 2 + u64::from(self.rng.below(spread));
                self.next_episode_at += Duration::from_millis(period_ms.max(1));
            }
            return now < self.bad_until;
        }
        self.rng.below(1000) < self.conditions.loss_permille
    }

    /// Admits a datagram to the link.
    ///
    /// Three effects compose, in the order a real link applies them: the wire
    /// may simply lose it; a full drop-tail queue may refuse it; otherwise it
    /// waits behind whatever is already queued, is clocked out at the link rate,
    /// and only then propagates. Delivery is therefore
    /// `transmit_end + one_way + jitter`, and an unmetered link collapses to the
    /// original `now + one_way + jitter`.
    /// Returns when the datagram will become deliverable, or `None` if the link
    /// lost it or a full queue refused it.
    pub fn enqueue(&mut self, now: Duration, to_host: bool, payload: Vec<u8>) -> Option<Duration> {
        self.admit(now, to_host, payload, false)
    }

    fn admit(
        &mut self,
        now: Duration,
        to_host: bool,
        payload: Vec<u8>,
        filler: bool,
    ) -> Option<Duration> {
        if filler {
            self.filler_sent += 1;
        } else {
            self.sent += 1;
        }
        if self.drops(now) {
            if filler {
                self.filler_dropped += 1;
            } else {
                self.dropped += 1;
            }
            return None;
        }

        let bps = self.conditions.rate_bps(to_host);
        let queue_bytes = self.conditions.queue_bytes;
        let transmit_end = if bps == 0 {
            now
        } else {
            let cursor = if to_host {
                &mut self.uplink_busy_until
            } else {
                &mut self.downlink_busy_until
            };
            if queue_bytes > 0 && standing_queue_bytes(*cursor, now, bps) >= queue_bytes {
                // Drop-tail. A bounded buffer sheds the newest arrival rather
                // than growing latency without limit.
                if filler {
                    self.filler_dropped += 1;
                } else {
                    self.dropped += 1;
                }
                self.queue_drops += 1;
                return None;
            }
            let start = (*cursor).max(now);
            *cursor = start + serialization_time(payload.len(), bps);
            *cursor
        };

        let jitter = if self.conditions.jitter_ms == 0 {
            0
        } else {
            u64::from(self.rng.below(self.conditions.jitter_ms as u32 * 2 + 1))
        };
        let delay = Duration::from_millis(self.conditions.rtt_ms / 2 + jitter);
        let deliver_at = transmit_end + delay;
        self.queue.push(InFlight {
            deliver_at,
            to_host,
            payload,
            filler,
        });
        Some(deliver_at)
    }

    pub fn due(&mut self, now: Duration) -> Vec<InFlight> {
        let mut ready = Vec::new();
        let mut retained = Vec::with_capacity(self.queue.len());
        for item in self.queue.drain(..) {
            if item.deliver_at <= now {
                ready.push(item);
            } else {
                retained.push(item);
            }
        }
        self.queue = retained;
        // Deliver in scheduled order; jitter alone already reorders datagrams.
        ready.sort_by_key(|item| item.deliver_at);
        ready
    }
}

/// How the client sizes its PreSend horizon while the session runs.
#[derive(Debug, Clone)]
pub enum Lookahead {
    /// A constant horizon, for isolating transport behavior from adaptation.
    Fixed(Duration),
    /// C++ `CalcPerformance`: a 1/150 EWMA of the mean, and nothing else.
    CppMean { average_us: i32, target_fps: i32 },
    /// The mean-plus-deviation budget from [`ControlLatencyEstimator`].
    Adaptive {
        estimator: ControlLatencyEstimator,
        target_fps: i32,
    },
}

impl Lookahead {
    /// Both adaptive modes convert a microsecond budget the same way C++ does:
    /// to a whole number of frames, clamped, then back to wall-clock.
    pub fn frames_to_duration(budget_us: i32, target_fps: i32) -> Duration {
        let frames = (target_fps.saturating_mul(budget_us) / 1_000_000)
            .saturating_add(1)
            .clamp(1, 15);
        Duration::from_micros((frames as u64 * 1_000_000) / target_fps.max(1) as u64)
    }

    pub fn current(&self) -> Duration {
        match self {
            Self::Fixed(lookahead) => *lookahead,
            Self::CppMean {
                average_us,
                target_fps,
            } => Self::frames_to_duration(*average_us, *target_fps),
            Self::Adaptive {
                estimator,
                target_fps,
            } => Self::frames_to_duration(estimator.budget_us(), *target_fps),
        }
    }

    pub fn observe(&mut self, delivery: Duration) {
        let sample_ms = delivery.as_millis().min(i32::MAX as u128) as i32;
        match self {
            Self::Fixed(_) => {}
            Self::CppMean { average_us, .. } => {
                *average_us = average_us
                    .wrapping_mul(149)
                    .wrapping_add(sample_ms.wrapping_mul(1_000))
                    / 150;
            }
            Self::Adaptive { estimator, .. } => estimator.observe(sample_ms),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Fixed(fixed) => format!("fixed {fixed:?}"),
            Self::CppMean { .. } => "cpp mean-only".to_string(),
            Self::Adaptive { .. } => "adaptive mean+deviation".to_string(),
        }
    }
}

/// What a client experienced replaying the control stream in lockstep.
#[derive(Debug, Clone, Default)]
pub struct LockstepPlayout {
    /// Per executed tick, how long the client blocked waiting for its control.
    pub stalls: Vec<Duration>,
    /// How far the last tick ended up behind its ideal slot.
    pub drift: Duration,
    /// The PreSend horizon in force at each tick — the price paid for the
    /// stalls that were avoided.
    pub horizons: Vec<Duration>,
}

impl LockstepPlayout {
    pub fn stalled(&self) -> Vec<Duration> {
        self.stalls
            .iter()
            .copied()
            .filter(|stall| !stall.is_zero())
            .collect()
    }
}

/// Lockstep playout model.
///
/// A client cannot execute control tick T before its packet arrives, and it
/// paces successive ticks one control period apart. Once a late packet pushes
/// execution past its slot the whole schedule slips, which is what a player
/// perceives as the game running slow. `catch_up` picks which of the two real
/// client behaviors to model. With it off the schedule slips permanently once a
/// packet is late, which is the game visibly running behind. With it on the
/// client races back to its ideal slot after every stall, so a late packet costs
/// a hitch instead of drift and the next late packet hitches again. The frame
/// scheduler decides which happens.
pub fn replay_lockstep(
    arrivals: &BTreeMap<u32, Duration>,
    ticks: usize,
    mut lookahead: Lookahead,
    catch_up: bool,
) -> LockstepPlayout {
    let mut playout = LockstepPlayout::default();
    let mut executed_at = Duration::ZERO;
    for tick in 0..ticks as u32 {
        playout.horizons.push(lookahead.current());
        let scheduled = CONTROL_PERIOD * tick + lookahead.current();
        let earliest = if catch_up {
            scheduled
        } else {
            scheduled.max(executed_at + CONTROL_PERIOD)
        };
        let Some(arrived) = arrivals.get(&tick) else {
            continue;
        };
        executed_at = earliest.max(*arrived);
        playout.stalls.push(executed_at.saturating_sub(earliest));
        // The engine samples delivery time and re-sizes PreSend as it goes.
        lookahead.observe(arrived.saturating_sub(CONTROL_PERIOD * tick));
    }
    let last_slot = CONTROL_PERIOD * (ticks.saturating_sub(1) as u32) + lookahead.current();
    playout.drift = executed_at.saturating_sub(last_slot);
    playout
}

/// One control-delivery experiment.
#[derive(Debug, Clone)]
pub struct ControlDeliveryConfig {
    pub conditions: LinkConditions,
    pub ticks: usize,
    pub seed: u64,
    /// Copies of each control datagram to put on the wire. `C4NetIOUDP` discards
    /// a packet number below its receive cursor, so a redundant copy is
    /// wire-legal and a C++ peer drops it without noticing.
    pub duplicates: u64,
    /// Stagger between copies, so one congestion burst is less likely to take
    /// all of them.
    pub duplicate_delay_ms: u64,
    /// Size of a bulk packet pushed through the *same* reliable-UDP stream as
    /// control, modelling a resource chunk. Zero disables it.
    ///
    /// This is the mechanism that actually freezes a session, and it is not the
    /// same thing as bandwidth contention: because delivery is strictly ordered,
    /// a chunk's fragments occupy sequence numbers *ahead of* every later
    /// control packet, so one lost fragment withholds all of them until the
    /// repair lands. Unlike `cross_traffic_*_bps`, which only competes for link
    /// capacity, this goes through the real endpoint and takes real packet
    /// numbers.
    pub bulk_packet_bytes: usize,
    /// How often such a chunk is sent.
    pub bulk_interval: Duration,
    pub lookahead: Lookahead,
    /// Which of the two real client pacings to replay; see [`replay_lockstep`].
    ///
    /// The frozen-time figures recorded in `PORT_STATUS.md` for the PreSend and
    /// redundancy divergences were all measured with this **on**, which that
    /// recipe does not mention. With it off the schedule slips instead of
    /// hitching, so a late packet is absorbed rather than counted and the same
    /// run reports roughly two orders of magnitude less frozen time. Reproducing
    /// those numbers therefore needs `LC_CATCHUP=1`: 80 ms / +-20 ms / 1% with
    /// `LC_DUP=1 LC_PRESEND=cpp` gives 27.11% here against the 27.19% on record.
    pub catch_up: bool,
}

impl Default for ControlDeliveryConfig {
    fn default() -> Self {
        Self {
            conditions: LinkConditions::default(),
            ticks: 400,
            seed: 0x5eed_1234,
            duplicates: 1,
            duplicate_delay_ms: 0,
            bulk_packet_bytes: 0,
            bulk_interval: Duration::from_secs(1),
            lookahead: Lookahead::Fixed(Duration::ZERO),
            catch_up: false,
        }
    }
}

/// Everything one run observed. Deliberately raw: a caller decides which
/// statistics to derive, and the report is the unit a chaos gate compares.
#[derive(Debug, Clone)]
pub struct LinkReport {
    pub conditions: LinkConditions,
    pub seed: u64,
    pub duplicates: u64,
    pub ticks: usize,
    pub presend_label: String,
    pub catch_up: bool,
    /// Per control tick, how long it took to become deliverable on the far side.
    pub latencies: Vec<Duration>,
    /// Control ticks that never became deliverable at all.
    pub never_arrived: usize,
    pub datagrams_sent: usize,
    pub datagrams_dropped: usize,
    /// Datagrams refused by a full drop-tail queue, counting the competing flow.
    /// Congestion loss and radio loss want different responses, so they are
    /// reported apart rather than folded into one number.
    pub queue_drops: usize,
    /// Competing bulk-flow datagrams offered to the link.
    pub filler_sent: usize,
    pub playout: LockstepPlayout,
}

impl LinkReport {
    pub fn controls_arrived(&self) -> usize {
        self.latencies.len()
    }

    /// Control packets that took longer than one control period to arrive —
    /// each one is a frame the whole session waits on.
    pub fn over_one_period(&self) -> usize {
        self.latencies
            .iter()
            .filter(|latency| **latency > CONTROL_PERIOD)
            .count()
    }

    pub fn sorted_latencies(&self) -> Vec<Duration> {
        let mut sorted = self.latencies.clone();
        sorted.sort_unstable();
        sorted
    }

    pub fn mean_latency(&self) -> Duration {
        mean(&self.latencies)
    }

    pub fn max_latency(&self) -> Duration {
        self.latencies.iter().max().copied().unwrap_or_default()
    }

    /// Fraction of the session spent blocked on a late control packet.
    ///
    /// Measured against the session's *nominal* length, so it exceeds 1.0 when
    /// the schedule collapses outright — a contended dial-up link reports
    /// several thousand percent, meaning the run took many times longer than the
    /// ticks it executed should have. That is informative rather than a bug, but
    /// do not read such a value as a percentage of anything.
    pub fn frozen_time_fraction(&self) -> f64 {
        let wall_clock = CONTROL_PERIOD * self.ticks as u32;
        if wall_clock.is_zero() {
            return 0.0;
        }
        let stalled_total: Duration = self.playout.stalled().iter().sum();
        stalled_total.as_secs_f64() / wall_clock.as_secs_f64()
    }
}

pub fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

pub fn mean(samples: &[Duration]) -> Duration {
    samples
        .iter()
        .sum::<Duration>()
        .checked_div(samples.len().max(1) as u32)
        .unwrap_or_default()
}

/// Drives two real reliable-UDP endpoints across an impaired link at the live
/// control cadence and replays the resulting lockstep playout.
pub fn run_control_delivery(config: &ControlDeliveryConfig) -> LinkReport {
    run_control_delivery_in_direction(config, false)
}

fn run_control_delivery_in_direction(
    config: &ControlDeliveryConfig,
    client_to_host: bool,
) -> LinkReport {
    let host_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_001);

    let mut now = Duration::ZERO;
    let mut host = ReliableUdpEndpointCore::new_at(now);
    let mut client = ReliableUdpEndpointCore::new_at(now);
    let mut link = Link::new(config.conditions, config.seed);

    // Handshake on a clean link so connection setup is not part of the sample.
    let mut pending: Vec<(bool, Vec<u8>)> = Vec::new();
    let step = host.connect_at(client_addr, now);
    for datagram in step.datagrams {
        pending.push((false, datagram.payload));
    }
    for _ in 0..64 {
        let batch = std::mem::take(&mut pending);
        if batch.is_empty() {
            break;
        }
        for (to_host, payload) in batch {
            let step = if to_host {
                host.receive_at(client_addr, &payload, now)
            } else {
                client.receive_at(host_addr, &payload, now)
            };
            for datagram in step.datagrams {
                pending.push((!to_host, datagram.payload));
            }
        }
    }

    let mut sent_at: BTreeMap<u32, Duration> = BTreeMap::new();
    let mut arrivals: BTreeMap<u32, Duration> = BTreeMap::new();
    let mut latencies: Vec<Duration> = Vec::new();
    let mut next_control_at = Duration::ZERO;
    let mut next_bulk_at = config.bulk_interval;
    let mut tick: u32 = 0;
    let deadline = CONTROL_PERIOD * (config.ticks as u32 + 40);
    let duplicates = config.duplicates.max(1);

    while now <= deadline {
        // The selected sender emits one control packet per control tick.
        if tick < config.ticks as u32 && now >= next_control_at {
            let payload = tick.to_le_bytes().to_vec();
            let step = if client_to_host {
                client.send_packet(host_addr, &payload)
            } else {
                host.send_packet(client_addr, &payload)
            };
            if let Ok(step) = step {
                sent_at.insert(tick, now);
                for datagram in step.datagrams {
                    // Each copy draws loss independently, which is the property
                    // that makes redundancy worth its bandwidth.
                    for copy in 0..duplicates {
                        let stagger = Duration::from_millis(config.duplicate_delay_ms * copy);
                        link.enqueue(now + stagger, client_to_host, datagram.payload.clone());
                    }
                }
            }
            tick += 1;
            next_control_at += CONTROL_PERIOD;
        }

        // A resource chunk on the same ordered stream as control.
        if config.bulk_packet_bytes > 0 && now >= next_bulk_at {
            let chunk = vec![0u8; config.bulk_packet_bytes];
            let step = if client_to_host {
                client.send_packet(host_addr, &chunk)
            } else {
                host.send_packet(client_addr, &chunk)
            };
            if let Ok(step) = step {
                for datagram in step.datagrams {
                    link.enqueue(now, client_to_host, datagram.payload);
                }
            }
            next_bulk_at += config.bulk_interval;
        }

        link.pump_cross_traffic(now, STEP);

        for item in link.due(now) {
            if item.filler {
                // Competing traffic occupies the link but belongs to nobody in
                // this session; handing it to an endpoint would be noise.
                continue;
            }
            let step: ReliableUdpStep = if item.to_host {
                host.receive_at(client_addr, &item.payload, now)
            } else {
                client.receive_at(host_addr, &item.payload, now)
            };
            for datagram in step.datagrams {
                link.enqueue(now, !item.to_host, datagram.payload);
            }
            for event in step.events {
                if let ReliableUdpEvent::Packet { payload, .. } = event {
                    if payload.len() == 4 {
                        let id = u32::from_le_bytes(payload[..4].try_into().expect("4 bytes"));
                        if let Some(sent) = sent_at.remove(&id) {
                            latencies.push(now.saturating_sub(sent));
                            arrivals.insert(id, now);
                        }
                    }
                }
            }
        }

        for (endpoint, to_host) in [(&mut host, false), (&mut client, true)] {
            let step = endpoint.timer_at(now);
            for datagram in step.datagrams {
                link.enqueue(now, to_host, datagram.payload);
            }
        }

        now += STEP;
    }

    let playout = replay_lockstep(
        &arrivals,
        config.ticks,
        config.lookahead.clone(),
        config.catch_up,
    );

    LinkReport {
        conditions: config.conditions,
        seed: config.seed,
        duplicates,
        ticks: config.ticks,
        presend_label: config.lookahead.label(),
        catch_up: config.catch_up,
        latencies,
        never_arrived: sent_at.len(),
        datagrams_sent: link.sent(),
        datagrams_dropped: link.dropped(),
        queue_drops: link.queue_drops(),
        filler_sent: link.filler_sent(),
        playout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perfect_config(ticks: usize) -> ControlDeliveryConfig {
        ControlDeliveryConfig {
            conditions: LinkConditions::perfect(),
            ticks,
            ..ControlDeliveryConfig::default()
        }
    }

    #[test]
    fn a_perfect_link_delivers_every_control_and_never_stalls() {
        let report = run_control_delivery(&perfect_config(64));

        assert_eq!(report.controls_arrived(), 64);
        assert_eq!(report.never_arrived, 0);
        assert_eq!(report.datagrams_dropped, 0);
        assert_eq!(report.over_one_period(), 0);
        assert!(
            report.playout.stalled().is_empty(),
            "an unimpaired link must not stall the playout, got {:?}",
            report.playout.stalled()
        );
        assert_eq!(report.frozen_time_fraction(), 0.0);
    }

    #[test]
    fn the_same_seed_produces_a_byte_identical_report() {
        // The whole chaos rig rests on this: a failing run must be replayable
        // from its seed alone. madsim ships the same check as
        // MADSIM_TEST_CHECK_DETERMINISM.
        let config = ControlDeliveryConfig {
            conditions: LinkConditions {
                rtt_ms: 150,
                jitter_ms: 40,
                loss_permille: 30,
                burst_ms: 0,
                ..LinkConditions::perfect()
            },
            ticks: 120,
            seed: 0x1234_5678,
            duplicates: 2,
            ..ControlDeliveryConfig::default()
        };

        let first = run_control_delivery(&config);
        let second = run_control_delivery(&config);

        assert_eq!(first.latencies, second.latencies);
        assert_eq!(first.never_arrived, second.never_arrived);
        assert_eq!(first.datagrams_sent, second.datagrams_sent);
        assert_eq!(first.datagrams_dropped, second.datagrams_dropped);
        assert_eq!(first.playout.stalls, second.playout.stalls);
        assert_eq!(first.playout.horizons, second.playout.horizons);
    }

    #[test]
    fn a_different_seed_produces_a_different_loss_pattern() {
        // Guards the inverse of the determinism check: a rig whose seed does
        // nothing would pass the test above vacuously.
        let base = ControlDeliveryConfig {
            conditions: LinkConditions {
                rtt_ms: 150,
                jitter_ms: 40,
                loss_permille: 100,
                burst_ms: 0,
                ..LinkConditions::perfect()
            },
            ticks: 200,
            seed: 1,
            ..ControlDeliveryConfig::default()
        };
        let other = ControlDeliveryConfig {
            seed: 2,
            ..base.clone()
        };

        let first = run_control_delivery(&base);
        let second = run_control_delivery(&other);

        assert!(first.datagrams_dropped > 0, "the loss injector must fire");
        assert!(second.datagrams_dropped > 0, "the loss injector must fire");
        assert_ne!(first.latencies, second.latencies);
    }

    #[test]
    fn loss_makes_control_late_and_the_playout_stall() {
        let lossy = ControlDeliveryConfig {
            conditions: LinkConditions {
                rtt_ms: 150,
                jitter_ms: 40,
                loss_permille: 50,
                burst_ms: 0,
                ..LinkConditions::perfect()
            },
            ticks: 200,
            ..ControlDeliveryConfig::default()
        };

        let report = run_control_delivery(&lossy);

        assert!(report.datagrams_dropped > 0);
        assert!(
            !report.playout.stalled().is_empty(),
            "5% loss over 200 ticks must stall at least once"
        );
        assert!(report.frozen_time_fraction() > 0.0);
    }

    #[test]
    fn burst_episodes_are_scheduled_in_time_not_per_datagram() {
        // PORT_STATUS.md:457-460 — an earlier per-datagram draw made every
        // redundant configuration manufacture its own extra loss, so redundancy
        // measured as harmful for a reason the physical link does not share.
        // Sending each datagram three times must not triple the drop count.
        let single = ControlDeliveryConfig {
            conditions: LinkConditions {
                rtt_ms: 80,
                jitter_ms: 10,
                loss_permille: 100,
                burst_ms: 60,
                ..LinkConditions::perfect()
            },
            ticks: 300,
            duplicates: 1,
            ..ControlDeliveryConfig::default()
        };
        let tripled = ControlDeliveryConfig {
            duplicates: 3,
            ..single.clone()
        };

        let one = run_control_delivery(&single);
        let three = run_control_delivery(&tripled);

        let one_rate = one.datagrams_dropped as f64 / one.datagrams_sent as f64;
        let three_rate = three.datagrams_dropped as f64 / three.datagrams_sent as f64;
        assert!(one.datagrams_dropped > 0, "the burst injector must fire");
        assert!(
            (one_rate - three_rate).abs() < 0.05,
            "drop *rate* must be a property of the link, not of how many copies \
             are sent: {one_rate:.3} with 1 copy vs {three_rate:.3} with 3"
        );
    }

    /// 33.6 kbit/s V.34 uplink = 4 200 B/s, so a 420-byte datagram occupies the
    /// link for exactly 100 ms.
    fn dialup_downlink() -> LinkConditions {
        LinkConditions {
            rtt_ms: 0,
            jitter_ms: 0,
            loss_permille: 0,
            burst_ms: 0,
            downlink_bps: 33_600,
            ..LinkConditions::perfect()
        }
    }

    #[test]
    fn a_narrow_link_serializes_a_datagram_at_its_bit_rate() {
        let mut link = Link::new(dialup_downlink(), 1);
        link.enqueue(Duration::ZERO, false, vec![0u8; 420]);

        assert!(
            link.due(Duration::from_millis(99)).is_empty(),
            "420 B at 4 200 B/s takes 100 ms to clock out"
        );
        assert_eq!(link.due(Duration::from_millis(100)).len(), 1);
    }

    #[test]
    fn datagrams_queue_behind_each_other_on_a_busy_link() {
        // The second datagram cannot start transmitting until the first has
        // finished, so it lands at 200 ms rather than 100 ms.
        let mut link = Link::new(dialup_downlink(), 1);
        link.enqueue(Duration::ZERO, false, vec![0u8; 420]);
        link.enqueue(Duration::ZERO, false, vec![0u8; 420]);

        assert_eq!(link.due(Duration::from_millis(100)).len(), 1);
        assert!(link.due(Duration::from_millis(199)).is_empty());
        assert_eq!(link.due(Duration::from_millis(200)).len(), 1);
    }

    #[test]
    fn a_finite_queue_tail_drops_once_the_standing_queue_is_full() {
        // Bufferbloat with a bound: 4 200 B of queue is one second of link at
        // dial-up rates. Everything beyond that is dropped, not buffered.
        let conditions = LinkConditions {
            queue_bytes: 4_200,
            ..dialup_downlink()
        };
        let mut link = Link::new(conditions, 1);
        for _ in 0..20 {
            link.enqueue(Duration::ZERO, false, vec![0u8; 420]);
        }

        assert_eq!(link.sent(), 20);
        assert!(
            link.dropped() > 0,
            "a bounded queue must tail-drop once it is full"
        );
        assert!(
            link.dropped() < 20,
            "it must still admit the datagrams that fit"
        );
    }

    #[test]
    fn an_unbounded_queue_bloats_instead_of_dropping() {
        // The failure mode this profile exists to reproduce: no loss at all,
        // but latency climbing without limit as the queue grows.
        let mut link = Link::new(dialup_downlink(), 1);
        for _ in 0..20 {
            link.enqueue(Duration::ZERO, false, vec![0u8; 420]);
        }

        assert_eq!(link.dropped(), 0, "an unbounded queue never drops");
        assert!(
            link.due(Duration::from_millis(1_999)).len() < 20,
            "the last datagram must still be waiting after ~2 s of bloat"
        );
        assert_eq!(
            link.due(Duration::from_millis(2_000)).len(),
            1,
            "20 x 100 ms of serialization means the last one lands at 2 s"
        );
    }

    #[test]
    fn uplink_and_downlink_are_independent() {
        // Asymmetric links are the norm, and the potato's uplink is what starves
        // first. Saturating one direction must not delay the other.
        let conditions = LinkConditions {
            downlink_bps: 33_600,
            uplink_bps: 0,
            ..dialup_downlink()
        };
        let mut link = Link::new(conditions, 1);
        link.enqueue(Duration::ZERO, false, vec![0u8; 4_200]);
        link.enqueue(Duration::ZERO, true, vec![0u8; 4_200]);

        let immediate = link.due(Duration::ZERO);
        assert_eq!(
            immediate.len(),
            1,
            "the unmetered uplink datagram should not wait behind the downlink"
        );
        assert!(immediate[0].to_host, "and it should be the uplink one");
    }

    /// The plan's `dialup` profile: 53.3 k down / 33.6 k up, 200 ms base RTT.
    fn dialup() -> LinkConditions {
        LinkConditions {
            rtt_ms: 200,
            jitter_ms: 30,
            loss_permille: 0,
            burst_ms: 0,
            downlink_bps: 53_300,
            uplink_bps: 33_600,
            queue_bytes: 0,
            cross_traffic_down_bps: 0,
            cross_traffic_up_bps: 0,
        }
    }

    #[test]
    fn a_competing_bulk_flow_inflates_control_latency_on_a_narrow_link() {
        // This is the whole point of the bandwidth model. Control alone is one
        // small datagram per 55 ms and fits on dial-up; a resource transfer
        // sharing the link is what buries it. Nothing here is lost — the link
        // has zero loss — yet control still arrives late.
        let quiet = run_control_delivery(&ControlDeliveryConfig {
            conditions: dialup(),
            ticks: 200,
            ..ControlDeliveryConfig::default()
        });
        let contended = run_control_delivery(&ControlDeliveryConfig {
            conditions: LinkConditions {
                // A bulk flow offering twice the downlink capacity.
                cross_traffic_down_bps: 106_600,
                ..dialup()
            },
            ticks: 200,
            ..ControlDeliveryConfig::default()
        });

        assert_eq!(contended.datagrams_dropped, 0, "no loss is configured");
        assert!(
            contended.mean_latency() > quiet.mean_latency() * 3,
            "a saturating bulk flow must inflate control latency: quiet {:?} vs contended {:?}",
            quiet.mean_latency(),
            contended.mean_latency()
        );
        // The signature of bufferbloat, as opposed to a merely slow link, is
        // that delay *grows*: the standing queue keeps building, so the worst
        // sample sits far above the average. A queue filling at a constant rate
        // approaches max/mean = 2; a flat link sits near 1. A fixed-delay model
        // cannot produce this shape no matter how the delay is tuned.
        let spread = |report: &LinkReport| {
            report.max_latency().as_secs_f64() / report.mean_latency().as_secs_f64().max(1e-9)
        };
        assert!(
            spread(&contended) > 1.5,
            "queueing delay should grow through the run: mean {:?}, max {:?}",
            contended.mean_latency(),
            contended.max_latency()
        );
        assert!(
            spread(&quiet) < 1.5,
            "the uncontended control arm must stay flat: mean {:?}, max {:?}",
            quiet.mean_latency(),
            quiet.max_latency()
        );
    }

    #[test]
    fn a_bounded_queue_caps_bloat_by_dropping_instead() {
        // The same contention against a 4 200 B (one second) drop-tail buffer.
        // Latency stops growing without limit; the cost moves to loss.
        let unbounded = run_control_delivery(&ControlDeliveryConfig {
            conditions: LinkConditions {
                cross_traffic_down_bps: 106_600,
                ..dialup()
            },
            ticks: 200,
            ..ControlDeliveryConfig::default()
        });
        let bounded = run_control_delivery(&ControlDeliveryConfig {
            conditions: LinkConditions {
                cross_traffic_down_bps: 106_600,
                queue_bytes: 4_200,
                ..dialup()
            },
            ticks: 200,
            ..ControlDeliveryConfig::default()
        });

        assert_eq!(
            unbounded.queue_drops, 0,
            "an unbounded buffer never drops, it only bloats"
        );
        assert!(
            bounded.queue_drops > 0,
            "a bounded buffer shows congestion as loss"
        );
        assert!(
            bounded.max_latency() < unbounded.max_latency(),
            "and it caps the worst-case delay: bounded {:?} vs unbounded {:?}",
            bounded.max_latency(),
            unbounded.max_latency()
        );
    }

    #[test]
    fn an_unmetered_link_is_unchanged_by_the_bandwidth_model() {
        // The recorded PORT_STATUS measurements must keep reproducing, so a
        // profile that sets no rate must behave exactly as before.
        let config = ControlDeliveryConfig {
            conditions: LinkConditions {
                rtt_ms: 80,
                jitter_ms: 20,
                loss_permille: 10,
                burst_ms: 0,
                ..LinkConditions::perfect()
            },
            ticks: 200,
            duplicates: 2,
            catch_up: true,
            lookahead: Lookahead::CppMean {
                average_us: 0,
                target_fps: 38,
            },
            ..ControlDeliveryConfig::default()
        };
        let report = run_control_delivery(&config);

        assert_eq!(report.conditions.downlink_bps, 0, "unmetered by default");
        assert_eq!(report.conditions.queue_bytes, 0, "unbounded by default");
        assert!(report.controls_arrived() > 0);
    }

    #[test]
    fn the_cpp_horizon_converts_microseconds_to_whole_clamped_frames() {
        // C4GameControlNetwork.cpp:382-447 — BoundBy(fps * avg / 1e6 + 1, 1, 15).
        assert_eq!(
            Lookahead::frames_to_duration(0, 38),
            Duration::from_micros(1_000_000 / 38)
        );
        // A six-second link saturates the clamp at 15 frames.
        assert_eq!(
            Lookahead::frames_to_duration(6_000_000, 38),
            Duration::from_micros(15 * 1_000_000 / 38)
        );
    }

    #[test]
    fn the_adaptive_horizon_exceeds_the_cpp_mean_on_a_jittery_link() {
        // The divergence recorded in PORT_STATUS.md:359-393: the mean sits below
        // roughly half of all delivery times, so it stalls on about half the
        // ticks; the envelope covers the tail it was blind to.
        let jittery = LinkConditions {
            rtt_ms: 150,
            jitter_ms: 40,
            loss_permille: 30,
            burst_ms: 0,
            ..LinkConditions::perfect()
        };
        let cpp = run_control_delivery(&ControlDeliveryConfig {
            conditions: jittery,
            ticks: 300,
            lookahead: Lookahead::CppMean {
                average_us: 0,
                target_fps: 38,
            },
            ..ControlDeliveryConfig::default()
        });
        let adaptive = run_control_delivery(&ControlDeliveryConfig {
            conditions: jittery,
            ticks: 300,
            lookahead: Lookahead::Adaptive {
                estimator: ControlLatencyEstimator::new(),
                target_fps: 38,
            },
            ..ControlDeliveryConfig::default()
        });

        assert!(
            mean(&adaptive.playout.horizons) > mean(&cpp.playout.horizons),
            "adaptive horizon {:?} should exceed the mean-only horizon {:?}",
            mean(&adaptive.playout.horizons),
            mean(&cpp.playout.horizons)
        );
        assert!(
            adaptive.frozen_time_fraction() < cpp.frozen_time_fraction(),
            "adaptive frozen {:.4} should beat mean-only {:.4}",
            adaptive.frozen_time_fraction(),
            cpp.frozen_time_fraction()
        );
    }
}

#[cfg(test)]
mod bulk_stream_tests {
    use super::*;

    fn lossy() -> LinkConditions {
        LinkConditions {
            rtt_ms: 80,
            jitter_ms: 20,
            loss_permille: 20,
            ..LinkConditions::perfect()
        }
    }

    fn control_under_bulk(bulk_packet_bytes: usize) -> (Duration, Duration) {
        let mut worst = Duration::ZERO;
        let mut mean_total = Duration::ZERO;
        let seeds = [1u64, 2, 3, 4, 5, 6, 7, 8];
        for seed in seeds {
            let report = run_control_delivery(&ControlDeliveryConfig {
                conditions: lossy(),
                ticks: 300,
                seed,
                duplicates: 3,
                bulk_packet_bytes,
                bulk_interval: Duration::from_millis(500),
                catch_up: true,
                lookahead: Lookahead::Adaptive {
                    estimator: ControlLatencyEstimator::new(),
                    target_fps: 38,
                },
                ..ControlDeliveryConfig::default()
            });
            worst = worst.max(report.max_latency());
            mean_total += report.mean_latency();
        }
        (mean_total / seeds.len() as u32, worst)
    }

    #[test]
    fn bulk_on_the_same_ordered_stream_delays_control_in_proportion_to_its_size() {
        // The mechanism behind the multi-second freezes, measured through the
        // real reliable-UDP layer rather than argued. Delivery is strictly
        // ordered, so a chunk's fragments take sequence numbers ahead of every
        // later control packet and one lost fragment withholds all of them.
        // Outstanding bulk per connection is `per_peer_cap * chunk_size`, so
        // these are the three configurations that matter.
        let (quiet_mean, quiet_worst) = control_under_bulk(0);
        let (cpp_mean, cpp_worst) = control_under_bulk(300 * 1024); // 100 KiB x3
        let (now_mean, now_worst) = control_under_bulk(30 * 1024); // 10 KiB x3
        let (tight_mean, tight_worst) = control_under_bulk(10 * 1024); // 10 KiB x1

        assert!(
            cpp_mean > now_mean && now_mean > tight_mean,
            "smaller outstanding bulk must mean less delayed control: \
             C++ {cpp_mean:?} -> shipped {now_mean:?} -> in-game {tight_mean:?}"
        );
        assert!(
            cpp_worst > now_worst,
            "and the worst case must improve too: {cpp_worst:?} -> {now_worst:?}"
        );
        // The tightest setting gets control most of the way back to a link
        // carrying no bulk at all, which is the point of narrowing it in game.
        assert!(
            tight_mean < quiet_mean + (now_mean - quiet_mean) / 2,
            "in-game window should recover most of the gap: quiet {quiet_mean:?}, \
             shipped {now_mean:?}, in-game {tight_mean:?}"
        );
        assert!(quiet_worst < tight_worst, "bulk still costs something");
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DialupControlProfile {
    control_period: Duration,
    link_bps: u64,
    one_way_delay: Duration,
    loss_permille: u32,
    background_wire_bps: u64,
    background_payload_bytes: usize,
    queue_bytes: u64,
    wire_overhead_bytes: usize,
    warmup_controls: usize,
    measured_controls: usize,
    drain: Duration,
}

#[cfg(test)]
fn dialup_control_profile() -> DialupControlProfile {
    DialupControlProfile {
        control_period: Duration::from_millis(56),
        link_bps: 33_600,
        one_way_delay: Duration::from_millis(150),
        loss_permille: 20,
        background_wire_bps: 20_000,
        background_payload_bytes: 512,
        queue_bytes: 4_200,
        wire_overhead_bytes: 32,
        warmup_controls: 256,
        measured_controls: 2_049,
        drain: Duration::from_secs(30),
    }
}

#[cfg(test)]
fn dialup_control_body(tick: u32) -> Vec<u8> {
    let packet = crate::encode_control_packet(&crate::LegacyControlFrame {
        client_id: 1,
        tick,
        timestamp_ms: 0,
        controls: vec![clonk_engine::ControlPacket::PlayerControl(
            clonk_engine::PlayerControlData {
                player: 0,
                command: 1,
                data: 0,
                by_client: 1,
            },
        )],
    })
    .expect("the fixed benchmark control is encodable");
    crate::transport::encode_complete_control_packet(&packet)
        .expect("the fixed benchmark PID_Control body is encodable")
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DialupLossKey {
    stream: u64,
    packet_counter: u64,
    emission: u32,
    copy: u32,
}

#[cfg(test)]
impl DialupLossKey {
    fn endpoint(to_host: bool, packet_counter: u64, emission: u32, copy: u32) -> Self {
        Self {
            stream: u64::from(to_host),
            packet_counter,
            emission,
            copy,
        }
    }

    fn background(packet_counter: u64) -> Self {
        Self {
            stream: 2,
            packet_counter,
            emission: 0,
            copy: 0,
        }
    }

    fn draw(self, seed: u64) -> u32 {
        let mut value = seed ^ self.stream.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        for component in [
            self.packet_counter,
            u64::from(self.emission),
            u64::from(self.copy),
        ] {
            value ^= component.wrapping_add(0x9e37_79b9_7f4a_7c15);
            value = value.wrapping_mul(0xbf58_476d_1ce4_e5b9);
            value ^= value >> 27;
        }
        (value ^ (value >> 31)) as u32
    }
}

#[cfg(test)]
fn dialup_loss_trace(seed: u64, packet_counters: u64, copies: u32) -> BTreeMap<(u64, u32), bool> {
    let loss_permille = dialup_control_profile().loss_permille;
    (0..packet_counters)
        .flat_map(|counter| {
            (0..copies).map(move |copy| {
                let key = DialupLossKey::endpoint(true, counter, 0, copy);
                ((counter, copy), key.draw(seed) % 1_000 < loss_permille)
            })
        })
        .collect()
}

#[cfg(test)]
#[derive(Debug)]
struct DialupInFlight {
    deliver_at: Duration,
    to_host: bool,
    payload: Vec<u8>,
    filler: bool,
}

#[cfg(test)]
#[derive(Debug)]
struct DialupWire {
    profile: DialupControlProfile,
    seed: u64,
    busy_until: [Duration; 2],
    in_flight: Vec<DialupInFlight>,
    background_debt: u128,
    background_offered: u64,
}

#[cfg(test)]
impl DialupWire {
    fn new(profile: DialupControlProfile, seed: u64) -> Self {
        Self {
            profile,
            seed,
            busy_until: [Duration::ZERO; 2],
            in_flight: Vec::new(),
            background_debt: 0,
            background_offered: 0,
        }
    }

    fn background_offered(&self) -> u64 {
        self.background_offered
    }

    fn pump_background(&mut self, now: Duration, elapsed: Duration) {
        self.background_debt = self.background_debt.saturating_add(
            u128::from(self.profile.background_wire_bps).saturating_mul(elapsed.as_nanos()),
        );
        let charged_bytes = self
            .profile
            .background_payload_bytes
            .saturating_add(self.profile.wire_overhead_bytes);
        let charge = (charged_bytes as u128)
            .saturating_mul(8)
            .saturating_mul(1_000_000_000);
        while self.background_debt >= charge {
            self.background_debt -= charge;
            let packet_counter = self.background_offered;
            self.background_offered += 1;
            let _ = self.admit(
                now,
                true,
                vec![0; self.profile.background_payload_bytes],
                DialupLossKey::background(packet_counter),
            );
        }
    }

    fn queued_wire_bytes(&self, now: Duration, to_host: bool) -> u64 {
        let backlog = self.busy_until[usize::from(to_host)].saturating_sub(now);
        let numerator = backlog
            .as_nanos()
            .saturating_mul(u128::from(self.profile.link_bps));
        let denominator = 8_000_000_000_u128;
        u64::try_from(numerator.div_ceil(denominator)).unwrap_or(u64::MAX)
    }

    fn admit(
        &mut self,
        now: Duration,
        to_host: bool,
        payload: Vec<u8>,
        loss_key: DialupLossKey,
    ) -> Option<Duration> {
        let charged_bytes = payload
            .len()
            .saturating_add(self.profile.wire_overhead_bytes);
        let queued_bytes = self.queued_wire_bytes(now, to_host);
        if queued_bytes.saturating_add(charged_bytes as u64) > self.profile.queue_bytes {
            return None;
        }

        let slot = usize::from(to_host);
        let transmit_start = self.busy_until[slot].max(now);
        let transmit_end =
            transmit_start + serialization_time(charged_bytes, self.profile.link_bps);
        self.busy_until[slot] = transmit_end;

        if loss_key.draw(self.seed) % 1_000 < self.profile.loss_permille {
            return None;
        }

        let deliver_at = transmit_end + self.profile.one_way_delay;
        self.in_flight.push(DialupInFlight {
            deliver_at,
            to_host,
            payload,
            filler: loss_key.stream == 2,
        });
        Some(deliver_at)
    }

    fn due(&mut self, now: Duration) -> Vec<DialupInFlight> {
        let mut due = Vec::new();
        let mut pending = Vec::with_capacity(self.in_flight.len());
        for packet in std::mem::take(&mut self.in_flight) {
            if packet.deliver_at <= now {
                due.push(packet);
            } else {
                pending.push(packet);
            }
        }
        self.in_flight = pending;
        due.sort_by_key(|packet| packet.deliver_at);
        due
    }
}

#[cfg(test)]
const DIALUP_DIGEST_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;

#[cfg(test)]
fn dialup_digest_packet(mut digest: u64, payload: &[u8]) -> u64 {
    for byte in (payload.len() as u64).to_le_bytes().iter().chain(payload) {
        digest ^= u64::from(*byte);
        digest = digest.wrapping_mul(0x0000_0100_0000_01b3);
    }
    digest
}

#[cfg(test)]
fn dialup_wire_packet_counter(wire: &[u8]) -> u64 {
    let kind = crate::reliable_udp_packet_kind(wire);
    let kind_tag = wire.first().copied().unwrap_or_default() & 0x7f;
    let packet_counter = match kind {
        Some(crate::ReliableUdpPacketKind::Data) => crate::decode_reliable_udp_data_fragment(wire)
            .ok()
            .map(|packet| packet.packet_number),
        Some(crate::ReliableUdpPacketKind::Check) => crate::decode_reliable_udp_check(wire)
            .ok()
            .map(|packet| packet.packet_number),
        Some(crate::ReliableUdpPacketKind::Connect) => crate::decode_reliable_udp_connect(wire)
            .ok()
            .flatten()
            .map(|packet| packet.packet_number),
        Some(crate::ReliableUdpPacketKind::ConnectOk) => {
            crate::decode_reliable_udp_connect_ok(wire)
                .ok()
                .map(|packet| packet.packet_number)
        }
        Some(crate::ReliableUdpPacketKind::Close) => crate::decode_reliable_udp_close(wire)
            .ok()
            .map(|packet| packet.packet_number),
        _ => None,
    }
    .map(u64::from)
    .unwrap_or_else(|| dialup_digest_packet(DIALUP_DIGEST_OFFSET, wire) & 0xffff_ffff);
    (u64::from(kind_tag) << 56) | packet_counter
}

#[cfg(test)]
#[derive(Debug)]
struct DialupScheduler {
    wire: DialupWire,
    emissions: BTreeMap<(bool, u64), u32>,
    initial_copy_counts: Vec<usize>,
}

#[cfg(test)]
impl DialupScheduler {
    fn new(profile: DialupControlProfile, seed: u64) -> Self {
        Self {
            wire: DialupWire::new(profile, seed),
            emissions: BTreeMap::new(),
            initial_copy_counts: Vec::new(),
        }
    }

    fn schedule(
        &mut self,
        endpoint: &ReliableUdpEndpointCore,
        peer: SocketAddr,
        step: ReliableUdpStep,
        now: Duration,
        to_host: bool,
    ) -> Vec<ReliableUdpEvent> {
        for datagram in step.datagrams {
            let packet_counter = dialup_wire_packet_counter(&datagram.payload);
            let emission = self.emissions.entry((to_host, packet_counter)).or_default();
            let current_emission = *emission;
            *emission = emission.saturating_add(1);
            let copies = endpoint
                .redundant_copies_for(peer, &datagram.payload)
                .saturating_add(1);
            if to_host
                && current_emission == 0
                && crate::reliable_udp_packet_kind(&datagram.payload)
                    == Some(crate::ReliableUdpPacketKind::Data)
            {
                self.initial_copy_counts.push(copies);
            }
            for copy in 0..copies {
                let _ = self.wire.admit(
                    now,
                    to_host,
                    datagram.payload.clone(),
                    DialupLossKey::endpoint(to_host, packet_counter, current_emission, copy as u32),
                );
            }
        }
        step.events
    }
}

#[cfg(test)]
#[derive(Debug)]
struct DialupObservation {
    profile: DialupControlProfile,
    expected_bodies: Vec<Vec<u8>>,
    sent_at: Vec<Duration>,
    expected_digest: u64,
    received_digest: u64,
    delivered_ticks: Vec<u32>,
    total_samples: Vec<Duration>,
    added_samples: Vec<Duration>,
    payloads_exact: bool,
    disconnects: Vec<crate::ReliableUdpDisconnectReason>,
}

#[cfg(test)]
impl DialupObservation {
    fn new(profile: DialupControlProfile) -> Self {
        Self {
            profile,
            expected_bodies: Vec::new(),
            sent_at: Vec::new(),
            expected_digest: DIALUP_DIGEST_OFFSET,
            received_digest: DIALUP_DIGEST_OFFSET,
            delivered_ticks: Vec::new(),
            total_samples: Vec::new(),
            added_samples: Vec::new(),
            payloads_exact: true,
            disconnects: Vec::new(),
        }
    }

    fn record_sent(&mut self, body: Vec<u8>, now: Duration) {
        self.expected_digest = dialup_digest_packet(self.expected_digest, &body);
        self.expected_bodies.push(body);
        self.sent_at.push(now);
    }

    fn observe(&mut self, on_host: bool, events: Vec<ReliableUdpEvent>, now: Duration) {
        for event in events {
            match event {
                ReliableUdpEvent::Packet { payload, .. } if on_host => {
                    self.received_digest = dialup_digest_packet(self.received_digest, &payload);
                    let packet = crate::transport::parse_complete_packet(&payload)
                        .ok()
                        .flatten()
                        .and_then(|message| match message {
                            crate::ControlMessage::Control(packet) => Some(packet),
                            _ => None,
                        });
                    let Some(packet) = packet else {
                        self.payloads_exact = false;
                        continue;
                    };
                    let tick = packet.tick();
                    let index = tick as usize;
                    self.payloads_exact &= packet.client_id() == 1
                        && self
                            .expected_bodies
                            .get(index)
                            .is_some_and(|expected| expected == &payload);
                    self.delivered_ticks.push(tick);
                    if index >= self.profile.warmup_controls {
                        let Some(sent_at) = self.sent_at.get(index) else {
                            self.payloads_exact = false;
                            continue;
                        };
                        let total = now.saturating_sub(*sent_at);
                        self.total_samples.push(total);
                        self.added_samples
                            .push(total.saturating_sub(self.profile.one_way_delay));
                    }
                }
                ReliableUdpEvent::Packet { .. } => self.payloads_exact = false,
                ReliableUdpEvent::Disconnected { reason, .. } => self.disconnects.push(reason),
                ReliableUdpEvent::Connected { .. } | ReliableUdpEvent::Puncher(_) => {}
            }
        }
    }
}

#[cfg(test)]
#[derive(Debug)]
struct DialupControlReport {
    total_samples: Vec<Duration>,
    added_samples: Vec<Duration>,
    delivered_ticks: Vec<u32>,
    payloads_exact: bool,
    expected_digest: u64,
    received_digest: u64,
    host_status: Option<crate::ReliableUdpPeerStatus>,
    client_status: Option<crate::ReliableUdpPeerStatus>,
    disconnects: Vec<crate::ReliableUdpDisconnectReason>,
    initial_copy_counts: Vec<usize>,
}

#[cfg(test)]
fn run_dialup_control(seed: u64) -> DialupControlReport {
    let profile = dialup_control_profile();
    let host_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 40_000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 40_001);
    let mut host = ReliableUdpEndpointCore::new_at(Duration::ZERO);
    let mut client = ReliableUdpEndpointCore::new_at(Duration::ZERO);

    // Establish the real endpoints without impairments; setup is outside the
    // client-control measurement, just as it is for the public transport rig.
    let mut pending = host
        .connect_at(client_addr, Duration::ZERO)
        .datagrams
        .into_iter()
        .map(|datagram| (false, datagram.payload))
        .collect::<Vec<_>>();
    for _ in 0..64 {
        let batch = std::mem::take(&mut pending);
        if batch.is_empty() {
            break;
        }
        for (to_host, payload) in batch {
            let step = if to_host {
                host.receive_at(client_addr, &payload, Duration::ZERO)
            } else {
                client.receive_at(host_addr, &payload, Duration::ZERO)
            };
            pending.extend(
                step.datagrams
                    .into_iter()
                    .map(|datagram| (!to_host, datagram.payload)),
            );
        }
    }
    assert_eq!(
        host.peer_status(client_addr),
        Some(crate::ReliableUdpPeerStatus::Working),
        "clean benchmark handshake must establish the host"
    );
    assert_eq!(
        client.peer_status(host_addr),
        Some(crate::ReliableUdpPeerStatus::Working),
        "clean benchmark handshake must establish the client"
    );

    let mut scheduler = DialupScheduler::new(profile, seed);
    let mut observation = DialupObservation::new(profile);
    let total_controls = profile.warmup_controls + profile.measured_controls;
    let last_control_at = profile.control_period * total_controls.saturating_sub(1) as u32;
    let deadline = last_control_at + profile.drain;
    let mut next_control_at = Duration::ZERO;
    let mut tick = 0_u32;
    let mut now = Duration::ZERO;

    while now <= deadline {
        if tick < total_controls as u32 && now >= next_control_at {
            let body = dialup_control_body(tick);
            observation.record_sent(body.clone(), now);
            let step = client
                .send_packet(host_addr, &body)
                .expect("the established benchmark client accepts PID_Control");
            let events = scheduler.schedule(&client, host_addr, step, now, true);
            observation.observe(false, events, now);
            tick += 1;
            next_control_at += profile.control_period;
        }

        scheduler.wire.pump_background(now, STEP);
        for packet in scheduler.wire.due(now) {
            if packet.filler {
                continue;
            }
            if packet.to_host {
                let step = host.receive_at(client_addr, &packet.payload, now);
                let events = scheduler.schedule(&host, client_addr, step, now, false);
                observation.observe(true, events, now);
            } else {
                let step = client.receive_at(host_addr, &packet.payload, now);
                let events = scheduler.schedule(&client, host_addr, step, now, true);
                observation.observe(false, events, now);
            }
        }

        let step = host.timer_at(now);
        let events = scheduler.schedule(&host, client_addr, step, now, false);
        observation.observe(true, events, now);

        let step = client.timer_at(now);
        let events = scheduler.schedule(&client, host_addr, step, now, true);
        observation.observe(false, events, now);
        now += STEP;
    }

    DialupControlReport {
        total_samples: observation.total_samples,
        added_samples: observation.added_samples,
        delivered_ticks: observation.delivered_ticks,
        payloads_exact: observation.payloads_exact,
        expected_digest: observation.expected_digest,
        received_digest: observation.received_digest,
        host_status: host.peer_status(client_addr),
        client_status: client.peer_status(host_addr),
        disconnects: observation.disconnects,
        initial_copy_counts: scheduler.initial_copy_counts,
    }
}

#[cfg(test)]
fn dialup_median_ns(samples: &[Duration]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    u64::try_from(percentile(&sorted, 0.5).as_nanos()).expect("dial-up latency fits u64 nanos")
}

#[cfg(test)]
fn dialup_sample_nanos(samples: &[Duration]) -> Vec<u64> {
    samples
        .iter()
        .map(|sample| {
            u64::try_from(sample.as_nanos()).expect("dial-up latency sample fits u64 nanos")
        })
        .collect()
}

#[cfg(test)]
struct DialupAdaptiveBaseline {
    pooled_total: u64,
    pooled_added: u64,
    seeds: [(&'static str, u64); 20],
}

#[cfg(test)]
fn dialup_adaptive_baseline() -> DialupAdaptiveBaseline {
    // These medians were recorded with this byte-identical harness immediately
    // before the single-send policy replaced adaptive immediate copies. Keep the
    // fixture here so a squash merge does not make the comparison depend on an
    // otherwise unreachable intermediate commit or a local target artifact.
    DialupAdaptiveBaseline {
        pooled_total: 878_000_000,
        pooled_added: 728_000_000,
        seeds: [
            ("0x0000000000000001", 811_000_000),
            ("0x0000000000000002", 943_000_000),
            ("0x0000000000000003", 939_000_000),
            ("0x0000000000000005", 908_000_000),
            ("0x0000000000000008", 722_000_000),
            ("0x000000000000000d", 819_000_000),
            ("0x0000000000000015", 1_004_000_000),
            ("0x0000000000000022", 994_000_000),
            ("0x0000000000000037", 790_000_000),
            ("0x0000000000000059", 697_000_000),
            ("0x0000000000000090", 797_000_000),
            ("0x00000000000000e9", 929_000_000),
            ("0x0000000000000179", 885_000_000),
            ("0x0000000000000262", 989_000_000),
            ("0x00000000000003db", 782_000_000),
            ("0x000000000000063d", 928_000_000),
            ("0x0000000000000a18", 780_000_000),
            ("0x0000000000001055", 948_000_000),
            ("0x0000000000001a6d", 782_000_000),
            ("0x0000000000002ac2", 945_000_000),
        ],
    }
}

#[cfg(test)]
fn dialup_benchmark_report_json() -> String {
    const SEEDS: [u64; 20] = [
        0x0000_0000_0000_0001,
        0x0000_0000_0000_0002,
        0x0000_0000_0000_0003,
        0x0000_0000_0000_0005,
        0x0000_0000_0000_0008,
        0x0000_0000_0000_000d,
        0x0000_0000_0000_0015,
        0x0000_0000_0000_0022,
        0x0000_0000_0000_0037,
        0x0000_0000_0000_0059,
        0x0000_0000_0000_0090,
        0x0000_0000_0000_00e9,
        0x0000_0000_0000_0179,
        0x0000_0000_0000_0262,
        0x0000_0000_0000_03db,
        0x0000_0000_0000_063d,
        0x0000_0000_0000_0a18,
        0x0000_0000_0000_1055,
        0x0000_0000_0000_1a6d,
        0x0000_0000_0000_2ac2,
    ];

    let profile = dialup_control_profile();
    let total_controls = profile.warmup_controls + profile.measured_controls;
    let expected_ticks = (0..total_controls as u32).collect::<Vec<_>>();
    let mut pooled_total = Vec::with_capacity(SEEDS.len() * profile.measured_controls);
    let mut pooled_added = Vec::with_capacity(SEEDS.len() * profile.measured_controls);
    let mut seed_reports = Vec::with_capacity(SEEDS.len());

    for seed in SEEDS {
        let report = run_dialup_control(seed);
        assert_eq!(report.total_samples.len(), profile.measured_controls);
        assert_eq!(report.added_samples.len(), profile.measured_controls);
        assert_eq!(report.delivered_ticks, expected_ticks);
        assert!(report.payloads_exact);
        assert_eq!(report.received_digest, report.expected_digest);
        assert_eq!(
            report.host_status,
            Some(crate::ReliableUdpPeerStatus::Working)
        );
        assert_eq!(
            report.client_status,
            Some(crate::ReliableUdpPeerStatus::Working)
        );
        assert!(report.disconnects.is_empty());
        assert_eq!(report.initial_copy_counts.len(), total_controls);

        let copy_histogram = report.initial_copy_counts.iter().copied().fold(
            BTreeMap::<usize, usize>::new(),
            |mut histogram, copies| {
                *histogram.entry(copies).or_default() += 1;
                histogram
            },
        );
        pooled_total.extend_from_slice(&report.total_samples);
        pooled_added.extend_from_slice(&report.added_samples);
        seed_reports.push(serde_json::json!({
            "seed": format!("0x{seed:016x}"),
            "total_p50_ns": dialup_median_ns(&report.total_samples),
            "added_p50_ns": dialup_median_ns(&report.added_samples),
            "total_samples_ns": dialup_sample_nanos(&report.total_samples),
            "added_samples_ns": dialup_sample_nanos(&report.added_samples),
            "initial_copy_histogram": copy_histogram,
            "controls_delivered": report.delivered_ticks.len(),
            "digest": format!("0x{:016x}", report.received_digest),
            "host_status": "working",
            "client_status": "working",
            "disconnects": 0,
        }));
    }

    serde_json::json!({
        "schema": "clonk-dialup-control-v1",
        "profile": {
            "direction": "client-to-host",
            "control_period_ms": profile.control_period.as_millis(),
            "link_bps_each_direction": profile.link_bps,
            "rtt_ms": profile.one_way_delay.as_millis() * 2,
            "loss_permille": profile.loss_permille,
            "loss_model": "independent-counter-keyed-after-serialization",
            "background_direction": "client-to-host",
            "background_wire_bps": profile.background_wire_bps,
            "background_payload_bytes": profile.background_payload_bytes,
            "queue_bytes": profile.queue_bytes,
            "wire_overhead_bytes": profile.wire_overhead_bytes,
            "warmup_controls": profile.warmup_controls,
            "measured_controls": profile.measured_controls,
            "drain_ms": profile.drain.as_millis(),
        },
        "pooled": {
            "samples": pooled_total.len(),
            "total_p50_ns": dialup_median_ns(&pooled_total),
            "added_p50_ns": dialup_median_ns(&pooled_added),
        },
        "seeds": seed_reports,
    })
    .to_string()
}

#[cfg(test)]
mod dialup_control_tests {
    use super::*;

    #[test]
    fn dialup_profile_pins_the_acceptance_conditions() {
        let profile = dialup_control_profile();

        assert_eq!(profile.control_period, Duration::from_millis(56));
        assert_eq!(profile.link_bps, 33_600);
        assert_eq!(profile.one_way_delay, Duration::from_millis(150));
        assert_eq!(profile.loss_permille, 20);
        assert_eq!(profile.background_wire_bps, 20_000);
        assert_eq!(profile.background_payload_bytes, 512);
        assert_eq!(profile.queue_bytes, 4_200);
        assert_eq!(profile.wire_overhead_bytes, 32);
        assert_eq!(profile.warmup_controls, 256);
        assert_eq!(profile.measured_controls, 2_049);
        assert_eq!(profile.drain, Duration::from_secs(30));
    }

    #[test]
    fn dialup_wire_charges_udp_ip_bytes_before_propagation() {
        let mut profile = dialup_control_profile();
        profile.loss_permille = 0;
        let mut wire = DialupWire::new(profile, 1);

        let delivered_at = wire
            .admit(
                Duration::ZERO,
                true,
                vec![0; 388],
                DialupLossKey::endpoint(true, 0, 0, 0),
            )
            .expect("a lossless empty queue admits the packet");

        assert_eq!(delivered_at, Duration::from_millis(250));
        assert!(wire.due(Duration::from_millis(249)).is_empty());
        assert_eq!(wire.due(Duration::from_millis(250)).len(), 1);
    }

    #[test]
    fn dialup_loss_is_counter_keyed_across_copy_policies() {
        let single = dialup_loss_trace(0x5eed_1234, 1_000, 1);
        let tripled = dialup_loss_trace(0x5eed_1234, 1_000, 3);
        let original = |trace: &BTreeMap<(u64, u32), bool>| {
            trace
                .iter()
                .filter_map(|(&(counter, copy), &lost)| (copy == 0).then_some((counter, lost)))
                .collect::<Vec<_>>()
        };

        assert_eq!(original(&single), original(&tripled));
        assert!(
            original(&single).iter().any(|(_, lost)| *lost),
            "the fixed trace must exercise loss"
        );
    }

    #[test]
    fn dialup_background_rate_counts_wire_bytes() {
        let mut profile = dialup_control_profile();
        profile.loss_permille = 0;
        let mut wire = DialupWire::new(profile, 1);

        wire.pump_background(Duration::ZERO, Duration::from_millis(1_088));

        assert_eq!(wire.background_offered(), 5);
        assert_eq!(wire.queued_wire_bytes(Duration::ZERO, true), 5 * 544);
        assert_eq!(wire.queued_wire_bytes(Duration::ZERO, false), 0);
    }

    #[test]
    fn dialup_sample_is_a_real_one_player_pid_control() {
        // The client in central mode sends its own `PID_Control` to the host
        // (oracle-src-pinned src/C4GameControlNetwork.cpp:156-168).
        let body = dialup_control_body(7);
        let message = crate::transport::parse_complete_packet(&body)
            .expect("the generated PID_Control parses")
            .expect("PID_Control is not ignored");
        let crate::ControlMessage::Control(packet) = message else {
            panic!("expected PID_Control, got {message:?}");
        };
        let frame = crate::decode_control_packet(&packet).expect("the control list decodes");

        assert_eq!((frame.client_id, frame.tick), (1, 7));
        assert_eq!(
            frame.controls,
            vec![clonk_engine::ControlPacket::PlayerControl(
                clonk_engine::PlayerControlData {
                    player: 0,
                    command: 1,
                    data: 0,
                    by_client: 1,
                }
            )]
        );
    }

    #[test]
    fn single_copy_control_and_background_fit_the_charged_uplink() {
        let profile = dialup_control_profile();
        let total_controls = profile.warmup_controls + profile.measured_controls;
        let duration_ns = profile.control_period.as_nanos() * total_controls as u128;
        let control_wire_bytes = (0..total_controls as u32)
            .map(|tick| {
                let fragments =
                    crate::encode_reliable_udp_data_fragments(0, &dialup_control_body(tick))
                        .expect("the benchmark control is encodable");
                assert_eq!(fragments.len(), 1);
                fragments[0].len() + profile.wire_overhead_bytes
            })
            .sum::<usize>() as u128;
        let background_bit_ns = u128::from(profile.background_wire_bps).saturating_mul(duration_ns);
        let control_bit_ns = control_wire_bytes
            .saturating_mul(8)
            .saturating_mul(1_000_000_000);
        let capacity_bit_ns = u128::from(profile.link_bps).saturating_mul(duration_ns);

        assert!(background_bit_ns + control_bit_ns <= capacity_bit_ns);
        assert!(background_bit_ns + control_bit_ns * 2 > capacity_bit_ns);
    }

    #[test]
    fn dialup_run_preserves_control_integrity_and_raw_samples() {
        let profile = dialup_control_profile();
        let report = run_dialup_control(0x5eed_1234);
        let total_controls = profile.warmup_controls + profile.measured_controls;

        assert_eq!(report.total_samples.len(), profile.measured_controls);
        assert_eq!(report.added_samples.len(), profile.measured_controls);
        assert_eq!(
            report.delivered_ticks,
            (0..total_controls as u32).collect::<Vec<_>>()
        );
        assert!(report.payloads_exact);
        assert_eq!(report.received_digest, report.expected_digest);
        assert_eq!(
            report.host_status,
            Some(crate::ReliableUdpPeerStatus::Working)
        );
        assert_eq!(
            report.client_status,
            Some(crate::ReliableUdpPeerStatus::Working)
        );
        assert!(report.disconnects.is_empty());
        assert_eq!(report.initial_copy_counts.len(), total_controls);
        assert!(
            report.initial_copy_counts.iter().all(|copies| *copies == 1),
            "C++ sends one physical reliable-UDP fragment before repair"
        );
        assert!(report
            .total_samples
            .iter()
            .zip(&report.added_samples)
            .all(|(total, added)| *added == total.saturating_sub(profile.one_way_delay)));
    }

    #[test]
    fn single_copy_halves_each_paired_dialup_seed() {
        let baseline = dialup_adaptive_baseline();
        let report: serde_json::Value = serde_json::from_str(&dialup_benchmark_report_json())
            .expect("the deterministic benchmark report is valid JSON");
        let pooled_candidate = report["pooled"]["total_p50_ns"]
            .as_u64()
            .expect("the report contains a total median");

        assert!(
            pooled_candidate.saturating_mul(2) <= baseline.pooled_total,
            "pooled total median must fall by at least 50%: {} -> {pooled_candidate} ns",
            baseline.pooled_total
        );
        let pooled_added_candidate = report["pooled"]["added_p50_ns"]
            .as_u64()
            .expect("the report contains a propagation-excluded median");
        assert!(
            pooled_added_candidate.saturating_mul(2) <= baseline.pooled_added,
            "pooled propagation-excluded median must fall by at least 50%: {} -> {pooled_added_candidate} ns",
            baseline.pooled_added
        );
        let candidate_seeds = report["seeds"]
            .as_array()
            .expect("the report contains per-seed results");
        assert_eq!(candidate_seeds.len(), baseline.seeds.len());
        for (candidate, (expected_seed, baseline_p50)) in candidate_seeds.iter().zip(baseline.seeds)
        {
            let seed = candidate["seed"]
                .as_str()
                .expect("the seed identifier is a string");
            let candidate_p50 = candidate["total_p50_ns"]
                .as_u64()
                .expect("the seed report contains a total median");

            assert_eq!(seed, expected_seed);
            assert!(
                candidate_p50.saturating_mul(2) <= baseline_p50,
                "seed {seed} total median must fall by at least 50%: {baseline_p50} -> {candidate_p50} ns"
            );
        }
    }

    #[test]
    #[ignore = "explicit deterministic performance report"]
    fn dialup_20_seed_report() {
        println!("{}", dialup_benchmark_report_json());
    }
}
