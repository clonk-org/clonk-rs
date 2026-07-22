// Parity: `this` evaluates to the current object context, not nil.
//
// C++ oracle: an unbound C4Script `this` yields the object the function runs
// on (C4V_C4Object), while a parameter or variable of that name wins normal
// identifier lookup.
//
// clonk-script stays host-agnostic: the engine provides an opaque typed object id,
// and the VM threads it through the call and returns it for an unresolved
// `this`. Plain nested calls inherit the same context (same object).

use clonk_script::{Engine, Value};
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
fn this_parameter_shadows_context_function_like_cpp() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            func Parameter(this) {
                var before = this;
                this = 9;
                return [before, this];
            }
            func Missing(this) { return this; }
            func Hoisted() {
                var before = this;
                var this = 11;
                return [before, this];
            }
            func ParameterBeatsVar(this) {
                var this = 12;
                return [this, VarN("this")];
            }
            func MutateReference(&this) { this += 1; return this; }
            func ForwardReference(this) {
                MutateReference(this);
                return this;
            }
            func ReferenceEntry() {
                var value = 20;
                var result = MutateReference(value);
                return [value, result];
            }
            func TakeContextReference(&value) {
                var before = value;
                value = 99;
                return before;
            }
            func ContextReferenceEntry() {
                return [TakeContextReference(this), this];
            }
            func Inner() { return this; }
            func Outer(this) { return Inner(); }
            func Fallback() {
                var side = 0;
                var from_call = this(side = 5);
                return [this, from_call, side];
            }
            func BoundCall(this) { return this(); }
            "#,
        )
        .expect("loads");

    let context = Value::Object(42);
    let call = |name: &str, args: &[Value]| {
        engine
            .call_with_locals_and_this(name, args, &HashMap::new(), context.clone())
            .expect("call succeeds")
            .0
    };

    assert_eq!(
        call("Parameter", &[Value::Int(7)]),
        Value::Array(vec![Value::Int(7), Value::Int(9)])
    );
    assert_eq!(call("Missing", &[]), Value::Nil);
    assert_eq!(
        call("Hoisted", &[]),
        Value::Array(vec![Value::Nil, Value::Int(11)])
    );
    assert_eq!(
        call("ParameterBeatsVar", &[Value::Int(7)]),
        Value::Array(vec![Value::Int(7), Value::Int(12)])
    );
    assert_eq!(
        call("ReferenceEntry", &[]),
        Value::Array(vec![Value::Int(21), Value::Int(21)])
    );
    assert_eq!(call("ForwardReference", &[Value::Int(20)]), Value::Int(21));
    let context_reference_error = engine
        .call_with_locals_and_this(
            "ContextReferenceEntry",
            &[],
            &HashMap::new(),
            Value::Object(42),
        )
        .expect_err("an unbound context-function result is not an lvalue");
    assert!(context_reference_error
        .to_string()
        .contains("expected \"&\""));
    assert_eq!(call("Outer", &[Value::Int(7)]), context);
    assert_eq!(
        call("Fallback", &[]),
        Value::Array(vec![Value::Object(42), Value::Object(42), Value::Int(5)])
    );

    let error = engine
        .call_with_locals_and_this(
            "BoundCall",
            &[Value::Int(7)],
            &HashMap::new(),
            Value::Object(42),
        )
        .expect_err("a bound this() must not call the context function");
    assert!(error
        .to_string()
        .contains("cannot call bound variable 'this'"));

    let mut constant_engine = Engine::new();
    constant_engine.register_constant("this", Value::Int(77));
    constant_engine
        .load_script(
            r#"
            #strict
            func ConstantFallback() { return this(); }
            "#,
        )
        .expect("the legacy hidden constant loads");
    let (constant_result, _) = constant_engine
        .call_with_locals_and_this("ConstantFallback", &[], &HashMap::new(), Value::Object(42))
        .expect("context fallback beats a same-name constant");
    assert_eq!(constant_result, Value::Object(42));

    let mut reference_call_engine = Engine::new();
    reference_call_engine
        .load_script(
            r#"
            #strict 3
            func &this() { return Local(0); }
            func &BoundReferenceCall(this) { return this(); }
            func BoundReferenceIncrement(this) { return ++this(); }
            "#,
        )
        .expect("reference-returning call fixture loads");
    let reference_call_error = reference_call_engine
        .call_with_locals_and_this(
            "BoundReferenceCall",
            &[Value::Int(7)],
            &HashMap::new(),
            Value::Object(42),
        )
        .expect_err("a bound this() must not escape through func &this");
    assert!(reference_call_error
        .to_string()
        .contains("cannot call bound variable 'this'"));
    let reference_increment_error = reference_call_engine
        .call_with_locals_and_this(
            "BoundReferenceIncrement",
            &[Value::Int(7)],
            &HashMap::new(),
            Value::Object(42),
        )
        .expect_err("a bound ++this() must not escape through func &this");
    assert!(reference_increment_error
        .to_string()
        .contains("cannot call bound variable 'this'"));
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
