//! Manual timing probe for the Fantasy pack's Temporary Tunnel spell
//! (`Fantasy.c4d/Magic.c4d/Tunnel.c4d`, `MTNL`), reported as dropping the game
//! to about 13 FPS while it digs (clonk-org/clonk-rs#1674).
//!
//! The spell is a one-frame-interval global effect. Each frame it walks one row
//! of the tunnel, and for every pixel in it calls `GetMaterial`, `GetTexture`
//! and a one-pixel `FreeRect` while digging, or a one-pixel `DrawMaterialQuad`
//! while closing, and appends to a 7200-slot array held in an effect variable.
//!
//! Run:
//!
//! ```sh
//! cargo nextest run --release -p clonk-engine-integration-tests --test engine_it \
//!   --run-ignored all --no-capture -E 'test(temporary_tunnel_profile::)'
//! ```

use std::time::{Duration, Instant};

use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use crate::support::EngineTestExt;
use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;

/// Starts the spell's effect the way `MTNL::StartTunnel` does, and reads the
/// ground at landscape coordinates.
const CASTER_PROBE: &str = r#"#strict
public func Dig(object caster, int angle)
{
    return AddEffect("TunnelUSpell", 0, 260, 1, 0, MTNL, caster, angle);
}
public func Ground(int x, int y)
{
    return Format("%d/%s", GetMaterial(x - GetX(), y - GetY()), GetTexture(x - GetX(), y - GetY()));
}
"#;

/// The spell is the heaviest user of an array held in an effect variable that
/// the shipped content has: it records every pixel it removes, one element at
/// a time, and 370 frames later reads them back in the same order to put the
/// ground back. Element reads and writes are served in place
/// (clonk-org/clonk-rs#1674), so this pins that what comes back is exactly
/// what was there.
#[test]
fn a_temporary_tunnel_puts_back_the_ground_it_dug() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Crystalvalley.c4s", 0);
    let owner = join_local_player(&mut engine, "Tunnel digger");
    let caster = engine.crew_cursor(owner).expect("the player has a crew");
    for _ in 0..200 {
        engine.tick_without_snapshot().expect("settling tick");
    }
    let feet = engine.test_object_snapshot(caster).position;
    engine
        .register_script_definition("TNLP", "Tunnel probe", CASTER_PROBE)
        .expect("the probe registers");
    // Held by the caster, so it neither falls nor leaves the landscape.
    let probe = engine
        .spawn_object(SpawnConfig::new("TNLP").with_container(caster))
        .expect("the probe spawns");
    // A column through the tunnel's middle and one towards each edge of it.
    let samples = [-12, 0, 12]
        .into_iter()
        .flat_map(|dx| {
            (30..110)
                .step_by(10)
                .map(move |dy| (feet.x + dx, feet.y + dy))
        })
        .collect::<Vec<_>>();
    let ground = |engine: &mut Engine| {
        let index = engine.test_object_index(probe);
        samples
            .iter()
            .map(|(x, y)| {
                engine
                    .call_object_function(index, "Ground", vec![Value::Int(*x), Value::Int(*y)])
                    .expect("the ground is readable")
            })
            .collect::<Vec<_>>()
    };

    let before = ground(&mut engine);
    let index = engine.test_object_index(probe);
    engine
        .call_object_function(
            index,
            "Dig",
            vec![Value::Object(caster.as_u64()), Value::Int(180)],
        )
        .expect("the effect starts");
    for _ in 0..121 {
        engine.tick_without_snapshot().expect("digging tick");
    }
    let open = ground(&mut engine);
    assert_ne!(open, before, "the tunnel is dug");
    for _ in 0..(250 + 125) {
        engine
            .tick_without_snapshot()
            .expect("waiting and closing tick");
    }
    assert_eq!(ground(&mut engine), before, "and closed again as it was");
}

fn mean_tick(engine: &mut Engine, frames: usize) -> Duration {
    let started = Instant::now();
    for _ in 0..frames {
        engine.tick_without_snapshot().expect("measured tick");
    }
    started.elapsed() / frames as u32
}

/// Twelve tunnels at once, side by side, so the digging phase lasts long enough
/// to attach a sampling profiler to the test binary.
#[test]
#[ignore = "manual profiling target"]
fn temporary_tunnel_digging_under_a_sampler() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Crystalvalley.c4s", 0);
    let owner = join_local_player(&mut engine, "Tunnel sampling");
    let caster = engine.crew_cursor(owner).expect("the player has a crew");
    for _ in 0..200 {
        engine.tick_without_snapshot().expect("settling tick");
    }
    let origin = engine.test_object_snapshot(caster).position;
    engine
        .register_script_definition("TNLP", "Tunnel probe", CASTER_PROBE)
        .expect("the probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("TNLP"))
        .expect("the probe spawns");
    let index = engine.test_object_index(probe);
    for column in 0..12 {
        let marker = engine.spawn_test_object(
            SpawnConfig::new("ROCK")
                .with_position(clonk_engine::Vector2::new(origin.x + 40 * column, origin.y)),
        );
        engine
            .call_object_function(
                index,
                "Dig",
                vec![Value::Object(marker.as_u64()), Value::Int(180)],
            )
            .expect("the effect starts");
    }
    let digging = mean_tick(&mut engine, 120);
    eprintln!("TUNNEL x12 digging={digging:?}");
}

#[test]
#[ignore = "manual timing probe"]
fn temporary_tunnel_tick_cost_by_phase() {
    let mut engine = load_installed_scenario("Fantasy.c4f/Crystalvalley.c4s", 0);
    let owner = join_local_player(&mut engine, "Tunnel timing");
    let caster = engine.crew_cursor(owner).expect("the player has a crew");
    for _ in 0..200 {
        engine.tick_without_snapshot().expect("settling tick");
    }
    let idle = mean_tick(&mut engine, 120);

    engine
        .register_script_definition("TNLP", "Tunnel probe", CASTER_PROBE)
        .expect("the probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("TNLP"))
        .expect("the probe spawns");
    let index = engine.test_object_index(probe);
    // Straight down, into the ground the Clonk stands on.
    engine
        .call_object_function(
            index,
            "Dig",
            vec![Value::Object(caster.as_u64()), Value::Int(180)],
        )
        .expect("the effect starts");

    // MTNL_Length = 120 frames of digging, MTNL_Duration = 250 of waiting,
    // then 120 of closing.
    let digging = mean_tick(&mut engine, 120);
    let waiting = mean_tick(&mut engine, 250);
    let closing = mean_tick(&mut engine, 120);
    let after = mean_tick(&mut engine, 120);
    eprintln!(
        "TUNNEL idle={idle:?} digging={digging:?} waiting={waiting:?} closing={closing:?} after={after:?}"
    );
}
