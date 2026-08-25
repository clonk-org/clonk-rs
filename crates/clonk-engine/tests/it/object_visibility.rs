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

pub(crate) fn shipped_invisibility_expires_when_its_timerless_interval_elapses(
    prepared: &PreparedInstalledScenario,
) {
    // Invisibility.c4d/Script.c defines no FxInvisPSpellTimer at all, so the
    // only thing that ends it is C4Effect::Execute's else arm: an effect with
    // a nonzero interval and no timer function is killed the moment that
    // interval elapses ("no timer function: mark dead after time elapsed",
    // C4Effect.cpp:342-357). Activate's single-caster branch asks for
    // interval 1400 — the "40sec" the shipped comment claims.
    let mut engine = prepared.instantiate();
    assert!(
        !engine.definition_script_has_function("MINV", "FxInvisPSpellTimer"),
        "the shipped spell must keep relying on the timerless-kill path"
    );
    let owner = join_local_player(&mut engine, "Invisibility expiry owner");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));
    let observer = join_local_player(&mut engine, "Invisibility expiry observer");
    crate::support::TestValueExt::test_value(engine.set_hostility(owner, observer, true));

    let spell = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64()), Value::Nil],
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    let cast = crate::support::TestValueExt::test_value(
        engine
            .test_object_snapshot(mage)
            .effects
            .into_iter()
            .find(|effect| effect.name == "InvisPSpell"),
    );
    assert_eq!(cast.interval, 1_400, "Activate's single-caster branch");
    assert!(!engine
        .snapshot()
        .object_visible_for_player(mage, observer, false));

    // The Activate tick above already advanced the effect once, so the
    // interval elapses on the 1400th.
    for _ in 0..1_398 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        engine
            .test_object_snapshot(mage)
            .effects
            .iter()
            .any(|effect| effect.name == "InvisPSpell" && effect.priority != 0),
        "the spell is still running one tick before its interval elapses"
    );

    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());

    let expired = engine.snapshot();
    let mage_snapshot = crate::support::TestValueExt::test_value(expired.object(mage));
    assert!(
        !mage_snapshot
            .effects
            .iter()
            .any(|effect| effect.name == "InvisPSpell" && effect.priority != 0),
        "the timerless interval kills the effect"
    );
    assert_eq!(
        mage_snapshot.visibility, VIS_ALL,
        "FxInvisPSpellStop restores the pre-cast visibility when the spell runs out"
    );
    assert_eq!(
        mage_snapshot.color_modulation, 0,
        "and its pre-cast colour modulation"
    );
    assert!(expired.object_visible_for_player(mage, observer, false));

    // A carrier that walks into a building keeps ticking its effects:
    // C4GameObjects::Execute walks the whole main list, and contents are in it
    // (C4Object.cpp:1953-2012). If a contained clonk stopped ticking, the
    // spell really would never run out.
    let container = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
    let spell = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64()), Value::Nil],
    );
    crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    crate::support::TestValueExt::test_value(engine.apply_object_update(
        mage,
        clonk_engine::ObjectUpdate::new().with_container(container),
    ));
    assert_eq!(
        engine.test_object_snapshot(mage).container,
        Some(container),
        "the invisible mage is now contained"
    );
    for _ in 0..1_399 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        !engine
            .test_object_snapshot(mage)
            .effects
            .iter()
            .any(|effect| effect.name == "InvisPSpell" && effect.priority != 0),
        "the interval still elapses while the carrier is contained"
    );
    assert_eq!(
        engine.test_object_snapshot(mage).visibility,
        VIS_ALL,
        "and the contained carrier's visibility is restored too"
    );
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

/// Invisibility does not tick down while its target is on the inactive list.
///
/// Reported as "invisibility never runs out" (clonk-org/clonk-rs#980). It is
/// the shipped behaviour, not a port defect, and the reason is structural:
/// `C4Game::ExecObjects` walks only the **active** list (C4Game.cpp:1582-1615)
/// and effect timers advance from `C4Object::Execute` (C4Object.cpp:1069-1090;
/// C4Effect.cpp:319-363). Dragon Rock's FoW generator hides objects with
/// `SetObjectStatus(C4OS_INACTIVE, …)` (FoWGenerator.c4d/Script.c:93,107), so
/// a mage sitting in the fog keeps its full remaining duration however long it
/// stays there — an invisibility that expired on wall-clock time instead would
/// be a gameplay change, and a divergence from the oracle.
pub(crate) fn shipped_invisibility_pauses_while_its_target_is_inactive(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    let owner = join_local_player(&mut engine, "Invisibility owner");
    let mage = crate::support::TestValueExt::test_value(engine.crew_cursor(owner));

    let spell = engine.spawn_test_object(SpawnConfig::new("MINV").with_owner(owner));
    engine.call_test_object_function(
        engine.test_object_index(spell),
        "Activate",
        vec![Value::Object(mage.as_u64()), Value::Nil],
    );

    // The generator's own hide/show calls, reached the way its script does.
    engine.register_test_definition(crate::support::TestValueExt::test_value(
        Definition::from_script(
            "FOWP",
            "Fog status probe",
            r#"#strict
        public func Hide(object target) { return SetObjectStatus(C4OS_INACTIVE(), target); }
        public func Show(object target) { return SetObjectStatus(C4OS_NORMAL(), target); }
        "#,
        ),
    ));
    let probe = engine.spawn_test_object(SpawnConfig::new("FOWP"));
    let call = |engine: &mut clonk_engine::Engine, function: &str| {
        let index = engine.test_object_index(probe);
        engine.call_test_object_function(index, function, vec![Value::Object(mage.as_u64())]);
    };
    let invisibility_timer = |engine: &clonk_engine::Engine| {
        crate::support::TestValueExt::test_value(
            engine
                .test_object_snapshot(mage)
                .effects
                .into_iter()
                .find(|effect| effect.name == "InvisPSpell" && effect.priority != 0)
                .map(|effect| effect.timer),
        )
    };

    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    let running = invisibility_timer(&engine);
    assert!(
        running > 0,
        "the spell is ticking down while the mage is out"
    );

    call(&mut engine, "Hide");
    for _ in 0..400 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert_eq!(
        invisibility_timer(&engine),
        running,
        "an inactive object is never executed, so its effects hold their time"
    );

    call(&mut engine, "Show");
    for _ in 0..5 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    assert!(
        invisibility_timer(&engine) > running,
        "reactivating the target resumes the same clock"
    );
}
