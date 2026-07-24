//! Empty-index array append (`array[]`) parity with C4Aul's
//! `AB_ARRAY_APPEND`: evaluating the postfix grows the array by one nil slot
//! and leaves a live reference to that new element.

use clonk_script::{Engine, Script, ScriptError, Value};
use std::rc::Rc;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn call(source: &str, args: &[Value]) -> Result<Value, ScriptError> {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script loads");
    engine.call("Test", args)
}

fn runtime_error(source: &str, args: &[Value]) -> String {
    match call(source, args).expect_err("array append must fail") {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn array_append_assignment_is_strict1_plus_only() {
    for directive in ["#strict", "#strict 2", "#strict 3"] {
        let source = format!("{directive}\nfunc Test(a, value) {{ a[] = value; return a; }}");
        assert_eq!(
            call(&source, &[Value::Array(vec![Value::Int(1)]), Value::Int(2)])
                .expect("strict array append succeeds"),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            "{directive}"
        );
    }

    let script = Script::compile("func Test(a, value) { a[] = value; return a; }")
        .expect("recovering parser returns the surviving script");
    let diagnostics = script
        .parse_diagnostics()
        .iter()
        .map(|error| error.message())
        .collect::<Vec<_>>();
    assert!(
        diagnostics
            .iter()
            .any(|message| message.contains("unexpected '['")),
        "NONSTRICT append must report C++'s bracket diagnostic, got {diagnostics:?}"
    );
}

#[test]
fn array_append_assignment_creates_its_slot_before_the_rhs_runs() {
    let mut engine = Engine::new();
    engine.register_host_function("Length", |args| {
        let length = match args.first() {
            Some(Value::Array(elements)) => elements.len(),
            other => panic!("Length expected an array, got {other:?}"),
        };
        Ok(Value::Int(length as i32))
    });
    engine
        .load_script(
            r#"
            #strict
            func Test() {
                var a = [];
                a[] = Length(a);
                return a;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("append assignment succeeds"),
        Value::Array(vec![Value::Int(1)])
    );
}

#[test]
fn array_append_compound_starts_from_a_nil_slot() {
    let strict2 = r#"
        #strict 2
        func Test() {
            var a = [1];
            a[] += 5;
            return a;
        }
    "#;
    assert_eq!(
        call(strict2, &[]).expect("legacy nil-to-zero compound append succeeds"),
        Value::Array(vec![Value::Int(1), Value::Int(5)])
    );

    let strict3 = r#"
        #strict 3
        func Test() {
            var a = [1];
            a[] += 5;
            return a;
        }
    "#;
    assert_eq!(
        runtime_error(strict3, &[]),
        "operator \"+=\" left side: got nil, but expected \"int\"!"
    );
}

#[test]
fn array_append_postincrement_and_read_each_grow_once() {
    let source = r#"
        #strict 2
        func Test() {
            var a = [1];
            var old = a[]++;
            var incremented = a[1];
            var read = a[];
            return [old, incremented, read, a];
        }
    "#;

    assert_eq!(
        call(source, &[]).expect("append increment and read succeed"),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(1),
            Value::Nil,
            Value::Array(vec![Value::Int(1), Value::Int(1), Value::Nil]),
        ])
    );
}

#[test]
fn array_append_remains_a_live_reference_across_later_growth() {
    let source = r#"
        #strict
        func Fill(&first, &second) {
            first = 7;
            second = 8;
        }
        func Test() {
            var a = [1];
            Fill(a[], a[]);
            return a;
        }
    "#;

    assert_eq!(
        call(source, &[]).expect("both appended references remain writable"),
        Value::Array(vec![Value::Int(1), Value::Int(7), Value::Int(8)])
    );
}

#[test]
fn array_append_rejects_non_arrays_with_cpp_errors() {
    let source = "#strict\nfunc Test(value) { value[] = 1; }";
    assert_eq!(
        runtime_error(source, &[Value::Nil]),
        "array append accesss: can't access nil as an array!"
    );
    assert_eq!(
        runtime_error(source, &[Value::Int(5)]),
        "array append accesss: can't access int as an array!"
    );
}

#[test]
fn array_append_reads_self_owned_temporary_arrays_as_nil() {
    let source = r#"
        #strict 2
        static calls;
        func MakeArray() {
            calls++;
            return [1];
        }
        func Test() {
            calls = 0;
            return [[1][], MakeArray()[], (([2] .. [3])[]), calls];
        }
    "#;

    assert_eq!(
        call(source, &[]).expect("temporary append reads collapse to nil"),
        Value::Array(vec![Value::Nil, Value::Nil, Value::Nil, Value::Int(1),])
    );
}

#[test]
fn self_owned_temporary_array_append_loses_its_reference() {
    assert_eq!(
        runtime_error("#strict\nfunc Test() { return [1][]++; }", &[]),
        "operator \"++\": got \"any\", but expected \"int&\"!"
    );

    let reference_parameter = r#"
        #strict
        func Set(&slot) { slot = 7; }
        func Test() { Set([1][]); }
    "#;
    assert_eq!(
        runtime_error(reference_parameter, &[]),
        "call to \"Set\" parameter 1: got \"any\", but expected \"&\"!"
    );
}

#[test]
fn temporary_array_assignment_preserves_cpp_error_order() {
    let source = r#"
        #strict
        static calls;
        func Reset() { calls = 0; }
        func MakeArray() { calls++; return [1]; }
        func Mark() { calls += 10; return 9; }
        func FailArray() { MakeArray()[] = Mark(); }
        func FailInt() { (5)[] = Mark(); }
        func Inspect() { return calls; }
    "#;
    let mut engine = Engine::new();
    engine.set_global_variables(clonk_script::new_global_variables());
    engine.load_script(source).expect("script loads");

    engine.call("Reset", &[]).unwrap();
    let error = engine.call("FailArray", &[]).expect_err("assignment fails");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "operator \"=\" left side: got \"any\", but expected \"&\"!"
    );
    assert_eq!(engine.call("Inspect", &[]).unwrap(), Value::Int(11));

    engine.call("Reset", &[]).unwrap();
    let error = engine.call("FailInt", &[]).expect_err("append fails first");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "array append accesss: can't access int as an array!"
    );
    assert_eq!(engine.call("Inspect", &[]).unwrap(), Value::Nil);
}

#[test]
fn array_append_retains_reference_returning_call_bases() {
    let source = r#"
        #strict
        func &GetArray(&value) { return value; }
        func Test() {
            var source = [1];
            GetArray(source)[] = 2;
            var assigned;
            (assigned = [])[] = 3;
            var appended = [];
            (appended[] = [])[] = 4;
            var coalesced;
            (coalesced ??= [])[] = 5;
            return [source, assigned, appended, coalesced];
        }
    "#;

    assert_eq!(
        call(source, &[]).expect("reference-returning base remains writable"),
        Value::Array(vec![
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            Value::Array(vec![Value::Int(3)]),
            Value::Array(vec![Value::Array(vec![Value::Int(4)])]),
            Value::Array(vec![Value::Int(5)]),
        ])
    );
}

#[test]
fn array_append_retains_found_failsafe_reference_calls() {
    let source = r#"
        #strict
        static source;
        func &GetArray() { return source; }
        func Test(target) {
            source = [1];
            target->~GetArray()[] = 2;
            return source;
        }
    "#;

    assert_eq!(
        call(source, &[Value::Object(7)]).expect("found failsafe func & retains reference"),
        Value::Array(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn reference_declared_function_can_return_a_temporary_append_value() {
    let source = r#"
        #strict
        static calls;
        func &MaybeReference() { return [1][]; }
        func Set(&slot) {}
        func Mark() { calls++; return 7; }
        func Fail() { MaybeReference() = Mark(); }
        func FailParameter() { Set(MaybeReference()); }
        func Inspect() { return calls; }
        func Test() { return MaybeReference(); }
    "#;

    let mut engine = Engine::new();
    engine.set_global_variables(clonk_script::new_global_variables());
    engine.load_script(source).expect("script loads");
    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Nil);

    let error = engine
        .call("Fail", &[])
        .expect_err("assignment requires the dynamic reference result");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "operator \"=\" left side: got \"any\", but expected \"&\"!"
    );
    assert_eq!(engine.call("Inspect", &[]).unwrap(), Value::Int(1));

    let error = engine
        .call("FailParameter", &[])
        .expect_err("reference parameter validates the dynamic call result");
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime error, got {error}");
    };
    assert_eq!(
        error.message(),
        "call to \"Set\" parameter 1: got \"any\", but expected \"&\"!"
    );
}

#[test]
fn value_returning_arrow_calls_do_not_use_the_reference_dispatch() {
    let source = r#"
        #strict
        global func Test(target) {
            return [target->MakeArray()[], target->MakeArray()[0]];
        }
    "#;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut engine = Engine::new();
    {
        let calls = Arc::clone(&calls);
        engine.register_method_dispatch(Arc::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Array(vec![Value::Int(42)]))
        }));
    }
    engine.register_method_reference_dispatch(Rc::new(|_| {
        panic!("a value-returning method must not use reference dispatch")
    }));
    engine.load_script(source).expect("script loads");

    assert_eq!(
        engine.call("Test", &[Value::Object(7)]).unwrap(),
        Value::Array(vec![Value::Nil, Value::Int(42)])
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn ordinary_index_reads_do_not_resize_or_detach_the_array() {
    let source = r#"
        #strict
        func Test() {
            var array = [1];
            var alias = array;
            var missing = array[5];
            var present = array[0];
            return [array, array == alias, missing, present];
        }
    "#;

    assert_eq!(
        call(source, &[]).expect("ordinary index reads stay value-only"),
        Value::Array(vec![
            Value::Array(vec![Value::Int(1)]),
            Value::Bool(true),
            Value::Nil,
            Value::Int(1),
        ])
    );
}

#[test]
fn array_append_rejects_non_array_temporary_values_with_cpp_errors() {
    assert_eq!(
        runtime_error("#strict\nfunc Test() { var value; return value[]; }", &[],),
        "array append accesss: can't access nil as an array!"
    );
    assert_eq!(
        runtime_error("#strict\nfunc Test() { return (5)[]; }", &[]),
        "array append accesss: can't access int as an array!"
    );
    assert_eq!(
        runtime_error(
            "#strict\nfunc MakeNil() { var value; return value; } func Test() { return MakeNil()[]; }",
            &[],
        ),
        "array append accesss: can't access nil as an array!"
    );
    assert_eq!(
        runtime_error("#strict\nfunc Test() { return [1][][0]; }", &[]),
        "indexed access [index]: array, map or string expected, but got nil"
    );
    assert_eq!(
        runtime_error("#strict 3\nfunc Test() { return [1][].x; }", &[]),
        "map access with .: map expected, but got nil!"
    );
    assert_eq!(
        runtime_error(
            "#strict\nfunc Test() { return GlobalN(\"missing\")[]; }",
            &[],
        ),
        "array append accesss: can't access nil as an array!"
    );
    assert_eq!(
        runtime_error(
            "#strict\nfunc Test() { Local(0) = [1]; return Local(-1)[]; }",
            &[],
        ),
        "array append accesss: can't access nil as an array!"
    );
}

#[test]
fn nested_array_append_target_errors_before_evaluating_rhs() {
    let source = r#"
        #strict 3
        static target, calls;
        func Init() {
            target = [];
            calls = nil;
        }
        func SideEffect() {
            calls++;
            return 7;
        }
        func FailIndex() { target[][0] = SideEffect(); }
        func FailCompoundIndex() { target[][0] += SideEffect(); }
        func FailProperty() { target[].x += SideEffect(); }
        func Inspect() { return [calls, target]; }
    "#;
    let mut engine = Engine::new();
    engine.set_global_variables(clonk_script::new_global_variables());
    engine.load_script(source).expect("script loads");
    for (function, expected) in [
        (
            "FailIndex",
            "indexed access [index]: array, map or string expected, but got nil",
        ),
        (
            "FailCompoundIndex",
            "indexed access [index]: array, map or string expected, but got nil",
        ),
        (
            "FailProperty",
            "map access with .: map expected, but got nil!",
        ),
    ] {
        engine.call("Init", &[]).expect("state initialized");
        let error = engine
            .call(function, &[])
            .expect_err("nested access into the new nil slot fails");
        let ScriptError::Runtime(error) = error else {
            panic!("expected runtime error, got {error}");
        };
        assert_eq!(error.message(), expected, "{function}");
        assert_eq!(
            engine.call("Inspect", &[]).expect("state remains readable"),
            Value::Array(vec![Value::Nil, Value::Array(vec![Value::Nil])]),
            "{function} must fail before evaluating its RHS"
        );
    }
}

#[test]
fn safe_array_append_uses_the_detached_navigation_value() {
    let source = r#"
        #strict 3
        func NilTarget(value) { return value?[]; }
        func ArrayTarget() {
            var value = [1];
            var result = value?[];
            return [result, value];
        }
    "#;
    let mut engine = Engine::new();
    engine.load_script(source).expect("safe append forms parse");

    assert_eq!(engine.call("NilTarget", &[Value::Nil]).unwrap(), Value::Nil);
    assert_eq!(
        engine.call("ArrayTarget", &[]).unwrap(),
        Value::Array(vec![Value::Nil, Value::Array(vec![Value::Int(1)])])
    );
}
