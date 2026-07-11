// Parity: `this` evaluates to the current object context, not nil.
//
// C++ oracle: in C4Script `this` yields the object the function runs on
// (C4V_C4Object). The Rust VM previously hardcoded `Expr::This => Value::Nil`
// (vm.rs), so any script that reads `this` as a value (e.g. `var me = this;`,
// passing `this` to a function, or `this == other`) diverged.
//
// lc-script stays host-agnostic: the engine provides an opaque typed object id,
// and the VM threads it through the call and returns it for `Expr::This`. Plain
// nested calls inherit the same `this` (they run on the same object).

use lc_script::{Engine, Value};
use std::collections::HashMap;

#[test]
fn this_returns_the_provided_object_context() {
    let mut engine = Engine::new();
    engine
        .load_script("func Test() { return this; }")
        .expect("loads");
    let this = Value::Object(42);
    let (result, _) = engine
        .call_with_locals_and_this("Test", &[], &HashMap::new(), this.clone())
        .expect("call succeeds");
    assert_eq!(result, this);
}

#[test]
fn this_defaults_to_nil_without_context() {
    let mut engine = Engine::new();
    engine
        .load_script("func Test() { return this; }")
        .expect("loads");
    assert_eq!(engine.call("Test", &[]).expect("call"), Value::Nil);
}

#[test]
fn this_is_inherited_by_nested_plain_calls() {
    let mut engine = Engine::new();
    engine
        .load_script("func Inner() { return this; } func Test() { return Inner(); }")
        .expect("loads");
    let this = Value::Object(7);
    let (result, _) = engine
        .call_with_locals_and_this("Test", &[], &HashMap::new(), this.clone())
        .expect("call succeeds");
    assert_eq!(result, this);
}

#[test]
fn object_values_compare_by_identity() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            func Same(other) { return this == other; }
            func Different(other) { return this != other; }
            "#,
        )
        .expect("loads");
    let this = Value::Object(7);
    assert_eq!(
        engine
            .call_with_locals_and_this("Same", &[Value::Object(7)], &HashMap::new(), this.clone())
            .expect("call succeeds")
            .0,
        Value::Bool(true)
    );
    assert_eq!(
        engine
            .call_with_locals_and_this("Different", &[Value::Object(8)], &HashMap::new(), this)
            .expect("call succeeds")
            .0,
        Value::Bool(true)
    );
}
