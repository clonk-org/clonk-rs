use super::*;

#[test]
fn exec_action_set_dir_respects_signed_directions_gate() -> Result<(), EngineError> {
    for (raw_directions, expected) in [
        (-2, Direction::Left),
        (0, Direction::Left),
        (1, Direction::Left),
        (2, Direction::Right),
    ] {
        let id = format!("D{:03}", raw_directions + 2);
        let mut definition = Definition::from_script(id.as_str(), "Direction gate", "")?;
        definition.configure_actions(
            None,
            HashMap::from([(
                    "Walk".to_string(),
                ActionSpec::default()
                    .with_procedure("WALK")
                    .with_directions(raw_directions),
            )]),
        );
        let mut engine = Engine::new();
        engine.register_definition(definition)?;
        let object = engine.spawn_object(
            SpawnConfig::new(id.as_str())
                .with_action(ActionState::new("Walk"))
                .with_direction(Direction::Left),
        )?;
        let index = engine.find_object_index(object).expect("object exists");
        let definition_id = engine.objects[index].definition_id.clone();

        engine.set_exec_action_direction(index, &definition_id, Direction::Right)?;

        assert_eq!(
            engine.objects[index].state.direction, expected,
                "Directions={raw_directions}"
        );
    }
    Ok(())
}
