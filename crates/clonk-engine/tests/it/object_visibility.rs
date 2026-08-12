use crate::support::real_scenario::{join_local_player, PreparedInstalledScenario};
use crate::support::EngineTestExt;
use clonk_engine::{Definition, SpawnConfig, VIS_ALL, VIS_ALLIES, VIS_GOD, VIS_OWNER};
use clonk_script::Value;

pub(crate) fn shipped_invisibility_spell_hides_and_restores_its_mage(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    assert!(
        engine.definition_script_has_function("MINV", "FxInvisPSpellStart"),
        "the installed MINV definition retains its shipped effect callback"
    );
    let owner = join_local_player(&mut engine, "Invisibility owner");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let observer = join_local_player(&mut engine, "Invisibility observer");
    crate::support::TestValueExt::test_value(engine.set_hostility(owner, observer, true));

    let spell = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64()), Value::Nil],
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    let hidden = engine.snapshot();
    let mage_snapshot = crate::support::TestValueExt::test_value(hidden.object(mage));
    assert_eq!(
        mage_snapshot.visibility,
        VIS_OWNER | VIS_ALLIES | VIS_GOD,
        "FxInvisPSpellStart installs the shipped visibility mask; effects={:?} color_mod={:#x}",
        mage_snapshot.effects,
        mage_snapshot.color_modulation,
    );
    assert_eq!(
        mage_snapshot.color_modulation, 0x7f7e_7efe,
        "the shipped pre-strict-3 zero modulation defaults to opaque white"
    );
    assert!(hidden.object_visible_for_player(mage, owner, false));
    assert!(!hidden.object_visible_for_player(mage, observer, false));
    assert!(hidden.object_visible_for_player(mage, -1, false));

    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "VSTP",
            "Visibility stop probe",
            r#"#strict
        public func Stop(object target)
        {
            return RemoveEffect("InvisPSpell", target);
        }
        "#,
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("VSTP"));
    engine.call_test_object_function(
        engine.test_object_index(probe),
        "Stop",
        vec![Value::Object(mage.as_u64())],
    );

    let restored = engine.snapshot();
    assert_eq!(
        restored.object(mage).expect("mage remains").visibility,
        VIS_ALL,
        "FxInvisPSpellStop restores the pre-cast visibility"
    );
    assert_eq!(
        restored
            .object(mage)
            .expect("mage remains")
            .color_modulation,
        0,
        "FxInvisPSpellStop restores the pre-cast color modulation"
    );
    assert!(restored.object_visible_for_player(mage, observer, false));
}

pub(crate) fn shipped_invisibility_recast_carries_remaining_time_into_reset_timer(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Invisibility owner");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    let cast = |engine: &mut clonk_engine::Engine| {
        let spell = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
        engine.call_test_object_function(
            engine.test_object_index(spell),
            "Activate",
            vec![Value::Object(mage.as_u64()), Value::Nil],
        );
    };

    cast(&mut engine);
    for _ in 0..7 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let before = crate::support::TestValueExt::test_value(
        engine
            .test_object_snapshot(mage)
            .effects
            .into_iter()
            .find(|effect| effect.name == "InvisPSpell"),
    );
    assert!(before.timer > 0, "the first spell has consumed some time");

    cast(&mut engine);
    let effects = engine
        .test_object_snapshot(mage)
        .effects
        .into_iter()
        .filter(|effect| effect.name == "InvisPSpell" && effect.priority != 0)
        .collect::<Vec<_>>();
    assert_eq!(effects.len(), 1, "FxInvisPSpellEffect merges the recast");
    assert_eq!(
        effects[0].number, before.number,
        "the recast extends the existing effect identity"
    );
    assert_eq!(
        effects[0].interval,
        before.interval - before.timer + 1_400,
        "FxInvisPSpellAdd carries the old remaining duration into ChangeEffect"
    );
    assert_eq!(
        effects[0].timer, 0,
        "ChangeEffect restarts the merged invisibility clock"
    );
    assert!(engine
        .test_object_snapshot(mage)
        .effects
        .iter()
        .any(|effect| effect.name == "InvisPSpell" && effect.priority == 0));
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    let after = engine
        .test_object_snapshot(mage)
        .effects
        .into_iter()
        .filter(|effect| effect.name == "InvisPSpell")
        .collect::<Vec<_>>();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].number, before.number);
    assert_eq!(after[0].timer, 1);
}
