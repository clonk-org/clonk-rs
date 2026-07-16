use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use lc_script::{Engine, RuntimeError, Value};

#[test]
fn failsafe_global_call_to_missing_function_evaluates_arguments_then_returns_nil() {
    let mut engine = Engine::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    engine.register_host_function("Mark", move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(1))
    });
    engine
        .load_script(
            r#"#strict 3
func Probe() { return global->~DefinitelyMissing(Mark()); }
"#,
        )
        .expect("strict-3 global failsafe parses");

    assert_eq!(
        engine.call("Probe", &[]).expect("missing call is failsafe"),
        Value::Nil
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn global_call_evaluates_extra_arguments_but_passes_only_ten_slots() {
    let mut engine = Engine::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&calls);
    engine.register_host_function("Mark", move |_| {
        observed.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(1))
    });
    engine.register_host_function("CountArgs", |args| Ok(Value::Int(args.len() as i32)));
    engine
        .load_script(
            r#"#strict 3
func Probe() {
    return global->CountArgs(
        Mark(), Mark(), Mark(), Mark(), Mark(), Mark(),
        Mark(), Mark(), Mark(), Mark(), Mark()
    );
}
"#,
        )
        .expect("strict-3 global call parses");

    assert_eq!(
        engine.call("Probe", &[]).expect("global call runs"),
        Value::Int(10)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 11);
}

#[test]
fn global_call_preserves_numbered_global_and_script_reference_returns() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("WriteNative", [0], |args| {
        let target = args
            .first()
            .ok_or_else(|| RuntimeError::new("WriteNative expects a target"))?;
        let value = args
            .get(1)
            .ok_or_else(|| RuntimeError::new("WriteNative expects a value"))?
            .read()?;
        assert!(target.write(value)?);
        Ok(Value::Bool(true))
    });
    engine
        .load_script(
            r#"#strict 3
func Write(&slot, value) { slot = value; }
global func & Forward(index) { return Global(index); }
func Probe() {
    Global(0) = 10;
    var first = global->Global(0);
    global->Global(0) = 20;
    var second = Global(0);
    Write(global->Global(0), 30);
    var third = Global(0);
    WriteNative(global->Global(0), 40);
    global->Forward(1) = 41;
    return [first, second, third, Global(0), Global(1)];
}
"#,
        )
        .expect("global reference calls parse");

    assert_eq!(
        engine.call("Probe", &[]).expect("reference calls run"),
        Value::Array(vec![
            Value::Int(10),
            Value::Int(20),
            Value::Int(30),
            Value::Int(40),
            Value::Int(41),
        ])
    );
}

#[test]
fn global_call_preserves_caller_var_slots() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"#strict 3
func Probe() {
    Var(0) = 5;
    var before = global->Var(0);
    global->Var(0) = 7;
    return [before, Var(0)];
}
"#,
        )
        .expect("global Var calls parse");

    assert_eq!(
        engine.call("Probe", &[]).expect("global Var calls run"),
        Value::Array(vec![Value::Int(5), Value::Int(7)])
    );
}

#[test]
fn global_call_preserves_named_global_references_and_missing_nil() {
    let globals = lc_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine
        .load_script(
            r#"#strict 3
static named;
func Probe() {
    named = 11;
    var before = global->GlobalN("named");
    global->GlobalN("named") = 12;
    return [before, named, global->GlobalN("missing")];
}
"#,
        )
        .expect("global GlobalN calls parse");

    assert_eq!(
        engine.call("Probe", &[]).expect("global GlobalN calls run"),
        Value::Array(vec![Value::Int(11), Value::Int(12), Value::Nil])
    );
    assert!(!globals.borrow().contains_key("missing"));
}

#[test]
fn adjacent_global_call_is_not_special_below_strict3() {
    for directive in ["", "#strict\n", "#strict 2\n"] {
        let mut engine = Engine::new();
        engine.register_host_function("F", |_| Ok(Value::Int(42)));
        engine
            .load_script(&format!(
                "{directive}func Probe() {{ return global->F(); }}"
            ))
            .expect("legacy global identifier form parses");
        let error = engine
            .call("Probe", &[])
            .expect_err("undefined legacy target must not become a global call");
        assert!(
            error.to_string().contains("undefined variable 'global'"),
            "unexpected error for directive {directive:?}: {error}"
        );
    }
}
