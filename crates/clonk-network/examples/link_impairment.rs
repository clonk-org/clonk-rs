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

use clonk_network::{ReliableUdpEndpointCore, ReliableUdpEvent, ReliableUdpStep};

/// C++ `C4GameControlNetwork` pacing: ControlRate 2 at the 38 FPS default
/// target is one control packet every other frame.
const CONTROL_PERIOD: Duration = Duration::from_millis(55);
const STEP: Duration = Duration::from_millis(1);

/// Deterministic loss/jitter source. This is presentation-free test tooling, so
/// it deliberately does not touch the synchronized `Random()` stream.
struct Lcg(u64);

impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
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
    rng: Lcg,
    queue: Vec<InFlight>,
    dropped: usize,
    sent: usize,
}

impl Link {
    /// One-way delay is half the round trip, plus a uniform jitter draw.
    fn enqueue(&mut self, now: Duration, to_host: bool, payload: Vec<u8>) {
        self.sent += 1;
        if self.loss_permille > 0 && self.rng.below(1000) < self.loss_permille {
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

fn main() {
    let rtt_ms = env_u64("LC_RTT_MS", 60);
    let jitter_ms = env_u64("LC_JITTER_MS", 10);
    let loss_permille = env_u64("LC_LOSS_PERMILLE", 10) as u32;
    let ticks = env_u64("LC_TICKS", 400) as usize;
    let seed = env_u64("LC_SEED", 0x5eed_1234);

    let host_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 40_001);

    let mut now = Duration::ZERO;
    let mut host = ReliableUdpEndpointCore::new_at(now);
    let mut client = ReliableUdpEndpointCore::new_at(now);
    let mut link = Link {
        rtt_ms,
        jitter_ms,
        loss_permille,
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
                    link.enqueue(now, false, datagram.payload);
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
    println!("loss             {loss_permille} permille");
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
    println!("max              {:?}", sorted.last().copied().unwrap_or_default());
    println!(
        "over one period  {stalls} ({:.1}%)",
        stalls as f64 / latencies.len().max(1) as f64 * 100.0
    );
}
