use super::*;

#[test]
fn object_action_fight_respects_no_other_action() {
    let mut engine = Engine::new();
    let mut definition = Definition::from_script("FLOK", "Fight-locked actor", "#strict\n")
        .expect("fight actor compiles");
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
    engine
        .register_definition(definition)
        .expect("fight actor registers");
    let locked = engine
        .spawn_object(SpawnConfig::new("FLOK").with_action(ActionState::new("Dead")))
        .expect("locked actor spawns");
    let target = engine
        .spawn_object(SpawnConfig::new("FLOK"))
        .expect("fight target spawns");

    engine.object_action_fight(locked, target);

    let locked = engine
        .object_snapshot(locked)
        .expect("locked actor remains");
    assert_eq!(locked.action.name, "Dead");
    assert_eq!(locked.action.target, None);
}
