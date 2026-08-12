use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
use clonk_engine::{AudioCommand, EffectState, Engine, ObjectId, PlayerConfig, SpawnConfig};
use clonk_script::Value;
use std::collections::HashMap;

const PLAYER: i32 = 1;

fn mars_engine() -> Engine {
    let mut engine = load_installed_scenario("ClonkMars.c4f/01_Fossae.c4s", 0);
    engine.register_test_player(PlayerConfig::new(PLAYER, "Mars oxygen tester"));
    engine
}

fn spawn_spaceclonk(engine: &mut Engine, oxygen: i32, warning: bool) -> ObjectId {
    engine.spawn_test_object(
        SpawnConfig::new("SCNK")
            .with_loaded(true)
            .with_owner(PLAYER)
            .with_controller(PLAYER)
            .with_alive(true)
            .with_crew_member(true)
            .with_energy(50_000)
            .with_local_vars(HashMap::from([
                ("O2".to_string(), Value::Int(oxygen)),
                ("O2Warning".to_string(), Value::Bool(warning)),
            ])),
    )
}

fn call_oxygen_timer(engine: &mut Engine, clonk: ObjectId) {
    let index = engine.test_object_index(clonk);
    engine.call_test_object_function(index, "FxO2Timer", Vec::new());
}

#[test]
fn mars_zero_oxygen_drains_health_before_death() {
    // ClonkMars Spaceclonk Script.c:168-170 calls DoEnergy(-20) once per O2
    // timer. Non-exact DoEnergy scales that to 20_000 raw energy
    // (C4Object.cpp:1372-1390), leaving a full-health 50_000-energy clonk alive.
    let mut engine = mars_engine();
    let clonk = spawn_spaceclonk(&mut engine, 0, false);

    call_oxygen_timer(&mut engine, clonk);

    let clonk = engine.test_object_snapshot(clonk);
    assert!(
        clonk.alive,
        "zero O2 must not kill a full-health clonk immediately"
    );
    assert_eq!(
        clonk.energy, 30_000,
        "the first zero-O2 tick drains 20,000 raw energy"
    );
}

#[test]
fn dead_mars_clonk_does_not_restart_the_low_oxygen_loop() {
    // AssignDeath clears effects inline before DoEnergy returns
    // (C4Object.cpp:1164-1177,1372-1392). The enclosing ClonkMars O2 timer
    // then resumes at Spaceclonk Script.c:172; a dead caller must not be able
    // to restart the loop that FxO2Stop just stopped at Script.c:183-190.
    let mut engine = mars_engine();
    let mut oxygen_effect = EffectState::new("O2").with_priority(100).with_interval(20);
    oxygen_effect.start_dispatched = true;
    let clonk = engine.spawn_test_object(
        SpawnConfig::new("SCNK")
            .with_loaded(true)
            .with_owner(PLAYER)
            .with_controller(PLAYER)
            .with_alive(true)
            .with_crew_member(true)
            .with_energy(10_000)
            .with_local_vars(HashMap::from([
                ("O2".to_string(), Value::Int(0)),
                ("O2Warning".to_string(), Value::Bool(true)),
            ]))
            .add_effect(oxygen_effect),
    );
    let index = engine.test_object_index(clonk);
    engine.objects[index].state.effects[0].command_target = Some(
        crate::support::TestValueExt::test_value(i32::try_from(clonk.as_u64())),
    );
    engine.pending_audio.clear();

    call_oxygen_timer(&mut engine, clonk);

    assert!(
        !engine.test_object_snapshot(clonk).alive,
        "the lethal oxygen tick kills the low-health Spaceclonk"
    );
    let warning_commands = engine
        .pending_audio
        .iter()
        .filter(|command| match command {
            AudioCommand::PlaySound { name, .. } | AudioCommand::StopSound { name, .. } => {
                name == "Warning_lowoxygen"
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    assert!(
        matches!(
            warning_commands.last(),
            Some(AudioCommand::StopSound { .. })
        ),
        "death must leave the low-O2 loop stopped; got {warning_commands:?}"
    );
}
