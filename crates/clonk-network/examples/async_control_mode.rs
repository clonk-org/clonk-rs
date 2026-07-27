//! What one slow peer costs everybody else, and what ControlMode 2 buys.
//!
//! A lockstep host cannot publish tick T until every client's control for T has
//! arrived, so the slowest link sets the pace for the whole session. C++'s
//! `CNM_Async` bounds that: once the host has waited
//! `ControlRate * AsyncMaxWait * 1000 / TargetFPS` past the moment it needed the
//! tick, it packs whichever clients did arrive and broadcasts that
//! (oracle-src-pinned src/C4GameControlNetwork.cpp:741-784). The absent client's
//! input is dropped, not deferred.
//!
//! This drives the real [`ControlCoordinator`] — the same aggregation the host
//! loop uses, including `force_current_tick` — across per-client impaired links,
//! and reports both sides of that trade: how late the shared tick became ready
//! (what every *other* player feels) and how many inputs the slow client lost
//! (what that player pays).
//!
//! ```text
//! LC_CLIENTS=8 LC_BAD_RTT_MS=300 LC_BAD_LOSS_PERMILLE=50 \
//!   cargo run --release -p clonk-network --example async_control_mode
//! ```

use std::collections::BTreeMap;
use std::env;
use std::time::Duration;

use clonk_network::{ClientId, ControlCoordinator, ControlLatencyEstimator, ControlPacket, Tick};

/// ControlRate 2 at the 36 FPS in-game tick.
const CONTROL_PERIOD: Duration = Duration::from_millis(55);
const STEP: Duration = Duration::from_millis(1);

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

/// C++ `C4GameControlNetwork::PackCompleteCtrl` deadline
/// (src/C4GameControlNetwork.cpp:754). Mirrored rather than imported because
/// `strict_async_control_wait` is crate-private to the session module.
fn async_budget(control_rate: i32, async_max_wait: i32, target_fps: i32) -> Duration {
    let ms = i64::from(control_rate) * i64::from(async_max_wait) * 1_000 / i64::from(target_fps);
    Duration::from_millis(ms.max(0) as u64 + 1)
}

/// When the host needs tick T, independent of when anyone sent it.
fn host_needs(tick: Tick) -> Duration {
    CONTROL_PERIOD * tick
}

struct Client {
    id: ClientId,
    rtt_ms: u64,
    jitter_ms: u64,
    loss_permille: u32,
    /// (arrival time, tick, sent at) for control still in flight to the host.
    in_flight: Vec<(Duration, Tick, Duration)>,
    /// The real PreSend sizer this port ships. Each client adapts to its own
    /// link, which is the whole point: a slow peer buys its own headroom
    /// instead of making the host wait.
    estimator: ControlLatencyEstimator,
    presend: bool,
    target_fps: i32,
    next_tick: Tick,
    presend_samples: Vec<Duration>,
}

impl Client {
    /// C++ converts the budget to whole frames and clamps to 1..15
    /// (src/C4GameControlNetwork.cpp:382-447).
    fn presend_horizon(&self) -> Duration {
        if !self.presend {
            return Duration::ZERO;
        }
        let frames = (self.target_fps.saturating_mul(self.estimator.budget_us()) / 1_000_000)
            .saturating_add(1)
            .clamp(1, 15);
        Duration::from_micros((frames as u64 * 1_000_000) / self.target_fps.max(1) as u64)
    }

    /// Retries a lost control packet on the next control period, which is what
    /// the reliable-UDP repair path effectively does for a dropped datagram.
    fn send(&mut self, now: Duration, tick: Tick, rng: &mut Lcg) {
        let sent_at = now;
        let mut attempt = now;
        loop {
            if self.loss_permille == 0 || rng.below(1000) >= self.loss_permille {
                let jitter = if self.jitter_ms == 0 {
                    0
                } else {
                    u64::from(rng.below(self.jitter_ms as u32 * 2 + 1))
                };
                let delay = Duration::from_millis(self.rtt_ms / 2 + jitter);
                self.in_flight.push((attempt + delay, tick, sent_at));
                return;
            }
            attempt += CONTROL_PERIOD;
            if attempt > now + CONTROL_PERIOD * 8 {
                return; // give up; models a tick that never lands
            }
        }
    }
}

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn percentile(sorted: &[Duration], fraction: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

struct Outcome {
    lateness: Vec<Duration>,
    dropped_inputs: usize,
    ticks_published: usize,
    slow_presend: Duration,
    good_presend: Duration,
}

fn run(
    async_mode: bool,
    presend: bool,
    clients: usize,
    ticks: usize,
    seed: u64,
    bad: (u64, u64, u32),
) -> Outcome {
    let mut rng = Lcg(seed);
    let budget = async_budget(2, env_u64("LC_ASYNC_MAX_WAIT", 2) as i32, 38);

    let mut peers: Vec<Client> = (0..clients)
        .map(|index| {
            let slow = index == 0;
            Client {
                id: index as ClientId,
                rtt_ms: if slow {
                    bad.0
                } else {
                    env_u64("LC_GOOD_RTT_MS", 60)
                },
                jitter_ms: if slow {
                    bad.1
                } else {
                    env_u64("LC_GOOD_JITTER_MS", 10)
                },
                loss_permille: if slow {
                    bad.2
                } else {
                    env_u64("LC_GOOD_LOSS_PERMILLE", 5) as u32
                },
                in_flight: Vec::new(),
                estimator: ControlLatencyEstimator::new(),
                presend,
                target_fps: 38,
                next_tick: 0,
                presend_samples: Vec::new(),
            }
        })
        .collect();

    let mut coordinator = ControlCoordinator::new(256);
    for peer in &peers {
        coordinator
            .register_client(peer.id)
            .expect("client registers");
    }

    let mut now = Duration::ZERO;
    let mut tick: Tick = 0;
    // When the host first needed the tick it is currently blocked on; C++'s
    // `iWaitStart`, set on reaching a control tick and cleared on publishing.
    let mut wait_start: Option<(Tick, Duration)> = None;
    let mut scheduled: BTreeMap<Tick, Duration> = BTreeMap::new();
    let mut outcome = Outcome {
        lateness: Vec::new(),
        dropped_inputs: 0,
        ticks_published: 0,
        slow_presend: Duration::ZERO,
        good_presend: Duration::ZERO,
    };
    let deadline = CONTROL_PERIOD * (ticks as u32 + 64);

    while now <= deadline {
        // Each client sends tick T early by its own PreSend horizon, so a slow
        // link buys headroom for itself rather than stalling the host.
        for peer in &mut peers {
            while (peer.next_tick as usize) < ticks {
                let due = host_needs(peer.next_tick).saturating_sub(peer.presend_horizon());
                if now < due {
                    break;
                }
                peer.presend_samples.push(peer.presend_horizon());
                let next = peer.next_tick;
                peer.send(now, next, &mut rng);
                peer.next_tick += 1;
            }
        }
        while (tick as usize) < ticks && host_needs(tick) <= now {
            scheduled.insert(tick, host_needs(tick));
            tick += 1;
        }

        let mut settled_now: Vec<clonk_network::ReadyBatch> = Vec::new();
        for peer in &mut peers {
            let arrived: Vec<(Tick, Duration)> = peer
                .in_flight
                .iter()
                .filter(|(at, _, _)| *at <= now)
                .map(|(_, t, sent)| (*t, *sent))
                .collect();
            peer.in_flight.retain(|(at, _, _)| *at > now);
            for (t, sent_at) in arrived {
                // The client learns what its link cost and re-sizes PreSend.
                let sample = now
                    .saturating_sub(sent_at)
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                peer.estimator.observe(sample);
                let packet = ControlPacket::builder(peer.id, t).payload(vec![0u8; 8]);
                if let Ok(outcome_of) = coordinator.ingest(packet) {
                    // Ordinary all-client packing completes inside `ingest`.
                    settled_now.extend(outcome_of.ready);
                }
            }
        }

        let current = coordinator.current_tick();
        if scheduled.contains_key(&current) && wait_start.is_none_or(|(t, _)| t != current) {
            wait_start = Some((current, now.max(scheduled[&current])));
        }

        let mut batches = settled_now;
        // `ingest` already drained anything that completed normally; this only
        // forces the tick the host has now waited too long for.
        if async_mode {
            if let Some((waiting_tick, started)) = wait_start {
                if waiting_tick == coordinator.current_tick() && now >= started + budget {
                    let forced = coordinator.force_current_tick();
                    for batch in &forced {
                        outcome.dropped_inputs += clients.saturating_sub(batch.packets().len());
                    }
                    batches.extend(forced);
                }
            }
        }

        for batch in batches {
            if let Some(sent_at) = scheduled.get(&batch.tick()) {
                outcome.lateness.push(now.saturating_sub(*sent_at));
                outcome.ticks_published += 1;
            }
            wait_start = None;
        }

        now += STEP;
    }

    let mean_of = |samples: &[Duration]| {
        samples
            .iter()
            .sum::<Duration>()
            .checked_div(samples.len().max(1) as u32)
            .unwrap_or_default()
    };
    outcome.slow_presend = mean_of(&peers[0].presend_samples);
    let good: Vec<Duration> = peers[1..]
        .iter()
        .flat_map(|p| p.presend_samples.clone())
        .collect();
    outcome.good_presend = mean_of(&good);
    outcome
}

fn main() {
    let clients = env_u64("LC_CLIENTS", 4) as usize;
    let ticks = env_u64("LC_TICKS", 400) as usize;
    let bad = (
        env_u64("LC_BAD_RTT_MS", 250),
        env_u64("LC_BAD_JITTER_MS", 60),
        env_u64("LC_BAD_LOSS_PERMILLE", 50) as u32,
    );
    let seeds: Vec<u64> = (0..16).map(|i| 0x5eed_1234 + i * 7919).collect();

    println!(
        "{clients} clients, one slow ({}ms rtt, +-{}ms jitter, {} permille loss); \
         others {}ms/{}ms/{} permille",
        bad.0,
        bad.1,
        bad.2,
        env_u64("LC_GOOD_RTT_MS", 60),
        env_u64("LC_GOOD_JITTER_MS", 10),
        env_u64("LC_GOOD_LOSS_PERMILLE", 5),
    );
    println!(
        "async budget {:?}\n",
        async_budget(2, env_u64("LC_ASYNC_MAX_WAIT", 2) as i32, 38)
    );
    println!(
        "{:<24} {:>9} {:>9} {:>9} {:>9} {:>10} {:>11}",
        "mode", "mean", "p95", "p99", "max", "pkts lost", "presend"
    );

    for (label, async_mode, presend) in [
        ("0 decentral, no presend", false, false),
        ("2 async, no presend", true, false),
        ("0 decentral + presend", false, true),
        ("2 async + presend", true, true),
    ] {
        let mut all = Vec::new();
        let mut dropped = 0usize;
        let mut slow_presend = Vec::new();
        for seed in &seeds {
            let outcome = run(async_mode, presend, clients, ticks, *seed, bad);
            all.extend(outcome.lateness);
            dropped += outcome.dropped_inputs;
            slow_presend.push(outcome.slow_presend);
        }
        all.sort_unstable();
        let total: Duration = all.iter().sum();
        let mean = total
            .checked_div(all.len().max(1) as u32)
            .unwrap_or_default();
        let slow_mean = slow_presend
            .iter()
            .sum::<Duration>()
            .checked_div(slow_presend.len().max(1) as u32)
            .unwrap_or_default();
        println!(
            "{label:<24} {:>9?} {:>9?} {:>9?} {:>9?} {:>10} {:>11?}",
            mean,
            percentile(&all, 0.95),
            percentile(&all, 0.99),
            all.last().copied().unwrap_or_default(),
            dropped,
            slow_mean,
        );
    }
}
