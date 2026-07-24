use crate::support::real_scenario::{join_local_player, PreparedInstalledScenario};
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
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");
    let observer = join_local_player(&mut engine, "Invisibility observer");
    engine
        .set_hostility(owner, observer, true)
        .expect("players become hostile");

    let spell = engine
        .spawn_object(SpawnConfig::new("MINV").with_owner(owner))
        .expect("the shipped MINV spell spawns");
    engine
        .call_object_function(
            engine.find_object_index(spell).expect("MINV index"),
            "Activate",
            vec![Value::Object(mage.as_u64()), Value::Nil],
        )
        .expect("the shipped invisibility spell activates");
    engine
        .tick_without_snapshot()
        .expect("the effect-start callback executes on the next engine pass");

    let hidden = engine.snapshot();
    let mage_snapshot = hidden.object(mage).expect("mage remains after cast");
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

    engine
        .register_definition(
            Definition::from_script(
                "VSTP",
                "Visibility stop probe",
                r#"#strict
public func Stop(object target)
{
    return RemoveEffect("InvisPSpell", target);
}
"#,
            )
            .expect("stop probe compiles"),
        )
        .expect("stop probe registers");
    let probe = engine
        .spawn_object(SpawnConfig::new("VSTP"))
        .expect("stop probe spawns");
    engine
        .call_object_function(
            engine.find_object_index(probe).expect("stop probe index"),
            "Stop",
            vec![Value::Object(mage.as_u64())],
        )
        .expect("the shipped invisibility stop callback runs");

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
    let mage = engine
        .crew_cursor(owner)
        .expect("Alchemy joins with its MCLK cursor");

    let cast = |engine: &mut clonk_engine::Engine| {
        let spell = engine
            .spawn_object(SpawnConfig::new("MINV").with_owner(owner))
            .expect("the shipped MINV spell spawns");
        engine
            .call_object_function(
                engine.find_object_index(spell).expect("MINV index"),
                "Activate",
                vec![Value::Object(mage.as_u64()), Value::Nil],
            )
            .expect("the shipped invisibility spell activates");
    };

    cast(&mut engine);
    for _ in 0..7 {
        engine
            .tick_without_snapshot()
            .expect("the invisibility timer advances");
    }
    let before = engine
        .object_snapshot(mage)
        .expect("mage remains after first cast")
        .effects
        .into_iter()
        .find(|effect| effect.name == "InvisPSpell")
        .expect("first invisibility effect exists");
    assert!(before.timer > 0, "the first spell has consumed some time");

    cast(&mut engine);
    let effects = engine
        .object_snapshot(mage)
        .expect("mage remains after recast")
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
        .object_snapshot(mage)
        .expect("mage remains after recast")
        .effects
        .iter()
        .any(|effect| effect.name == "InvisPSpell" && effect.priority == 0));
    engine
        .tick_without_snapshot()
        .expect("the mage's next Execute cleans the dead recast node");
    let after = engine
        .object_snapshot(mage)
        .expect("mage remains after cleanup")
        .effects
        .into_iter()
        .filter(|effect| effect.name == "InvisPSpell")
        .collect::<Vec<_>>();
    assert_eq!(after.len(), 1);
    assert_eq!(after[0].number, before.number);
    assert_eq!(after[0].timer, 1);
}
