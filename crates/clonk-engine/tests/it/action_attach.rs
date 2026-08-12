use clonk_engine::{
    ActionSpec, ActionState, Definition, Engine, ObjectVertex, SpawnConfig, Vector2,
};
use std::collections::HashMap;

const BASIC_SCRIPT: &str = r#"
global func Initialize(state, random) { return 0; }

global func Step(state, frame, random) { return 0; }
"#;

#[test]
fn attach_procedure_synchronizes_position_and_container() -> Result<(), Box<dyn std::error::Error>>
{
    let mut engine = Engine::new();

    let mut chest_definition = Definition::from_script("Chest", "Chest", BASIC_SCRIPT)?;
    let mut chest_actions = HashMap::new();
    chest_actions.insert("Idle".to_string(), ActionSpec::default());
    chest_definition.configure_actions(Some("Idle".to_string()), chest_actions);
    engine.register_definition(chest_definition)?;

    let mut anchor_definition = Definition::from_script("Anchor", "Anchor", BASIC_SCRIPT)?;
    let mut anchor_actions = HashMap::new();
    anchor_actions.insert("Idle".to_string(), ActionSpec::default());
    anchor_definition.configure_actions(Some("Idle".to_string()), anchor_actions);
    engine.register_definition(anchor_definition)?;

    let mut attached_definition = Definition::from_script("Attached", "Attached", BASIC_SCRIPT)?;
    let mut attach_actions = HashMap::new();
    attach_actions.insert("Attach".to_string(), ActionSpec::for_procedure("Attach"));
    attached_definition.configure_actions(Some("Attach".to_string()), attach_actions);
    engine.register_definition(attached_definition)?;

    // The chest carries the anchor: contained objects copy the container's
    // position every frame (C4Object::CopyMotion, C4Movement.cpp:518-529),
    // so the anchor can only sit at (100, 80) if its container does.
    let chest_id = engine.spawn_object(
        SpawnConfig::new("Chest")
            .with_position(Vector2::new(100, 80))
            .with_action(ActionState::new("Idle")),
    )?;

    let anchor_id = engine.spawn_object(
        SpawnConfig::new("Anchor")
            .with_position(Vector2::new(100, 80))
            .with_vertices(vec![ObjectVertex::new(0, 0), ObjectVertex::new(6, -4)])
            .with_action(ActionState::new("Idle"))
            .with_container(chest_id),
    )?;

    let mut attach_state = ActionState::new("Attach");
    attach_state.data = (1 << 8) | 1;
    attach_state.target = Some(anchor_id);

    let attached_id = engine.spawn_object(
        SpawnConfig::new("Attached")
            .with_position(Vector2::new(10, 10))
            .with_vertices(vec![ObjectVertex::new(0, 0), ObjectVertex::new(2, 3)])
            .with_action(attach_state),
    )?;

    let snapshot = engine.tick()?;

    let attached = crate::support::TestValueExt::test_value(snapshot.object(attached_id));
    // DFA_ATTACH ForcePositions to the vertex-aligned spot in ExecAction
    // (C4Object.cpp:5330-5336), but the same frame's ExecMovement runs
    // CopyMotion for contained objects (C4Movement.cpp:556-561), which
    // overrides x/y with the CONTAINER's position — the vertex math only
    // sticks for uncontained targets (see
    // attach_procedure_positions_by_vertices_when_uncontained).
    assert_eq!(attached.position, Vector2::new(100, 80));
    assert_eq!(attached.velocity, Vector2::new(0, 0));
    assert_eq!(attached.container, Some(chest_id));

    let chest_snapshot = crate::support::TestValueExt::test_value(snapshot.object(chest_id));
    assert!(chest_snapshot.contents.contains(&anchor_id));
    assert!(chest_snapshot.contents.contains(&attached_id));

    Ok(())
}

#[test]
fn attach_procedure_positions_by_vertices_when_uncontained(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = Engine::new();

    let mut anchor_definition = Definition::from_script("Anchor", "Anchor", BASIC_SCRIPT)?;
    let mut anchor_actions = HashMap::new();
    anchor_actions.insert("Idle".to_string(), ActionSpec::default());
    anchor_definition.configure_actions(Some("Idle".to_string()), anchor_actions);
    engine.register_definition(anchor_definition)?;

    let mut attached_definition = Definition::from_script("Attached", "Attached", BASIC_SCRIPT)?;
    let mut attach_actions = HashMap::new();
    attach_actions.insert("Attach".to_string(), ActionSpec::for_procedure("Attach"));
    attached_definition.configure_actions(Some("Attach".to_string()), attach_actions);
    engine.register_definition(attached_definition)?;

    let anchor_id = engine.spawn_object(
        SpawnConfig::new("Anchor")
            .with_position(Vector2::new(100, 80))
            .with_vertices(vec![ObjectVertex::new(0, 0), ObjectVertex::new(6, -4)])
            .with_action(ActionState::new("Idle")),
    )?;

    let mut attach_state = ActionState::new("Attach");
    attach_state.data = (1 << 8) | 1;
    attach_state.target = Some(anchor_id);

    let attached_id = engine.spawn_object(
        SpawnConfig::new("Attached")
            .with_position(Vector2::new(10, 10))
            .with_vertices(vec![ObjectVertex::new(0, 0), ObjectVertex::new(2, 3)])
            .with_action(attach_state),
    )?;

    let snapshot = engine.tick()?;

    // ForcePosition math (C4Object.cpp:5330-5336): target pos + target
    // vertex - own vertex = (100+6-2, 80-4-3); nothing overrides it for
    // an uncontained attach.
    let attached = crate::support::TestValueExt::test_value(snapshot.object(attached_id));
    assert_eq!(attached.position, Vector2::new(104, 73));
    assert_eq!(attached.velocity, Vector2::new(0, 0));
    assert_eq!(attached.container, None);

    Ok(())
}
