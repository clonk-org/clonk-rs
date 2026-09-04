use crate::support::real_scenario::{join_local_player_on_team, load_installed_scenario};
use clonk_engine::{Engine, ObjectSnapshot};
use clonk_script::Value;

fn objects(engine: &Engine, definition: &str) -> Vec<ObjectSnapshot> {
    engine
        .snapshot()
        .objects
        .into_iter()
        .filter(|object| object.definition_id == definition)
        .collect()
}

#[test]
fn gidl_race_preserves_player_crew_during_map_selection() {
    let mut engine = load_installed_scenario("Collection.c4f/Fun.c4f/GIDL_Race.c4s", 0);
    let player = join_local_player_on_team(&mut engine, "GIDL probe", 1);
    for _ in 0..20 {
        engine.tick_without_snapshot().expect("lobby tick");
    }
    engine
        .call_scenario_script_function("SelectMap2", vec![Value::C4Id("_LAT".to_string())])
        .expect("select Atoll");
    // LegacyClonk deactivates crew before the section teardown so
    // C4Game::LoadScenarioSection keeps it (GIDL_Race.c4s/System.c4g/LvlStart.c:21-24;
    // C4Game.cpp:4190-4201), then reactivates it after loading the map
    // (GIDL_Race.c4s/System.c4g/LvlStart.c:108-110).
    assert_eq!(engine.crew_members(player).len(), 1);
    assert_eq!(objects(&engine, "RGDL").len(), 1);
    assert_eq!(objects(&engine, "STRP").len(), 1);
    assert!(!engine.snapshot().game_over);
    assert_eq!(engine.debug_current_scenario_section(), "Atoll");
    for _ in 0..200 {
        engine.tick_without_snapshot().expect("race tick");
    }
    assert_eq!(engine.crew_members(player).len(), 1);
    assert_eq!(objects(&engine, "RGDL").len(), 1);
    assert_eq!(objects(&engine, "STRP").len(), 1);
    assert!(!engine.snapshot().game_over);
}
