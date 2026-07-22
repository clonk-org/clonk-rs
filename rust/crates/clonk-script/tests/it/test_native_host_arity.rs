use std::sync::{Arc, Mutex};

use clonk_script::{Engine, RuntimeError, Value};

#[test]
fn native_host_arity_pads_missing_and_discards_surplus_after_evaluation() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();

    let mark_events = events.clone();
    engine.register_host_function_with_arity("Mark", 1, move |args| {
        let value = args[0].clone();
        mark_events.lock().unwrap().push(value.clone());
        Ok(value)
    });

    let capture_events = events.clone();
    engine.register_host_function_with_arity("Capture", 2, move |args| {
        assert_eq!(args.len(), 2);
        let value = Value::Array(args.to_vec());
        capture_events.lock().unwrap().push(value.clone());
        Ok(value)
    });

    let zero_events = events.clone();
    engine.register_host_function_with_arity("Zero", 0, move |args| {
        assert!(args.is_empty());
        zero_events
            .lock()
            .unwrap()
            .push(Value::String("zero".into()));
        Ok(Value::Int(9))
    });

    engine
        .load_script(
            r#"
            #strict
            func Test() {
                var surplus = Capture(Mark(1), Mark(2), Mark(3));
                var missing = Capture(Mark(4));
                var zero = Zero(Mark(5));
                return [surplus, missing, zero];
            }
            "#,
        )
        .unwrap();

    assert_eq!(
        engine.call("Test", &[]).unwrap(),
        Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(4), Value::Nil]),
            Value::Int(9),
        ])
    );
    assert_eq!(
        *events.lock().unwrap(),
        vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Int(4),
            Value::Array(vec![Value::Int(4), Value::Nil]),
            Value::Int(5),
            Value::String("zero".into()),
        ]
    );
}

#[test]
fn surplus_argument_error_prevents_native_dispatch() {
    let capture_calls = Arc::new(Mutex::new(0usize));
    let mut engine = Engine::new();

    engine.register_host_function_with_arity("Mark", 1, |args| Ok(args[0].clone()));
    engine
        .register_host_function_with_arity("Fail", 0, |_| Err(RuntimeError::new("surplus failed")));
    let calls = capture_calls.clone();
    engine.register_host_function_with_arity("Capture", 2, move |_| {
        *calls.lock().unwrap() += 1;
        Ok(Value::Nil)
    });
    engine
        .load_script("func Test() { return Capture(Mark(1), Mark(2), Fail()); }")
        .unwrap();

    let error = engine.call("Test", &[]).unwrap_err();
    assert!(error.to_string().contains("surplus failed"));
    assert_eq!(*capture_calls.lock().unwrap(), 0);
}

#[test]
fn declared_arity_applies_to_direct_global_and_inherited_native_calls() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    let observed = calls.clone();
    engine.register_host_function_with_arity("Native", 2, move |args| {
        observed.lock().unwrap().push(args.to_vec());
        Ok(Value::Array(args.to_vec()))
    });
    assert_eq!(
        engine.call("Native", &[Value::Int(7)]).unwrap(),
        Value::Array(vec![Value::Int(7), Value::Nil])
    );
    engine
        .load_script(
            r#"
            #strict 3
            func Native(a, b, c) { return inherited(a, b, c); }
            func ViaInherited() { return Native(1, 2, 3); }
            func ViaGlobal() { return global->Native(4, 5, 6); }
            "#,
        )
        .unwrap();

    assert_eq!(
        engine.call("ViaInherited", &[]).unwrap(),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        engine.call("ViaGlobal", &[]).unwrap(),
        Value::Array(vec![Value::Int(4), Value::Int(5)])
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            vec![Value::Int(7), Value::Nil],
            vec![Value::Int(1), Value::Int(2)],
            vec![Value::Int(4), Value::Int(5)],
        ]
    );
}

#[test]
fn reference_native_arity_preserves_kept_refs_and_pads_missing_values() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    let mark_events = events.clone();
    engine.register_host_function_with_arity("Mark", 1, move |args| {
        mark_events.lock().unwrap().push(args[0].clone());
        Ok(args[0].clone())
    });
    engine.register_host_reference_function_with_arity("RefCapture", 2, [0], |args| {
        assert_eq!(args.len(), 2);
        assert!(args[0].is_reference());
        let second = args[1].read()?;
        assert!(!args[1].is_reference());
        args[0].write(Value::Int(8))?;
        Ok(second)
    });
    engine
        .load_script(
            r#"
            #strict
            func Test() {
                var value = 4;
                var surplus = RefCapture(value, Mark(2), Mark(3));
                var missing = RefCapture(value);
                return [value, surplus, missing];
            }
            "#,
        )
        .unwrap();

    assert_eq!(
        engine.call("Test", &[]).unwrap(),
        Value::Array(vec![Value::Int(8), Value::Int(2), Value::Nil])
    );
    assert_eq!(*events.lock().unwrap(), vec![Value::Int(2), Value::Int(3)]);
}

#[test]
fn effect_var_private_write_argument_bypasses_public_arity() {
    let value = Arc::new(Mutex::new(Value::Int(3)));
    let marks = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();

    let captured = value.clone();
    engine.register_host_function_with_arity("EffectVar", 3, move |args| {
        match args {
            [_, _, _, replacement] => {
                *captured.lock().unwrap() = replacement.clone();
                Ok(replacement.clone())
            }
            [_, _, _] => Ok(captured.lock().unwrap().clone()),
            _ => panic!("EffectVar received an invalid public or private frame"),
        }
    });
    let mark_events = marks.clone();
    engine.register_host_function_with_arity("Mark", 1, move |args| {
        mark_events.lock().unwrap().push(args[0].clone());
        Ok(args[0].clone())
    });
    engine
        .load_script(
            r#"
            #strict
            func Test() {
                var target;
                var before = EffectVar(0, target, 1, Mark(9));
                EffectVar(0, target, 1) = 7;
                return [before, EffectVar(0, target, 1)];
            }
            "#,
        )
        .unwrap();

    assert_eq!(
        engine.call("Test", &[]).unwrap(),
        Value::Array(vec![Value::Int(3), Value::Int(7)])
    );
    assert_eq!(*marks.lock().unwrap(), vec![Value::Int(9)]);
    assert_eq!(*value.lock().unwrap(), Value::Int(7));
}
