// Parity: `this` evaluates to the current object context, not nil.
//
// C++ oracle: in C4Script `this` yields the object the function runs on
// (C4V_C4Object). The Rust VM previously hardcoded `Expr::This => Value::Nil`
// (vm.rs), so any script that reads `this` as a value (e.g. `var me = this;`,
// passing `this` to a function, or `this == other`) diverged.
//
// lc-script stays host-agnostic: the engine provides an opaque `this` Value
// (in this port an object reference is `Proplist {"id": <number>}`), and the
// VM threads it through the call and returns it for `Expr::This`. Plain nested
// calls inherit the same `this` (they run on the same object).

use lc_script::{Engine, Value};
use std::collections::HashMap;

fn object_ref(id: i32) -> Value {
    Value::Proplist(HashMap::from([("id".to_string(), Value::Int(id))]))
}

#[test]
fn this_returns_the_provided_object_context() {
    let mut engine = Engine::new();
    engine
        .load_script("func Test() { return this; }")
        .expect("loads");
    let this = object_ref(42);
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
    let this = object_ref(7);
    let (result, _) = engine
        .call_with_locals_and_this("Test", &[], &HashMap::new(), this.clone())
        .expect("call succeeds");
    assert_eq!(result, this);
}
