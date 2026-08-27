use super::*;
use crate::lib_test_support::{spawn_fixture, EngineTestExt};

/// `Construction` adds an effect whose `FxStart` runs inline, creating two
/// children and then lowering the first one's category.
///
/// C4Game::NewObject links each child into Game.Objects at creation time and
/// only then runs its callbacks, so the later SetCategory reaches
/// C4Object::Resort, which sets Unsorted and defers the move to the next
/// ExecObjects (oracle-src-pinned src/C4Game.cpp:1100-1142;
/// src/C4ObjectList.cpp:134-175; src/C4Object.h:312; src/C4Object.cpp:4094-4100).
/// The creation-time order therefore survives the callback-final category.
#[test]
fn construction_effect_start_preserves_creation_time_master_order() {
    let mut carrier = test_definition(
        "CCFX",
        "Construction carrier effect start",
        r#"#strict 3
func Construction()
{
    AddEffect("Spawn", this(), 100, 0, this());
    return true;
}

func FxSpawnStart()
{
    var first = CreateObject(CHIO, 0, 0, -1);
    CreateObject(CLOW, 0, 0, -1);
    SetCategory(1, first);
    return 0;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    let mut high = test_definition("CHIO", "High category child", "");
    high.set_category(CATEGORY_OBJECT);
    let mut low = test_definition("CLOW", "Low category child", "");
    low.set_category(CATEGORY_STRUCTURE);

    let mut engine = Engine::with_seed(37);
    engine.register_test_definition(carrier);
    engine.register_test_definition(high);
    engine.register_test_definition(low);
    spawn_fixture!(engine, "CCFX");

    let children = engine
        .execution
        .exec_list
        .iter()
        .rev()
        .filter_map(|id| {
            let object = &engine.objects[engine.test_object_index(*id)];
            matches!(object.definition_id.as_str(), "CHIO" | "CLOW")
                .then_some(object.definition_id.as_str())
        })
        .collect::<Vec<_>>();
    assert_eq!(children, vec!["CHIO", "CLOW"]);
}

/// `Construction` and `Initialize` both create objects, and `Initialize`
/// queries the child `Construction` built.
///
/// C4Game::NewObject holds one live `Game.Objects` across both callbacks and
/// returns only after every nested creation has completed (oracle-src-pinned
/// src/C4Game.cpp:1100-1142), so Initialize observes the Construction-created
/// child and neither phase's creations are lost. Shipped `Basement72.c4d`
/// relies on exactly that: its `Construction` builds the basement that the
/// including structure's `Initialize` must not drop.
#[test]
fn initialize_sees_the_child_construction_created() {
    let mut carrier = test_definition(
        "CPHS",
        "Cross-phase creation carrier",
        r#"#strict 3
local seen;

func Construction()
{
    CreateObject(CEAR, 0, 0, -1);
    return true;
}

func Initialize()
{
    seen = GetID(FindObject(CEAR));
    CreateObject(CLAT, 0, 0, -1);
    return true;
}
"#,
    );
    carrier.set_c4_callback_convention(true);
    let mut early = test_definition("CEAR", "Construction child", "");
    early.set_category(CATEGORY_OBJECT);
    let mut late = test_definition("CLAT", "Initialize child", "");
    late.set_category(CATEGORY_STRUCTURE);

    let mut engine = Engine::with_seed(41);
    engine.register_test_definition(carrier);
    engine.register_test_definition(early);
    engine.register_test_definition(late);
    let carrier_id = spawn_fixture!(engine, "CPHS");

    let carrier_index = engine.test_object_index(carrier_id);
    assert_eq!(
        engine.objects[carrier_index].state.local_vars.get("seen"),
        Some(&Value::C4Id("CEAR".to_owned())),
        "Initialize must observe the object Construction created"
    );
    let children = engine
        .objects
        .iter()
        .map(|object| object.definition_id.as_str())
        .filter(|id| matches!(*id, "CEAR" | "CLAT"))
        .collect::<Vec<_>>();
    assert_eq!(
        children,
        vec!["CEAR", "CLAT"],
        "neither creation phase may drop the other's children"
    );
}
