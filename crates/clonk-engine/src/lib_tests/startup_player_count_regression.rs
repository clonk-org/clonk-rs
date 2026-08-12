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
    crate::TestValueExt::test_value(restored.restore_state(&state));
    assert_eq!(restored.startup_player_count(), Some(0));

    let mut legacy = crate::TestValueExt::test_value(serde_json::to_value(&state));
    crate::TestValueExt::test_value(legacy.as_object_mut()).remove("startup_player_count");
    let legacy: EngineState = crate::TestValueExt::test_value(serde_json::from_value(legacy));
    let mut seeded = Engine::new();
    seeded.freeze_startup_player_count(3);
    crate::TestValueExt::test_value(seeded.restore_state(&legacy));
    assert_eq!(seeded.startup_player_count(), Some(3));
}
