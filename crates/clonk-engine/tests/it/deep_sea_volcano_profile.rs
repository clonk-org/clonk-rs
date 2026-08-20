//! Manual probe: what do Deep Sea's volcanoes cost per frame?
//!
//! clonk-org/clonk-rs#497 reports "significant frame drops" from volcanoes in
//! `FarWorlds.c4f/Deep.c4s`. That scenario sets `Volcano=16,8`, so
//! `C4Weather::Execute`'s volcano gate fires continuously, and each launch
//! drops lava into a map that is mostly water.
//!
//! This measures the same scenario twice — once as shipped, once with the
//! volcano level forced to zero — and reports the per-frame tick cost of each
//! along with the live PXS and mass-mover counts that accumulate. The gap
//! between the two runs is the volcano's price; the counts say whether that
//! price is the launches themselves or the material they leave behind.
//!
//! It reports numbers and asserts nothing about them, so it is `#[ignore]`d
//! like the tree's other manual timing probes. Run it with:
//!
//! ```sh
//! cargo nextest run -p clonk-engine-integration-tests --test engine_it \
//!     --run-ignored all --no-capture -E 'test(deep_sea_volcano_profile::)'
//! ```
//!
//! # Recorded measurement
//!
//! Warm process and filesystem cache, seed 0, aarch64 host, 600 frames.
//!
//! | frame | shipped ms/frame | volcano-free ms/frame | shipped PXS |
//! |---|---|---|---|
//! | 100 | **5.085** | 2.026 | 3 |
//! | 200 | 3.240 | 2.034 | 1 |
//! | 300 | 3.104 | 2.036 | 35 |
//! | 400 | 2.763 | 2.295 | **171** |
//! | 500 | 2.284 | 2.389 | 3 |
//! | 600 | 2.497 | 2.048 | 0 |
//!
//! Volcanoes cost 614.6 ms over the 600 frames — 1.024 ms/frame, a 1.5x
//! multiple on the simulation.
//!
//! What that rules in and out. The simulation cost is real but modest, and it
//! is front-loaded: the first hundred frames run at 2.5x while the eruption is
//! carving, then settle. Live mass movers stay at **zero** throughout and the
//! object count never moves off ~1024, so neither is involved — the transient
//! is PXS, peaking at 171 and draining back to nothing.
//!
//! It does NOT reproduce "significant frame drops" on its own at this scale, so
//! either the reporter's machine multiplies that 1 ms into something visible,
//! or the cost is on the presentation side this probe cannot see: the volcano's
//! `Advance` runs `DrawLine` and `DrawMaterialQuad` on every step
//! (`Objects.c4d/Effects.c4d/Volcano.c4d/Script.c:39-43`), and each of those
//! dirties the landscape the frontend re-uploads. Measuring that needs a
//! rendered session, not this harness.

use crate::support::real_scenario::prepare_installed_scenario;
use clonk_engine::Engine;
use std::time::Instant;

const FRAMES: u32 = 600;
const SAMPLE_EVERY: u32 = 100;

struct Sample {
    frame: u32,
    elapsed_ms: f64,
    pxs: usize,
    movers: usize,
    objects: usize,
}

fn run(engine: &mut Engine, label: &str) -> Vec<Sample> {
    let mut samples = Vec::new();
    let start = Instant::now();
    for frame in 1..=FRAMES {
        engine.tick().expect("deep sea ticks");
        if frame % SAMPLE_EVERY == 0 {
            samples.push(Sample {
                frame,
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
                pxs: engine.pxs_system.iter().count(),
                movers: engine.mass_movers.live_movers(),
                objects: engine.objects.len(),
            });
        }
    }
    eprintln!("--- {label} ---");
    eprintln!("  frame |   total ms |  ms/frame |   pxs | movers | objects");
    let mut previous = (0_u32, 0.0_f64);
    for sample in &samples {
        let span = sample.elapsed_ms - previous.1;
        let frames = f64::from(sample.frame - previous.0);
        eprintln!(
            "  {:>5} | {:>10.1} | {:>9.3} | {:>5} | {:>6} | {:>7}",
            sample.frame,
            sample.elapsed_ms,
            span / frames,
            sample.pxs,
            sample.movers,
            sample.objects
        );
        previous = (sample.frame, sample.elapsed_ms);
    }
    samples
}

#[test]
#[ignore = "manual timing probe"]
fn deep_sea_volcano_cost_per_frame() {
    let prepared = prepare_installed_scenario("FarWorlds.c4f/Deep.c4s", 0);

    let mut shipped = prepared.instantiate();
    let with_volcanoes = run(&mut shipped, "shipped (Volcano=16,8)");

    let mut quiet = prepared.instantiate();
    let mut environment = quiet.environment();
    environment.volcano = 0;
    quiet.set_environment(environment);
    let without_volcanoes = run(&mut quiet, "volcano level forced to zero");

    let total_with = with_volcanoes.last().map(|s| s.elapsed_ms).unwrap_or(0.0);
    let total_without = without_volcanoes
        .last()
        .map(|s| s.elapsed_ms)
        .unwrap_or(0.0);
    eprintln!(
        "\nvolcanoes cost {:.1} ms over {FRAMES} frames ({:.3} ms/frame, {:.1}x)",
        total_with - total_without,
        (total_with - total_without) / f64::from(FRAMES),
        if total_without > 0.0 {
            total_with / total_without
        } else {
            0.0
        }
    );
}
