//! `ObjectId` is an identity, not a label — even where two objects share one.
//!
//! clonk-org/clonk-rs#1497. Native enumeration numbers are unique by
//! construction: `C4GameObjects::PostLoad` denumerates every pointer through
//! `ObjectPointer`, so a repeated number cannot address two objects
//! (`src/C4GameObjects.cpp:534-560`). Rust rebuilds the world from a
//! `Vec<Object>` and a restored or network-derived payload can hand it two
//! entries carrying the same number. The engine already tolerates that rather
//! than failing closed — `object_ids_are_unique` switches the frame walk to
//! index identity so each duplicate still executes exactly once.
//!
//! Scenario-section teardown did not. It collected the departing ids into a
//! `HashSet<ObjectId>` and `retain`ed the object list on membership, so
//! deleting one duplicate deleted its retained inactive twin along with it —
//! the one object `C4Game::LoadScenarioSection` is required to keep
//! (`src/C4Game.cpp:4190-4201`).

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

fn section(name: &str, width: u32) -> scenario::ScenarioSectionSpec {
    scenario::ScenarioSectionSpec {
        name: name.to_string(),
        source_group: None,
        landscape: Some(section_landscape(width, 40)),
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

/// One retained inactive object and one departing active object share a
/// number. Only the active one may be deleted.
fn section_engine_with_duplicated_number() -> (Engine, ObjectId) {
    let mut engine = Engine::with_seed(5);
    crate::TestValueExt::test_value(engine.register_script_definition(
        "DUPE",
        "DUPE",
        "func Nop() { return 0; }",
    ));
    engine.configure_scenario_sections(&[section("Main", 80), section("Other", 80)]);
    engine.set_landscape(section_landscape(80, 40));

    let retained =
        crate::TestValueExt::test_value(engine.spawn_object(SpawnConfig::new("DUPE".to_string())));
    let index = engine
        .find_object_index(retained)
        .expect("the spawned object is live");
    engine.objects[index].state.status = ObjectStatus::Inactive;
    engine.insert_into_inactive_list(retained, false);

    // The restored twin: an ordinary active object carrying the retained
    // object's number, which both object lists address by that number alone.
    let mut twin = engine.objects[index].clone();
    twin.state.status = ObjectStatus::Normal;
    engine.objects.push(twin);
    engine.execution.exec_list.push(retained);
    engine.note_objects_changed();

    (engine, retained)
}

#[test]
fn section_teardown_keeps_a_retained_object_whose_number_a_departing_object_reuses() {
    let (mut engine, retained) = section_engine_with_duplicated_number();

    assert!(
        crate::TestValueExt::test_value(engine.load_scenario_section("Other", 0, Vec::new())),
        "the target section is registered"
    );

    assert_eq!(
        engine
            .objects
            .iter()
            .filter(|object| object.id == retained)
            .count(),
        1,
        "the retained inactive object survives and the departing twin does not"
    );
    assert_eq!(
        engine
            .objects
            .iter()
            .find(|object| object.id == retained)
            .map(|object| object.state.status),
        Some(ObjectStatus::Inactive),
        "the survivor is the inactive one"
    );
    assert!(
        engine.execution.inactive.contains(&retained),
        "the survivor keeps its inactive-list membership"
    );
}
