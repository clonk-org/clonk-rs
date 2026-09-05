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
    engine.register_test_script_definition(
        "PEER",
        "Departing peer",
        "#strict 3\nfunc Go() { return LoadScenarioSection(\"Other\", 0); }\n",
    );
    engine.register_test_script_definition(
        "SWCH",
        "Switching caller",
        "#strict 3\n\
         func Plain() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Retain() { SetObjectStatus(C4OS_INACTIVE, this()); \
                         return LoadScenarioSection(\"Other\", 0); }\n\
         func Arm() { return AddEffect(\"Switch\", this(), 10, 1, this()); }\n\
         func FxSwitchTimer() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Delegate(object peer) { return peer->Go(); }\n",
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

/// The effect-event fold reaches the same switch through its own player-command
/// application, and holds the same stale `idx` across it.
///
/// `C4Effect`'s timer is an ordinary script frame, so it may call
/// `LoadScenarioSection` exactly as a definition callback can
/// (`C4Script.cpp:5401-5408`). Native runs the switch synchronously and the
/// effect's own object is simply gone afterwards; Rust read
/// `self.objects[idx]` for the post-callback container comparison after the
/// section had already rebuilt the list.
#[test]
fn a_section_switch_from_an_effect_timer_leaves_no_stale_caller_slot() {
    let mut engine = switching_engine();
    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    crate::TestValueExt::test_value(engine.call_object_function(index, "Arm", Vec::new()));

    let index = engine.test_object_index(caller);
    let switch = crate::TestValueExt::test_value(
        engine.objects[index]
            .state
            .effects
            .iter()
            .find(|effect| effect.name == "Switch")
            .cloned(),
    );
    let definition_id = engine.objects[index].definition_id.clone();
    crate::TestValueExt::test_value(engine.dispatch_object_effect_events(
        index,
        &definition_id,
        vec![EffectEvent::timer(switch)],
    ));

    assert!(
        engine.find_object_index(caller).is_none(),
        "the effect's own object departs with the section"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}

/// The ordinary frame walk reaches the switch too, holding the same slot.
///
/// An effect timer running inside `advance_tick` is where content actually
/// calls `LoadScenarioSection`. That fold applies the resulting player command
/// and then keeps using the index it captured beforehand.
#[test]
fn a_section_switch_from_a_ticked_effect_leaves_no_stale_caller_slot() {
    let mut engine = switching_engine();
    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    crate::TestValueExt::test_value(engine.call_object_function(index, "Arm", Vec::new()));

    crate::TestValueExt::test_value(engine.tick());

    assert!(
        engine.find_object_index(caller).is_none(),
        "the ticking caller departs with the section"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}

/// A switch requested by a *foreign* object reaches the caller's outcome too.
///
/// `peer->Go()` runs in the peer's context and comes back as a nested object
/// outcome, whose fold applies player commands and then reads that foreign
/// object's slot for the post-callback container comparison — after the object
/// departed with its section.
#[test]
fn a_section_switch_from_a_delegated_call_leaves_no_stale_foreign_slot() {
    let mut engine = switching_engine();
    let peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    crate::TestValueExt::test_value(engine.call_object_function(
        index,
        "Delegate",
        vec![object_reference_value(peer)],
    ));

    assert!(
        engine.find_object_index(peer).is_none(),
        "the delegated callee departs with the section"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}

/// A `TimerCall` definition callback reaches `advance_tick`'s other folds.
///
/// `C4Object::Execute` runs `Def->TimerCall` after the effect walk
/// (`C4Object.cpp:1085-1091`), so this is a second, independent way for
/// ordinary content to request the switch from inside the frame walk — and it
/// lands in a different fold than an effect timer does. A definition built
/// through `register_script_definition` cannot reach it: its `Timer` interval
/// defaults to 35 (`C4Def.cpp:298`) and it declares no `TimerCall`.
#[test]
fn a_section_switch_from_a_definition_timer_call_leaves_no_stale_caller_slot() {
    let mut engine = switching_engine();
    let mut definition = crate::TestValueExt::test_value(Definition::from_script(
        "TIMR",
        "Timer caller",
        "#strict 3\nfunc Tick() { return LoadScenarioSection(\"Other\", 0); }\n",
    ));
    definition.set_timer(1);
    definition.set_timer_call(Some("Tick".to_string()));
    crate::TestValueExt::test_value(engine.register_definition(definition));

    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "TIMR", with_position: Vector2::new(60, 50));

    crate::TestValueExt::test_value(engine.tick());

    assert!(
        engine.find_object_index(caller).is_none(),
        "the TimerCall caller departs with the section"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}

/// An action `PhaseCall` is the third frame-walk route to the switch.
///
/// `C4Object::ExecAction` runs the phase callback before the effect walk and
/// TimerCall (`C4Object.cpp:1069-1091`), so its outcome is folded in a
/// different place again.
#[test]
fn a_section_switch_from_an_action_phase_call_leaves_no_stale_caller_slot() {
    let mut engine = switching_engine();
    let probe = ActionSpec::default()
        .with_delay(1)
        .with_length(100)
        .with_phase_call("Switch");
    let mut definition = crate::TestValueExt::test_value(Definition::from_script(
        "ACTS",
        "Action caller",
        "#strict 3\n\
         func Switch() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Begin() { return SetAction(\"Probe\"); }\n",
    ));
    definition.set_c4_callback_convention(true);
    definition.configure_actions(None, HashMap::from([("Probe".to_string(), probe)]));
    crate::TestValueExt::test_value(engine.register_definition(definition));

    let _peer = spawn_fixture!(engine, "PEER", with_position: Vector2::new(30, 50));
    let caller = spawn_fixture!(engine, "ACTS", with_position: Vector2::new(60, 50));
    let index = engine.test_object_index(caller);
    crate::TestValueExt::test_value(engine.call_object_function(index, "Begin", Vec::new()));

    crate::TestValueExt::test_value(engine.tick());

    assert!(
        engine.find_object_index(caller).is_none(),
        "the action-callback caller departs with the section"
    );
    assert!(
        engine.objects.is_empty(),
        "an empty target section installs no objects at all"
    );
}
