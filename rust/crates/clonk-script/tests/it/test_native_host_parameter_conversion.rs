//! Native engine functions run the same typed C4Value conversion pass as C++
//! before either debugger hooks or the native body can observe their arguments.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use clonk_script::{C4VType, DebuggerHooks, Engine, Script, ScriptError, Value, ValueMap};

fn runtime_message(error: ScriptError) -> String {
    match error {
        ScriptError::Runtime(error) => error.to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

fn type_name(value_type: C4VType) -> &'static str {
    match value_type {
        C4VType::Any => "any",
        C4VType::Int => "int",
        C4VType::Bool => "bool",
        C4VType::C4Id => "id",
        C4VType::C4Object => "object",
        C4VType::String => "string",
        C4VType::Array => "array",
        C4VType::Map => "map",
        C4VType::Ref => "&",
    }
}

#[test]
fn native_host_parameter_conversion_respects_caller_strictness_and_rejects_maps_as_objects() {
    // A legacy function copied into a strict-3 destination still eagerly
    // clears falsy arguments according to its declaring script (pOrgScript).
    let legacy_calls = Arc::new(AtomicUsize::new(0));
    let observed_legacy_calls = Arc::clone(&legacy_calls);
    let mut strict_destination = Engine::new();
    strict_destination.register_host_function("CaptureObject", move |args| {
        observed_legacy_calls.fetch_add(1, Ordering::SeqCst);
        Ok(args[0].clone())
    });
    assert!(
        strict_destination.set_host_function_parameter_types("CaptureObject", [C4VType::C4Object])
    );
    strict_destination.add_script(
        Script::compile("#strict 3\nfunc DestinationOwn() { return true; }")
            .expect("strict destination compiles"),
    );
    let mut legacy_source = Engine::new();
    legacy_source.add_script(
        Script::compile("func LegacyFalse() { return CaptureObject(1 == 2); }")
            .expect("legacy source compiles"),
    );
    strict_destination.merge_from(&legacy_source);

    assert_eq!(
        strict_destination
            .call("LegacyFalse", &[])
            .expect("legacy falsy value converts eagerly to native nil"),
        Value::Nil
    );
    assert_eq!(legacy_calls.load(Ordering::SeqCst), 1);

    // Conversely, a strict-3 function copied into a legacy destination keeps
    // its false tag. bool->object and map->object are both table errors and
    // must fail before the native body runs.
    let strict_calls = Arc::new(AtomicUsize::new(0));
    let observed_strict_calls = Arc::clone(&strict_calls);
    let mut legacy_destination = Engine::new();
    legacy_destination.register_host_function("CaptureObject", move |args| {
        observed_strict_calls.fetch_add(1, Ordering::SeqCst);
        Ok(args[0].clone())
    });
    assert!(
        legacy_destination.set_host_function_parameter_types("CaptureObject", [C4VType::C4Object])
    );
    legacy_destination.add_script(
        Script::compile("func DestinationOwn() { return true; }")
            .expect("legacy destination compiles"),
    );
    let mut strict_source = Engine::new();
    strict_source.add_script(
        Script::compile(
            r#"#strict 3
func StrictFalse() { return CaptureObject(1 == 2); }
func StrictMap() { return CaptureObject({ answer = 42 }); }
"#,
        )
        .expect("strict source compiles"),
    );
    legacy_destination.merge_from(&strict_source);

    assert_eq!(
        runtime_message(
            legacy_destination
                .call("StrictFalse", &[])
                .expect_err("strict false must not be folded to nil")
        ),
        r#"call to "CaptureObject" parameter 1: got "bool", but expected "object"!"#
    );
    assert_eq!(
        runtime_message(
            legacy_destination
                .call("StrictMap", &[])
                .expect_err("a map is not an object")
        ),
        r#"call to "CaptureObject" parameter 1: got "map", but expected "object"!"#
    );
    assert_eq!(
        strict_calls.load(Ordering::SeqCst),
        0,
        "conversion failures happen before the native body"
    );
}

#[test]
fn native_host_boundary_applies_the_strict_cpp_conversion_matrix() {
    use C4VType::{Any, Array, Bool, C4Id, C4Object, Int, Map, String as C4String};

    let sources = vec![
        Value::Nil,
        Value::Int(7),
        Value::Bool(true),
        Value::C4Id("CLNK".into()),
        Value::Object(7),
        Value::String("text".into()),
        Value::Array(vec![Value::Int(1)]),
        Value::Proplist(ValueMap::from([("answer", Value::Int(42))])),
    ];
    let targets = [Any, Int, Bool, C4Id, C4Object, C4String, Array, Map];
    // Rows are the sources above; columns are the targets above. DirectOld
    // cells are false because parameter conversion always calls ConvertTo in
    // strict mode, independently of the caller's #strict level.
    let legal = [
        [true, true, true, true, true, true, true, true],
        [true, true, true, true, false, false, false, false],
        [true, true, true, false, false, false, false, false],
        [true, false, true, true, false, false, false, false],
        [true, false, true, false, true, false, false, false],
        [true, false, true, false, false, true, false, false],
        [true, false, true, false, false, false, true, false],
        [true, false, true, false, false, false, false, true],
    ];

    for (source_index, source) in sources.iter().enumerate() {
        for (target_index, target) in targets.iter().copied().enumerate() {
            let body_calls = Arc::new(AtomicUsize::new(0));
            let observed_body_calls = Arc::clone(&body_calls);
            let mut engine = Engine::new();
            engine.register_host_function("Probe", move |args| {
                observed_body_calls.fetch_add(1, Ordering::SeqCst);
                Ok(args[0].clone())
            });
            assert!(engine.set_host_function_parameter_types("Probe", [target]));

            let result = engine.call("Probe", std::slice::from_ref(source));
            if legal[source_index][target_index] {
                let expected = if matches!(source, Value::Int(7)) && target == C4Id {
                    Value::C4Id("0007".into())
                } else {
                    source.clone()
                };
                assert_eq!(
                    result.expect("legal C4ScriptCnvMap cell reaches the native"),
                    expected,
                    "{} -> {}",
                    source.type_name(),
                    type_name(target)
                );
                assert_eq!(body_calls.load(Ordering::SeqCst), 1);
            } else {
                assert_eq!(
                    runtime_message(result.expect_err("illegal C4ScriptCnvMap cell must fail")),
                    format!(
                        "call to \"Probe\" parameter 1: got \"{}\", but expected \"{}\"!",
                        source.type_name(),
                        type_name(target)
                    ),
                    "{} -> {}",
                    source.type_name(),
                    type_name(target)
                );
                assert_eq!(
                    body_calls.load(Ordering::SeqCst),
                    0,
                    "an illegal conversion cannot enter the native body"
                );
            }
        }
    }
}

#[test]
fn native_int_to_id_conversion_mutates_valid_bounds_and_rejects_outside_them() {
    let body_calls = Arc::new(AtomicUsize::new(0));
    let observed_body_calls = Arc::clone(&body_calls);
    let mut engine = Engine::new();
    engine.register_host_function("ToId", move |args| {
        observed_body_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(args[0].c4v_type(), C4VType::C4Id);
        Ok(args[0].clone())
    });
    assert!(engine.set_host_function_parameter_types("ToId", [C4VType::C4Id]));
    engine
        .load_script("#strict 3\nfunc Convert(value) { return ToId(value); }")
        .expect("strict conversion wrapper compiles");

    for (input, expected) in [
        // AB_FUNC reuses parameter slot zero as its return slot. Conversion
        // already changed that slot to C4ID(0), so C4Value::Set's identical
        // data/type early return preserves the exceptional tag.
        (0, Value::C4Id("NONE".into())),
        (1, Value::C4Id("0001".into())),
        (9999, Value::C4Id("9999".into())),
    ] {
        assert_eq!(
            engine
                .call("Convert", &[Value::Int(input)])
                .expect("0..=9999 converts to C4ID"),
            expected,
            "input {input}"
        );
    }
    for input in [-1, 10_000] {
        assert_eq!(
            runtime_message(
                engine
                    .call("Convert", &[Value::Int(input)])
                    .expect_err("out-of-range int cannot convert to C4ID")
            ),
            r#"call to "ToId" parameter 1: got "int", but expected "id"!"#,
            "input {input}"
        );
    }
    assert_eq!(
        body_calls.load(Ordering::SeqCst),
        3,
        "only the three legal values reach the native"
    );
}

#[test]
fn native_no_caller_eagerly_clears_falsy_values_but_preserves_nil_for_int_and_bool() {
    let mut engine = Engine::new();
    engine.register_host_function("Observe", |args| Ok(Value::Array(args.to_vec())));
    assert!(engine.set_host_function_parameter_types(
        "Observe",
        [
            C4VType::Int,
            C4VType::Bool,
            C4VType::Any,
            C4VType::Any,
            C4VType::Any,
            C4VType::Any,
            C4VType::Any,
            C4VType::Any,
            C4VType::Any,
        ]
    ));

    let empty_map = Value::Proplist(ValueMap::new());
    assert_eq!(
        engine
            .call(
                "Observe",
                &[
                    Value::Nil,
                    Value::Nil,
                    Value::Int(0),
                    Value::Bool(false),
                    Value::Object(0),
                    Value::C4Id("NONE".into()),
                    Value::String(String::new().into()),
                    Value::Array(Vec::new()),
                    empty_map.clone(),
                ],
            )
            .expect("callerless native call converts"),
        Value::Array(vec![
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::Nil,
            Value::String(String::new().into()),
            Value::Array(Vec::new()),
            empty_map,
        ])
    );
}

#[test]
fn native_signature_pads_and_truncates_after_all_surplus_arguments_run() {
    let marks = Arc::new(Mutex::new(Vec::new()));
    let observed_marks = Arc::clone(&marks);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let observed_calls = Arc::clone(&calls);
    let mut engine = Engine::new();
    engine.register_host_function("Mark", move |args| {
        observed_marks
            .lock()
            .expect("mark log")
            .push(args[0].clone());
        Ok(args[0].clone())
    });
    engine.register_host_function("Probe", move |args| {
        observed_calls.lock().expect("call log").push(args.to_vec());
        Ok(Value::Array(args.to_vec()))
    });
    assert!(engine.set_host_function_parameter_types("Probe", [C4VType::Int, C4VType::Bool]));
    engine
        .load_script(
            r#"#strict 3
func Missing() { return Probe(Mark(7)); }
func Surplus() { return Probe(Mark(1), Mark(2), Mark(3)); }
"#,
        )
        .expect("arity probes compile");

    assert_eq!(
        engine.call("Missing", &[]).expect("missing slot is padded"),
        Value::Array(vec![Value::Int(7), Value::Nil]),
        "native int/bool slots preserve padded nil instead of bridging to zero"
    );
    assert_eq!(
        engine
            .call("Surplus", &[])
            .expect("surplus slot is truncated"),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
    assert_eq!(
        *marks.lock().expect("mark log"),
        vec![Value::Int(7), Value::Int(1), Value::Int(2), Value::Int(3),],
        "every supplied argument executes left-to-right before arity balancing"
    );
    assert_eq!(
        *calls.lock().expect("call log"),
        vec![
            vec![Value::Int(7), Value::Nil],
            vec![Value::Int(1), Value::Int(2)],
        ]
    );
}

#[test]
fn native_ref_requires_a_reference_while_non_ref_receives_a_dereferenced_copy() {
    let body_calls = Arc::new(AtomicUsize::new(0));
    let observed_body_calls = Arc::clone(&body_calls);
    let mut engine = Engine::new();
    engine.register_host_reference_function("Inspect", [0], move |args| {
        observed_body_calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(args.len(), 2);
        assert!(args[0].is_reference());
        assert!(!args[1].is_reference());
        assert_eq!(args[1].read()?, Value::Int(4));
        assert!(args[0].write(Value::Int(9))?);
        assert!(!args[1].write(Value::Int(99))?);
        args[1].read()
    });
    assert!(engine.set_host_function_parameter_types("Inspect", [C4VType::Ref, C4VType::Int]));

    let (result, final_args) = engine
        .call_with_ref_args("Inspect", &[Value::Int(3), Value::Int(4)])
        .expect("a native ref accepts the first host-side reference");
    assert_eq!(result, Value::Int(4));
    assert_eq!(final_args, vec![Value::Int(9), Value::Int(4)]);

    assert_eq!(
        runtime_message(
            engine
                .call("Inspect", &[])
                .expect_err("a padded nil is not a reference")
        ),
        r#"call to "Inspect" parameter 1: got "any", but expected "&"!"#
    );
    assert_eq!(
        body_calls.load(Ordering::SeqCst),
        1,
        "the missing-reference call does not enter the native"
    );
}

#[test]
fn native_conversion_precedes_debugger_and_body_and_exposes_mutated_values() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let debugger_order = Arc::clone(&order);
    let body_order = Arc::clone(&order);
    let mut engine = Engine::new();
    engine.register_host_function("Probe", move |args| {
        assert_eq!(args, [Value::C4Id("0007".into())]);
        body_order.lock().expect("order log").push("body");
        Ok(args[0].clone())
    });
    assert!(engine.set_host_function_parameter_types("Probe", [C4VType::C4Id]));
    let mut hooks = DebuggerHooks::new();
    hooks.set_on_call(move |name, args| {
        if name == "Probe" {
            assert_eq!(args, [Value::C4Id("0007".into())]);
            debugger_order.lock().expect("order log").push("debugger");
        }
    });
    engine.set_debugger_hooks(hooks);

    assert_eq!(
        engine
            .call("Probe", &[Value::Int(7)])
            .expect("int converts to id"),
        Value::C4Id("0007".into())
    );
    assert_eq!(*order.lock().expect("order log"), vec!["debugger", "body"]);

    assert_eq!(
        runtime_message(
            engine
                .call("Probe", &[Value::String("bad".into())])
                .expect_err("string cannot convert to id")
        ),
        r#"call to "Probe" parameter 1: got "string", but expected "id"!"#
    );
    assert_eq!(
        *order.lock().expect("order log"),
        vec!["debugger", "body"],
        "failed conversion reaches neither debugger nor native body"
    );
}
