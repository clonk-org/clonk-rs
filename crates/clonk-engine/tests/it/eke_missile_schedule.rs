use crate::support::real_scenario::load_installed_scenario;
use clonk_engine::{EffectVarValue, SpawnConfig};
use clonk_script::Value;

#[test]
fn eke_missile_scheduled_explosion_uses_its_power_local() {
    // Helpers.c runs FxIntScheduleTimer with the effect command target as its
    // object context, and FnEval therefore selects that object's definition
    // locals and functions (oracle-src-pinned C4Effect.cpp:342-350;
    // C4Script.cpp:4501-4512; C4AulExec.cpp:1658-1706).
    let mut engine = load_installed_scenario(
        "EkeReloaded.c4f/InterplanetaryCivilwar.c4f/MissileMatch.c4s",
        0,
    );
    let launcher = engine
        .spawn_object(SpawnConfig::new("RL5B").with_loaded(true))
        .expect("real Eke rocket launcher spawns");
    let missile = engine
        .spawn_object(SpawnConfig::new("MS5B").with_loaded(true))
        .expect("real Eke missile spawns");

    let missile_index = engine
        .find_object_index(missile)
        .expect("the missile has an index");
    engine
        .call_object_function(
            missile_index,
            "Launch",
            vec![Value::Object(launcher.as_u64()), Value::Nil],
        )
        .expect("the missile launches from the real rocket launcher");
    assert_eq!(
        engine
            .object_snapshot(missile)
            .expect("the launched missile remains live")
            .local_vars
            .get("power"),
        Some(&Value::Int(50))
    );

    let missile_index = engine
        .find_object_index(missile)
        .expect("the launched missile has an index");
    engine
        .call_object_function(missile_index, "BlowUp", Vec::new())
        .expect("the missile schedules its explosion");
    let scheduled = engine
        .object_snapshot(missile)
        .expect("the scheduled missile remains live until the timer")
        .effects
        .into_iter()
        .find(|effect| effect.name == "IntSchedule" && effect.priority == 1)
        .expect("BlowUp arms the real one-shot IntSchedule effect");
    assert_eq!(
        scheduled.vars,
        [
            EffectVarValue::String("Explode(power)".into()),
            EffectVarValue::Int(1)
        ],
        "the real Helpers.c effect retains the missile's explosion expression"
    );

    engine
        .tick_without_snapshot()
        .expect("the scheduled explosion executes");
    assert!(
        engine.object_snapshot(missile).is_none(),
        "eval resolves MS5B's power local and Explode removes the missile"
    );
}
