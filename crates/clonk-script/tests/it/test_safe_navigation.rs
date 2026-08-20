//! Strict-3 safe navigation (`?->`, `?[...]`, `?.`) mirrors the C++
//! `AB_JUMPNIL` region: a nil receiver skips the complete contiguous
//! navigation suffix, including index and method-argument evaluation, and
//! the final `AB_DEREF` makes the result an rvalue.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use clonk_script::{Engine, Script, Value, ValueMap};

fn diagnostic_messages(source: &str) -> Vec<String> {
    match Script::compile(source) {
        Ok(script) => script
            .parse_diagnostics()
            .iter()
            .map(|error| error.message().to_string())
            .collect(),
        Err(error) => vec![error.message().to_string()],
    }
}

fn assert_diagnostic_contains(source: &str, expected: &str) {
    let messages = diagnostic_messages(source);
    assert!(
        messages.iter().any(|message| message.contains(expected)),
        "missing diagnostic {expected:?} for {source:?}; got {messages:?}"
    );
}

#[test]
fn strict3_safe_navigation_supports_method_index_and_property_access() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            func GetX() { return 0; }
            func ReadMethod(target) { return target?->GetX(); }
            func ReadIndex(target, index) { return target?[index]; }
            func ReadProperty(target) { return target?.key; }
            "#,
        )
        .expect("safe-navigation forms load");

    let method_calls = Arc::new(AtomicUsize::new(0));
    {
        let method_calls = Arc::clone(&method_calls);
        engine.register_method_dispatch(Arc::new(move |args: &[Value]| {
            method_calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(args[0], Value::Object(7));
            assert_eq!(args[1], Value::String("GetX".into()));
            assert_eq!(args[2], Value::Bool(false));
            Ok(Value::Int(42))
        }));
    }

    assert_eq!(
        engine.call("ReadMethod", &[Value::Nil]).expect("nil skips"),
        Value::Nil
    );
    assert_eq!(method_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        engine
            .call("ReadMethod", &[Value::Object(7)])
            .expect("object method runs"),
        Value::Int(42)
    );
    assert_eq!(method_calls.load(Ordering::SeqCst), 1);

    assert_eq!(
        engine
            .call("ReadIndex", &[Value::Nil, Value::Int(0)])
            .expect("nil index skips"),
        Value::Nil
    );
    assert_eq!(
        engine
            .call(
                "ReadIndex",
                &[Value::Array(vec![Value::Int(11)]), Value::Int(0)],
            )
            .expect("array index runs"),
        Value::Int(11)
    );

    assert_eq!(
        engine
            .call("ReadProperty", &[Value::Nil])
            .expect("nil property skips"),
        Value::Nil
    );
    assert_eq!(
        engine
            .call(
                "ReadProperty",
                &[Value::Proplist(ValueMap::from([(
                    "key".to_string(),
                    Value::String("value".into()),
                )]))],
            )
            .expect("proplist property runs"),
        Value::String("value".into())
    );
}

#[test]
fn nil_receiver_skips_arguments_and_is_evaluated_once() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            static calls;
            func Mark() { calls++; return 0; }
            func MakeNil() { calls++; return nil; }

            func SkipIndexArgument() {
                calls = 0;
                var target;
                var result = target?[Mark()];
                return [result, calls];
            }
            func SkipMethodArgument() {
                calls = 0;
                var target;
                var result = target?->~Missing(Mark());
                return [result, calls];
            }
            func EvaluateReceiverOnce() {
                calls = 0;
                var result = MakeNil()?.key;
                return [result, calls];
            }
            func SkipMixedSuffix() {
                calls = 0;
                var target;
                var result = target?.missing[Mark()];
                return [result, calls];
            }
            func ContinueAfterSuffix() {
                calls = 0;
                var target;
                var result = target?.key ?? Mark();
                return [result, calls];
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(
        engine
            .call("SkipIndexArgument", &[])
            .expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(0)])
    );
    assert_eq!(
        engine
            .call("SkipMethodArgument", &[])
            .expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(0)])
    );
    assert_eq!(
        engine
            .call("EvaluateReceiverOnce", &[])
            .expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(1)])
    );
    assert_eq!(
        engine.call("SkipMixedSuffix", &[]).expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(0)])
    );
    assert_eq!(
        engine
            .call("ContinueAfterSuffix", &[])
            .expect("call succeeds"),
        Value::Array(vec![Value::Int(0), Value::Int(1)])
    );
}

#[test]
fn only_nil_triggers_the_guard() {
    let mut engine = Engine::new();
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = Arc::clone(&calls);
        engine.register_host_function("Mark", move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Int(0))
        });
    }
    engine
        .load_script(
            r#"
            #strict 3
            func Probe() { return 0?[Mark()]; }
            "#,
        )
        .expect("script loads");

    engine
        .call("Probe", &[])
        .expect_err("zero is non-nil, so indexing still runs and fails");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn guard_covers_the_contiguous_suffix_but_not_a_new_nonnil_intermediate() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            func Contiguous(target) { return target?.missing.key; }
            func Nested(target) { return target?.missing?.key; }
            "#,
        )
        .expect("script loads");

    assert_eq!(
        engine
            .call("Contiguous", &[Value::Nil])
            .expect("the full suffix is skipped"),
        Value::Nil
    );

    let empty = Value::Proplist(ValueMap::new());
    let error = engine
        .call("Contiguous", std::slice::from_ref(&empty))
        .expect_err("a nil intermediate is not guarded by the receiver's question mark");
    assert!(
        error.to_string().contains("nil"),
        "unexpected error: {error}"
    );

    assert_eq!(
        engine
            .call("Nested", &[empty])
            .expect("the second question mark guards the nil intermediate"),
        Value::Nil
    );
}

#[test]
fn safe_navigation_is_strict3_only_and_requires_a_navigation_operator() {
    for strict_prefix in ["", "#strict\n", "#strict 2\n"] {
        for suffix in ["?.key", "?[0]", "?->GetX()"] {
            let source = format!("{strict_prefix}func Probe(value) {{ return value{suffix}; }}");
            assert_diagnostic_contains(&source, "unexpected '?'");
        }
    }

    for expression in ["value?", "value? + 1", "value?()", "value? ?"] {
        let source = format!("#strict 3\nfunc Probe(value) {{ return {expression}; }}");
        assert_diagnostic_contains(&source, "navigation operator (->, [], .)");
    }
}

run_cases! {
    eval_inherits_the_calling_functions_strict_level:
            r#"
            #strict 3
            func Probe() { return eval("{ key = 8 }?.key"); }
            "#,
        "Probe", &[] => Value::Int(8);
}

#[test]
fn nil_coalescing_remains_distinct_from_lone_question_navigation() {
    let mut engine = Engine::new();
    engine
        .load_script("#strict 3\nfunc Probe(value) { return value ?? 7; }")
        .expect("nil coalescing still lexes and parses");

    assert_eq!(engine.call("Probe", &[Value::Nil]).unwrap(), Value::Int(7));
    assert_eq!(
        engine.call("Probe", &[Value::Int(0)]).unwrap(),
        Value::Int(0)
    );
}

#[test]
fn safe_navigation_result_is_an_rvalue_for_reference_parameters() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("ObserveRef", [0], |args| {
        assert!(!args[0].is_reference());
        assert_eq!(args[0].read()?, Value::Int(1));
        assert!(!args[0].write(Value::Int(9))?);
        Ok(Value::Nil)
    });
    engine
        .load_script(
            r#"
            #strict 3
            func Probe() {
                var source = { key = 1 };
                ObserveRef(source?.key);
                return source.key;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(1),
        "AB_DEREF must prevent the host reference parameter from aliasing source.key"
    );
}
