use crate::support::real_scenario::{join_local_player, load_tutorial};
use clonk_engine::{Engine, ParticleLayer};

fn catalog_has(engine: &Engine, name: &str) -> bool {
    engine
        .particle_render_catalog()
        .iter()
        .any(|definition| definition.core.name == name)
}

/// The engine's own fire, on shipped content: `FnFxFireTimer`'s emitter
/// (oracle-src-pinned src/C4Effect.cpp:660-769) spawns a double set of
/// particles per execution — the first quarter the normal `Fire` def and the
/// remaining three quarters the additive `Fire2` — dealt to the burning
/// object's own back/front lists.
///
/// The stock defs are named exactly `Fire` and `Fire2`
/// (content/Objects.c4d/Effects.c4d/Particles.c4d/Fire{,2}.c4d/Particle.txt),
/// which is what `SetDefParticles` looks up (src/C4Particles.cpp:485-486), so
/// this also pins that the shipped names still resolve `IsFireParticleLoaded`.
#[test]
fn burning_clonk_emits_shipped_fire_and_fire2_particles() {
    let mut engine = load_tutorial(1, 0);
    let owner = join_local_player(&mut engine, "Burning clonk fire particles");
    let clonk = engine
        .crew_cursor(owner)
        .expect("Tutorial01 joins with a selected clonk");

    assert!(
        catalog_has(&engine, "Fire") && catalog_has(&engine, "Fire2"),
        "the shipped fire defs load under the names SetDefParticles resolves",
    );
    assert!(
        engine
            .particle_render_catalog()
            .iter()
            .find(|definition| definition.core.name == "Fire2")
            .is_some_and(|definition| definition.core.additive != 0),
        "Fire2 is the additive def that gives the flame its glow",
    );

    // The crew joins above the landscape and falls in. Setting it alight
    // before it lands would put every particle at a negative world y, which
    // fxStdExec culls against `YOff` (src/C4Particles.cpp:695) — in C++ too.
    for _ in 0..40 {
        engine.tick_without_snapshot().expect("the scenario settles");
    }
    let index = engine.find_object_index(clonk).expect("the clonk is live");
    assert!(
        engine.objects[index].state.position.y > 0,
        "the clonk has landed inside the map before it is set alight",
    );
    assert!(engine
        .incinerate_object(index, owner, false, None)
        .expect("Incinerate completes"));
    assert!(engine.particle_system().particles().is_empty());

    // The emitter is gated on `iTime % 4` outside C4Fx_FireMode_Object
    // (src/C4Effect.cpp:673-674); a clonk is C4D_Living, so it burns in
    // C4Fx_FireMode_LivingVeg and waits for the fourth execution.
    for _ in 0..4 {
        engine.tick_without_snapshot().expect("the fire executes");
    }

    let particles = engine.particle_system().particles();
    let fire = particles
        .iter()
        .filter(|particle| particle.def_name == "Fire")
        .count();
    let fire2 = particles
        .iter()
        .filter(|particle| particle.def_name == "Fire2")
        .count();
    assert_eq!(
        fire + fire2,
        particles.len(),
        "every emitted particle is one of the two shipped fire defs",
    );
    assert!(
        fire2 > fire,
        "the additive Fire2 dominates the double set (got {fire} Fire, {fire2} Fire2)",
    );
    assert!(
        particles.iter().all(|particle| matches!(
            particle.layer,
            ParticleLayer::ObjectBack(id) | ParticleLayer::ObjectFront(id) if id == clonk
        )),
        "engine fire is dealt to the burning object's own particle lists, \
         not the global one a script CreateParticle would use",
    );
}
