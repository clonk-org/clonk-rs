//! A whole lockstep session under load: many clients, each with its own link
//! *and its own machine*.
//!
//! [`crate::sim`] answers "what does this link do to control delivery". This
//! answers the question that actually matters for a mixed group: **what does one
//! bad participant cost everybody else?** It drives the real
//! [`ControlCoordinator`], including `force_current_tick`, so the async deadline
//! and the aggregation are the shipped ones rather than a model of them.
//!
//! The piece that does not exist anywhere else in the repo is the CPU model. A
//! slow *computer* is invisible to ping, so it is invisible to the PreSend
//! horizon (`C4GameControlNetwork::CalcPerformance` derives that from ping alone,
//! and `iTargetFPS` is a hardcoded constant rather than a measurement). Its only
//! symptom is that it reaches each control tick later than the last, so its own
//! input is stamped later and later until the host stops waiting for it. That is
//! modelled here directly:
//!
//! ```text
//! reach(T) = max( reach(T-1) + cpu_cost_per_control_tick, aggregate_arrival(T) )
//! ```
//!
//! and the client stamps its input for tick `T + presend` at `reach(T)`. A
//! machine whose `cpu_cost_per_control_tick` exceeds the control period can never
//! recover: the left-hand term grows without bound, which is exactly the
//! "everyone waits for the slowest" failure lockstep is famous for.

use std::collections::BTreeMap;
use std::time::Duration;

use crate::control::{ControlCoordinator, ControlPacket};
use crate::control_latency::ControlLatencyEstimator;
use crate::sim::{Link, LinkConditions, SimRng, CONTROL_PERIOD, STEP};
use crate::udp::RELIABLE_UDP_RECHECK_INTERVAL;
use crate::{ClientId, Tick};

/// C++ `C4Application` runs the in-game tick at 28 ms; ControlRate 2 makes one
/// control tick two frames, which is the 55 ms [`CONTROL_PERIOD`].
pub const FRAME_INTERVAL: Duration = Duration::from_millis(28);

/// How expensive one simulation frame is on a given machine.
///
/// Multipliers are relative to the M4 Max reference the engine's
/// `scenario_profile` numbers were taken on. Simulation and blitting are kept
/// apart because they diverge by 2-5x on small ARM cores: a Pi 1 is ~16x slower
/// than a modern core on branchy integer work but ~75x slower on memory
/// bandwidth, and a single scalar would be wrong for both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CpuProfile {
    /// Integer/fixed-point simulation multiplier.
    pub k_sim: u32,
    /// Memory-bandwidth (software blit) multiplier. Reserved for the render
    /// budget; simulation pacing uses `k_sim`.
    pub k_blit: u32,
    /// Per-frame cost on the reference machine.
    pub reference_frame: Duration,
    /// Chance per control tick of an SD-card-class I/O stall, in per mille.
    pub io_stall_permille: u32,
    /// How long such a stall lasts.
    pub io_stall_ms: u64,
}

impl CpuProfile {
    /// The reference machine: everything runs at its measured cost.
    pub fn reference() -> Self {
        Self {
            k_sim: 1,
            k_blit: 1,
            // ClonkMars 03_Chaos, 128 objects: 3.90 ms mean on an M4 Max.
            reference_frame: Duration::from_micros(3_900),
            io_stall_permille: 0,
            io_stall_ms: 0,
        }
    }

    /// Raspberry Pi 4 B class: `K_sim` 9, `K_blit` 10.
    pub fn pi4() -> Self {
        Self {
            k_sim: 9,
            k_blit: 10,
            io_stall_permille: 5,
            io_stall_ms: 40,
            ..Self::reference()
        }
    }

    /// Pi Zero 2 W / Pi 3 class — the plan's default `potato`.
    pub fn potato() -> Self {
        Self {
            k_sim: 20,
            k_blit: 25,
            io_stall_permille: 10,
            io_stall_ms: 75,
            ..Self::reference()
        }
    }

    /// Pi 1 / Zero, single core, ARMv6, no NEON.
    pub fn pi1() -> Self {
        Self {
            k_sim: 55,
            k_blit: 75,
            io_stall_permille: 20,
            io_stall_ms: 125,
            ..Self::reference()
        }
    }

    /// What one simulation frame costs on this machine, before I/O stalls.
    pub fn frame_cost(&self) -> Duration {
        self.reference_frame * self.k_sim
    }

    /// What one control tick costs, at `control_rate` frames per control tick.
    ///
    /// A machine is viable exactly when this stays under [`CONTROL_PERIOD`].
    pub fn control_tick_cost(&self, control_rate: u32) -> Duration {
        self.frame_cost() * control_rate
    }

    /// True when this machine cannot sustain the tick rate even on average, so
    /// no amount of network tuning will keep it in the session.
    pub fn is_overloaded(&self, control_rate: u32) -> bool {
        self.control_tick_cost(control_rate) > CONTROL_PERIOD
    }

    fn io_stall(&self, rng: &mut SimRng) -> Duration {
        if self.io_stall_permille == 0 || rng.below(1000) >= self.io_stall_permille {
            return Duration::ZERO;
        }
        Duration::from_millis(self.io_stall_ms)
    }
}

/// One participant's machine and link.
#[derive(Debug, Clone)]
pub struct ClientProfile {
    pub conditions: LinkConditions,
    pub cpu: CpuProfile,
}

impl ClientProfile {
    /// A healthy participant: fast machine, ordinary broadband.
    pub fn healthy() -> Self {
        Self {
            conditions: LinkConditions {
                rtt_ms: 60,
                jitter_ms: 10,
                loss_permille: 5,
                ..LinkConditions::perfect()
            },
            cpu: CpuProfile::reference(),
        }
    }
}

/// Where a client's PreSend horizon gets its sample.
///
/// The two are separated so the harness can measure the shipped behaviour
/// against the change, rather than only modelling whichever one is current.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PresendSource {
    /// C++ `CalcPerformance`: route ping and nothing else
    /// (src/C4GameControlNetwork.cpp:404-430). Blind to a client that is slow
    /// rather than distant, because a weak machine has a perfectly healthy ping.
    Ping,
    /// The larger of route ping and the lateness actually measured for the tick
    /// just consumed — arrival against the cadence, the quantity the host
    /// already records as `ClientPerformanceStats::wait_ms`.
    #[default]
    MeasuredLateness,
}

/// One session to simulate.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub clients: Vec<ClientProfile>,
    pub ticks: usize,
    pub seed: u64,
    pub control_rate: u32,
    pub target_fps: i32,
    /// C++ `CNM_Async`: bound the host's wait at
    /// `ControlRate * AsyncMaxWait * 1000 / TargetFPS`, then pack whoever
    /// arrived. Zero disables it, giving C++'s `CNM_Decentral` blocking wait.
    pub async_max_wait_frames: i32,
    /// Whether clients size PreSend at all.
    pub presend: bool,
    /// Which signal they size it from.
    pub presend_source: PresendSource,
}

impl SessionConfig {
    pub fn async_budget(&self) -> Duration {
        if self.async_max_wait_frames <= 0 {
            return Duration::MAX;
        }
        let ms =
            i64::from(self.control_rate as i32) * i64::from(self.async_max_wait_frames) * 1_000
                / i64::from(self.target_fps.max(1));
        Duration::from_millis(ms.max(0) as u64 + 1)
    }
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            clients: vec![ClientProfile::healthy(); 4],
            ticks: 400,
            seed: 0x5eed_1234,
            control_rate: 2,
            target_fps: 38,
            async_max_wait_frames: 2,
            presend: true,
            presend_source: PresendSource::default(),
        }
    }
}

/// What one participant experienced.
#[derive(Debug, Clone, Default)]
pub struct ClientOutcome {
    pub id: ClientId,
    /// Control ticks this client executed.
    pub executed: usize,
    /// Ticks whose aggregate had not arrived when the client was ready for it.
    pub blocked_ticks: usize,
    /// Total time blocked waiting for a shared tick.
    pub blocked_total: Duration,
    pub worst_block: Duration,
    /// This client's own input that the host discarded as late.
    pub dropped_inputs: usize,
    /// How far behind its ideal slot the client finished.
    pub drift: Duration,
    /// Mean PreSend horizon it chose.
    pub mean_horizon: Duration,
    /// True when its machine could not sustain the tick rate at all.
    pub overloaded: bool,
}

impl ClientOutcome {
    /// Fraction of scheduled ticks on which this client blocked. Asserted
    /// alongside `mean_horizon`, never alone: any implementation can drive
    /// blocking to zero by buying latency.
    pub fn blocked_fraction(&self, ticks: usize) -> f64 {
        self.blocked_ticks as f64 / ticks.max(1) as f64
    }

    pub fn frozen_time_fraction(&self, ticks: usize) -> f64 {
        let nominal = CONTROL_PERIOD * ticks.max(1) as u32;
        self.blocked_total.as_secs_f64() / nominal.as_secs_f64()
    }
}

/// What the whole session experienced.
#[derive(Debug, Clone)]
pub struct SessionReport {
    pub ticks: usize,
    pub seed: u64,
    pub clients: Vec<ClientOutcome>,
    /// Ticks the host published without a full set, because the async deadline
    /// expired first.
    pub forced_ticks: usize,
    /// Ticks the host never managed to publish at all.
    pub unpublished_ticks: usize,
}

impl SessionReport {
    /// The healthy participants — everyone whose machine keeps up. These are the
    /// players whose experience must not degrade when a potato joins.
    pub fn healthy(&self) -> impl Iterator<Item = &ClientOutcome> {
        self.clients.iter().filter(|client| !client.overloaded)
    }

    /// Worst blocked fraction among clients that are not themselves the problem.
    pub fn worst_healthy_blocked_fraction(&self) -> f64 {
        self.healthy()
            .map(|client| client.blocked_fraction(self.ticks))
            .fold(0.0_f64, f64::max)
    }

    pub fn worst_healthy_frozen_fraction(&self) -> f64 {
        self.healthy()
            .map(|client| client.frozen_time_fraction(self.ticks))
            .fold(0.0_f64, f64::max)
    }
}

/// A client's in-flight control, plus the aggregate coming back to it.
struct Peer {
    id: ClientId,
    profile: ClientProfile,
    link: Link,
    estimator: ControlLatencyEstimator,
    /// When this client finished executing the previous control tick.
    reached_at: Duration,
    /// Next control tick it will execute.
    next_tick: Tick,
    /// Control ticks it has already stamped and sent.
    sent_through: Option<Tick>,
    /// (arrival, tick, sent_at) heading to the host.
    uplink: Vec<(Duration, Tick, Duration)>,
    /// (arrival, tick) heading back to this client.
    downlink: Vec<(Duration, Tick)>,
    /// Control is a *reliable* channel in both engines: a lost datagram is
    /// repaired rather than abandoned. Without modelling that, a single drop
    /// strands a client for the rest of the session and every comparison
    /// measures the repair gap instead of the thing under test.
    uplink_repair: Vec<(Duration, Tick)>,
    downlink_repair: Vec<(Duration, Tick)>,
    /// Aggregates that have arrived.
    arrived: BTreeMap<Tick, Duration>,
    /// When this client stamped each tick, so it can measure the full
    /// stamp-to-aggregate round trip once the tick comes back. That round trip
    /// — not a one-way hop — is what PreSend has to cover, and it is what C++
    /// approximates with `iHostPing` in central mode.
    stamped_at: BTreeMap<Tick, Duration>,
    horizons: Vec<Duration>,
    outcome: ClientOutcome,
}

impl Peer {
    /// C++ converts the budget to whole frames and clamps to 1..15.
    fn presend_frames(&self, config: &SessionConfig) -> i32 {
        if !config.presend {
            return 0;
        }
        (config.target_fps.saturating_mul(self.estimator.budget_us()) / 1_000_000)
            .saturating_add(1)
            .clamp(1, 15)
    }

    fn presend_ticks(&self, config: &SessionConfig) -> u32 {
        (self.presend_frames(config) as u32).div_ceil(config.control_rate.max(1))
    }

    fn horizon(&self, config: &SessionConfig) -> Duration {
        Duration::from_micros(
            (self.presend_frames(config) as u64 * 1_000_000) / config.target_fps.max(1) as u64,
        )
    }
}

/// Runs one lockstep session and reports what each participant experienced.
pub fn run_session(config: &SessionConfig) -> SessionReport {
    let mut rng = SimRng::new(config.seed);
    let mut coordinator = ControlCoordinator::new(256);
    let mut peers: Vec<Peer> = config
        .clients
        .iter()
        .enumerate()
        .map(|(index, profile)| {
            let id = index as ClientId;
            coordinator.register_client(id).expect("client registers");
            Peer {
                id,
                profile: profile.clone(),
                link: Link::new(profile.conditions, config.seed ^ (index as u64 + 1)),
                estimator: ControlLatencyEstimator::new(),
                reached_at: Duration::ZERO,
                next_tick: 0,
                sent_through: None,
                uplink: Vec::new(),
                downlink: Vec::new(),
                uplink_repair: Vec::new(),
                downlink_repair: Vec::new(),
                arrived: BTreeMap::new(),
                stamped_at: BTreeMap::new(),
                horizons: Vec::new(),
                outcome: ClientOutcome {
                    id,
                    overloaded: profile.cpu.is_overloaded(config.control_rate),
                    ..ClientOutcome::default()
                },
            }
        })
        .collect();

    let mut now = Duration::ZERO;
    let mut published: BTreeMap<Tick, Duration> = BTreeMap::new();
    // Ticks aggregated but not yet due: the host is itself a participant and
    // reaches control tick T at its own cadence, never sooner.
    let mut pending_release: BTreeMap<Tick, Duration> = BTreeMap::new();
    let mut wait_start: Option<(Tick, Duration)> = None;
    let mut forced_ticks = 0usize;
    let budget = config.async_budget();
    // Give the session generous slack past its nominal length so a struggling
    // client's tail is measured rather than truncated.
    let deadline = CONTROL_PERIOD * (config.ticks as u32) * 8;

    while now <= deadline {
        // --- repair anything the wire lost ----------------------------------
        for peer in &mut peers {
            let due: Vec<(Duration, Tick)> = peer
                .uplink_repair
                .iter()
                .filter(|(at, _)| *at <= now)
                .copied()
                .collect();
            peer.uplink_repair.retain(|(at, _)| *at > now);
            for (_, tick) in due {
                let sent_at = peer.stamped_at.get(&tick).copied().unwrap_or(now);
                match peer.link.enqueue(now, true, vec![0u8; 8]) {
                    Some(arrival) => peer.uplink.push((arrival, tick, sent_at)),
                    None => peer
                        .uplink_repair
                        .push((now + RELIABLE_UDP_RECHECK_INTERVAL, tick)),
                }
            }

            let due: Vec<(Duration, Tick)> = peer
                .downlink_repair
                .iter()
                .filter(|(at, _)| *at <= now)
                .copied()
                .collect();
            peer.downlink_repair.retain(|(at, _)| *at > now);
            for (_, tick) in due {
                match peer.link.enqueue(now, false, vec![0u8; 16]) {
                    Some(arrival) => peer.downlink.push((arrival, tick)),
                    None => peer
                        .downlink_repair
                        .push((now + RELIABLE_UDP_RECHECK_INTERVAL, tick)),
                }
            }
        }

        // --- clients stamp and send their own input -------------------------
        for peer in &mut peers {
            let horizon = peer.horizon(config);
            let presend = peer.presend_ticks(config);
            // At the moment it executes tick T it stamps input for T + presend.
            // A client that has not started yet still seeds the first ticks, or
            // the session could never begin.
            let stamp_through = peer.next_tick.saturating_add(presend);
            let already = peer.sent_through.map_or(0, |tick| tick + 1);
            if stamp_through >= already && peer.reached_at <= now {
                for tick in already..=stamp_through {
                    if tick as usize >= config.ticks {
                        break;
                    }
                    peer.horizons.push(horizon);
                    // A lost stamp simply never reaches the host, which is what
                    // the async deadline exists to survive.
                    peer.stamped_at.insert(tick, now);
                    match peer.link.enqueue(now, true, vec![0u8; 8]) {
                        Some(arrival) => peer.uplink.push((arrival, tick, now)),
                        None => peer
                            .uplink_repair
                            .push((now + RELIABLE_UDP_RECHECK_INTERVAL, tick)),
                    }
                    peer.sent_through = Some(tick);
                }
            }
        }

        // --- host ingests -----------------------------------------------------
        let mut ready_now: Vec<Tick> = Vec::new();
        for peer in &mut peers {
            let arrived: Vec<(Tick, Duration)> = peer
                .uplink
                .iter()
                .filter(|(at, _, _)| *at <= now)
                .map(|(_, tick, sent)| (*tick, *sent))
                .collect();
            peer.uplink.retain(|(at, _, _)| *at > now);
            for (tick, _sent_at) in arrived {
                let packet = ControlPacket::builder(peer.id, tick).payload(vec![0u8; 8]);
                if let Ok(outcome) = coordinator.ingest(packet) {
                    ready_now.extend(outcome.ready.iter().map(|batch| batch.tick()));
                }
            }
        }

        // --- host publishes, forcing the tick if the async deadline expired ---
        let current = coordinator.current_tick();
        if (current as usize) < config.ticks {
            let needed_at = CONTROL_PERIOD * current;
            if now >= needed_at && wait_start.is_none_or(|(tick, _)| tick != current) {
                wait_start = Some((current, now.max(needed_at)));
            }
        }
        if let Some((tick, started)) = wait_start {
            if tick == coordinator.current_tick() && now >= started.saturating_add(budget) {
                let forced = coordinator.force_current_tick();
                for batch in &forced {
                    let present: Vec<ClientId> = batch
                        .packets()
                        .iter()
                        .map(|packet| packet.client_id())
                        .collect();
                    for peer in &mut peers {
                        if !present.contains(&peer.id) {
                            peer.outcome.dropped_inputs += 1;
                        }
                    }
                    forced_ticks += 1;
                    ready_now.push(batch.tick());
                }
            }
        }

        for tick in ready_now {
            let due = (CONTROL_PERIOD * tick).max(now);
            pending_release.insert(tick, due);
        }

        let releasable: Vec<Tick> = pending_release
            .iter()
            .filter(|(_, due)| **due <= now)
            .map(|(tick, _)| *tick)
            .collect();
        for tick in releasable {
            pending_release.remove(&tick);
            published.entry(tick).or_insert(now);
            wait_start = None;
            // Broadcast the aggregate back down every client's own link.
            for peer in &mut peers {
                match peer.link.enqueue(now, false, vec![0u8; 16]) {
                    Some(arrival) => peer.downlink.push((arrival, tick)),
                    None => peer
                        .downlink_repair
                        .push((now + RELIABLE_UDP_RECHECK_INTERVAL, tick)),
                }
            }
        }

        // --- clients receive and execute -------------------------------------
        for peer in &mut peers {
            let landed: Vec<Tick> = peer
                .downlink
                .iter()
                .filter(|(at, _)| *at <= now)
                .map(|(_, tick)| *tick)
                .collect();
            peer.downlink.retain(|(at, _)| *at > now);
            for tick in landed {
                peer.arrived.entry(tick).or_insert(now);
                // Size the horizon from how *late* the aggregate was against
                // this tick's own slot — never from the round trip, which
                // already contains the lookahead and would drive the horizon up
                // until it pinned the 1..15 clamp. This is the client-side
                // analogue of the host's `ClientPerformanceStats::wait_ms`
                // (`session/api.rs:228`), which is likewise arrival measured
                // against the cadence rather than against a send time.
                peer.stamped_at.remove(&tick);
                // Route ping is what C++ measures: a round trip on this link,
                // entirely independent of whether the machine kept up.
                let ping_ms = peer.profile.conditions.rtt_ms.min(i32::MAX as u64) as i32;
                let lateness_ms = now
                    .saturating_sub(CONTROL_PERIOD * tick)
                    .as_millis()
                    .min(i32::MAX as u128) as i32;
                let sample = match config.presend_source {
                    PresendSource::Ping => ping_ms,
                    PresendSource::MeasuredLateness => ping_ms.max(lateness_ms),
                };
                peer.estimator.observe(sample);
            }

            // Execute every control tick whose aggregate has arrived and whose
            // CPU cost has been paid.
            while (peer.next_tick as usize) < config.ticks {
                // Same stall definition as `sim::replay_lockstep`: a tick is due
                // at its slot shifted by the PreSend horizon, and no sooner than
                // one pace after the previous tick. "Pace" is the control period
                // or this machine's own cost per control tick, whichever is
                // slower — which is precisely what makes a potato fall behind.
                // Waiting for a tick that is not due yet is ordinary lockstep
                // idle, not a stall, and must not be counted as one.
                let horizon = peer.horizon(config);
                let pace =
                    CONTROL_PERIOD.max(peer.profile.cpu.control_tick_cost(config.control_rate));
                let scheduled = CONTROL_PERIOD * peer.next_tick + horizon;
                let earliest = scheduled.max(peer.reached_at);
                let Some(arrived_at) = peer.arrived.get(&peer.next_tick).copied() else {
                    break;
                };
                let executed_at = earliest.max(arrived_at);
                if now < executed_at {
                    break;
                }
                let stall = executed_at.saturating_sub(earliest);
                if !stall.is_zero() {
                    peer.outcome.blocked_ticks += 1;
                    peer.outcome.blocked_total += stall;
                    peer.outcome.worst_block = peer.outcome.worst_block.max(stall);
                }
                peer.reached_at = executed_at + pace + peer.profile.cpu.io_stall(&mut rng);
                peer.outcome.executed += 1;
                peer.next_tick += 1;
            }
        }

        now += STEP;
    }

    let ticks = config.ticks;
    let clients = peers
        .into_iter()
        .map(|mut peer| {
            if (peer.next_tick as usize) < ticks {
                let due = (CONTROL_PERIOD * peer.next_tick).max(peer.reached_at);
                let stranded = now.saturating_sub(due);
                if !stranded.is_zero() {
                    peer.outcome.blocked_ticks += 1;
                    peer.outcome.blocked_total += stranded;
                    peer.outcome.worst_block = peer.outcome.worst_block.max(stranded);
                }
            }
            peer.outcome.drift = peer
                .reached_at
                .saturating_sub(CONTROL_PERIOD * ticks as u32);
            peer.outcome.mean_horizon = crate::sim::mean(&peer.horizons);
            peer.outcome
        })
        .collect();

    SessionReport {
        ticks,
        seed: config.seed,
        clients,
        forced_ticks,
        unpublished_ticks: ticks.saturating_sub(published.len()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with(clients: Vec<ClientProfile>, ticks: usize) -> SessionConfig {
        SessionConfig {
            clients,
            ticks,
            ..SessionConfig::default()
        }
    }

    #[test]
    fn a_reference_machine_sustains_the_control_cadence() {
        // 3.90 ms/frame at ControlRate 2 is 7.8 ms against a 55 ms period.
        let cpu = CpuProfile::reference();
        assert!(!cpu.is_overloaded(2));
        assert_eq!(cpu.control_tick_cost(2), Duration::from_micros(7_800));
    }

    #[test]
    fn a_pi_cannot_sustain_a_heavy_scenario() {
        // The measurement that decides the whole policy: at 128 objects a Pi 4
        // needs 9 x 3.90 ms = 35 ms per *frame*, and 70 ms per control tick
        // against a 55 ms budget. No network tuning rescues that.
        assert!(CpuProfile::pi4().is_overloaded(2));
        assert!(CpuProfile::potato().is_overloaded(2));
        assert!(CpuProfile::pi1().is_overloaded(2));
        assert_eq!(
            CpuProfile::pi4().control_tick_cost(2),
            Duration::from_micros(70_200)
        );
    }

    #[test]
    fn a_light_scenario_is_within_reach_of_a_pi() {
        // MeltMe, 26 objects: 410 us/frame on the reference machine, so a Pi 4
        // needs 7.4 ms per control tick and fits comfortably. "A Pi can play
        // small scenarios and cannot play large ones" is a measurement, not a
        // slogan.
        let light = CpuProfile {
            reference_frame: Duration::from_micros(410),
            ..CpuProfile::pi4()
        };
        assert!(!light.is_overloaded(2));
    }

    #[test]
    fn an_all_healthy_session_runs_without_a_single_stall() {
        let report = run_session(&config_with(vec![ClientProfile::healthy(); 4], 200));

        assert_eq!(report.unpublished_ticks, 0, "every tick must publish");
        for client in &report.clients {
            assert!(!client.overloaded);
            assert_eq!(client.executed, report.ticks, "client {}", client.id);
            assert_eq!(
                client.blocked_ticks, 0,
                "client {} stalled {} times on a healthy link",
                client.id, client.blocked_ticks
            );
            // 0.5% loss occasionally makes one stamp miss the async deadline.
            // That is the mechanism working, not a regression, but it must stay
            // rare enough to be invisible in play.
            assert!(
                client.dropped_inputs * 20 < report.ticks,
                "client {} lost {} of {} inputs",
                client.id,
                client.dropped_inputs,
                report.ticks
            );
        }
    }

    #[test]
    fn the_horizon_converges_on_the_link_instead_of_pinning_the_clamp() {
        // Sizing the horizon from *lateness* is what makes it converge. Sizing
        // it from the stamp-to-aggregate round trip does not: that measurement
        // contains the lookahead itself, so the horizon feeds on its own output
        // and climbs until it hits C++'s 1..15 frame clamp (370 ms at 38 fps),
        // which is 6x more input lag than the link needs. This pins the shape
        // the Tier 1 PreSend change depends on.
        let report = run_session(&config_with(vec![ClientProfile::healthy(); 4], 200));

        let clamp = Duration::from_micros(15 * 1_000_000 / 38);
        for client in &report.clients {
            assert!(
                client.mean_horizon < clamp,
                "client {} pinned the PreSend clamp at {:?}",
                client.id,
                client.mean_horizon
            );
            assert!(
                client.mean_horizon >= Duration::from_millis(60),
                "client {} must still cover its 60 ms round trip, got {:?}",
                client.id,
                client.mean_horizon
            );
        }
    }

    #[test]
    fn the_same_seed_reproduces_the_session_exactly() {
        let config = config_with(
            vec![
                ClientProfile {
                    cpu: CpuProfile::potato(),
                    ..ClientProfile::healthy()
                },
                ClientProfile::healthy(),
                ClientProfile::healthy(),
            ],
            150,
        );

        let first = run_session(&config);
        let second = run_session(&config);

        for (left, right) in first.clients.iter().zip(second.clients.iter()) {
            assert_eq!(left.blocked_ticks, right.blocked_ticks);
            assert_eq!(left.blocked_total, right.blocked_total);
            assert_eq!(left.dropped_inputs, right.dropped_inputs);
            assert_eq!(left.executed, right.executed);
        }
        assert_eq!(first.forced_ticks, second.forced_ticks);
    }

    #[test]
    fn a_slow_machine_drags_the_healthy_players_down_with_it() {
        // The failure this whole rig exists to reproduce. The potato's ping is
        // fine; only its frame loop is slow, so PreSend never grows to cover it
        // and the host stops waiting.
        let mut clients = vec![ClientProfile::healthy(); 4];
        clients[0].cpu = CpuProfile::potato();
        let report = run_session(&config_with(clients, 200));

        let potato = &report.clients[0];
        assert!(potato.overloaded, "the potato cannot sustain the cadence");
        // It falls behind in *wall clock*, not in tick count: at 20x the
        // reference cost a control tick takes 156 ms against a 55 ms period, so
        // it still runs every tick, just nearly three times too slowly. Drift is
        // therefore the symptom to assert on; a tick counter would miss it.
        let nominal = CONTROL_PERIOD * report.ticks as u32;
        assert!(
            potato.drift > nominal,
            "it must end up more than a session-length behind, drifted {:?} over {:?}",
            potato.drift,
            nominal
        );
        // BASELINE DEFECT, pinned deliberately so the Tier 1 work has something
        // to invert. The healthy players are dragged along with the potato: the
        // host waits the full async budget (ControlRate * AsyncMaxWait * 1000 /
        // TargetFPS = 106 ms at defaults) on *every* tick before giving up on a
        // peer that is permanently late, so a straggler that never recovers
        // costs everyone ~106 ms per tick. Over a nominal 11 s session the
        // healthy clients finish about 10 s behind — the game runs at roughly
        // half speed for four players because of one.
        //
        // This is the residual PORT_STATUS.md:353-357 already warns about
        // ("a peer whose latency consistently exceeds the budget is dropped on
        // nearly every tick"), now measured end to end rather than argued.
        for healthy in report.healthy() {
            assert!(
                healthy.drift > nominal / 2,
                "expected the known defect: healthy client {} dragged along, \
                 drifted {:?} over a nominal {:?}",
                healthy.id,
                healthy.drift,
                nominal
            );
        }
        assert!(
            report.forced_ticks > 0,
            "the host must give up waiting for it at least once"
        );
        assert!(
            potato.dropped_inputs > 0,
            "and its input must be the input that gets discarded"
        );
    }

    #[test]
    fn async_mode_protects_the_healthy_players_from_the_slow_one() {
        // The policy decision, measured: with the async deadline the fast
        // players stop waiting on the straggler. Without it they block on
        // essentially every tick.
        let mut clients = vec![ClientProfile::healthy(); 4];
        clients[0].cpu = CpuProfile::potato();

        let protected = run_session(&SessionConfig {
            async_max_wait_frames: 2,
            ..config_with(clients.clone(), 200)
        });
        let blocking = run_session(&SessionConfig {
            async_max_wait_frames: 0,
            ..config_with(clients, 200)
        });

        assert!(
            protected.worst_healthy_frozen_fraction() < blocking.worst_healthy_frozen_fraction(),
            "async must reduce what the healthy players pay: {:.3} vs {:.3}",
            protected.worst_healthy_frozen_fraction(),
            blocking.worst_healthy_frozen_fraction()
        );
    }
}
