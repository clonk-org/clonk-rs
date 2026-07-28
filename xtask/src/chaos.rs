//! `cargo xtask chaos` — the potato-on-a-bad-link regression harness.
//!
//! Report-only by design. `docs/PERFORMANCE.md` ("Baseline collection") is
//! explicit that timings from an arbitrary machine must not become blocking
//! thresholds until enough comparable samples exist, so `verify` prints deltas
//! against a recorded baseline and does not fail on them.
//!
//! Two kinds of check *do* fail, because neither is a timing measurement:
//!
//! * **Determinism.** The same seed must produce a byte-identical report. If it
//!   does not, every other number here is noise and the harness is broken.
//! * **Coverage.** Borrowed from Antithesis's `sometimes` assertions: if the
//!   fault injectors never fired, the suite passed vacuously. A chaos test whose
//!   injector silently stopped working looks exactly like a chaos test that
//!   found nothing.
//!
//! Percentiles are taken once over the merged sample set from every seed, never
//! averaged across seeds — you cannot average a percentile.
//!
//! **Known limitation.** Each participant gets its own independent simulated
//! link, so the host's uplink is not shared between peers. Adding participants
//! therefore does not add host-side contention, and `potato-dialup-8p` currently
//! reports the same numbers as `potato-dialup`. Real host uplink is one pipe
//! carrying N copies of every aggregate, which is exactly where a larger game
//! degrades first — see `chaos/README.md`.

use std::fmt::Write as _;
use std::fs;
use std::time::Duration;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clonk_network::{
    run_control_delivery, run_session, ClientProfile, ControlDeliveryConfig, CpuProfile,
    LinkConditions, Lookahead, PresendSource, SessionConfig, SessionReport,
};

/// Committed seed corpus. Fixed, never drawn at random: a gate that picks fresh
/// seeds is flaky by construction, and a seed that ever finds a bug belongs in
/// here permanently (the `.proptest-regressions` idiom).
const SEEDS: [u64; 16] = [
    0x5eed_1234,
    0x5eed_2345,
    0x5eed_3456,
    0x5eed_4567,
    0xdead_beef,
    0x0bad_cafe,
    0x1357_9bdf,
    0x2468_ace0,
    0xfeed_face,
    0xc0ff_ee00,
    0x1122_3344,
    0x5566_7788,
    0x99aa_bbcc,
    0xddee_ff00,
    0x0f0f_0f0f,
    0xa5a5_a5a5,
];

const DEFAULT_TICKS: usize = 200;
const BASELINE_PATH: &str = "chaos/baseline.json";

/// A named `(link, cpu)` pairing. The two are deliberately separable: a single
/// blended "lag" number cannot tell "buy a CPU" from "buy an ISP", and the fixes
/// differ, so the suite carries profiles that isolate each.
struct Profile {
    name: &'static str,
    description: &'static str,
    /// Impaired participant's link. `None` means it gets the healthy link.
    link: Option<LinkConditions>,
    /// Impaired participant's machine. `None` means it gets a reference machine.
    cpu: Option<CpuProfile>,
    clients: usize,
}

fn dialup() -> LinkConditions {
    LinkConditions {
        rtt_ms: 200,
        jitter_ms: 30,
        loss_permille: 14,
        burst_ms: 0,
        downlink_bps: 53_300,
        uplink_bps: 33_600,
        queue_bytes: 4_200,
        cross_traffic_down_bps: 0,
        cross_traffic_up_bps: 0,
    }
}

fn hotel_wifi() -> LinkConditions {
    LinkConditions {
        rtt_ms: 35,
        jitter_ms: 25,
        loss_permille: 130,
        burst_ms: 80,
        downlink_bps: 4_000_000,
        uplink_bps: 1_000_000,
        queue_bytes: 125_000,
        cross_traffic_down_bps: 0,
        cross_traffic_up_bps: 1_500_000,
    }
}

fn profiles() -> Vec<Profile> {
    vec![
        Profile {
            name: "healthy",
            description: "four good machines on ordinary broadband; the control arm",
            link: None,
            cpu: None,
            clients: 4,
        },
        Profile {
            name: "slow-cpu-only",
            description: "one Pi-class machine on a good link; isolates the CPU knob",
            link: None,
            cpu: Some(CpuProfile::potato()),
            clients: 4,
        },
        Profile {
            name: "bad-link-only",
            description: "one dial-up link on a good machine; isolates the network knob",
            link: Some(dialup()),
            cpu: None,
            clients: 4,
        },
        Profile {
            name: "potato-dialup",
            description: "the worst case: Pi-class machine on 33.6k dial-up",
            link: Some(dialup()),
            cpu: Some(CpuProfile::potato()),
            clients: 4,
        },
        Profile {
            name: "pi4-hotel-wifi",
            description: "a Pi 4 on congested hotel wifi with a saturated uplink",
            link: Some(hotel_wifi()),
            cpu: Some(CpuProfile::pi4()),
            clients: 4,
        },
        Profile {
            name: "potato-dialup-8p",
            description:
                "same, with 8 participants; see the host-uplink limitation in chaos/README.md",
            link: Some(dialup()),
            cpu: Some(CpuProfile::potato()),
            clients: 8,
        },
    ]
}

impl Profile {
    fn session(&self, seed: u64, ticks: usize) -> SessionConfig {
        self.session_with(seed, ticks, PresendSource::default())
    }

    fn session_with(
        &self,
        seed: u64,
        ticks: usize,
        presend_source: PresendSource,
    ) -> SessionConfig {
        let mut clients = vec![ClientProfile::healthy(); self.clients];
        if let Some(link) = self.link {
            clients[0].conditions = link;
        }
        if let Some(cpu) = self.cpu {
            clients[0].cpu = cpu;
        }
        SessionConfig {
            clients,
            ticks,
            seed,
            presend_source,
            ..SessionConfig::default()
        }
    }
}

/// Everything one profile's seed sweep produced. Every field is an exact
/// integer under the virtual clock, so the whole struct is bit-reproducible.
struct ProfileMetrics {
    name: String,
    description: String,
    seeds: usize,
    ticks: usize,
    /// Worst blocked-tick fraction among the *healthy* participants, per mille.
    /// This is the number that answers the actual question: what does the bad
    /// peer cost everybody else?
    healthy_blocked_permille_median: u64,
    healthy_blocked_permille_worst: u64,
    /// How far behind the nominal schedule the healthy participants ended.
    healthy_drift_ms_median: u64,
    healthy_drift_ms_worst: u64,
    /// The impaired participant's own drift, for contrast.
    impaired_drift_ms_median: u64,
    /// Input dropped because the host stopped waiting, summed over the sweep.
    dropped_inputs_total: u64,
    /// Ticks the host had to force past a missing participant.
    forced_ticks_total: u64,
    /// Mean PreSend horizon the healthy participants chose. Reported *beside*
    /// the blocking numbers on purpose: any change can drive blocking to zero by
    /// spending input latency, so a one-sided figure rewards the wrong thing.
    healthy_horizon_us_median: u64,
    unpublished_total: u64,
}

fn percentile_u64(sorted: &[u64], fraction: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn median(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    percentile_u64(values, 0.5)
}

fn worst(values: &mut [u64]) -> u64 {
    values.sort_unstable();
    values.last().copied().unwrap_or(0)
}

/// Coverage counters, checked once over the whole sweep.
#[derive(Default)]
struct Coverage {
    stalls: u64,
    forced: u64,
    dropped_inputs: u64,
    datagram_drops: u64,
    queue_drops: u64,
}

impl Coverage {
    fn missing(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        if self.stalls == 0 {
            missing.push("no participant ever stalled");
        }
        if self.forced == 0 {
            missing.push("the async deadline never fired");
        }
        if self.dropped_inputs == 0 {
            missing.push("no input was ever dropped");
        }
        if self.datagram_drops == 0 {
            missing.push("the loss injector never fired");
        }
        if self.queue_drops == 0 {
            missing.push("no queue ever overflowed");
        }
        missing
    }
}

fn measure(
    profile: &Profile,
    ticks: usize,
    presend_source: PresendSource,
    coverage: &mut Coverage,
) -> ProfileMetrics {
    let mut healthy_blocked = Vec::new();
    let mut healthy_drift = Vec::new();
    let mut impaired_drift = Vec::new();
    let mut healthy_horizon = Vec::new();
    let mut dropped_inputs_total = 0u64;
    let mut forced_ticks_total = 0u64;
    let mut unpublished_total = 0u64;

    for seed in SEEDS {
        let report: SessionReport = run_session(&profile.session_with(seed, ticks, presend_source));

        for client in report.healthy() {
            healthy_blocked.push((client.blocked_fraction(report.ticks) * 1_000.0).round() as u64);
            healthy_drift.push(client.drift.as_millis() as u64);
            healthy_horizon.push(client.mean_horizon.as_micros() as u64);
        }
        // Client 0 is the impaired one by construction; in `healthy` it is just
        // another good participant, which keeps the column meaningful there too.
        impaired_drift.push(report.clients[0].drift.as_millis() as u64);

        let dropped: usize = report.clients.iter().map(|c| c.dropped_inputs).sum();
        let stalls: usize = report.clients.iter().map(|c| c.blocked_ticks).sum();
        dropped_inputs_total += dropped as u64;
        forced_ticks_total += report.forced_ticks as u64;
        unpublished_total += report.unpublished_ticks as u64;
        coverage.stalls += stalls as u64;
        coverage.forced += report.forced_ticks as u64;
        coverage.dropped_inputs += dropped as u64;
    }

    // The transport view of the same link, which is where datagram-level loss
    // and queue overflow are visible at all.
    if let Some(link) = profile.link {
        for seed in SEEDS.iter().take(4) {
            let report = run_control_delivery(&ControlDeliveryConfig {
                conditions: link,
                ticks,
                seed: *seed,
                duplicates: 3,
                duplicate_delay_ms: 0,
                bulk_packet_bytes: 0,
                bulk_interval: Duration::from_millis(500),
                lookahead: Lookahead::Adaptive {
                    estimator: clonk_network::ControlLatencyEstimator::new(),
                    target_fps: 38,
                },
                catch_up: true,
            });
            coverage.datagram_drops += report.datagrams_dropped as u64;
            coverage.queue_drops += report.queue_drops as u64;
        }
    }

    ProfileMetrics {
        name: profile.name.to_string(),
        description: profile.description.to_string(),
        seeds: SEEDS.len(),
        ticks,
        healthy_blocked_permille_median: median(&mut healthy_blocked.clone()),
        healthy_blocked_permille_worst: worst(&mut healthy_blocked),
        healthy_drift_ms_median: median(&mut healthy_drift.clone()),
        healthy_drift_ms_worst: worst(&mut healthy_drift),
        impaired_drift_ms_median: median(&mut impaired_drift),
        dropped_inputs_total,
        forced_ticks_total,
        healthy_horizon_us_median: median(&mut healthy_horizon),
        unpublished_total,
    }
}

/// Same seed twice must give the same answer, or nothing else here means
/// anything. `madsim` ships this as `MADSIM_TEST_CHECK_DETERMINISM`.
fn check_determinism(ticks: usize) -> Result<()> {
    let profile = &profiles()[3];
    let config = profile.session(SEEDS[0], ticks.min(120));
    let first = run_session(&config);
    let second = run_session(&config);
    for (left, right) in first.clients.iter().zip(second.clients.iter()) {
        if left.blocked_ticks != right.blocked_ticks
            || left.blocked_total != right.blocked_total
            || left.dropped_inputs != right.dropped_inputs
            || left.executed != right.executed
            || left.drift != right.drift
        {
            bail!(
                "chaos harness is not deterministic: client {} differed between two runs of \
                 the same seed. Every number this tool reports is meaningless until that is \
                 fixed; do not retry, reproduce it.",
                left.id
            );
        }
    }
    if first.forced_ticks != second.forced_ticks {
        bail!("chaos harness is not deterministic: forced-tick count differed");
    }
    Ok(())
}

fn render_table(metrics: &[ProfileMetrics]) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<20} {:>10} {:>10} {:>11} {:>11} {:>9} {:>8} {:>10}",
        "profile", "blocked", "blocked", "healthy", "impaired", "dropped", "forced", "horizon"
    );
    let _ = writeln!(
        out,
        "{:<20} {:>10} {:>10} {:>11} {:>11} {:>9} {:>8} {:>10}",
        "", "p50 ‰", "max ‰", "drift p50", "drift p50", "inputs", "ticks", "p50"
    );
    for metric in metrics {
        let _ = writeln!(
            out,
            "{:<20} {:>10} {:>10} {:>10}ms {:>10}ms {:>9} {:>8} {:>8}us",
            metric.name,
            metric.healthy_blocked_permille_median,
            metric.healthy_blocked_permille_worst,
            metric.healthy_drift_ms_median,
            metric.impaired_drift_ms_median,
            metric.dropped_inputs_total,
            metric.forced_ticks_total,
            metric.healthy_horizon_us_median,
        );
    }
    out
}

fn to_json(metrics: &[ProfileMetrics]) -> String {
    let mut out = String::from("{\n  \"schema_version\": 1,\n  \"profiles\": [\n");
    for (index, metric) in metrics.iter().enumerate() {
        let _ = write!(
            out,
            concat!(
                "    {{\n",
                "      \"name\": \"{}\",\n",
                "      \"description\": \"{}\",\n",
                "      \"seeds\": {},\n",
                "      \"ticks\": {},\n",
                "      \"healthy_blocked_permille_median\": {},\n",
                "      \"healthy_blocked_permille_worst\": {},\n",
                "      \"healthy_drift_ms_median\": {},\n",
                "      \"healthy_drift_ms_worst\": {},\n",
                "      \"impaired_drift_ms_median\": {},\n",
                "      \"dropped_inputs_total\": {},\n",
                "      \"forced_ticks_total\": {},\n",
                "      \"healthy_horizon_us_median\": {},\n",
                "      \"unpublished_total\": {}\n",
                "    }}"
            ),
            metric.name,
            metric.description,
            metric.seeds,
            metric.ticks,
            metric.healthy_blocked_permille_median,
            metric.healthy_blocked_permille_worst,
            metric.healthy_drift_ms_median,
            metric.healthy_drift_ms_worst,
            metric.impaired_drift_ms_median,
            metric.dropped_inputs_total,
            metric.forced_ticks_total,
            metric.healthy_horizon_us_median,
            metric.unpublished_total,
        );
        out.push_str(if index + 1 == metrics.len() {
            "\n"
        } else {
            ",\n"
        });
    }
    out.push_str("  ]\n}\n");
    out
}

/// Pulls one integer field out of the recorded baseline without a JSON parser.
/// The file is written by `to_json` above and is not user-authored, so a
/// line-oriented read is honest rather than fragile.
fn baseline_lookup(baseline: &str, name: &str, field: &str) -> Option<u64> {
    let profile_start = baseline.find(&format!("\"name\": \"{name}\""))?;
    let rest = &baseline[profile_start..];
    let end = rest.find("\n    }").unwrap_or(rest.len());
    let block = &rest[..end];
    let key = format!("\"{field}\": ");
    let value_start = block.find(&key)? + key.len();
    let value = block[value_start..]
        .split(|c: char| !c.is_ascii_digit())
        .next()?;
    value.parse().ok()
}

fn repo_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .context("locating the repository root from the xtask manifest directory")
}

pub(crate) fn command(args: &[String]) -> Result<()> {
    let mut ticks = DEFAULT_TICKS;
    let mut presend_source = PresendSource::default();
    let mut mode = "run".to_string();
    let mut json_path: Option<PathBuf> = None;
    let mut only: Option<String> = None;

    let mut rest = args.iter();
    if let Some(first) = rest.next() {
        if !first.starts_with("--") {
            mode = first.clone();
        }
    }
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--ticks" => {
                ticks = iter
                    .next()
                    .and_then(|value| value.parse().ok())
                    .context("--ticks needs a number")?;
            }
            "--json" => {
                json_path = Some(PathBuf::from(
                    iter.next().context("--json needs a path")?.as_str(),
                ));
            }
            "--profile" => {
                only = Some(iter.next().context("--profile needs a name")?.clone());
            }
            "--presend" => {
                presend_source = match iter.next().map(String::as_str) {
                    Some("ping") => PresendSource::Ping,
                    Some("measured") => PresendSource::MeasuredLateness,
                    other => bail!(
                        "--presend takes `ping` (C++ CalcPerformance) or `measured`, got {other:?}"
                    ),
                };
            }
            _ => {}
        }
    }

    if !matches!(mode.as_str(), "run" | "record" | "verify") {
        bail!("unknown chaos command `{mode}`; expected run, record or verify");
    }

    check_determinism(ticks)?;

    let selected: Vec<Profile> = profiles()
        .into_iter()
        .filter(|profile| only.as_ref().is_none_or(|name| profile.name == *name))
        .collect();
    if selected.is_empty() {
        bail!("no chaos profile matched `{}`", only.unwrap_or_default());
    }

    let mut coverage = Coverage::default();
    let metrics: Vec<ProfileMetrics> = selected
        .iter()
        .map(|profile| measure(profile, ticks, presend_source, &mut coverage))
        .collect();

    let presend_label = match presend_source {
        PresendSource::Ping => "ping (C++ CalcPerformance)",
        PresendSource::MeasuredLateness => "measured lateness",
    };
    println!(
        "chaos: {} profiles x {} seeds x {ticks} ticks, presend from {presend_label} \
         (virtual clock; every number is exact)\n",
        metrics.len(),
        SEEDS.len()
    );
    print!("{}", render_table(&metrics));
    for metric in &metrics {
        println!("  {:<20} {}", metric.name, metric.description);
    }

    // Coverage is only meaningful over the whole suite, so it is skipped when a
    // single profile was selected.
    if only.is_none() {
        let missing = coverage.missing();
        if !missing.is_empty() {
            bail!(
                "chaos coverage failed — the suite passed vacuously because {}. \
                 A fault injector that stopped firing looks exactly like a clean run.",
                missing.join(", ")
            );
        }
        println!(
            "\ncoverage ok: {} stalls, {} forced ticks, {} dropped inputs, {} datagram drops \
             ({} from full queues)",
            coverage.stalls,
            coverage.forced,
            coverage.dropped_inputs,
            coverage.datagram_drops,
            coverage.queue_drops
        );
    }

    let root = repo_root()?;
    let baseline_file = root.join(BASELINE_PATH);
    let json = to_json(&metrics);

    if let Some(path) = json_path {
        fs::write(&path, &json).with_context(|| format!("writing {}", path.display()))?;
        println!("\nwrote {}", path.display());
    }

    match mode.as_str() {
        "record" => {
            if only.is_some() {
                bail!("`chaos record` rewrites the whole baseline; drop --profile");
            }
            if presend_source != PresendSource::default() {
                bail!("`chaos record` must use the shipped PreSend source; drop --presend");
            }
            if let Some(parent) = baseline_file.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            fs::write(&baseline_file, &json)
                .with_context(|| format!("writing {}", baseline_file.display()))?;
            println!("\nrecorded baseline at {BASELINE_PATH}");
        }
        "verify" => {
            let baseline = fs::read_to_string(&baseline_file).with_context(|| {
                format!("reading {BASELINE_PATH}; run `cargo xtask chaos record` first")
            })?;
            println!("\n-- change against {BASELINE_PATH} --");
            let mut compared = 0usize;
            for metric in &metrics {
                for (field, current) in [
                    (
                        "healthy_blocked_permille_median",
                        metric.healthy_blocked_permille_median,
                    ),
                    ("healthy_drift_ms_median", metric.healthy_drift_ms_median),
                    ("dropped_inputs_total", metric.dropped_inputs_total),
                    (
                        "healthy_horizon_us_median",
                        metric.healthy_horizon_us_median,
                    ),
                ] {
                    let Some(recorded) = baseline_lookup(&baseline, &metric.name, field) else {
                        continue;
                    };
                    compared += 1;
                    if recorded == current {
                        continue;
                    }
                    let direction = if current < recorded {
                        "better"
                    } else {
                        "WORSE"
                    };
                    println!(
                        "  {:<20} {:<34} {} -> {} ({direction})",
                        metric.name, field, recorded, current
                    );
                }
            }
            if compared == 0 {
                bail!("baseline contained none of the expected fields; re-record it");
            }
            println!(
                "\nReport-only: deltas above are not a pass/fail signal. Promote a threshold \
                 only after docs/PERFORMANCE.md's baseline-collection rule is satisfied."
            );
        }
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seed_corpus_has_no_duplicates() {
        // A duplicated seed silently narrows the sweep while still looking like
        // 16 samples.
        let mut seen = SEEDS.to_vec();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), SEEDS.len());
    }

    #[test]
    fn every_profile_has_a_distinct_name() {
        let mut names: Vec<&str> = profiles().iter().map(|profile| profile.name).collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }

    #[test]
    fn the_impaired_client_actually_differs_from_the_healthy_ones() {
        // Guards the profile table itself: a profile whose impaired client was
        // accidentally left at healthy settings would report a clean result and
        // look like good news.
        for profile in profiles() {
            let config = profile.session(SEEDS[0], 8);
            let impaired = &config.clients[0];
            let healthy = &config.clients[1];
            let differs = impaired.conditions != healthy.conditions || impaired.cpu != healthy.cpu;
            assert_eq!(
                differs,
                profile.link.is_some() || profile.cpu.is_some(),
                "profile `{}` does not impair the client it claims to",
                profile.name
            );
        }
    }

    #[test]
    fn baseline_lookup_reads_back_what_to_json_wrote() {
        let metrics = vec![ProfileMetrics {
            name: "sample".to_string(),
            description: "d".to_string(),
            seeds: 2,
            ticks: 3,
            healthy_blocked_permille_median: 11,
            healthy_blocked_permille_worst: 22,
            healthy_drift_ms_median: 33,
            healthy_drift_ms_worst: 44,
            impaired_drift_ms_median: 55,
            dropped_inputs_total: 66,
            forced_ticks_total: 77,
            healthy_horizon_us_median: 88,
            unpublished_total: 99,
        }];
        let json = to_json(&metrics);

        assert_eq!(
            baseline_lookup(&json, "sample", "healthy_blocked_permille_median"),
            Some(11)
        );
        assert_eq!(
            baseline_lookup(&json, "sample", "healthy_horizon_us_median"),
            Some(88)
        );
        assert_eq!(
            baseline_lookup(&json, "sample", "unpublished_total"),
            Some(99)
        );
        assert_eq!(baseline_lookup(&json, "missing", "unpublished_total"), None);
    }
}
