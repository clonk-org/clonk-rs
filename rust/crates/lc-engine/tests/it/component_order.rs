use lc_engine::{Definition, DefinitionComponent, Engine, EngineState, ObjectId, SpawnConfig};
use lc_script::Value;
use std::collections::HashMap;

const SCRIPT: &str = r#"
#strict

func SeedAndRead()
{
  SetComponent(ZERO, 0);
  SetComponent(IROC, 3);
  SetComponent(ZERO, 2);
  return [GetComponent(0, 0), GetComponent(0, 1),
          GetComponent(ZERO), GetComponent(IROC)];
}
"#;

fn engine_with_bag() -> (Engine, ObjectId) {
    let mut engine = Engine::new();
    engine
        .register_definition(Definition::from_script("BAG_", "Bag", SCRIPT).expect("compile"))
        .expect("register bag");
    let bag = engine
        .spawn_object(SpawnConfig::new("BAG_"))
        .expect("spawn bag");
    (engine, bag)
}

#[test]
fn dynamic_object_components_keep_cpp_insertion_order_and_zero_entries() {
    // C4IDList::SetIDCount(..., true) appends a missing ID even at count 0;
    // later count writes do not reorder it. FnGetComponent's indexed form
    // reads that runtime list, not DefCore order (C4IDList.cpp:38-45,85-103;
    // C4Script.cpp:2653-2709).
    let (mut engine, bag) = engine_with_bag();
    let bag_index = engine.find_object_index(bag).expect("bag index");
    assert_eq!(
        engine
            .call_object_function(bag_index, "SeedAndRead", Vec::new())
            .expect("component script runs"),
        Value::Array(vec![
            Value::C4Id("ZERO".to_string()),
            Value::C4Id("IROC".to_string()),
            Value::Int(2),
            Value::Int(3),
        ])
    );

    let snapshot = engine.object_snapshot(bag).expect("bag snapshot");
    assert_eq!(snapshot.component_order, ["ZERO", "IROC"]);
    assert_eq!(snapshot.components.get("ZERO"), Some(&2));
    assert_eq!(snapshot.components.get("IROC"), Some(&3));

    let state: EngineState = serde_json::from_str(
        &serde_json::to_string(&engine.capture_state()).expect("component state serializes"),
    )
    .expect("component state deserializes");
    let (mut restored, _) = engine_with_bag();
    restored.restore_state(&state).expect("restore component list");
    let restored = restored.object_snapshot(bag).expect("restored bag");
    assert_eq!(restored.component_order, ["ZERO", "IROC"]);
}

#[test]
fn definition_order_and_duplicate_component_slots_survive_restore() {
    // C4IDList is an ordered vector, not a map. DefCore and Objects.txt may
    // contain duplicate IDs; the shipped Bazooka has ENAP twice. CompileFunc
    // preserves both slots (C4IDList.cpp:239-260).
    let script = r#"
#strict
func ReadOrder()
{
  return [GetComponent(0, 0), GetComponent(0, 1), GetComponent(0, 2)];
}
"#;
    let mut engine = Engine::new();
    let mut definition =
        Definition::from_script("ORDR", "Ordered", script).expect("definition compiles");
    definition.set_components(vec![
        DefinitionComponent {
            id: "ZZZZ".to_owned(),
            count: 2,
        },
        DefinitionComponent {
            id: "AAAA".to_owned(),
            count: 1,
        },
        DefinitionComponent {
            id: "AAAA".to_owned(),
            count: 1,
        },
    ]);
    engine
        .register_definition(definition.clone())
        .expect("definition registers");
    let object = engine
        .spawn_object(SpawnConfig::new("ORDR").with_components(HashMap::from([
            ("AAAA".to_owned(), 1),
            ("ZZZZ".to_owned(), 2),
        ])))
        .expect("ordered object spawns");
    let index = engine.find_object_index(object).expect("object index");
    assert_eq!(
        engine
            .call_object_function(index, "ReadOrder", Vec::new())
            .expect("indexed component read succeeds"),
        Value::Array(vec![
            Value::C4Id("ZZZZ".to_owned()),
            Value::C4Id("AAAA".to_owned()),
            Value::C4Id("AAAA".to_owned()),
        ])
    );

    let state: EngineState = serde_json::from_str(
        &serde_json::to_string(&engine.capture_state()).expect("ordered state serializes"),
    )
    .expect("ordered state deserializes");
    let mut restored = Engine::new();
    restored
        .register_definition(definition)
        .expect("restore definition registers");
    restored.restore_state(&state).expect("ordered state restores");
    assert_eq!(
        restored
            .object_snapshot(object)
            .expect("restored object exists")
            .component_order,
        ["ZZZZ", "AAAA", "AAAA"]
    );
}
