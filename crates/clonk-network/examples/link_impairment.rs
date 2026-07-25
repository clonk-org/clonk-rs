//! Lockstep control delivery over an impaired link.
//!
//! Drives two real [`ReliableUdpEndpointCore`] endpoints across a simulated
//! link with configurable one-way delay, jitter and packet loss, at the control
//! cadence a live game uses (ControlRate 2 at 36 FPS => one control packet every
//! 55 ms). It reports how long each control packet took to become deliverable
//! on the far side, which is exactly the quantity a lockstep stall is made of:
//! every client blocks until the control for its next tick arrives.
//!
//! Time is virtual and the loss pattern comes from a seeded LCG, so a given
//! (seed, delay, jitter, loss) is byte-for-byte reproducible and two builds can
//! be compared directly.
//!
//! ```text
//! LC_RTT_MS=80 LC_JITTER_MS=20 LC_LOSS_PERMILLE=20 \
//!   cargo run --release -p clonk-network --example link_impairment
//! ```
//!
//! This measures the transport only. It does not run the simulation, so the
//! numbers are a floor on stall duration, not a whole-frame budget.

use std::collections::BTreeMap;
use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use clonk_network::{
    ControlLatencyEstimator, ReliableUdpEndpointCore, ReliableUdpEvent, ReliableUdpStep,
};

/// C++ `C4GameControlNetwork` pacing: ControlRate 2 at the 38 FPS default
/// target is one control packet every other frame.
const CONTROL_PERIOD: Duration = Duration::from_millis(55);
const STEP: Duration = Duration::from_millis(1);

/// Deterministic loss/jitter source. This is presentation-free test tooling, so
/// it deliberately does not touch the synchronized `Random()` stream.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, bound: u32) -> u32 {
        if bound == 0 {
            0
        } else {
            self.next_u32() % bound
        }
    }
}

struct InFlight {
    deliver_at: Duration,
    to_host: bool,
    payload: Vec<u8>,
}

struct Link {
    rtt_ms: u64,
    jitter_ms: u64,
    loss_permille: u32,
    /// Mean length of a correlated loss episode, in milliseconds. Zero gives
    /// independent Bernoulli loss. Real links drop in bursts — a queue
    /// overflows, a radio fades — and a burst is exactly the case a redundant
    /// copy sent in the same breath as the original cannot survive.
    burst_ms: u64,
    bad_until: Duration,
    next_episode_at: Duration,
    rng: Lcg,
    queue: Vec<InFlight>,
    dropped: usize,
    sent: usize,
}

impl Link {
    fn drops(&mut self, now: Duration) -> bool {
        if self.loss_permille == 0 {
            return false;
        }
        if self.burst_ms > 0 {
            // Episodes are scheduled in TIME, not drawn per datagram. Drawing
            // per datagram would make an extra copy of a 50-byte control packet
            // as likely to trigger a radio fade or a queue overflow as the
            // original, so any redundant configuration would manufacture its
            // own extra loss and measure as worse for a reason the physical
            // link does not share. With an absolute schedule the fraction of
            // datagrams landing inside a bad window converges to loss_permille
            // however many are sent, which is what makes the comparison fair.
            while now >= self.next_episode_at {
                self.bad_until = self.next_episode_at + Duration::from_millis(self.burst_ms);
                let mean_period_ms = (self.burst_ms * 1000 / u64::from(self.loss_permille).max(1))
                    .max(self.burst_ms + 1);
                let spread = u32::try_from(mean_period_ms).unwrap_or(u32::MAX).max(1);
                let period_ms = mean_period_ms / 2 + u64::from(self.rng.below(spread));
                self.next_episode_at += Duration::from_millis(period_ms.max(1));
            }
            return now < self.bad_until;
        }
        self.rng.below(1000) < self.loss_permille
    }

    /// One-way delay is half the round trip, plus a uniform jitter draw.
    fn enqueue(&mut self, now: Duration, to_host: bool, payload: Vec<u8>) {
        self.sent += 1;
        if self.drops(now) {
            self.dropped += 1;
            return;
        }
        let jitter = if self.jitter_ms == 0 {
            0
        } else {
            u64::from(self.rng.below(self.jitter_ms as u32 * 2 + 1))
        };
        let delay = Duration::from_millis(self.rtt_ms / 2 + jitter);
        self.queue.push(InFlight {
            deliver_at: now + delay,
            to_host,
            payload,
        });
    }

    fn due(&mut self, now: Duration) -> Vec<InFlight> {
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

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

/// Lockstep playout model.
///
/// A client cannot execute control tick T before its packet arrives, and it
/// paces successive ticks one control period apart. Once a late packet pushes
/// execution past its slot the whole schedule slips, which is what a player
/// perceives as the game running slow. `lookahead` is the PreSend horizon in
/// milliseconds: the budget a packet has to arrive in before it stalls anyone.
/// `catch_up` picks which of the two real client behaviors to model. With it
/// off the schedule slips permanently once a packet is late, which is the game
/// visibly running behind. With it on the client races back to its ideal slot
/// after every stall, so a late packet costs a hitch instead of drift and the
/// next late packet hitches again. The frame scheduler decides which happens.
fn replay_lockstep(
    arrivals: &BTreeMap<u32, Duration>,
    ticks: usize,
    lookahead: Lookahead,
    catch_up: bool,
) -> (Vec<Duration>, Duration, Vec<Duration>) {
    let mut stalls = Vec::new();
    let mut horizons = Vec::new();
    let mut executed_at = Duration::ZERO;
    let mut lookahead = lookahead;
    for tick in 0..ticks as u32 {
        horizons.push(lookahead.current());
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
        stalls.push(executed_at.saturating_sub(earliest));
        // The engine samples delivery time and re-sizes PreSend as it goes.
        lookahead.observe(arrived.saturating_sub(CONTROL_PERIOD * tick));
    }
    let last_slot = CONTROL_PERIOD * (ticks as u32 - 1) + lookahead.current();
    let drift = executed_at.saturating_sub(last_slot);
    (stalls, drift, horizons)
}

/// How the client sizes its PreSend horizon while the session runs.
enum Lookahead {
    /// A constant horizon, for isolating transport behavior from adaptation.
    Fixed(Duration),
    /// C++ `CalcPerformance`: a 1/150 EWMA of the mean, and nothing else.
    CppMean { average_us: i32, target_fps: i32 },
    /// The mean-plus-deviation budget from `ControlLatencyEstimator`.
    Adaptive {
        estimator: ControlLatencyEstimator,
        target_fps: i32,
    },
}

impl Lookahead {
    /// Both adaptive modes convert a microsecond budget the same way C++ does:
    /// to a whole number of frames, clamped, then back to wall-clock.
    fn frames_to_duration(budget_us: i32, target_fps: i32) -> Duration {
        let frames = (target_fps.saturating_mul(budget_us) / 1_000_000)
            .saturating_add(1)
            .clamp(1, 15);
        Duration::from_micros((frames as u64 * 1_000_000) / target_fps.max(1) as u64)
    }

    fn current(&self) -> Duration {
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

    fn observe(&mut self, delivery: Duration) {
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
}

fn main() {
    let rtt_ms = env_u64("LC_RTT_MS", 60);
    let jitter_ms = env_u64("LC_JITTER_MS", 10);
    let loss_permille = env_u64("LC_LOSS_PERMILLE", 10) as u32;
    let ticks = env_u64("LC_TICKS", 400) as usize;
    let seed = env_u64("LC_SEED", 0x5eed_1234);
    // Copies of each control datagram to put on the wire. C4NetIOUDP discards a
    // packet number below its receive cursor, so a redundant copy is wire-legal
    // and a C++ peer drops it without noticing. `LC_DUP_DELAY_MS` staggers the
    // copies so one congestion burst is less likely to take all of them.
    let duplicates = env_u64("LC_DUP", 1).max(1);
    let duplicate_delay_ms = env_u64("LC_DUP_DELAY_MS", 0);
    // PreSend horizon. `LC_PRESEND=cpp` replays C4GameControlNetwork's
    // mean-only sizing, `adaptive` replays ControlLatencyEstimator, and the
    // default holds LC_LOOKAHEAD_MS constant.
    let target_fps = env_u64("LC_TARGET_FPS", 38) as i32;
    let presend_mode = env::var("LC_PRESEND").unwrap_or_else(|_| "fixed".to_string());
    let lookahead = match presend_mode.as_str() {
        "cpp" => Lookahead::CppMean {
            average_us: 0,
            target_fps,
        },
        "adaptive" => Lookahead::Adaptive {
            estimator: ControlLatencyEstimator::new(),
            target_fps,
        },
        _ => Lookahead::Fixed(Duration::from_millis(env_u64("LC_LOOKAHEAD_MS", 0))),
    };

    let host_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_001);

    let mut now = Duration::ZERO;
    let mut host = ReliableUdpEndpointCore::new_at(now);
    let mut client = ReliableUdpEndpointCore::new_at(now);
    let mut link = Link {
        rtt_ms,
        jitter_ms,
        loss_permille,
        burst_ms: env_u64("LC_BURST_MS", 0),
        bad_until: Duration::ZERO,
        next_episode_at: Duration::ZERO,
        rng: Lcg(seed),
        queue: Vec::new(),
        dropped: 0,
        sent: 0,
    };

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
    let deadline = CONTROL_PERIOD * (ticks as u32 + 40);

    while now <= deadline {
        // The host emits one control packet per control tick.
        if tick < ticks as u32 && now >= next_control_at {
            let payload = tick.to_le_bytes().to_vec();
            if let Ok(step) = host.send_packet(client_addr, &payload) {
                sent_at.insert(tick, now);
                for datagram in step.datagrams {
                    // Each copy draws loss independently, which is the property
                    // that makes redundancy worth its bandwidth.
                    for copy in 0..duplicates {
                        let stagger = Duration::from_millis(duplicate_delay_ms * copy);
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

    let mut sorted = latencies.clone();
    sorted.sort_unstable();
    let one_way = Duration::from_millis(rtt_ms / 2);
    // A control packet that takes longer than one control period to arrive is a
    // frame the whole session waits on.
    let stalls = latencies.iter().filter(|d| **d > CONTROL_PERIOD).count();
    let total: Duration = latencies.iter().sum();

    println!("rtt              {rtt_ms}ms (one-way {one_way:?})");
    println!("jitter           +0..{}ms", jitter_ms * 2);
    println!(
        "loss             {loss_permille} permille{}",
        match env_u64("LC_BURST_MS", 0) {
            0 => " (independent)".to_string(),
            burst => format!(" (bursts of ~{burst}ms)"),
        }
    );
    println!("seed             {seed:#x}");
    println!("control period   {CONTROL_PERIOD:?}");
    println!("controls sent    {ticks}");
    println!("controls arrived {}", latencies.len());
    println!("never arrived    {}", sent_at.len());
    println!(
        "datagrams        {} sent, {} dropped",
        link.sent, link.dropped
    );
    println!(
        "mean             {:?}",
        total
            .checked_div(latencies.len().max(1) as u32)
            .unwrap_or_default()
    );
    println!("p50              {:?}", percentile(&sorted, 0.50));
    println!("p95              {:?}", percentile(&sorted, 0.95));
    println!("p99              {:?}", percentile(&sorted, 0.99));
    println!(
        "max              {:?}",
        sorted.last().copied().unwrap_or_default()
    );
    println!(
        "over one period  {stalls} ({:.1}%)",
        stalls as f64 / latencies.len().max(1) as f64 * 100.0
    );

    let catch_up = env_u64("LC_CATCHUP", 0) != 0;
    let presend_label = match &lookahead {
        Lookahead::Fixed(fixed) => format!("fixed {fixed:?}"),
        Lookahead::CppMean { .. } => "cpp mean-only".to_string(),
        Lookahead::Adaptive { .. } => "adaptive mean+deviation".to_string(),
    };
    let (lockstep_stalls, drift, horizons) = replay_lockstep(&arrivals, ticks, lookahead, catch_up);
    let stalled: Vec<Duration> = lockstep_stalls
        .iter()
        .copied()
        .filter(|stall| !stall.is_zero())
        .collect();
    let stalled_total: Duration = stalled.iter().sum();
    let mut stalled_sorted = stalled.clone();
    stalled_sorted.sort_unstable();
    let wall_clock = CONTROL_PERIOD * ticks as u32;
    println!();
    let pacing = if catch_up { "catch-up" } else { "slip" };
    println!("-- lockstep playout (presend {presend_label}, duplicates {duplicates}, {pacing}) --");
    println!(
        "frames stalled   {} of {ticks} ({:.1}%)",
        stalled.len(),
        stalled.len() as f64 / ticks as f64 * 100.0
    );
    println!("stall total      {stalled_total:?}");
    println!(
        "stall worst      {:?}",
        stalled_sorted.last().copied().unwrap_or_default()
    );
    println!("stall p99        {:?}", percentile(&stalled_sorted, 0.99));
    println!("schedule slip    {drift:?}");
    // The price of a bigger horizon is input latency, so report it next to the
    // stalls it buys off rather than letting the win stand on its own.
    let horizon_total: Duration = horizons.iter().sum();
    println!(
        "input lag mean   {:?}",
        horizon_total
            .checked_div(horizons.len().max(1) as u32)
            .unwrap_or_default()
    );
    println!(
        "input lag max    {:?}",
        horizons.iter().max().copied().unwrap_or_default()
    );
    println!(
        "time lost        {:.2}% of a {wall_clock:?} session",
        stalled_total.as_secs_f64() / wall_clock.as_secs_f64() * 100.0
    );
}
