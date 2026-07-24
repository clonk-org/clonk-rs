use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use clonk_script::{Engine, RuntimeError, Value};

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
fn setglobal_builtin_returns_value_and_persists_numbered_slot() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"#strict 3
func Put() { return SetGlobal(3, 42); }
func Read() { return Global(3); }
"#,
        )
        .expect("SetGlobal script parses");

    assert_eq!(
        engine.call("Put", &[]).expect("SetGlobal writes"),
        Value::Int(42)
    );
    assert_eq!(
        engine
            .call("Read", &[])
            .expect("Global reads the later call"),
        Value::Int(42)
    );
}

#[test]
fn setglobal_builtin_and_global_lvalue_share_the_same_slot() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"#strict 3
func Mix() {
    Global(3) = 17;
    var result = global->SetGlobal(3, 42);
    Global(3) += 1;
    return [result, Global(3)];
}
func Read() { return Global(3); }
"#,
        )
        .expect("mixed SetGlobal script parses");

    assert_eq!(
        engine.call("Mix", &[]).expect("both write paths run"),
        Value::Array(vec![Value::Int(42), Value::Int(43)])
    );
    assert_eq!(
        engine.call("Read", &[]).expect("shared slot persists"),
        Value::Int(43)
    );
}

#[test]
fn setglobal_builtin_normalizes_native_any_values_by_caller_strictness() {
    fn register_falsy_hosts(engine: &mut Engine) {
        engine.register_host_function("TypedZero", |_| Ok(Value::Int(0)));
        engine.register_host_function("TypedFalse", |_| Ok(Value::Bool(false)));
    }

    let script = r#"
func Probe() {
    SetGlobal(3, SetGlobal(1, TypedZero()));
    SetGlobal(4, SetGlobal(2, TypedFalse()));
}
func Read(index) { return Global(index); }
"#;
    let mut nonstrict = Engine::new();
    register_falsy_hosts(&mut nonstrict);
    nonstrict
        .load_script(script)
        .expect("nonstrict SetGlobal script parses");
    nonstrict.call("Probe", &[]).expect("nonstrict call runs");
    let nonstrict_values = (1..=4)
        .map(|index| {
            nonstrict
                .call("Read", &[Value::Int(index)])
                .expect("nonstrict global read runs")
        })
        .collect::<Vec<_>>();
    assert_eq!(nonstrict_values, vec![Value::Nil; 4]);

    let mut strict = Engine::new();
    register_falsy_hosts(&mut strict);
    strict
        .load_script(&format!("#strict 3\n{script}"))
        .expect("strict SetGlobal script parses");
    strict.call("Probe", &[]).expect("strict call runs");
    let strict_values = (1..=4)
        .map(|index| {
            strict
                .call("Read", &[Value::Int(index)])
                .expect("strict global read runs")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        strict_values,
        vec![
            Value::Int(0),
            Value::Bool(false),
            Value::Int(0),
            Value::Bool(false),
        ]
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
    let globals = clonk_script::new_global_variables();
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
