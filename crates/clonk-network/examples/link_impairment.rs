//! Lockstep control delivery over an impaired link.
//!
//! A thin CLI over [`clonk_network::run_control_delivery`]. The model itself
//! lives in `clonk_network::sim` so it is covered by the clippy gate and can be
//! driven from tests and from `cargo xtask chaos`.
//!
//! ```text
//! LC_RTT_MS=80 LC_JITTER_MS=20 LC_LOSS_PERMILLE=20 \
//!   cargo run --release -p clonk-network --example link_impairment
//! ```
//!
//! This measures the transport only. It does not run the simulation, so the
//! numbers are a floor on stall duration, not a whole-frame budget.

use std::env;
use std::time::Duration;

use clonk_network::{
    mean, percentile, run_control_delivery, ControlDeliveryConfig, ControlLatencyEstimator,
    LinkConditions, LinkReport, Lookahead, CONTROL_PERIOD,
};

fn env_u64(key: &str, default: u64) -> u64 {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn main() {
    // `LC_DOWN_BPS`/`LC_UP_BPS` default to 0, meaning unmetered, so a run that
    // sets neither reproduces the pre-bandwidth measurements exactly.
    let conditions = LinkConditions {
        rtt_ms: env_u64("LC_RTT_MS", 60),
        jitter_ms: env_u64("LC_JITTER_MS", 10),
        loss_permille: env_u64("LC_LOSS_PERMILLE", 10) as u32,
        burst_ms: env_u64("LC_BURST_MS", 0),
        downlink_bps: env_u64("LC_DOWN_BPS", 0),
        uplink_bps: env_u64("LC_UP_BPS", 0),
        queue_bytes: env_u64("LC_QUEUE_BYTES", 0),
        cross_traffic_down_bps: env_u64("LC_CROSS_DOWN_BPS", 0),
        cross_traffic_up_bps: env_u64("LC_CROSS_UP_BPS", 0),
    };

    // PreSend horizon. `LC_PRESEND=cpp` replays C4GameControlNetwork's
    // mean-only sizing, `adaptive` replays ControlLatencyEstimator, and the
    // default holds LC_LOOKAHEAD_MS constant.
    let target_fps = env_u64("LC_TARGET_FPS", 38) as i32;
    let lookahead = match env::var("LC_PRESEND")
        .unwrap_or_else(|_| "fixed".to_string())
        .as_str()
    {
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

    let config = ControlDeliveryConfig {
        conditions,
        ticks: env_u64("LC_TICKS", 400) as usize,
        seed: env_u64("LC_SEED", 0x5eed_1234),
        duplicates: env_u64("LC_DUP", 1).max(1),
        duplicate_delay_ms: env_u64("LC_DUP_DELAY_MS", 0),
        lookahead,
        catch_up: env_u64("LC_CATCHUP", 0) != 0,
    };

    print_report(&run_control_delivery(&config));
}

fn print_report(report: &LinkReport) {
    let sorted = report.sorted_latencies();
    let conditions = report.conditions;

    println!(
        "rtt              {}ms (one-way {:?})",
        conditions.rtt_ms,
        conditions.one_way()
    );
    println!("jitter           +0..{}ms", conditions.jitter_ms * 2);
    println!(
        "loss             {} permille{}",
        conditions.loss_permille,
        match conditions.burst_ms {
            0 => " (independent)".to_string(),
            burst => format!(" (bursts of ~{burst}ms)"),
        }
    );
    if conditions.downlink_bps != 0 || conditions.uplink_bps != 0 {
        println!(
            "capacity         {} down / {} up bps{}",
            conditions.downlink_bps,
            conditions.uplink_bps,
            match conditions.queue_bytes {
                0 => " (unbounded queue)".to_string(),
                bytes => format!(" ({bytes} B drop-tail queue)"),
            }
        );
    }
    if conditions.cross_traffic_down_bps != 0 || conditions.cross_traffic_up_bps != 0 {
        println!(
            "cross traffic    {} down / {} up bps ({} datagrams offered)",
            conditions.cross_traffic_down_bps, conditions.cross_traffic_up_bps, report.filler_sent
        );
    }
    println!("seed             {:#x}", report.seed);
    println!("control period   {CONTROL_PERIOD:?}");
    println!("controls sent    {}", report.ticks);
    println!("controls arrived {}", report.controls_arrived());
    println!("never arrived    {}", report.never_arrived);
    println!(
        "datagrams        {} sent, {} dropped ({} of them by a full queue)",
        report.datagrams_sent, report.datagrams_dropped, report.queue_drops
    );
    println!("mean             {:?}", report.mean_latency());
    println!("p50              {:?}", percentile(&sorted, 0.50));
    println!("p95              {:?}", percentile(&sorted, 0.95));
    println!("p99              {:?}", percentile(&sorted, 0.99));
    println!(
        "max              {:?}",
        sorted.last().copied().unwrap_or_default()
    );
    println!(
        "over one period  {} ({:.1}%)",
        report.over_one_period(),
        report.over_one_period() as f64 / report.controls_arrived().max(1) as f64 * 100.0
    );

    let stalled = report.playout.stalled();
    let stalled_total: Duration = stalled.iter().sum();
    let mut stalled_sorted = stalled.clone();
    stalled_sorted.sort_unstable();
    let wall_clock = CONTROL_PERIOD * report.ticks as u32;
    let pacing = if report.catch_up { "catch-up" } else { "slip" };

    println!();
    println!(
        "-- lockstep playout (presend {}, duplicates {}, {pacing}) --",
        report.presend_label, report.duplicates
    );
    println!(
        "frames stalled   {} of {} ({:.1}%)",
        stalled.len(),
        report.ticks,
        stalled.len() as f64 / report.ticks.max(1) as f64 * 100.0
    );
    println!("stall total      {stalled_total:?}");
    println!(
        "stall worst      {:?}",
        stalled_sorted.last().copied().unwrap_or_default()
    );
    println!("stall p99        {:?}", percentile(&stalled_sorted, 0.99));
    println!("schedule slip    {:?}", report.playout.drift);
    // The price of a bigger horizon is input latency, so report it next to the
    // stalls it buys off rather than letting the win stand on its own.
    println!("input lag mean   {:?}", mean(&report.playout.horizons));
    println!(
        "input lag max    {:?}",
        report
            .playout
            .horizons
            .iter()
            .max()
            .copied()
            .unwrap_or_default()
    );
    println!(
        "time lost        {:.2}% of a {wall_clock:?} session",
        report.frozen_time_fraction() * 100.0
    );
}
