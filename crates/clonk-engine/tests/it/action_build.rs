use clonk_engine::{
    ActionSpec, ActionState, Definition, Engine, PhysicalInfo, SpawnConfig, Vector2,
    CATEGORY_OBJECT, FULL_CON,
};
use clonk_resources::{Group, ResourceDefinition};
use std::{collections::HashMap, fs};

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }
"#;

fn builder_definition() -> Definition {
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "Builder",
        "Builder",
        BASIC_SCRIPT,
    ));
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    actions.insert(
        "Build".to_string(),
        ActionSpec::default()
            .with_procedure("Build")
            .with_length(10)
            .with_step(1),
    );
    definition.configure_actions(Some("Idle".to_string()), actions);
    // A crew/object builder stops when Target::Build reports FullCon. C++
    // deliberately exempts structure builders with no target from stopping
    // (src/C4Object.cpp:5010-5016), so this fixture must not be a structure.
    definition.set_category(CATEGORY_OBJECT);
    // CanConstruct=1 is C++'s legacy sentinel for normal (100%) speed.
    definition.set_physical(PhysicalInfo {
        can_construct: 1,
        ..PhysicalInfo::default()
    });
    definition
}

fn target_definition() -> Definition {
    let mut definition = crate::support::TestValueExt::test_value(Definition::from_script(
        "Target",
        "Target",
        BASIC_SCRIPT,
    ));
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    definition.configure_actions(Some("Idle".to_string()), actions);
    definition.set_mass(100);
    definition
}

#[test]
fn resource_turn_to_c4id_adapt_precedes_live_morph_lookup() -> Result<(), Box<dyn std::error::Error>>
{
    fn load_definition(
        root: &std::path::Path,
        id: &str,
        burn_to: &str,
        construct_to: &str,
    ) -> Result<Definition, Box<dyn std::error::Error>> {
        let path = root.join(format!("{id}.c4d"));
        fs::create_dir(&path)?;
        fs::write(
            path.join("DefCore.txt"),
            format!(
                "[DefCore]\nid={id}\nName={id}\nCategory=C4D_Structure\nMass=100\n\
                 BurnTo={burn_to}\nConstructTo={construct_to}\n"
            ),
        )?;
        fs::write(path.join("Script.c"), BASIC_SCRIPT)?;
        let resource = ResourceDefinition::load(&Group::open(&path)?)?;
        Ok(Definition::from_resource(&resource)?)
    }

    let temp = tempfile::tempdir()?;
    let overlong = load_definition(temp.path(), "TURN", "ASH1tail", "DONEtail")?;
    let short = load_definition(temp.path(), "SHRT", "ASH", "DON")?;

    assert_eq!(overlong.burn_turn_to(), Some("ASH1"));
    assert_eq!(overlong.build_turn_to(), Some("DONE"));
    assert_eq!(short.burn_turn_to(), None);
    assert_eq!(short.build_turn_to(), None);

    let mut engine = Engine::new();
    engine.register_definition(builder_definition())?;
    engine.register_definition(overlong)?;
    engine.register_definition(short)?;
    for id in ["ASH1", "DONE", "ASH", "DON"] {
        engine.register_definition(Definition::from_script(id, id, BASIC_SCRIPT)?)?;
    }

    let overlong_burn = engine.spawn_object(SpawnConfig::new("TURN"))?;
    let short_burn = engine.spawn_object(SpawnConfig::new("SHRT"))?;
    for object in [overlong_burn, short_burn] {
        let index = crate::support::TestValueExt::test_value(engine.find_object_index(object));
        assert!(engine.incinerate_object(index, 0, false, None)?);
    }
    assert_eq!(
        engine
            .object_snapshot(overlong_burn)
            .expect("overlong burn object survives")
            .definition_id,
        "ASH1",
        "BurnTo resolves the truncated four-byte target"
    );
    assert_eq!(
        engine
            .object_snapshot(short_burn)
            .expect("short burn object survives")
            .definition_id,
        "SHRT",
        "a registered three-byte target must not make short BurnTo live"
    );

    let overlong_build = engine.spawn_object(
        SpawnConfig::new("TURN")
            .with_position(Vector2::new(0, 0))
            .with_construction(0),
    )?;
    let short_build = engine.spawn_object(
        SpawnConfig::new("SHRT")
            .with_position(Vector2::new(20, 0))
            .with_construction(0),
    )?;
    for (target, x) in [(overlong_build, 0), (short_build, 20)] {
        let mut action = ActionState::new("Build");
        action.target = Some(target);
        engine.spawn_object(
            SpawnConfig::new("Builder")
                .with_position(Vector2::new(x, 0))
                .with_action(action),
        )?;
    }

    let snapshot = engine.tick()?;
    let overlong_built = crate::support::TestValueExt::test_value(snapshot.object(overlong_build));
    let short_built = crate::support::TestValueExt::test_value(snapshot.object(short_build));
    assert!(overlong_built.construction > 0, "the Build tick succeeded");
    assert!(short_built.construction > 0, "the Build tick succeeded");
    assert_eq!(
        overlong_built.definition_id, "DONE",
        "ConstructTo resolves the truncated four-byte target"
    );
    assert_eq!(
        short_built.definition_id, "SHRT",
        "a registered three-byte target must not make short ConstructTo live"
    );

    Ok(())
}

#[test]
fn build_procedure_advances_construction_and_stops_when_complete(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();
    engine.register_definition(builder_definition())?;
    engine.register_definition(target_definition())?;

    let target_id = engine.spawn_object(
        SpawnConfig::new("Target")
            .with_position(Vector2::new(0, 0))
            .with_construction(0),
    )?;

    let mut build_state = ActionState::new("Build");
    build_state.target = Some(target_id);
    let builder_id = engine.spawn_object(
        SpawnConfig::new("Builder")
            .with_position(Vector2::new(0, 0))
            .with_action(build_state),
    )?;

    let first_tick = engine.tick()?;
    let target_snapshot = crate::support::TestValueExt::test_value(first_tick.object(target_id));
    let expected_delta = (10 * 100 * 150) / 100;
    assert_eq!(target_snapshot.construction, expected_delta);

    let mut built = false;
    let mut last_snapshot = first_tick;
    for _ in 0..200 {
        let snapshot = engine.tick()?;
        let target_snapshot = crate::support::TestValueExt::test_value(snapshot.object(target_id));
        if target_snapshot.construction >= FULL_CON {
            last_snapshot = snapshot;
            built = true;
            break;
        }
        last_snapshot = snapshot;
    }

    assert!(built, "target should reach full construction");
    let builder_snapshot =
        crate::support::TestValueExt::test_value(last_snapshot.object(builder_id));
    assert_eq!(
        builder_snapshot.action.name, "Build",
        "the FullCon crossing frame still sees Target::Build succeed"
    );

    // The next Build frame sees Target::Build fail at FullCon and executes
    // ObjectComStop immediately (src/C4Object.cpp:5033-5042).
    let stopped = engine.tick()?;
    assert_eq!(
        stopped
            .object(builder_id)
            .expect("builder exists after stopping")
            .action
            .name,
        "Idle"
    );

    Ok(())
}
