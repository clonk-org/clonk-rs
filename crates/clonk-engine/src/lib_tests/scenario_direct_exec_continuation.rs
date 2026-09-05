//! Scenario and Game.ScriptEngine DirectExec keep their temporary VM frame
//! alive while `LoadScenarioSection` performs its synchronous replacement.

use super::*;

fn section_landscape(width: u32, height: u32) -> Landscape {
    let mut landscape =
        crate::TestValueExt::test_value(Landscape::new(width, vec![0; width as usize]));
    landscape.set_world_height(height as i32);
    landscape.set_pixel_grid(landscape::PixelGrid::new(
        width,
        height,
        vec![0; (width * height) as usize],
        vec![0, 100],
        vec![None, Some("Earth".into())],
        vec![None; 2],
    ));
    landscape
}

fn section(name: &str) -> scenario::ScenarioSectionSpec {
    scenario::ScenarioSectionSpec {
        name: name.to_string(),
        source_group: None,
        landscape: Some(section_landscape(80, 40)),
        landscape_systems: scenario::ScenarioLandscapeSystems::default(),
        exact_landscape: false,
        texmap_lookups: Vec::new(),
        resynthesize_static_map: false,
        map_creator: None,
        s2_overload: None,
        gravity: scenario::LegacyC4SVal::new(100, 0, 10, 200),
        post_init_map_callbacks: map_creator_s2::PostInitMapCallbacks::default(),
        keep_map_creator: false,
        no_initialize: false,
        objects: Vec::new(),
        scenario_values: scenario::ScenarioValueStore::default(),
        environment: EnvironmentSettings::default(),
        base_reject_entrance_enabled: true,
        base_extinguish_enabled: true,
    }
}

fn engine_with_sections() -> Engine {
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), section("Other")]);
    engine.set_landscape(section_landscape(80, 40));
    engine
        .register_definition(
            Definition::from_script("OBSV", "Departing observer", "#strict 3\n")
                .expect("observer definition compiles"),
        )
        .expect("observer definition registers");
    engine
        .spawn_object(SpawnConfig::new("OBSV").with_id(ObjectId::new(77)))
        .expect("observer starts active in the source section");
    engine
}

#[test]
fn scenario_direct_exec_resumes_after_section_switch() {
    let mut engine = engine_with_sections();
    engine
        .install_scenario_script("Scenario", "#strict 3\n")
        .expect("scenario host installs");

    // C4Script.cpp:5401-5408 returns only after C4Game.cpp:4190-4208
    // removes active objects. A deferred switch would leave Object(77)
    // visible to the remaining expression even if its final section is right.
    let value = engine
        .direct_exec_scenario_script(
            "LoadScenarioSection(\"Other\", 0) * 10 + !!this() + !!Object(77)",
            "scenario DirectExec",
            Some(3),
        )
        .expect("scenario DirectExec completes after the switch");

    assert_eq!(value, Value::Int(10));
    assert_eq!(engine.debug_current_scenario_section(), "Other");
}

#[test]
fn global_direct_exec_resumes_after_section_switch() {
    let mut engine = engine_with_sections();

    // C4Script.cpp:5401-5408 returns only after C4Game.cpp:4190-4208
    // removes active objects. A deferred switch would leave Object(77)
    // visible to the remaining expression even if its final section is right.
    let value = engine
        .direct_exec_script_control_global(
            "LoadScenarioSection(\"Other\", 0) * 10 + !!this() + !!Object(77)",
            "global DirectExec",
            Some(3),
        )
        .expect("global DirectExec completes after the switch");

    assert_eq!(value, Value::Int(10));
    assert_eq!(engine.debug_current_scenario_section(), "Other");
}

#[test]
fn global_direct_exec_preserves_failed_section_switch_result() {
    let mut engine = engine_with_sections();

    // C4Script.cpp:5401-5408 returns only after C4Game.cpp:4190-4208
    // removes active objects. A deferred switch would leave Object(77)
    // visible to the remaining expression even if its final section is right.
    let value = engine
        .direct_exec_script_control_global(
            "LoadScenarioSection(\"Missing\", 0) * 10 + !!this() + !!Object(77)",
            "global DirectExec",
            Some(3),
        )
        .expect("global DirectExec completes after a failed switch");

    assert_eq!(value, Value::Int(1));
    assert_eq!(engine.debug_current_scenario_section(), "Main");
}
