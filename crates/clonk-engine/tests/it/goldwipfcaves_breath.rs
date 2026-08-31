use crate::support::real_scenario::{join_local_player, prepare_installed_scenario};
use crate::support::EngineTestExt;

const SCENARIO: &str = "Collection.c4f/Settling.c4f/Goldwipfcaves.c4s";

#[test]
fn breathable_goldwipfcaves_objects_complete_the_fifth_tick() {
    let mut engine = prepare_installed_scenario(SCENARIO, 0).instantiate();
    join_local_player(&mut engine, "Goldwipfcaves breath refill");

    for frame in 1..=4 {
        engine
            .tick_without_snapshot()
            .unwrap_or_else(|error| panic!("Goldwipfcaves frame {frame} executes: {error}"));
    }

    let wipf = engine
        .first_object_for_definition("WIPF")
        .expect("Goldwipfcaves contains a saved Goldwipf");
    let wipf_index = engine.test_object_index(wipf);
    assert_eq!(
        engine.object_physical(wipf_index).breath,
        -2_009_260_032,
        "LP64 strtol narrowing preserves the packed Breath=50000000000000 input"
    );
    assert_eq!(engine.test_object_snapshot(wipf).breath, i32::MAX);

    // The pinned Linux C++ build computes takebreath=138223617, performs the
    // failsafe DeepBreath call, then wraps Breath += takebreath to the physical
    // value without aborting (C4Object.cpp:911-920).
    engine
        .tick_without_snapshot()
        .unwrap_or_else(|error| panic!("Goldwipfcaves frame 5 executes: {error}"));
    assert_eq!(engine.test_object_snapshot(wipf).breath, -2_009_260_032);
}
