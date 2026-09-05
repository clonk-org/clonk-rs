//! A section switch requested from an object callback rebuilds the object
//! list under that callback's own outcome.
//!
//! clonk-org/clonk-rs#1498. `C4Game::LoadScenarioSection` removes every active
//! object and installs the target section's own (C4Game.cpp:4194-4208), so by
//! the time the host call returns, the caller's slot in `Engine::objects` may
//! belong to a different object or not exist at all. The callback fold applies
//! player commands — the channel `LoadScenarioSection` travels on — and *then*
//! writes the caller's own update through the `index` it captured beforehand.
//!
//! Neither existing coverage reached this: the scenario-global `Switch()`
//! fixture calls the host function outside any object context, and the section
//! fixtures whose target spawns objects merely refill the slot.

use super::*;
use crate::lib_test_support::{spawn_fixture, EngineTestExt};

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
        landscape: Some(section_landscape(200, 100)),
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

fn switching_engine() -> Engine {
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), section("Other")]);
    engine.set_landscape(section_landscape(200, 100));
    engine.register_test_script_definition("PEER", "Departing peer", "#strict 3\n");
    engine.register_test_script_definition(
        "SWCH",
        "Switching caller",
        "#strict 3\n\
         func Plain() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Retain() { SetObjectStatus(C4OS_INACTIVE, this()); \
                         return LoadScenarioSection(\"Other\", 0); }\n",
    );
    engine
}

#[test]
fn a_section_switch_from_an_object_callback_leaves_no_stale_caller_slot() {
    let mut engine = switching_engine();
    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    let switched =
        crate::TestValueExt::test_value(engine.call_object_function(index, "Plain", Vec::new()));

    assert_eq!(
        switched,
        Value::Int(1),
        "FnLoadScenarioSection reports the accepted switch (C4Script.cpp:5401-5408)"
    );
    assert!(
        engine.find_object_index(caller).is_none(),
        "the caller is an ordinary active object and the switch removes it"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}

/// Deactivating before switching, in one callback, must retain the object.
///
/// `C4Object::StatusDeactivate` runs synchronously: by the time the script
/// reaches `LoadScenarioSection` the object is already out of `Game.Objects`
/// and in `InactiveObjects`, so the active-list teardown never touches it
/// (C4Object.cpp:5987-6009; C4Game.cpp:4194-4201). Rust records the caller's
/// own status update in the callback outcome and applies it *after* the player
/// command carrying the switch, so the teardown still saw an active object and
/// deleted the one the script asked to keep. `preserve_ids` — collected from
/// the host context's preview, where the deactivation is already visible — is
/// what carries that decision across the deferral.
#[test]
fn a_section_switch_retains_an_object_the_same_callback_deactivated() {
    let mut engine = switching_engine();
    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    let switched =
        crate::TestValueExt::test_value(engine.call_object_function(index, "Retain", Vec::new()));

    assert_eq!(switched, Value::Int(1), "the switch is accepted");
    let index = engine
        .find_object_index(caller)
        .expect("the deactivated caller survives its own section switch");
    assert_eq!(
        engine.objects[index].state.status,
        ObjectStatus::Inactive,
        "it crosses as an inactive object"
    );
    assert!(
        engine.execution.inactive.contains(&caller),
        "and holds inactive-list membership on the other side"
    );
    assert_eq!(
        engine.objects.len(),
        1,
        "the ordinary active peer is still removed"
    );
}
