//! `SetLocal(index, value, object)` targets the supplied object's numbered
//! local slot, just like the read-side `Local(index, object)`. The arrow form
//! supplies that object as the implicit final argument (C4Script.cpp:3409-3414).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use clonk_script::{value_cell, Engine, RuntimeError, Script, Value, ValueCell};

type CellTable = Rc<RefCell<HashMap<(u64, String), ValueCell>>>;

fn engine_with_numbered_local_hook() -> (Engine, CellTable) {
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
fn setlocal_with_foreign_target_writes_only_that_objects_slot_and_returns_value() {
    let (mut engine, cells) = engine_with_numbered_local_hook();
    engine.add_script(
        Script::compile(
            r#"
                #strict
                public func Assign(target) {
                    Local(0) = 17;
                    var result = SetLocal(0, 1, target);
                    return [result, Local(0), Local(0, target)];
                }
            "#,
        )
        .expect("compiles"),
    );

    let (result, _) = engine
        .call_with_locals_and_this(
            "Assign",
            &[Value::Object(9)],
            &HashMap::new(),
            Value::Object(3),
        )
        .expect("foreign SetLocal succeeds");

    assert_eq!(
        result,
        Value::Array(vec![Value::Int(1), Value::Int(17), Value::Int(1)]),
        "SetLocal returns the assigned value, leaves the caller untouched, and updates the target"
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(9, "__local_0".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(1)),
        "the write reached the foreign object's numbered-local cell"
    );
}

#[test]
fn arrow_form_setlocal_writes_the_target_without_world_method_dispatch() {
    let (mut engine, cells) = engine_with_numbered_local_hook();
    engine.register_method_dispatch(Arc::new(|args| {
        Err(RuntimeError::new(format!(
            "unexpected world method dispatch: {args:?}"
        )))
    }));
    engine.add_script(
        Script::compile(
            r#"
                #strict
                public func Assign(target) {
                    var result = target->SetLocal(13, 42);
                    return [result, target->Local(13)];
                }
            "#,
        )
        .expect("compiles"),
    );

    assert_eq!(
        engine
            .call("Assign", &[Value::Object(7)])
            .expect("arrow SetLocal is handled as an engine function"),
        Value::Array(vec![Value::Int(42), Value::Int(42)])
    );
    assert_eq!(
        cells
            .borrow()
            .get(&(7, "__local_13".to_string()))
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(42))
    );
}

#[test]
fn setlocal_without_target_still_writes_the_executing_object() {
    let (mut engine, cells) = engine_with_numbered_local_hook();
    engine.add_script(
        Script::compile(
            r#"
                #strict
                public func Assign() {
                    var result = SetLocal(4, 33);
                    return [result, Local(4)];
                }
            "#,
        )
        .expect("compiles"),
    );

    let (result, _) = engine
        .call_with_locals_and_this("Assign", &[], &HashMap::new(), Value::Object(3))
        .expect("default-target SetLocal succeeds");
    assert_eq!(result, Value::Array(vec![Value::Int(33), Value::Int(33)]));
    assert!(
        cells.borrow().is_empty(),
        "the executing object's own slot does not go through the foreign-object hook"
    );
}

#[test]
fn setlocal_evaluates_an_explicit_self_target_expression_exactly_once() {
    let (mut engine, cells) = engine_with_numbered_local_hook();
    let evaluations = Arc::new(AtomicUsize::new(0));
    {
        let evaluations = Arc::clone(&evaluations);
        engine.register_host_function("SelfTarget", move |_| {
            evaluations.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Object(3))
        });
    }
    engine.add_script(
        Script::compile(
            r#"
                #strict
                public func Assign() {
                    var result = SetLocal(5, 23, SelfTarget());
                    return [result, Local(5)];
                }
            "#,
        )
        .expect("compiles"),
    );

    let (result, _) = engine
        .call_with_locals_and_this("Assign", &[], &HashMap::new(), Value::Object(3))
        .expect("self-target SetLocal succeeds");
    assert_eq!(result, Value::Array(vec![Value::Int(23), Value::Int(23)]));
    assert_eq!(
        evaluations.load(Ordering::SeqCst),
        1,
        "the third argument expression is evaluated exactly once"
    );
    assert!(
        cells.borrow().is_empty(),
        "an explicit self target still uses the executing object's own slot"
    );
}
