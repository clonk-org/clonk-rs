use clonk_engine::{Definition, Engine, SpawnConfig};
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
            .register_definition(
                Definition::from_script(id, name, "#strict\n").expect("fixture script compiles"),
            )
            .expect("fixture definition registers");
    }
    engine
        .register_definition(
            Definition::from_script("MAKR", "Creator", CREATOR_SCRIPT)
                .expect("creator script compiles"),
        )
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
