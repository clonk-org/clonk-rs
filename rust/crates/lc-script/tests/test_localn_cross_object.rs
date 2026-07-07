//! The cross-object LocalN form: FnLocalN (C4Script.cpp:4591-4605) returns
//! `pVarN->GetRef()` — a REFERENCE into the TARGET object's named locals —
//! so both reads and lvalue writes work on other objects
//! (`LocalN("iWater", pObj) = 90`, GoldRush DoInitialize). The VM is
//! world-agnostic: a host-registered cell hook supplies the live cell.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use lc_script::{value_cell, Engine, Script, Value, ValueCell};

type CellTable = Rc<RefCell<HashMap<(u64, String), ValueCell>>>;

fn engine_with_stub_hook() -> (Engine, CellTable) {
    let cells: CellTable = Rc::new(RefCell::new(HashMap::new()));
    let mut engine = Engine::new();
    let hook_cells = Rc::clone(&cells);
    engine.register_local_cell_hook(Rc::new(move |target: &Value, name: &str| {
        let Value::Object(id) = target else {
            return None;
        };
        Some(
            hook_cells
                .borrow_mut()
                .entry((*id, name.to_string()))
                .or_insert_with(|| value_cell(Value::Nil))
                .clone(),
        )
    }));
    (engine, cells)
}

#[test]
fn cross_object_localn_reads_and_writes_through_the_host_cell() {
    let (mut engine, cells) = engine_with_stub_hook();
    engine.add_script(
        Script::compile(
            "public func Poke(target) {\n\
                 LocalN(\"iWater\", target) = 90;\n\
                 return LocalN(\"iWater\", target);\n\
             }",
        )
        .expect("compiles"),
    );
    assert_eq!(
        engine
            .call("Poke", &[Value::Object(7)])
            .expect("call succeeds"),
        Value::Int(90)
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(7, "iWater".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(90)),
        "the write went through the host cell"
    );
}

#[test]
fn arrow_form_localn_read_resolves_the_target_object_local() {
    // `pObj->LocalN("iWater")`: the `->` operator supplies Obj=pObj, so
    // FnLocalN reads pObj's named local (C4Script.cpp:4598-4611, pObj
    // defaulting to cthr->Obj). The method-call READ form must resolve
    // through the same host cell as the two-argument form — Goal.c4d's
    // `curr_goal->LocalN("missionPassword")` depends on it.
    let (mut engine, cells) = engine_with_stub_hook();
    engine.add_script(
        Script::compile(
            "public func Peek(target) {\n\
                 LocalN(\"iWater\", target) = 77;\n\
                 return target->LocalN(\"iWater\");\n\
             }",
        )
        .expect("compiles"),
    );
    assert_eq!(
        engine
            .call("Peek", &[Value::Object(4)])
            .expect("call succeeds"),
        Value::Int(77),
        "the arrow-form read resolved the target's named local"
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(4, "iWater".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(77))
    );
}

#[test]
fn falsy_target_falls_back_to_the_executing_object() {
    // FnLocalN: `if (!pObj) pObj = cthr->Obj` (C4Script.cpp:4593-4596) —
    // a nil/0 target means the executing object, NOT the hook.
    let (mut engine, cells) = engine_with_stub_hook();
    engine.add_script(
        Script::compile(
            "local own;\n\
             public func SelfPoke() { LocalN(\"own\", 0) = 5; return own; }",
        )
        .expect("compiles"),
    );
    let locals = HashMap::new();
    let (value, finals) = engine
        .call_with_locals("SelfPoke", &[], &locals)
        .expect("call succeeds");
    assert_eq!(value, Value::Int(5));
    assert_eq!(finals.get("own"), Some(&Value::Int(5)));
    assert!(cells.borrow().is_empty(), "self form never asks the hook");
}

#[test]
fn cross_object_localn_supports_compound_assignment() {
    // The WaterTower pattern: `LocalN("iWater", pObj) += x` — compound
    // operators read-modify-write the same reference cell.
    let (mut engine, cells) = engine_with_stub_hook();
    engine.add_script(
        Script::compile(
            "public func Fill(target, amount) {\n\
                 LocalN(\"iWater\", target) = 10;\n\
                 LocalN(\"iWater\", target) += amount;\n\
                 return LocalN(\"iWater\", target);\n\
             }",
        )
        .expect("compiles"),
    );
    assert_eq!(
        engine
            .call("Fill", &[Value::Object(3), Value::Int(25)])
            .expect("call succeeds"),
        Value::Int(35)
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(3, "iWater".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(35))
    );
}
