use crate::support::real_scenario::{join_local_player, load_installed_scenario};
use clonk_engine::ParticleLayer;
use clonk_script::Value;

#[test]
fn eke_flamethrower_launches_global_fire2_particles() {
    // The shipped FI5B carrier is intentionally transparent. C++ converts
    // these caller-local coordinates to world coordinates and puts an omitted
    // target's particles in GlobalParticles (oracle-src-pinned
    // src/C4Script.cpp:4863-4879; src/C4Particles.cpp:378-418), which the viewport draws after normal
    // objects (src/C4Viewport.cpp:1071-1079).
    let mut engine =
        load_installed_scenario("EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MeltMe.c4s", 0);
    let owner = join_local_player(&mut engine, "Eke flamethrower particles");
    let clonk = engine
        .crew_cursor(owner)
        .expect("MeltMe joins with a selected SFT");
    let flamethrower = engine
        .object_snapshot(clonk)
        .expect("the SFT is live")
        .contents
        .iter()
        .copied()
        .find(|&object| {
            engine
                .object_snapshot(object)
                .is_some_and(|snapshot| snapshot.definition_id == "FT5B")
        })
        .expect("MeltMe equips the SFT with FT5B");

    let flamethrower_index = engine
        .find_object_index(flamethrower)
        .expect("the FT5B has an index");
    assert_eq!(
        engine
            .call_object_function(
                flamethrower_index,
                "CreateFire",
                vec![Value::Object(clonk.as_u64()), Value::Int(1)],
            )
            .expect("the shipped CreateFire callback completes"),
        Value::Int(1)
    );

    let snapshot = engine.snapshot();
    let flames: Vec<_> = snapshot
        .particles
        .iter()
        .filter(|particle| particle.definition_id == "Fire2")
        .collect();
    assert_eq!(
        flames.len(),
        8,
        "FI5B::Flying creates eight shipped Fire2 visuals per callback"
    );
    assert!(
        flames
            .iter()
            .all(|particle| matches!(particle.layer, ParticleLayer::Global)),
        "C++ leaves FI5B's target argument null, so Fire2 uses the global particle list"
    );
    assert!(
        engine
            .particle_render_catalog()
            .iter()
            .find(|definition| definition.core.name == "Fire2")
            .and_then(|definition| definition.graphics.as_ref())
            .is_some(),
        "the shipped Fire2 render definition retains its Graphics.png"
    );

    engine
        .tick_without_snapshot()
        .expect("the launched FI5B and global particles execute");
    let advanced = engine.snapshot();
    let advanced_flames: Vec<_> = advanced
        .particles
        .iter()
        .filter(|particle| particle.definition_id == "Fire2")
        .collect();
    assert!(
        !advanced_flames.is_empty(),
        "FI5B continuously replenishes the short-lived Fire2 particles"
    );
}
