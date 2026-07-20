use lc_engine::{
    ActionSpec, ActionState, Definition, Engine, PhysicalInfo, SpawnConfig, Vector2,
    CATEGORY_OBJECT, FULL_CON,
};
use std::collections::HashMap;

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }
"#;

fn builder_definition() -> Definition {
    let mut definition = Definition::from_script("Builder", "Builder", BASIC_SCRIPT).unwrap();
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
    let mut definition = Definition::from_script("Target", "Target", BASIC_SCRIPT).unwrap();
    let mut actions = HashMap::new();
    actions.insert("Idle".to_string(), ActionSpec::default());
    definition.configure_actions(Some("Idle".to_string()), actions);
    definition.set_mass(100);
    definition
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
    let target_snapshot = first_tick
        .object(target_id)
        .expect("target exists after first tick");
    let expected_delta = (10 * 100 * 150) / 100;
    assert_eq!(target_snapshot.construction, expected_delta);

    let mut built = false;
    let mut last_snapshot = first_tick;
    for _ in 0..200 {
        let snapshot = engine.tick()?;
        let target_snapshot = snapshot
            .object(target_id)
            .expect("target exists while building");
        if target_snapshot.construction >= FULL_CON {
            last_snapshot = snapshot;
            built = true;
            break;
        }
        last_snapshot = snapshot;
    }

    assert!(built, "target should reach full construction");
    let builder_snapshot = last_snapshot
        .object(builder_id)
        .expect("builder exists after construction");
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
