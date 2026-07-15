use lc_engine::{SpawnConfig, Vector2};
use lc_script::Value;

use crate::support::real_scenario::load_installed_scenario;

#[test]
fn gold_rush_do_change_section_loads_ash_city_landscape() {
    let mut engine = load_installed_scenario("Western.c4f/Goldrush.c4s", 0);
    // SaveObjects expects the campaign wagon created by InitializePlayer.
    // A playerless, empty wagon keeps this test focused on the section
    // boundary instead of recursively staging a complete campaign inventory.
    engine
        .spawn_object(
            SpawnConfig::new("COAC").with_position(Vector2::new(-10_000, -10_000)),
        )
        .expect("the section-transfer wagon spawns");
    // Make the final DoInitializeSection call take its shipped already-run
    // guard. Section-specific setup is independent of the loader boundary.
    engine
        .call_scenario_script_function("DoInitializeSection", Vec::new())
        .expect("the first section setup guard advances");
    engine
        .call_scenario_script_function("DoInitializeSection", Vec::new())
        .expect("the second section setup guard advances");

    assert_eq!(
        engine.landscape().expect("GoldRush main landscape").width(),
        4_350
    );
    engine
        .call_scenario_script_function(
            "ChangeSection",
            vec![Value::String("AshCity".to_string())],
        )
        .expect("the shipped ChangeSection callback runs");
    engine
        .call_scenario_script_function("DoChangeSection", Vec::new())
        .expect("the shipped DoChangeSection callback recognizes LoadScenarioSection");

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
