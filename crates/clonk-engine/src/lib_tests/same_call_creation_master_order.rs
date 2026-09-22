use super::*;
use crate::lib_test_support::{spawn_fixture, EngineTestExt};

/// Creates one object of the given definition and asks whether the legacy
/// `FindObject` the next statement makes finds it.
const CREATE_THEN_FIND: &str = r#"#strict 2
func CreateThenFind(id kind)
{
    var made = CreateObject(kind, 0, 0, -1);
    return FindObject(kind) == made;
}
"#;

/// C4Game::NewObject links a new object into Game.Objects before
/// Construction runs (oracle-src-pinned src/C4Game.cpp:1122-1138), and
/// C4Game::FindObject walks that list from `Objects.First`
/// (src/C4Game.cpp:1367-1391). A StaticBack object skips the same-id search
/// and goes in front of the first link whose category is no higher than its
/// own (src/C4ObjectList.cpp:155-175), so it precedes every object of its
/// kind that was already there, and the very next statement finds it.
#[test]
fn find_object_finds_a_static_back_object_created_earlier_in_the_same_call() {
    let mut goal = test_definition("SBGL", "Static back goal", "");
    goal.set_category(CATEGORY_STATIC_BACK);
    let mut engine = Engine::with_seed(43);
    engine.register_test_definition(goal);
    engine.register_test_definition(test_definition(
        "CTFP",
        "Create then find",
        CREATE_THEN_FIND,
    ));
    spawn_fixture!(engine, "SBGL");
    let probe = spawn_fixture!(engine, "CTFP");

    let index = engine.test_object_index(probe);
    let found = engine
        .call_object_function(
            index,
            "CreateThenFind",
            vec![Value::C4Id("SBGL".to_owned())],
        )
        .expect("the probe runs");
    assert_eq!(found, Value::Bool(true));
}

/// Any other object goes in front of the first object with its category and
/// id (src/C4ObjectList.cpp:155-163), so it too precedes its kin at once.
#[test]
fn find_object_finds_an_object_created_earlier_in_the_same_call_ahead_of_its_kin() {
    let mut item = test_definition("OBIT", "Object item", "");
    item.set_category(CATEGORY_OBJECT);
    let mut engine = Engine::with_seed(47);
    engine.register_test_definition(item);
    engine.register_test_definition(test_definition(
        "CTFP",
        "Create then find",
        CREATE_THEN_FIND,
    ));
    spawn_fixture!(engine, "OBIT");
    let probe = spawn_fixture!(engine, "CTFP");

    let index = engine.test_object_index(probe);
    let found = engine
        .call_object_function(
            index,
            "CreateThenFind",
            vec![Value::C4Id("OBIT".to_owned())],
        )
        .expect("the probe runs");
    assert_eq!(found, Value::Bool(true));
}
