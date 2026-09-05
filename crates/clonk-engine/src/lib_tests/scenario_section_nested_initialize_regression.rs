//! A child created during `Initialize` may request a synchronous section load.
//!
//! clonk-org/clonk-rs#1495 and clonk-org/clonk-rs#1496. `C4Game::NewObject`
//! makes the child live before its callbacks and completes those callbacks
//! synchronously (C4Game.cpp:1100-1142; C4Object.cpp:1428-1515). A nested
//! `LoadScenarioSection` then removes active objects before installing the
//! destination section (C4Game.cpp:4190-4208). The child and its parent must
//! resume their own VM frames from that committed boundary, while explicit
//! destination numbers must refer only to the newly loaded replacements.

use super::*;
use crate::lib_test_support::EngineTestExt;

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

#[test]
fn nested_initialize_section_switch_resumes_before_explicit_replacements() {
    let mut other = section("Other");
    for (handle, id) in [("parent-replacement", 77), ("child-replacement", 78)] {
        other.objects.push(scenario::ScenarioSpawn {
            handle: Some(handle.to_string()),
            container_handle: None,
            contents_handles: Vec::new(),
            info_name: None,
            config: SpawnConfig::new("REPL")
                .with_id(ObjectId::new(id))
                .with_loaded(true),
        });
    }

    let mut engine = Engine::with_seed(0);
    engine.configure_scenario_sections(&[section("Main"), other]);
    engine.set_landscape(section_landscape(200, 100));

    let observer = r#"#strict 3
static nested_trace, nested_lifecycle_order, nested_initialize_count;
static nested_self_reference, nested_initialize_status, nested_object_number;

global func ResetNestedInitializeTrace()
{
    nested_trace = nested_lifecycle_order = nested_initialize_count = 0;
    nested_self_reference = nested_initialize_status = nested_object_number = 0;
    return true;
}

global func RecordNestedTrace(int value)
{
    nested_trace = nested_trace * 10 + value;
    return true;
}

global func RecordNestedInitialize(int order, int count, int self_reference, int status, int number)
{
    nested_lifecycle_order = order;
    nested_initialize_count = count;
    nested_self_reference = self_reference;
    nested_initialize_status = status;
    nested_object_number = number;
    return true;
}

global func ReadNestedTrace() { return nested_trace; }
global func ReadNestedLifecycleOrder() { return nested_lifecycle_order; }
global func ReadNestedInitializeCount() { return nested_initialize_count; }
global func ReadNestedSelfReference() { return nested_self_reference; }
global func ReadNestedInitializeStatus() { return nested_initialize_status; }
global func ReadNestedObjectNumber() { return nested_object_number; }
"#;
    assert_eq!(
        engine
            .install_global_scripts(&[("System.c4g/NestedInitialize.c".into(), observer.into(),)]),
        1
    );
    crate::TestValueExt::test_value(
        engine.call_engine_global_function("ResetNestedInitializeTrace", &[]),
    );

    let mut parent = test_definition(
        "INITP",
        "Initial lifecycle parent",
        r#"#strict 3
func Construction()
{
    RecordNestedTrace(6);
}

func Completion()
{
    RecordNestedTrace(7);
}

func Initialize()
{
    RecordNestedTrace(8);
    CreateObject(CHLD, 0, 0, -1);
    RecordNestedTrace(9);
    return true;
}
"#,
    );
    parent.set_c4_callback_convention(true);
    engine.register_test_definition(parent);

    let mut child = test_definition(
        "CHLD",
        "Nested section switcher",
        r#"#strict 3
local LifecycleOrder, InitializeCount;

func Construction()
{
    LifecycleOrder = 1;
    RecordNestedTrace(1);
}

func Initialize()
{
    LifecycleOrder = LifecycleOrder * 10 + 2;
    InitializeCount++;
    RecordNestedInitialize(
        LifecycleOrder,
        InitializeCount,
        !!this(),
        GetObjectStatus(this()),
        ObjectNumber(this())
    );
    RecordNestedTrace(2);
    var switched = LoadScenarioSection("Other", 0);
    RecordNestedTrace(switched);
    RecordNestedTrace(3);
    return switched;
}
"#,
    );
    child.set_c4_callback_convention(true);
    engine.register_test_definition(child);
    engine.register_test_definition(test_definition("REPL", "Replacement", "#strict 3\n"));

    let result = engine.spawn_object_with_initial_lifecycle(
        SpawnConfig::new("INITP")
            .with_id(ObjectId::new(77))
            .with_construction(FULL_CON),
        None,
    );

    let trace = engine.call_engine_global_function("ReadNestedTrace", &[]);
    assert!(
        matches!(&result, Ok(None)),
        "nested switch result={result:?}, section={}, trace={trace:?}, objects={:?}",
        engine.debug_current_scenario_section(),
        engine
            .objects
            .iter()
            .map(|object| (object.id, object.definition_id.as_str()))
            .collect::<Vec<_>>(),
    );
    assert_eq!(engine.debug_current_scenario_section(), "Other");
    assert_eq!(engine.objects.len(), 2);
    for id in [ObjectId::new(77), ObjectId::new(78)] {
        let index = engine
            .find_object_index(id)
            .expect("the explicit destination replacement remains materialized");
        assert_eq!(engine.objects[index].definition_id, "REPL");
    }

    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedTrace", &[],)
        ),
        Value::Int(67_812_139),
        "parent and child callbacks resume in native nested order after the committed switch",
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedLifecycleOrder", &[],)
        ),
        Value::Int(12),
        "Construction completes before the nested child's Initialize callback",
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedInitializeCount", &[],)
        ),
        Value::Int(1),
        "the nested child is initialized once before the switch",
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedSelfReference", &[],)
        ),
        Value::Bool(true),
        "the child observes its own live reference during Initialize (C4Script !!this() is bool)",
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedInitializeStatus", &[],)
        ),
        Value::Int(1),
        "the child observes normal status before LoadScenarioSection",
    );
    assert_eq!(
        crate::TestValueExt::test_value(
            engine.call_engine_global_function("ReadNestedObjectNumber", &[],)
        ),
        Value::Int(78),
        "CreateObject materializes the child before the explicit replacement is loaded",
    );
}
