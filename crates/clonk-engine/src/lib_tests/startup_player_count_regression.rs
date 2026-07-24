use super::*;

#[test]
fn startup_player_count_state_preserves_exact_zero_and_legacy_seed() {
    let mut source = Engine::new();
    assert_eq!(source.freeze_startup_player_count(0), 0);
    assert_eq!(source.freeze_startup_player_count(7), 0);

    let state = source.capture_state();
    assert_eq!(state.startup_player_count, Some(0));

    let mut restored = Engine::new();
    restored.freeze_startup_player_count(9);
    restored.restore_state(&state).expect("state restores");
    assert_eq!(restored.startup_player_count(), Some(0));

    let mut legacy = serde_json::to_value(&state).expect("state serializes");
    legacy
        .as_object_mut()
        .expect("state is an object")
        .remove("startup_player_count");
    let legacy: EngineState = serde_json::from_value(legacy).expect("legacy state parses");
    let mut seeded = Engine::new();
    seeded.freeze_startup_player_count(3);
    seeded
        .restore_state(&legacy)
        .expect("legacy state restores");
    assert_eq!(seeded.startup_player_count(), Some(3));
}
