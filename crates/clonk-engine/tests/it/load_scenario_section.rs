use clonk_engine::{
    LcgRng, LegacyCString, ScriptControlData, ScriptControlPolicy, ScriptStrictness, SpawnConfig,
    Vector2, SCRIPT_SCOPE_GLOBAL,
};
use clonk_script::Value;

use crate::support::real_scenario::{load_installed_scenario, PreparedInstalledScenario};

pub(super) fn gold_rush_do_change_section_loads_ash_city_landscape(
    prepared: &PreparedInstalledScenario,
) {
    let mut engine = prepared.instantiate();
    // SaveObjects expects the campaign wagon created by InitializePlayer.
    // A playerless, empty wagon keeps this test focused on the section
    // boundary instead of recursively staging a complete campaign inventory.
    crate::support::TestValueExt::test_value(
        engine.spawn_object(SpawnConfig::new("COAC").with_position(Vector2::new(-10_000, -10_000))),
    );
    // Make the final DoInitializeSection call take its shipped already-run
    // guard. Section-specific setup is independent of the loader boundary.
    crate::support::TestValueExt::test_value(
        engine.call_scenario_script_function("DoInitializeSection", Vec::new()),
    );
    crate::support::TestValueExt::test_value(
        engine.call_scenario_script_function("DoInitializeSection", Vec::new()),
    );

    assert_eq!(
        engine.landscape().expect("GoldRush main landscape").width(),
        4_350
    );
    crate::support::TestValueExt::test_value(engine.call_scenario_script_function(
        "ChangeSection",
        vec![Value::String("AshCity".to_string().into())],
    ));
    crate::support::TestValueExt::test_value(
        engine.call_scenario_script_function("DoChangeSection", Vec::new()),
    );

    assert_eq!(
        engine
            .landscape()
            .expect("Ash City section landscape")
            .width(),
        3_000,
        "DoChangeSection replaces the main landscape with SectAshCity.c4g"
    );
    assert_eq!(engine.debug_current_scenario_section(), "AshCity");
    assert_eq!(engine.debug_last_scenario_section_flags(), Some(3));
}

fn replay_script(source: &str) -> ScriptControlData {
    ScriptControlData {
        target_object: SCRIPT_SCOPE_GLOBAL,
        strictness: ScriptStrictness::Strict3,
        script: crate::support::TestValueExt::test_value(LegacyCString::from_bytes(
            source.as_bytes().to_vec(),
        )),
        by_client: 0,
    }
}

fn run_replayed_section_switch(prelude: &str) -> (LcgRng, Vec<(i32, i32)>) {
    let mut engine = load_installed_scenario("Western.c4f/Goldrush.c4s", 23);
    for _ in 0..2 {
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    crate::support::TestValueExt::test_value(crate::support::TestValueExt::test_value(
        engine.execute_script_control(&replay_script(prelude), ScriptControlPolicy::replay(false)),
    ));
    crate::support::TestValueExt::test_value(crate::support::TestValueExt::test_value(
        engine.execute_script_control(
            &replay_script("LoadScenarioSection(\"AshCity\", 0)"),
            ScriptControlPolicy::replay(false),
        ),
    ));
    assert_eq!(engine.debug_current_scenario_section(), "AshCity");
    assert_eq!(
        engine
            .landscape()
            .expect("Ash City section landscape")
            .width(),
        3_000
    );

    let post_load_rng = engine.debug_rng_clone();
    let mut sync_ledgers = Vec::new();
    for _ in 0..4 {
        let check = engine.sync_check(0);
        sync_ledgers.push((check.random_count, check.random3));
        crate::support::TestValueExt::test_value(engine.tick_without_snapshot());
    }
    (post_load_rng, sync_ledgers)
}

#[test]
fn replayed_section_load_realigns_random_count_and_random3_across_runs() {
    let (first_rng, first_checks) = run_replayed_section_switch("Random(17)");
    let (second_rng, second_checks) =
        run_replayed_section_switch("Random(17) + Random(19) + Random(23)");

    let mut expected = LcgRng::seed_from_u64(23);
    let _ = expected.random(1);
    expected.trace_index = first_rng.trace_index;
    assert_eq!(first_rng, expected);
    assert_eq!(second_rng, expected);
    assert_eq!(first_checks[0], (501, 0));
    assert_eq!(first_checks, second_checks);
}
