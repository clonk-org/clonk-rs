use crate::support::real_scenario::load_installed_scenario;
use crate::support::EngineTestExt;
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
    let launcher = engine.spawn_test_object(SpawnConfig::new("RL5B").with_loaded(true));
    let missile = engine.spawn_test_object(SpawnConfig::new("MS5B").with_loaded(true));

    let missile_index = engine.test_object_index(missile);
    engine.call_test_object_function(
        missile_index,
        "Launch",
        vec![Value::Object(launcher.as_u64()), Value::Nil],
    );
    assert_eq!(
        engine.test_object_snapshot(missile).local_vars.get("power"),
        Some(&Value::Int(50))
    );

    let missile_index = engine.test_object_index(missile);
    engine.call_test_object_function(missile_index, "BlowUp", Vec::new());
    let scheduled = crate::support::TestValueExt::test_value(
        engine
            .test_object_snapshot(missile)
            .effects
            .into_iter()
            .find(|effect| effect.name == "IntSchedule" && effect.priority == 1),
    );
    assert_eq!(
        scheduled.vars,
        [
            EffectVarValue::String("Explode(power)".into()),
            EffectVarValue::Int(1)
        ],
        "the real Helpers.c effect retains the missile's explosion expression"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    assert!(
        engine.object_snapshot(missile).is_none(),
        "eval resolves MS5B's power local and Explode removes the missile"
    );
}
