use crate::support::real_scenario::load_tutorial;
use clonk_engine::{Engine, JoinPlayerConfig, Landscape, Vector2};

fn load_tutorial07() -> (Engine, i32) {
    let mut engine = load_tutorial(7, 0);
    let owner = engine
        .join_player(JoinPlayerConfig {
            name: "Tutorial 7 virtual player".to_owned(),
            player_info_id: 0,
            score: 0,
            rounds: 0,
            rounds_won: 0,
            rounds_lost: 0,
            total_playing_time: 0,
            team: None,
            color_dw: 0xff_00_00,
            pref_color: 0,
            pref_position: 0,
            crew: Vec::new(),
            control_style: true,
            auto_context_menu: true,
            startup_player_count: 1,
        })
        .expect("local Tutorial07 virtual player joins")
        .number();
    (engine, owner)
}

#[test]
fn tutorial07_workshop_basement_keeps_cpp_pre_growth_creation_position() {
    // Numeric oracle: unmodified C++ Tutorial07 with LC_PIN_SEED=0, logged
    // immediately after C4Player::ScenarioInit. PlaceReadyBase calls
    // CreateObjectConstruction(FullCon,true) (C4Player.cpp:580-600), which
    // prepares terrain before NewObject (C4Game.cpp:1191-1238). WRKS is
    // created with construction bottom y=209; its included BAS7 Construction
    // callback creates the basement at object y+8 before initial DoCon
    // (Basement72.c4d/Script.c:72-78; C4Object.cpp:1428-1511). Initial growth
    // therefore lifts WRKS to y=184 and BAS7 to y=213. The probe recorded
    // GetX/GetY for both objects and Surface8 density at the two workshop
    // crossing columns below.
    let (engine, _) = load_tutorial07();
    let workshop = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "WRKS")
        .expect("Tutorial07 creates WRKS");
    let basement = engine
        .snapshot()
        .objects
        .into_iter()
        .find(|object| object.definition_id == "BAS7")
        .expect("WRKS Construction creates BAS7");

    assert_eq!(workshop.position, Vector2::new(150, 184));
    assert_eq!(basement.position, Vector2::new(150, 213));
    let grid = engine
        .landscape()
        .and_then(Landscape::pixel_grid)
        .expect("Tutorial07 has an exact Surface8 grid");
    let crossing_densities =
        [145, 129].map(|x| (x, grid.density_at(x, 208), grid.density_at(x, 209)));
    assert_eq!(
        crossing_densities,
        [(145, Some(0), Some(100)), (129, Some(0), Some(100))]
    );
}
