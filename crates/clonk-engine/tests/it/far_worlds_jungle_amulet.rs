use crate::support::real_scenario::{prepare_installed_scenario, PreparedInstalledScenario};
use crate::support::PreparedScenarioSubcase;
use clonk_engine::{SpawnConfig, Vector2};
use clonk_script::Value;

#[test]
fn jungle_amulet_real_scenario_subcases() {
    let prepared = prepare_installed_scenario("FarWorlds.c4f/Jungle.c4s", 0);
    let subcases: &[PreparedScenarioSubcase] = &[
        (
            "upgrade_initializes_the_new_definition_inline",
            jungle_amulet_upgrade_initializes_the_new_definition_inline,
        ),
        (
            "poison_amulet_denies_the_shipped_poison_arrow_curse_inline",
            jungle_poison_amulet_denies_the_shipped_poison_arrow_curse_inline,
        ),
    ];
    let mut failures = Vec::new();

    for &(name, subcase) in subcases {
        eprintln!("running Jungle amulet subcase `{name}`");
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| subcase(&prepared))).is_err() {
            eprintln!("Jungle amulet subcase `{name}` failed; continuing batch");
            failures.push(name);
        }
    }

    if !failures.is_empty() {
        panic!(
            "{} Jungle amulet subcase(s) failed: {}",
            failures.len(),
            failures.join(", ")
        );
    }
}

fn jungle_amulet_upgrade_initializes_the_new_definition_inline(
    prepared: &PreparedInstalledScenario,
) {
    // AMUL::Upgrade changes its own definition and immediately calls
    // `this()->~Initialize()`. C++ resolves that arrow call against the
    // object's live, newly installed Def, so every upgraded amulet performs
    // its new Initialize before Upgrade returns
    // (FarWorlds.c4d/Jungle.c4d/Items.c4d/Tools.c4d/Amulet.c4d/Script.c:42-46;
    // src/C4Object.cpp:1205-1231; src/C4AulExec.cpp:1216-1305).
    let mut engine = prepared.instantiate();

    for (offset, upgraded, effect, action) in [
        (0, "AMPH", Some("PhysicalBless"), None),
        (20, "AMPO", Some("BanPoison"), None),
        (40, "AMMA", None, Some("Be")),
    ] {
        let clonk = engine
            .spawn_object(SpawnConfig::new("JCLK").with_position(Vector2::new(100 + offset, 100)))
            .expect("the shipped Jungle Clonk spawns");
        let amulet = engine
            .spawn_object(
                SpawnConfig::new("AMUL")
                    .with_position(Vector2::new(100 + offset, 100))
                    .with_container(clonk),
            )
            .expect("the shipped base amulet spawns in the Clonk");
        let index = engine
            .find_object_index(amulet)
            .expect("the base amulet remains live");

        engine
            .call_object_function(
                index,
                "Upgrade",
                vec![Value::C4Id(upgraded.into()), Value::Object(clonk.as_u64())],
            )
            .expect("the shipped Upgrade callback completes");

        let amulet = engine
            .object_snapshot(amulet)
            .expect("the upgraded amulet remains live");
        assert_eq!(amulet.definition_id, upgraded);
        assert_eq!(
            amulet.container,
            Some(clonk),
            "ChangeDef preserves the contained object's identity"
        );
        if let Some(action) = action {
            assert_eq!(
                amulet.action.name, action,
                "{upgraded} Initialize must run against the new ActMap"
            );
            assert_eq!(amulet.local_vars.get("iSelection"), Some(&Value::Nil));
        }

        let clonk = engine
            .object_snapshot(clonk)
            .expect("the amulet carrier remains live");
        assert_eq!(
            clonk.action.name, "Magic",
            "Upgrade performs the shipped Clonk magic action"
        );
        if let Some(effect) = effect {
            let entry = clonk
                .effects
                .iter()
                .find(|entry| entry.name == effect)
                .unwrap_or_else(|| {
                    panic!("{upgraded} Initialize must install {effect} before Upgrade returns")
                });
            let expected_priority = if upgraded == "AMPH" { 182 } else { 242 };
            assert_eq!(entry.priority, expected_priority);
            assert_eq!(entry.interval, 0);
            assert_eq!(entry.command_id.as_deref(), Some(upgraded));
        }
        if upgraded == "AMPH" {
            let physical = clonk
                .temporary_physical
                .expect("PhysicalBless starts synchronously");
            assert_eq!(physical.can_hangle, 1);
            assert_eq!(physical.dig, 50_000);
            assert_eq!(physical.walk, 80_000);
            assert_eq!(physical.jump, 50_000);
            assert_eq!(physical.throw, 60_000);
            assert_eq!(physical.swim, 100_000);
            assert_eq!(physical.scale, 80_000);
            assert_eq!(physical.hangle, 80_000);
            assert_eq!(physical.fight, 90_000);
            assert_eq!(physical.breath, 70_000);
            assert_eq!(
                clonk.physical_changes,
                [
                    ("CanHangle".into(), 0),
                    ("Dig".into(), 40_000),
                    ("Walk".into(), 70_000),
                    ("Jump".into(), 40_000),
                    ("Throw".into(), 50_000),
                    ("Swim".into(), 70_000),
                    ("Scale".into(), 30_000),
                    ("Hangle".into(), 30_000),
                    ("Fight".into(), 50_000),
                    ("Breath".into(), 50_000),
                ]
            );
        }
    }
}

fn jungle_poison_amulet_denies_the_shipped_poison_arrow_curse_inline(
    prepared: &PreparedInstalledScenario,
) {
    // PARW::HitTarget adds PoisonCurse at priority 182. C++ asks every
    // existing effect with at least that priority before validating the new
    // effect; AMPO's priority-242 FxBanPoisonEffect returns -1, so the curse
    // never becomes live (FarWorlds.c4d/Jungle.c4d/Items.c4d/Weapons.c4d/
    // PoisonArrowPack.c4d/PoisonArrow.c4d/Script.c:20-25;
    // FarWorlds.c4d/Jungle.c4d/Items.c4d/Tools.c4d/Amulet.c4d/Immun.c4d/
    // Script.c:20-31; src/C4Effect.cpp:97-116,271-285).
    let mut engine = prepared.instantiate();
    let protected = engine
        .spawn_object(SpawnConfig::new("JCLK").with_position(Vector2::new(100, 100)))
        .expect("the protected shipped Jungle Clonk spawns");
    let unprotected = engine
        .spawn_object(SpawnConfig::new("JCLK").with_position(Vector2::new(140, 100)))
        .expect("the control shipped Jungle Clonk spawns");
    let amulet = engine
        .spawn_object(
            SpawnConfig::new("AMUL")
                .with_position(Vector2::new(100, 100))
                .with_container(protected),
        )
        .expect("the shipped base amulet spawns in the protected Clonk");
    let amulet_index = engine
        .find_object_index(amulet)
        .expect("the shipped base amulet remains live");
    engine
        .call_object_function(
            amulet_index,
            "Upgrade",
            vec![
                Value::C4Id("AMPO".into()),
                Value::Object(protected.as_u64()),
            ],
        )
        .expect("the shipped poison-immunity Upgrade completes");

    for target in [protected, unprotected] {
        let arrow = engine
            .spawn_object(
                SpawnConfig::new("PARW").with_position(
                    engine
                        .object_snapshot(target)
                        .expect("the arrow target remains live")
                        .position,
                ),
            )
            .expect("a fresh shipped poison arrow spawns");
        let arrow_index = engine
            .find_object_index(arrow)
            .expect("the fresh poison arrow remains live");
        engine
            .call_object_function(
                arrow_index,
                "HitTarget",
                vec![Value::Object(target.as_u64()), Value::Nil],
            )
            .expect("the shipped poison-arrow hit callback completes");
    }

    let protected_snapshot = engine
        .object_snapshot(protected)
        .expect("the protected Jungle Clonk remains live");
    assert!(
        protected_snapshot
            .effects
            .iter()
            .any(|effect| effect.name == "BanPoison"),
        "the AMPO protection must survive the rejected curse"
    );

    let unprotected_snapshot = engine
        .object_snapshot(unprotected)
        .expect("the control Jungle Clonk remains live");
    assert!(
        unprotected_snapshot
            .effects
            .iter()
            .any(|effect| effect.name == "PoisonCurse" && effect.priority != 0),
        "the fresh control arrow must prove the shipped poison path executed"
    );

    assert!(
        protected_snapshot
            .effects
            .iter()
            .all(|effect| effect.name != "PoisonCurse" || effect.priority == 0),
        "C++ denies PoisonCurse synchronously before it becomes live"
    );
    assert!(protected_snapshot
        .effects
        .iter()
        .any(|effect| effect.name == "PoisonCurse" && effect.priority == 0));
    engine
        .tick_without_snapshot()
        .expect("the protected Clonk's next Execute cleans the dead curse");
    assert!(engine
        .object_snapshot(protected)
        .expect("the protected Jungle Clonk remains live")
        .effects
        .iter()
        .all(|effect| effect.name != "PoisonCurse"));
}
