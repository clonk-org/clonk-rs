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
}

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
}

impl Default for LinkConditions {
    fn default() -> Self {
        Self {
            rtt_ms: 60,
            jitter_ms: 10,
            loss_permille: 10,
            burst_ms: 0,
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
        }
    }

    pub fn one_way(&self) -> Duration {
        Duration::from_millis(self.rtt_ms / 2)
    }
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
        }
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

    /// One-way delay is half the round trip, plus a uniform jitter draw.
    pub fn enqueue(&mut self, now: Duration, to_host: bool, payload: Vec<u8>) {
        self.sent += 1;
        if self.drops(now) {
            self.dropped += 1;
            return;
        }
        let jitter = if self.conditions.jitter_ms == 0 {
            0
        } else {
            u64::from(self.rng.below(self.conditions.jitter_ms as u32 * 2 + 1))
        };
        let delay = Duration::from_millis(self.conditions.rtt_ms / 2 + jitter);
        self.queue.push(InFlight {
            deliver_at: now + delay,
            to_host,
            payload,
        });
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

    /// Fraction of the session spent blocked on a late control packet.
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
    let mut tick: u32 = 0;
    let deadline = CONTROL_PERIOD * (config.ticks as u32 + 40);
    let duplicates = config.duplicates.max(1);

    while now <= deadline {
        // The host emits one control packet per control tick.
        if tick < config.ticks as u32 && now >= next_control_at {
            let payload = tick.to_le_bytes().to_vec();
            if let Ok(step) = host.send_packet(client_addr, &payload) {
                sent_at.insert(tick, now);
                for datagram in step.datagrams {
                    // Each copy draws loss independently, which is the property
                    // that makes redundancy worth its bandwidth.
                    for copy in 0..duplicates {
                        let stagger = Duration::from_millis(config.duplicate_delay_ms * copy);
                        link.enqueue(now + stagger, false, datagram.payload.clone());
                    }
                }
            }
            tick += 1;
            next_control_at += CONTROL_PERIOD;
        }

        for item in link.due(now) {
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
