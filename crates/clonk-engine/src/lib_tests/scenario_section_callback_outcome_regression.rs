//! A section switch requested from an object callback rebuilds the object
//! list under that callback's own outcome.
//!
//! clonk-org/clonk-rs#1495 and clonk-org/clonk-rs#1496. `C4Game::LoadScenarioSection` removes every active
//! object and installs the target section's own (C4Game.cpp:4194-4208), so by
//! the time the host call returns, the caller's slot in `Engine::objects` may
//! belong to a different object or not exist at all. The continuation owns the
//! VM frame while the engine performs the switch, then validates the caller's
//! allocation token before folding its suffix.
//!
//! These fixtures cover ordinary callbacks, action/effect paths, global
//! effects, pending creation phases, inactive retention and same-number
//! replacement.

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
    switching_engine_with_other(section("Other"))
}

fn switching_engine_with_other(other: scenario::ScenarioSectionSpec) -> Engine {
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), other]);
    engine.set_landscape(section_landscape(200, 100));
    engine.register_test_script_definition(
        "PEER",
        "Departing peer",
        "#strict 3\n\
         func Go() { return LoadScenarioSection(\"Other\", 0); }\n\
         func FxForeignDamage(object target, int number, int change, int cause, int caused_by) {\
             RecordDamageTrace(1);\
             var switched = LoadScenarioSection(\"Other\", 0);\
             RecordDamageTrace(switched);\
             SetDamage(77);\
             RecordDamageTrace(2);\
             return change + 5;\
         }\n",
    );
    engine.register_test_script_definition(
        "SWCH",
        "Switching caller",
        "#strict 3\n\
         func Plain() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Missing() { var switched = LoadScenarioSection(\"Missing\", 0); \
                          return switched; }\n\
         func Retain() { SetObjectStatus(C4OS_INACTIVE, this()); \
                         var switched = LoadScenarioSection(\"Other\", 0); \
                         return switched * 10 + !!this(); }\n\
         func CreateRetain() { var kept = CreateObject(CHLD, 0, 0, -1); \
                               var aliases = [kept]; \
                               SetObjectStatus(C4OS_INACTIVE, kept); \
                               var dropped = CreateObject(CHLD, 1, 0, -1); \
                               var switched = LoadScenarioSection(\"Other\", 0); \
                               return switched * 100 + !!kept * 10 + !!aliases[0] + !!dropped; }\n\
         func FutureReference() { var future = Object(99); \
                                  var switched = LoadScenarioSection(\"Other\", 0); \
                                  return switched * 10 + !!future; }\n\
         func Arm() { return AddEffect(\"Switch\", this(), 10, 1, this()); }\n\
         func FxSwitchTimer() { return LoadScenarioSection(\"Other\", 0); }\n\
         func Delegate(object peer) { return peer->Go(); }\n",
    );
    engine.register_test_script_definition("FUTR", "Future object", "#strict 3\n");
    engine
}

fn install_damage_continuation_fixture(mut engine: Engine) -> Engine {
    let trace = "#strict 3\n\
                 static trace;\n\
                 global func ResetDamageTrace() { trace = 0; return true; }\n\
                 global func RecordDamageTrace(int value) { trace = trace * 10 + value; return true; }\n\
                 global func ReadDamageTrace() { return trace; }\n";
    assert_eq!(
        engine.install_global_scripts(&[(
            "System.c4g/DamageContinuation.c".to_string(),
            trace.to_string(),
        )]),
        1,
    );
    // Register observer globals before compiling the effect definition. The
    // native callback table is already linked during startup, so an
    // unresolved RecordDamageTrace is a fixture setup error rather than a
    // continuation outcome (C4Aul.cpp:130-148).
    let mut damage_definition = crate::TestValueExt::test_value(Definition::from_script(
        "DMGS",
        "Damage continuation caller",
        "#strict 3\n\
         func Arm() { return AddEffect(\"Switch\", this(), 10, 1, this()); }\n\
         func ArmForeign(object target) { return AddEffect(\"Foreign\", this(), 10, 1, target); }\n\
         func FxSwitchDamage(object target, int number, int change, int cause, int caused_by) {\
             RecordDamageTrace(1);\
             var switched = LoadScenarioSection(\"Other\", 0);\
             RecordDamageTrace(switched);\
             RecordDamageTrace(2);\
             return change + 5;\
         }\n",
    ));
    // This fixture exercises C4Effect's native callback argument contract:
    // the first argument is a C4Object, not the command-DSL state proplist.
    // Keep strict-3 conversion errors from masking the continuation boundary
    // under test (C4Effect.cpp:345-363; C4AulExec.cpp:1610-1627).
    damage_definition.set_c4_callback_convention(true);
    crate::TestValueExt::test_value(engine.register_definition(damage_definition));
    crate::TestValueExt::test_value(engine.call_engine_global_function("ResetDamageTrace", &[]));
    engine
}

fn damage_switching_engine() -> Engine {
    // Keep this fixture's destination absent. The callback must observe the
    // native false result from a missing section, rather than accidentally
    // exercising the successful switch path supplied by `switching_engine`.
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main")]);
    engine.set_landscape(section_landscape(200, 100));
    install_damage_continuation_fixture(engine)
}

fn damage_replacement_switching_engine() -> Engine {
    install_damage_continuation_fixture(replacement_switching_engine())
}

fn foreign_damage_replacement_switching_engine() -> Engine {
    let mut engine = install_damage_continuation_fixture(replacement_switching_engine());
    // The effect command target supplies FxForeignDamage's script owner. Keep
    // this definition in the replacement fixture so the callback is resolved
    // before the target is removed (C4Effect.cpp:128-129,345).
    let mut foreign_definition = crate::TestValueExt::test_value(Definition::from_script(
        "PEER",
        "Foreign damage target",
        "#strict 3\n\
         func FxForeignDamage(object target, int number, int change, int cause, int caused_by) {\
             RecordDamageTrace(1);\
             var switched = LoadScenarioSection(\"Other\", 0);\
             RecordDamageTrace(switched);\
             DoDamage(77);\
             RecordDamageTrace(2);\
             return change + 5;\
         }\n",
    ));
    foreign_definition.set_c4_callback_convention(true);
    crate::TestValueExt::test_value(engine.register_definition(foreign_definition));
    engine
}

fn replacement_switching_engine() -> Engine {
    let mut other = section("Other");
    other.objects.push(scenario::ScenarioSpawn {
        handle: Some("42".to_string()),
        container_handle: None,
        contents_handles: Vec::new(),
        info_name: None,
        config: SpawnConfig::new("REPL")
            .with_id(ObjectId::new(42))
            .with_loaded(true),
    });
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), other]);
    engine.set_landscape(section_landscape(200, 100));
    engine.register_test_script_definition(
        "SWCH",
        "Switching caller",
        "#strict 3\n\
         local Marker;\n\
         func Replace() { var old = this(); var aliases = [old]; Marker = 77; \
                         var switched = LoadScenarioSection(\"Other\", 0); \
                         var replacement = Object(42); \
                         return !old && !aliases[0] && !replacement->ReadMarker(); }\n",
    );
    engine.register_test_script_definition(
        "REPL",
        "Replacement",
        "#strict 3\nlocal Marker; func ReadMarker() { return Marker; }\n",
    );
    engine
}

fn creation_switching_engine() -> Engine {
    let mut engine = switching_engine();
    engine.register_test_script_definition(
        "CHLD",
        "Created child",
        "#strict 3\n\
         local LifecycleOrder; local InitializeCount; local SelfReference; local InitializeStatus;\n\
         func Construction() { LifecycleOrder = 1; }\n\
         func Initialize() { LifecycleOrder = LifecycleOrder * 10 + 2; \
                              InitializeCount++; SelfReference = this(); \
                              InitializeStatus = GetObjectStatus(this()); \
                              CreateObject(NEST, 0, 0, -1); return true; }\n\
         func ReadLifecycleOrder() { return LifecycleOrder; }\n\
         func ReadInitializeCount() { return InitializeCount; }\n\
         func ReadSelfReference() { return !!SelfReference; }\n\
         func ReadInitializeStatus() { return InitializeStatus; }\n",
    );
    engine.register_test_script_definition("NEST", "Nested child", "#strict 3\n");
    engine
}

fn creation_replacement_switching_engine() -> Engine {
    let mut other = section("Other");
    other.objects.push(scenario::ScenarioSpawn {
        handle: Some("replacement".to_string()),
        container_handle: None,
        contents_handles: Vec::new(),
        info_name: None,
        config: SpawnConfig::new("REPL")
            .with_id(ObjectId::new(77))
            .with_loaded(true),
    });
    let mut engine = switching_engine_with_other(other);
    engine.register_test_script_definition(
        "CHLD",
        "Created child",
        "#strict 3\nfunc Initialize() { CreateObject(NEST, 0, 0, -1); return true; }\n",
    );
    engine.register_test_script_definition("NEST", "Nested child", "#strict 3\n");
    engine.register_test_script_definition("REPL", "Replacement", "#strict 3\n");
    engine
}

fn initial_lifecycle_creation_switching_engine(initialize: bool) -> Engine {
    let mut engine = switching_engine();
    engine.register_test_script_definition(
        "CHLD",
        "Created child",
        "#strict 3\n\
         local LifecycleOrder; local InitializeCount; local SelfReference; local InitializeStatus;\n\
         func Construction() { LifecycleOrder = 1; }\n\
         func Initialize() { LifecycleOrder = LifecycleOrder * 10 + 2; InitializeCount++; \
                              SelfReference = this(); InitializeStatus = GetObjectStatus(this()); }\n\
         func ReadLifecycleOrder() { return LifecycleOrder; }\n\
         func ReadInitializeCount() { return InitializeCount; }\n\
         func ReadSelfReference() { return !!SelfReference; }\n\
         func ReadInitializeStatus() { return InitializeStatus; }\n",
    );
    let lifecycle = if initialize {
        "func Construction() { return true; }\n\
         func Initialize() { var kept = CreateObject(CHLD, 0, 0, -1); \
                              SetObjectStatus(C4OS_INACTIVE, kept); \
                              var active = CreateObject(CHLD, 1, 0, -1); \
                              return LoadScenarioSection(\"Other\", 0); }\n"
    } else {
        "func Construction() { var kept = CreateObject(CHLD, 0, 0, -1); \
                               SetObjectStatus(C4OS_INACTIVE, kept); \
                               var active = CreateObject(CHLD, 1, 0, -1); \
                               return LoadScenarioSection(\"Other\", 0); }\n"
    };
    engine.register_test_script_definition("INITC", "Initial lifecycle switcher", lifecycle);
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

/// Menu/control DirectExec is also a temporary C4Aul frame. Its expression
/// suffix must run after the synchronous section load, with `this` cleared
/// when the active caller departed (C4Object.cpp:3756-3760;
/// C4AulExec.cpp:1658-1706).
#[test]
fn direct_exec_expression_resumes_after_section_switch() {
    let mut engine = switching_engine();
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    let result = engine.direct_exec_on_object(
        index,
        "LoadScenarioSection(\"Other\", 0) * 10 + !!this()",
        "MenuCommand",
    );

    assert_eq!(
        crate::TestValueExt::test_value(result),
        Value::Int(10),
        "DirectExec resumes its suffix with the native false this value after removal",
    );
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    assert!(engine.find_object_index(caller).is_none());
}

#[test]
fn a_failed_section_switch_returns_false_and_keeps_the_callback_live() {
    let mut engine = switching_engine();
    let caller = spawn_fixture!(engine, "SWCH", with_position: Vector2::new(60, 50));

    let index = engine.test_object_index(caller);
    let result =
        crate::TestValueExt::test_value(engine.call_object_function(index, "Missing", Vec::new()));

    assert_eq!(
        result,
        Value::Int(0),
        "a missing section is a native false result"
    );
    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert!(
        engine.find_object_index(caller).is_some(),
        "a failed load must not retire the requesting object"
    );
}

#[test]
fn a_section_switch_clears_old_this_before_same_id_replacement_resumes() {
    let mut engine = replacement_switching_engine();
    let caller = spawn_fixture!(engine, "SWCH", with_id: ObjectId::new(42));

    let index = engine.test_object_index(caller);
    let result =
        crate::TestValueExt::test_value(engine.call_object_function(index, "Replace", Vec::new()));

    assert_eq!(
        result,
        Value::Bool(true),
        "the suspended suffix sees old this as nil"
    );
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let replacement_index = engine
        .find_object_index(ObjectId::new(42))
        .expect("the destination object reuses the caller number");
    assert_eq!(engine.objects[replacement_index].definition_id, "REPL");
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

    assert_eq!(
        switched,
        Value::Int(11),
        "the switch is accepted and the retained caller resumes with this",
    );
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

/// A Destruction callback may deactivate an object that appears later in the
/// active-list walk. That object survives the section switch, so a suspended
/// caller's local/reference cell must retain it until the suffix resumes.
/// Clearing every active id before teardown would incorrectly turn \`future\`
/// into nil (C4Object.cpp:287-306; C4Game.cpp:4190-4201).
#[test]
fn destruction_deactivation_preserves_suspended_reference_to_future_object() {
    let mut engine = switching_engine();
    engine.register_test_script_definition(
        "KILL",
        "Destruction deactivator",
        "#strict 3\n\
         func Destruction() { SetObjectStatus(C4OS_INACTIVE, Object(99)); }\n\
         func FutureReference() { var future = Object(99); \
                                  var switched = LoadScenarioSection(\"Other\", 0); \
                                  return switched * 10 + !!future; }\n",
    );
    // The active-list walk starts at Objects.First, represented by the
    // reverse of exec_list. Spawn the future object first so KILL's
    // Destruction runs before FUTR and deactivates that future link; native
    // then skips it rather than invoking its own AssignRemoval.
    let future = spawn_fixture!(
        engine,
        "FUTR",
        with_id: ObjectId::new(99),
        with_position: Vector2::new(70, 50)
    );
    let caller = spawn_fixture!(
        engine,
        "KILL",
        with_id: ObjectId::new(41),
        with_position: Vector2::new(60, 50)
    );
    let caller_walk_position = engine
        .execution
        .exec_list
        .iter()
        .rev()
        .position(|id| *id == caller)
        .expect("the destruction caller is in the active walk");
    let future_walk_position = engine
        .execution
        .exec_list
        .iter()
        .rev()
        .position(|id| *id == future)
        .expect("the future object is in the active walk");
    assert!(
        caller_walk_position < future_walk_position,
        "the deactivator must run before the future object in Objects.First order"
    );

    let index = engine.test_object_index(caller);
    let result = engine.call_test_object_function(index, "FutureReference", Vec::new());

    assert_eq!(result, Value::Int(11));
    let future_index = engine
        .find_object_index(future)
        .expect("Destruction deactivated the future object instead of removing it");
    assert_eq!(
        engine.objects[future_index].state.status,
        ObjectStatus::Inactive
    );
    assert!(
        engine.execution.inactive.contains(&future),
        "the deactivated future object remains in InactiveObjects"
    );
}

/// Objects created by the suspended callback are already linked and fully
/// initialized before the callback reaches LoadScenarioSection. An inactive
/// child keeps its allocation and every nested creation made by Initialize is
/// ordered before the caller's switch; an active sibling departs with the old
/// section (C4Game.cpp:1085-1142,4190-4208).
#[test]
fn a_section_switch_keeps_only_the_same_callback_inactive_creation() {
    let mut engine = creation_switching_engine();
    let caller = spawn_fixture!(
        engine,
        "SWCH",
        with_id: ObjectId::new(77),
        with_position: Vector2::new(60, 50)
    );

    let index = engine.test_object_index(caller);
    let result = crate::TestValueExt::test_value(engine.call_object_function(
        index,
        "CreateRetain",
        Vec::new(),
    ));

    assert_eq!(
        result,
        Value::Int(111),
        "the inactive creation and its alias survive while the active sibling is cleared",
    );
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let children = engine
        .objects
        .iter()
        .filter(|object| object.definition_id == "CHLD")
        .collect::<Vec<_>>();
    assert_eq!(
        children.len(),
        1,
        "the retained child is materialized exactly once"
    );
    assert_eq!(children[0].state.status, ObjectStatus::Inactive);
    let child_index = engine
        .find_object_index(children[0].id)
        .expect("retained child remains callable after section switch");
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadLifecycleOrder", Vec::new()),
        Value::Int(12),
        "the retained child completed Construction then Initialize before deactivation",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeCount", Vec::new()),
        Value::Int(1),
        "the retained child Initialize phase is not replayed",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadSelfReference", Vec::new()),
        Value::Bool(true),
        "the retained child keeps its own object reference",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeStatus", Vec::new()),
        Value::Int(1),
        "Initialize observed the child in its normal status",
    );
    assert!(
        engine
            .objects
            .iter()
            .all(|object| object.definition_id != "NEST"),
        "nested Initialize creations from the departing section are removed"
    );
}

/// A destination object may reuse the departing callback's explicit number.
/// The inactive object created earlier in the same callback still crosses the
/// boundary once, while the active sibling and the old caller are removed.
/// The destination replacement must remain authoritative, and nested
/// `Initialize` creation must not replay after the switch
/// (C4Game.cpp:1085-1142, 4190-4208).
#[test]
fn a_section_switch_keeps_creation_once_when_destination_reuses_caller_id() {
    let mut engine = creation_replacement_switching_engine();
    let caller = spawn_fixture!(
        engine,
        "SWCH",
        with_id: ObjectId::new(77),
        with_position: Vector2::new(60, 50)
    );

    let index = engine.test_object_index(caller);
    let result = crate::TestValueExt::test_value(engine.call_object_function(
        index,
        "CreateRetain",
        Vec::new(),
    ));

    assert_eq!(
        result,
        Value::Int(111),
        "the suspended frame retains only its already-inactive creation",
    );
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let replacement = engine
        .find_object_index(ObjectId::new(77))
        .expect("the destination explicitly reuses the departing caller ID");
    assert_eq!(engine.objects[replacement].definition_id, "REPL");
    assert_eq!(
        engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "CHLD")
            .count(),
        1,
        "the inactive creation is materialized exactly once",
    );
    assert_eq!(
        engine
            .objects
            .iter()
            .filter(|object| object.definition_id == "NEST")
            .count(),
        0,
        "the departing active sibling and nested creation do not reappear",
    );
}

/// Engine-owned creation uses the same callback driver as a script-created
/// object. If Construction switches sections, the destination's replacement
/// must not be mistaken for the object whose initial DoCon/Initialize phase
/// is still pending (C4Game.cpp:1100-1142, 4190-4208).
#[test]
fn an_initial_lifecycle_switch_does_not_run_post_switch_phases_on_replacement() {
    let mut other = section("Other");
    other.objects.push(scenario::ScenarioSpawn {
        handle: Some("replacement".to_string()),
        container_handle: None,
        contents_handles: Vec::new(),
        info_name: None,
        config: SpawnConfig::new("REPL")
            .with_id(ObjectId::new(77))
            .with_loaded(true),
    });
    let mut engine = switching_engine_with_other(other);
    engine.register_test_script_definition(
        "INITL",
        "Initial lifecycle switcher",
        "#strict 3\nfunc Construction() { return LoadScenarioSection(\"Other\", 0); }\n",
    );
    engine.register_test_script_definition("REPL", "Replacement", "#strict 3\n");

    let result = engine.spawn_object_with_initial_lifecycle(
        SpawnConfig::new("INITL")
            .with_id(ObjectId::new(77))
            .with_construction(FULL_CON),
        None,
    );

    assert!(matches!(result, Ok(None)));
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let replacement = engine
        .find_object_index(ObjectId::new(77))
        .expect("the destination replacement remains materialized");
    assert_eq!(engine.objects[replacement].definition_id, "REPL");
    assert_eq!(engine.objects.len(), 1);
}

#[test]
fn construction_switch_materializes_inactive_and_active_children_in_order() {
    let mut engine = initial_lifecycle_creation_switching_engine(false);
    let result = engine.spawn_object_with_initial_lifecycle(
        SpawnConfig::new("INITC").with_construction(FULL_CON),
        None,
    );

    assert!(matches!(result, Ok(None)));
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let children = engine
        .objects
        .iter()
        .filter(|object| object.definition_id == "CHLD")
        .collect::<Vec<_>>();
    assert_eq!(
        children.len(),
        1,
        "only the deactivated child crosses the switch"
    );
    assert_eq!(children[0].state.status, ObjectStatus::Inactive);
    let child_index = engine
        .find_object_index(children[0].id)
        .expect("retained child remains callable after section switch");
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadLifecycleOrder", Vec::new()),
        Value::Int(12),
        "Construction runs before Initialize exactly once on the retained child",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeCount", Vec::new()),
        Value::Int(1),
        "Initialize is not replayed after the section switch",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadSelfReference", Vec::new()),
        Value::Bool(true),
        "the retained child keeps its own object reference",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeStatus", Vec::new()),
        Value::Int(1),
        "Initialize observed the child in its normal status",
    );
}

#[test]
fn initialize_switch_materializes_inactive_and_active_children_in_order() {
    let mut engine = initial_lifecycle_creation_switching_engine(true);
    let result = engine.spawn_object_with_initial_lifecycle(
        SpawnConfig::new("INITC").with_construction(FULL_CON),
        None,
    );

    assert!(matches!(result, Ok(None)));
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    let children = engine
        .objects
        .iter()
        .filter(|object| object.definition_id == "CHLD")
        .collect::<Vec<_>>();
    assert_eq!(
        children.len(),
        1,
        "only the deactivated child crosses the switch"
    );
    assert_eq!(children[0].state.status, ObjectStatus::Inactive);
    let child_index = engine
        .find_object_index(children[0].id)
        .expect("retained child remains callable after section switch");
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadLifecycleOrder", Vec::new()),
        Value::Int(12),
        "Construction runs before Initialize exactly once on the retained child",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeCount", Vec::new()),
        Value::Int(1),
        "Initialize is not replayed after the section switch",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadSelfReference", Vec::new()),
        Value::Bool(true),
        "the retained child keeps its own object reference",
    );
    assert_eq!(
        engine.call_test_object_function(child_index, "ReadInitializeStatus", Vec::new()),
        Value::Int(1),
        "Initialize observed the child in its normal status",
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

/// `C4Effect::DoDamage` resumes the same callback after the synchronous host
/// call and replaces the running change with its returned integer
/// (`C4Effect.cpp:427-437`, `C4Script.cpp:5401-5408`). A failed section load
/// must resume with native false, preserve the carrier, and apply the returned
/// damage through the still-live object.
#[test]
fn failed_section_switch_from_fx_damage_returns_false_and_keeps_damage_carrier() {
    let mut engine = damage_switching_engine();
    let caller = spawn_fixture!(engine, "DMGS", with_position: Vector2::new(60, 50));
    let index = engine.test_object_index(caller);
    let arm =
        crate::TestValueExt::test_value(engine.call_object_function(index, "Arm", Vec::new()));
    assert!(
        matches!(arm, Value::Int(value) if value > 0),
        "Arm returns an effect handle"
    );
    assert_eq!(
        engine.objects[index].state.effects.len(),
        1,
        "Arm installs the effect"
    );
    assert_eq!(engine.objects[index].state.effects[0].priority, 10);
    assert_eq!(
        engine.objects[index].state.effects[0].command_target,
        Some(caller.as_u64() as i32),
        "the effect keeps its command target callback owner",
    );
    let index = engine.test_object_index(caller);
    engine.objects[index].state.alive = false;

    // DoDamage still invokes the head effect for an initial zero change
    // (C4Effect.cpp:427-437); the continuation must resume that frame before
    // the zero-result stop condition is checked.
    crate::TestValueExt::test_value(engine.change_object_damage(index, 0, 0, OWNER_NONE));

    assert_eq!(engine.debug_current_scenario_section(), "Main");
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_engine_global_function("ReadDamageTrace", &[])),
        Value::Int(102),
        "the resumed callback sees LoadScenarioSection's native false and runs its suffix",
    );
    let index = engine.test_object_index(caller);
    assert_eq!(engine.objects[index].state.damage, 5);
}

/// A successful section switch can remove the effect carrier before the
/// resumed `Fx*Damage` frame returns. The suffix still runs and its result is
/// observable, while `DoDamage` must not apply that result through the stale
/// caller index or to a destination object reusing the numeric ID
/// (`C4Object.cpp:1330-1343`).
#[test]
fn successful_section_switch_from_fx_damage_drops_removed_carrier_after_suffix() {
    let mut engine = damage_replacement_switching_engine();
    let caller = spawn_fixture!(
        engine,
        "DMGS",
        with_id: ObjectId::new(42),
        with_position: Vector2::new(60, 50)
    );
    let index = engine.test_object_index(caller);
    let arm = engine.call_test_object_function(index, "Arm", Vec::new());
    assert!(
        matches!(arm, Value::Int(value) if value > 0),
        "Arm returns an effect handle"
    );
    assert_eq!(
        engine.objects[index].state.effects.len(),
        1,
        "Arm installs the effect"
    );
    assert_eq!(engine.objects[index].state.effects[0].priority, 10);
    let index = engine.test_object_index(caller);
    engine.objects[index].state.alive = false;

    crate::TestValueExt::test_value(engine.change_object_damage(index, 10, 0, OWNER_NONE));

    assert_eq!(engine.debug_current_scenario_section(), "Other");
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_engine_global_function("ReadDamageTrace", &[])),
        Value::Int(112),
        // C4Game::InitGame only links the script engine in InitGameFirstPart
        // (C4Game.cpp:2390-2418,2592-2622). A section load passes a non-null
        // section to InitGame (C4Game.cpp:4190-4223), so the existing global
        // trace remains 1 and the resumed callback appends 1 then 2.
        "the resumed callback suffix runs after the accepted switch",
    );
    let replacement_index = engine
        .find_object_index(caller)
        .expect("the target section reuses the caller's numeric ID");
    assert_eq!(engine.objects[replacement_index].definition_id, "REPL");
    assert_eq!(engine.objects.len(), 1);
}

/// A damage effect may execute in a command-target definition different from
/// the carrier. If that target is removed and its number is reused by the
/// destination section, the resumed frame's `this` must stay null rather than
/// retargeting the replacement (`C4Effect.cpp:201-212`; `C4AulExec.cpp:1638-1648`).
#[test]
fn fx_damage_does_not_retarget_a_replaced_foreign_command_target() {
    let mut engine = foreign_damage_replacement_switching_engine();
    let carrier = spawn_fixture!(
        engine,
        "DMGS",
        with_id: ObjectId::new(41),
        with_position: Vector2::new(60, 50)
    );
    let command_target = spawn_fixture!(
        engine,
        "PEER",
        with_id: ObjectId::new(42),
        with_position: Vector2::new(70, 50)
    );
    let carrier_index = engine.test_object_index(carrier);
    let arm = crate::TestValueExt::test_value(engine.call_object_function(
        carrier_index,
        "ArmForeign",
        vec![object_reference_value(command_target)],
    ));
    assert!(
        matches!(arm, Value::Int(value) if value > 0),
        "ArmForeign returns an effect handle"
    );
    assert_eq!(
        engine.objects[carrier_index].state.effects.len(),
        1,
        "ArmForeign installs the effect"
    );
    assert_eq!(engine.objects[carrier_index].state.effects[0].priority, 10);
    assert_eq!(
        engine.objects[carrier_index].state.effects[0].command_target,
        Some(command_target.as_u64() as i32),
        "the effect dispatches through the foreign target",
    );
    let carrier_index = engine.test_object_index(carrier);
    engine.objects[carrier_index].state.alive = false;

    crate::TestValueExt::test_value(engine.change_object_damage(carrier_index, 10, 0, OWNER_NONE));

    assert_eq!(engine.debug_current_scenario_section(), "Other");
    assert_eq!(
        crate::TestValueExt::test_value(engine.call_engine_global_function("ReadDamageTrace", &[])),
        Value::Int(112),
        "the foreign callback resumes and executes its suffix",
    );
    let replacement_index = engine
        .find_object_index(command_target)
        .expect("the destination object reuses the command-target number");
    assert_eq!(engine.objects[replacement_index].definition_id, "REPL");
    assert_eq!(
        engine.objects[replacement_index].state.damage, 0,
        "a recycled numeric ID must not receive the old callback's this write",
    );
}

/// A global effect callback is also a synchronous C4Aul frame. Its affected
/// object is nil, but a suffix host query must observe the destination section
/// before the callback returns (`C4Effect.cpp:319-363`; `C4Script.cpp:5401-5408`).
#[test]
fn a_section_switch_from_a_global_effect_resumes_before_the_callback_returns() {
    let mut other = section("Other");
    other.objects.push(scenario::ScenarioSpawn {
        handle: Some("1".to_string()),
        container_handle: None,
        contents_handles: Vec::new(),
        info_name: None,
        config: SpawnConfig::new("REPL")
            .with_id(ObjectId::new(1))
            .with_loaded(true),
    });
    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), other]);
    engine.set_landscape(section_landscape(200, 100));
    engine.register_test_script_definition("REPL", "Destination object", "#strict 3\n");
    assert_eq!(
        engine.install_global_scripts(&[(
            "System.c4g/GlobalContinuation.c".to_string(),
            r#"#strict 3
static trace;

global func ResetGlobalContinuationTrace()
{
    trace = 0;
    return true;
}

global func ReadGlobalContinuationTrace()
{
    return trace;
}

global func FxSwitchTimer(target, number, time)
{
    trace = trace * 10 + 1;
    var switched = LoadScenarioSection("Other", 0);
    trace = trace * 10 + switched;
    trace = trace * 10 + ObjectCount();
    return 0;
}
"#
            .to_string(),
        )]),
        1,
    );
    crate::TestValueExt::test_value(
        engine.call_engine_global_function("ResetGlobalContinuationTrace", &[]),
    );
    let mut effect = EffectState::new("Switch").with_interval(1);
    effect.number = 1;
    engine.global_effects.push(effect);

    crate::TestValueExt::test_value(engine.tick_without_snapshot());

    assert_eq!(engine.debug_current_scenario_section(), "Other");
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadGlobalContinuationTrace", &[])
        ),
        Value::Int(111),
        "the global callback suffix sees the accepted switch and destination object",
    );
}
