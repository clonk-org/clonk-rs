use clonk_engine::{Engine, SpawnConfig};
use clonk_script::Value;

const CREATOR_SCRIPT: &str = r#"#strict
public func CreateThenContain()
{
    // C4Script order of Hazard's Arena_RelaunchClonk
    // (content Hazard.c4d/System.c4g/Arena.c:74-97): the contained object is
    // created first, its container second, and Enter binds them afterwards.
    var content = CreateObject(CNTT, 10, 10, -1);
    var container = CreateObject(BOXX, 20, 20, -1);
    content->Enter(container);
    return content;
}
"#;

/// `FnCreateObject` hands back a live `C4Object` immediately, so a later
/// `C4Object::Enter` in the same call binds two existing objects
/// (`src/C4Script.cpp` FnCreateObject; `src/C4Object.cpp:1560-1620`). The
/// staged Rust spawn queue materializes both after the call returns and must
/// reach that same state instead of failing the frame on a container that is
/// still queued behind its content.
#[test]
fn enter_binds_a_container_created_after_its_content() {
    let mut engine = Engine::new();
    for (id, name) in [("CNTT", "Contained object"), ("BOXX", "Container")] {
        engine
            .register_script_definition(id, name, "#strict\n")
            .expect("fixture definition registers");
    }
    engine
        .register_script_definition("MAKR", "Creator", CREATOR_SCRIPT)
        .expect("creator registers");
    let creator = engine
        .spawn_object(SpawnConfig::new("MAKR"))
        .expect("creator spawns");
    let creator_index = engine.find_object_index(creator).expect("creator index");

    let content = match engine
        .call_object_function(creator_index, "CreateThenContain", Vec::new())
        .expect("the create-then-enter call completes like C++")
    {
        Value::Object(id) => id,
        other => panic!("CreateObject must return the new object: {other:?}"),
    };

    let snapshot = engine.snapshot();
    let container = snapshot
        .objects
        .iter()
        .find(|object| object.definition_id == "BOXX")
        .expect("the container materializes");
    let content = snapshot
        .objects
        .iter()
        .find(|object| object.id.as_u64() == content)
        .expect("the contained object materializes");
    assert_eq!(
        content.container,
        Some(container.id),
        "Enter must bind the content to the container created after it"
    );
    assert!(
        container.contents.contains(&content.id),
        "the container's contents list must hold the entered object"
    );
}

const SCENARIO_SCRIPT: &str = r#"#strict
protected func Initialize()
{
    // Eke Reloaded's CaptureTheFlag::InitializeClonk shape (content
    // EkeReloaded.c4d/GoalsAndRules.c4d/CaptureTheFlag.c4d/Script.c:72-82):
    // the entering object is created first, its container second, and Enter
    // binds them afterwards.
    var content = CreateObject(CNTT, 10, 10, -1);
    var container = CreateObject(BOXX, 20, 20, -1);
    Enter(container, content);
    return 1;
}
"#;

/// The same C++ rule holds when the create-then-`Enter` call arrives from the
/// scenario script instead of an object callback: `FnCreateObject` hands back
/// a live `C4Object` either way, so `Enter` binds two existing objects
/// (`src/C4Script.cpp` FnCreateObject; `src/C4Object.cpp:1560-1620`). The Rust
/// host stages both creations into one `ScenarioBatch`, so `apply_scenario_batch`
/// has to hold the link until the container queued behind its content
/// materializes — as the spawn-queue applier already does. Without that, the
/// container lookup fails and the whole batch aborts, dropping every later
/// effect of the call.
#[test]
fn scenario_script_enter_binds_a_container_created_after_its_content() {
    let mut engine = Engine::new();
    for (id, name) in [("CNTT", "Contained object"), ("BOXX", "Container")] {
        engine
            .register_script_definition(id, name, "#strict\n")
            .expect("fixture definition registers");
    }
    engine
        .install_scenario_script_with_convention("Script.c", SCENARIO_SCRIPT, true)
        .expect("the create-then-enter scenario Initialize completes like C++");

    let snapshot = engine.snapshot();
    let container = snapshot
        .objects
        .iter()
        .find(|object| object.definition_id == "BOXX")
        .expect("the container materializes");
    let content = snapshot
        .objects
        .iter()
        .find(|object| object.definition_id == "CNTT")
        .expect("the contained object materializes");
    assert_eq!(
        content.container,
        Some(container.id),
        "Enter must bind the content to the container created after it"
    );
    assert!(
        container.contents.contains(&content.id),
        "the container's contents list must hold the entered object"
    );
}
