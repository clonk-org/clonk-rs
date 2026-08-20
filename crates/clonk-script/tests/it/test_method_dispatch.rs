//! `obj->Method(args)` is a DIRECT OBJECT CALL (AB_CALL/AB_CALLFS,
//! C4AulExec.cpp:1216-1305): the target value is evaluated, a falsy target
//! throws even for `->~` (:1224-1226), and the function resolves against the
//! TARGET's context — not the calling script. The VM is world-agnostic, so
//! an engine-registered method-dispatch hook performs the cross-object
//! resolution. That includes `this`, whose live definition may have changed
//! while the current callback remains on the stack.

use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clonk_script::{clear_active_object_references, Engine, Value};

#[test]
fn object_target_routes_through_the_method_dispatch_hook() {
    let source = r#"
        global func Probe(target) { return target->Who(5, "x"); }
    "#;
    let log: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    {
        let log = Arc::clone(&log);
        engine.register_method_dispatch(Arc::new(move |args: &[Value]| {
            log.lock().unwrap().push(args.to_vec());
            Ok(Value::Int(99))
        }));
    }
    assert_eq!(
        engine
            .call("Probe", &[Value::Object(7)])
            .expect("call succeeds"),
        Value::Int(99)
    );
    let calls = log.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        vec![
            Value::Object(7),
            Value::String("Who".into()),
            Value::Bool(false),
            Value::Int(5),
            Value::String("x".into()),
        ]
    );
}

#[test]
fn failsafe_arrow_passes_the_failsafe_flag() {
    let source = r#"
        global func Maybe() {}
        global func Probe(target) { return target->~Maybe(); }
    "#;
    let log: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    {
        let log = Arc::clone(&log);
        engine.register_method_dispatch(Arc::new(move |args: &[Value]| {
            log.lock().unwrap().push(args.to_vec());
            Ok(Value::Nil)
        }));
    }
    engine
        .call("Probe", &[Value::Object(7)])
        .expect("call succeeds");
    assert_eq!(
        log.lock().unwrap()[0][2],
        Value::Bool(true),
        "->~ sets the failsafe flag"
    );
}

#[test]
fn falsy_target_is_an_error_even_for_failsafe_calls() {
    // C4AulExec.cpp:1224-1226: "Object call: target is zero!" — the ~ only
    // covers a MISSING FUNCTION, not a missing target.
    let source = r#"
        global func Maybe() {}
        global func Probe(target) { return target->~Maybe(); }
    "#;
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    engine.register_method_dispatch(Arc::new(|_: &[Value]| Ok(Value::Nil)));
    for target in [Value::Nil, Value::C4Id("00000".into())] {
        let error = engine
            .call("Probe", &[target])
            .expect_err("falsy target throws");
        assert!(error.to_string().contains("target is zero"), "got: {error}");
    }
}

#[test]
fn globally_unresolved_failsafe_arrow_discards_a_zero_target_after_evaluating_operands() {
    // C4AulParse.cpp:3215-3231: a globally unresolved failsafe arrow call
    // evaluates and discards its arguments and target, then pushes nil without
    // emitting AB_CALLFS. The runtime zero-target check is therefore bypassed.
    let source = r#"
        #strict
        static target_calls, argument_calls, continued;

        func ZeroTarget() { target_calls++; return 0; }
        func SideEffectArg() { argument_calls++; return 42; }
        func Probe() {
            target_calls = argument_calls = continued = 0;
            var result = ZeroTarget()->~GloballyMissing(SideEffectArg());
            continued++;
            return [result, target_calls, argument_calls, continued];
        }
    "#;
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);

    assert_eq!(
        engine
            .call("Probe", &[])
            .expect("globally missing failsafe call continues"),
        Value::Array(vec![
            Value::Nil,
            Value::Int(1),
            Value::Int(1),
            Value::Int(1),
        ])
    );
}

#[test]
fn engine_wide_known_failsafe_name_preserves_zero_target_validation() {
    // C4AulParse.cpp:3215 and C4AulExec.cpp:1224-1226: a same-named
    // function anywhere in the engine makes the parser emit AB_CALLFS, whose
    // runtime zero-target guard still runs before target-specific lookup.
    let source = r#"
        global func Probe(target) { return target->~KnownElsewhere(); }
    "#;
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    engine.register_method_dispatch(Arc::new(|_: &[Value]| Ok(Value::Nil)));
    engine.register_direct_call_function_probe(Rc::new(|name| name == "KnownElsewhere"));

    let error = engine
        .call("Probe", &[Value::Nil])
        .expect_err("an engine-wide known name retains AB_CALLFS");
    assert!(
        error.to_string().contains("Object call: target is zero!"),
        "got: {error}"
    );
}

#[test]
fn removal_during_arguments_stops_before_bare_local_method_dispatch() {
    // Parse_Params evaluates Clear first, then AB_CALL observes that
    // AssignRemoval cleared its retained receiver and errors before Method
    // runs (C4Object.cpp:312; C4AulExec.cpp:1216-1226).
    let source = r#"#strict 3
        func Method(ignored) { Mark(); return 99; }
        func Probe() { return Target()->Method(Clear()); }
    "#;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    engine.register_host_function("Target", |_| Ok(Value::Object(7)));
    engine.register_host_function("Clear", |_| {
        clear_active_object_references(7);
        Ok(Value::Nil)
    });
    {
        let calls = Arc::clone(&calls);
        engine.register_host_function("Mark", move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Nil)
        });
    }

    let error = engine
        .call("Probe", &[])
        .expect_err("the cleared receiver stops before local dispatch");
    assert!(
        error.to_string().contains("Object call: target is zero!"),
        "got: {error}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "Method did not run");
}

#[test]
fn null_target_arrow_calls_consume_random_before_the_unchanged_error() {
    let source = r#"
        global func Maybe(value) { return value; }
        global func Plain(target) { return target->Maybe(Random(10)); }
        global func Failsafe(target) { return target->~Maybe(Random(10)); }
    "#;
    let draws = Arc::new(AtomicUsize::new(0));
    let observed_draws = Arc::clone(&draws);
    let mut engine = Engine::new();
    engine.register_host_function("Random", move |args| {
        assert_eq!(args, [Value::Int(10)]);
        Ok(Value::Int(
            observed_draws.fetch_add(1, Ordering::SeqCst) as i32
        ))
    });
    crate::support::load_script(&mut engine, source);

    for (function, expected_draws) in [("Plain", 1), ("Failsafe", 2)] {
        let error = engine
            .call(function, &[Value::Nil])
            .expect_err("a zero target still throws");
        assert!(
            error.to_string().contains("Object call: target is zero!"),
            "got: {error}"
        );
        assert_eq!(
            draws.load(Ordering::SeqCst),
            expected_draws,
            "{function} must advance the deterministic RNG ledger first"
        );
    }
}

#[test]
fn id_target_dispatches_a_definition_call() {
    // AB_CALL accepts "object" or "id" targets (C4AulExec.cpp:1229-1247).
    let source = r#"
        global func Probe() { return ROCK->Density(); }
    "#;
    let log: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    {
        let log = Arc::clone(&log);
        engine.register_method_dispatch(Arc::new(move |args: &[Value]| {
            log.lock().unwrap().push(args.to_vec());
            Ok(Value::Int(50))
        }));
    }
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(50)
    );
    assert_eq!(log.lock().unwrap()[0][0], Value::C4Id("ROCK".into()));
}

#[test]
fn self_target_routes_through_the_live_world_dispatch() {
    // AB_CALL reads pDestObj->Def at execution time. It cannot assume the
    // callback's ScriptEngine still owns `this`, because ChangeDef swaps the
    // live definition inline while the old callback remains on the stack.
    let source = r#"
        global func Who() { return 42; }
        global func Probe() { return this()->Who(); }
    "#;
    let log: Arc<Mutex<Vec<Vec<Value>>>> = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    {
        let log = Arc::clone(&log);
        engine.register_method_dispatch(Arc::new(move |args: &[Value]| {
            log.lock().unwrap().push(args.to_vec());
            Ok(Value::Int(99))
        }));
    }
    let (value, _) = engine
        .call_with_locals_and_this(
            "Probe",
            &[],
            &std::collections::HashMap::new(),
            Value::Object(3),
        )
        .expect("call succeeds");
    assert_eq!(value, Value::Int(99));
    assert_eq!(log.lock().unwrap()[0][0], Value::Object(3));
}

#[test]
fn arrow_func_ref_result_writes_through_the_dispatch_reference() {
    // C4AulExec.cpp:1290-1299 passes the call-target stack cell as the
    // callee's return slot; AB_RETURN keeps a `func &` reference there
    // (:1054-1067), and AB_Set writes through it (:858-865).
    let source = r#"
        global func Mark(target) {
            target->SacrificeMade() = 1;
            return 7;
        }
    "#;
    let slot = clonk_script::value_cell(Value::Nil);
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    {
        let slot = Rc::clone(&slot);
        engine.register_method_reference_dispatch(Rc::new(move |args: &[Value]| {
            assert_eq!(args[0], Value::Object(9));
            assert_eq!(args[1], Value::String("SacrificeMade".into()));
            assert_eq!(args[2], Value::Bool(false));
            Ok(clonk_script::ValueReference::from_cell(Rc::clone(&slot)))
        }));
    }

    assert_eq!(
        engine
            .call("Mark", &[Value::Object(9)])
            .expect("call succeeds"),
        Value::Int(7)
    );
    assert_eq!(*slot.borrow(), Value::Int(1));
}
