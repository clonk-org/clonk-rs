use super::*;

#[test]
fn object_action_fight_respects_no_other_action() {
    let mut engine = Engine::new();
    let mut definition = test_definition("FLOK", "Fight-locked actor", "#strict\n");
    definition.configure_actions(
        Some("Fight".to_string()),
        HashMap::from([
            (
                "Fight".to_string(),
                ActionSpec::default().with_procedure("FIGHT"),
            ),
            (
                "Dead".to_string(),
                ActionSpec::default().with_no_other_action(true),
            ),
        ]),
    );
    crate::TestValueExt::test_value(engine.register_definition(definition));
    let locked = crate::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("FLOK").with_action(ActionState::new("Dead"))),
    );
    let target = crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("FLOK")));

    engine.object_action_fight(locked, target);

    let locked = crate::TestValueExt::test_value(engine.object_snapshot(locked));
    assert_eq!(locked.action.name, "Dead");
    assert_eq!(locked.action.target, None);
}
