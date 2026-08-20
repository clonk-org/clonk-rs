//! The cross-object LocalN form: FnLocalN (C4Script.cpp:4591-4605) returns
//! `pVarN->GetRef()` — a REFERENCE into the TARGET object's named locals —
//! so both reads and lvalue writes work on other objects
//! (`LocalN("iWater", pObj) = 90`, GoldRush DoInitialize). The VM is
//! world-agnostic: a host-registered cell hook supplies the live cell.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use clonk_script::{value_cell, Engine, Value, ValueCell};

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
    crate::support::load_script(
        &mut engine,
        "public func Poke(target) {\n\
         LocalN(\"iWater\", target) = 90;\n\
         return LocalN(\"iWater\", target);\n\
     }",
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
    crate::support::load_script(
        &mut engine,
        "public func Peek(target) {\n\
         LocalN(\"iWater\", target) = 77;\n\
         return target->LocalN(\"iWater\");\n\
     }",
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
fn object_index_and_property_read_named_local_cells() {
    let (mut engine, cells) = engine_with_stub_hook();
    cells.borrow_mut().extend([
        ((7, "iWater".to_string()), value_cell(Value::Int(77))),
        (
            (7, "items".to_string()),
            value_cell(Value::Array(vec![Value::Int(9)])),
        ),
    ]);
    crate::support::load_script(
        &mut engine,
        "#strict 3\n\
     local iWater;\n\
     public func Peek(target, key, items_key) {\n\
         return [target[key], target.iWater, target[\"unset\"], target.unset,\n\
                 target[items_key][0], target.items[0]];\n\
     }\n\
     public func PeekSelf(key) { return [this[key], this.iWater]; }\n\
     public func BadKey(target) { return target[1]; }",
    );

    assert_eq!(
        engine
            .call(
                "Peek",
                &[
                    Value::Object(7),
                    Value::String("iWater".to_string().into()),
                    Value::String("items".to_string().into()),
                ],
            )
            .expect("call succeeds"),
        Value::Array(vec![
            Value::Int(77),
            Value::Int(77),
            Value::Nil,
            Value::Nil,
            Value::Int(9),
            Value::Int(9),
        ]),
    );
    let self_locals = HashMap::from([("iWater".to_string(), Value::Int(88))]);
    let (self_result, _) = engine
        .call_with_locals_and_this(
            "PeekSelf",
            &[Value::String("iWater".to_string().into())],
            &self_locals,
            Value::Object(7),
        )
        .expect("self-object access succeeds");
    assert_eq!(
        self_result,
        Value::Array(vec![Value::Int(88), Value::Int(88)]),
        "the executing object's live locals take precedence over the foreign hook",
    );
    let error = engine
        .call("BadKey", &[Value::Object(7)])
        .expect_err("object indexing rejects a non-string key");
    assert!(
        error
            .to_string()
            .contains("indexed access on object: only string keys are allowed"),
        "got: {error}"
    );
}

#[test]
fn object_index_assignment_evaluates_base_key_then_rhs_once() {
    let (mut engine, cells) = engine_with_stub_hook();
    cells
        .borrow_mut()
        .insert((7, "money".to_string()), value_cell(Value::Nil));
    let trace = Arc::new(Mutex::new(Vec::new()));
    for (name, marker, value) in [
        ("MarkBase", 1, Value::Int(0)),
        ("MarkKey", 2, Value::String("money".to_string().into())),
        ("MarkRhs", 3, Value::Int(9)),
    ] {
        let trace = Arc::clone(&trace);
        engine.register_host_function(name, move |_| {
            trace.lock().expect("trace locks").push(marker);
            Ok(value.clone())
        });
    }
    crate::support::load_script(
        &mut engine,
        "#strict 3\n\
     public func Assign(other) {\n\
         var targets = [other];\n\
         targets[MarkBase()][MarkKey()] = MarkRhs();\n\
         return other.money;\n\
     }",
    );

    assert_eq!(
        engine
            .call("Assign", &[Value::Object(7)])
            .expect("assignment succeeds"),
        Value::Int(9),
    );
    assert_eq!(*trace.lock().expect("trace locks"), vec![1, 2, 3]);
    assert_eq!(
        cells
            .borrow()
            .get(&(7, "money".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(9)),
    );
}

#[test]
fn object_index_dereferences_the_base_after_key_side_effects() {
    let (mut engine, cells) = engine_with_stub_hook();
    cells.borrow_mut().extend([
        ((7, "money".to_string()), value_cell(Value::Int(1))),
        ((8, "money".to_string()), value_cell(Value::Int(2))),
    ]);
    crate::support::load_script(
        &mut engine,
        "#strict 3\n\
     static current, replacement;\n\
     private func SelectReplacement() {\n\
         current = replacement;\n\
         return \"money\";\n\
     }\n\
     public func ReadAfterSwitch(first, second) {\n\
         current = first;\n\
         replacement = second;\n\
         return current[SelectReplacement()] + 0;\n\
     }\n\
     public func ReadTrackedAfterSwitch(first, second) {\n\
         current = first;\n\
         replacement = second;\n\
         return current[SelectReplacement()];\n\
     }\n\
     public func WriteAfterSwitch(first, second) {\n\
         current = first;\n\
         replacement = second;\n\
         current[SelectReplacement()] = 9;\n\
     }",
    );

    assert_eq!(
        engine
            .call("ReadAfterSwitch", &[Value::Object(7), Value::Object(8)])
            .expect("read succeeds"),
        Value::Int(2),
        "the key expression replaced the retained base cell before it was dereferenced",
    );
    assert_eq!(
        engine
            .call(
                "ReadTrackedAfterSwitch",
                &[Value::Object(7), Value::Object(8)],
            )
            .expect("tracked read succeeds"),
        Value::Int(2),
    );
    engine
        .call("WriteAfterSwitch", &[Value::Object(7), Value::Object(8)])
        .expect("write succeeds");
    assert_eq!(
        cells
            .borrow()
            .get(&(7, "money".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(1)),
        "the stale pre-key object was not modified",
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(8, "money".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(9)),
        "the post-key object received the write",
    );
}

#[test]
fn arrow_form_numbered_local_read_resolves_the_target_object_slot() {
    // `pObj->Local(0)`: `->` supplies Obj=pObj, so FnLocal returns
    // pObj->Local[0] (C4Script.cpp:3423-3433, pObj defaulting to cthr->Obj).
    // The method-call READ form must resolve through the same host cell as
    // `Local(0, pObj)` — Hazard's Ammo.c `return(ammo->Local(0))` depends on
    // it. The numbered hook key is `__local_{index}`.
    let (mut engine, cells) = engine_with_stub_hook();
    cells
        .borrow_mut()
        .insert((8, "__local_0".to_string()), value_cell(Value::Int(55)));
    crate::support::load_script(
        &mut engine,
        "public func Peek(target) { return target->Local(0); }",
    );
    assert_eq!(
        engine
            .call("Peek", &[Value::Object(8)])
            .expect("call succeeds"),
        Value::Int(55),
        "the arrow-form read resolved the target's numbered local slot"
    );
}

#[test]
fn falsy_target_falls_back_to_the_executing_object() {
    // FnLocalN: `if (!pObj) pObj = cthr->Obj` (C4Script.cpp:4593-4596) —
    // a nil/0 target means the executing object, NOT the hook.
    let (mut engine, cells) = engine_with_stub_hook();
    crate::support::load_script(
        &mut engine,
        "local own;\n\
     public func SelfPoke() { LocalN(\"own\", 0) = 5; return own; }",
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
    crate::support::load_script(
        &mut engine,
        "public func Fill(target, amount) {\n\
         LocalN(\"iWater\", target) = 10;\n\
         LocalN(\"iWater\", target) += amount;\n\
         return LocalN(\"iWater\", target);\n\
     }",
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
