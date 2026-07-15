//! Empty-index array append (`array[]`) parity with C4Aul's
//! `AB_ARRAY_APPEND`: evaluating the postfix grows the array by one nil slot
//! and leaves a live reference to that new element.

use lc_script::{Engine, Script, ScriptError, Value};

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
    for strict_level in 1..=3 {
        let source =
            format!("#strict {strict_level}\nfunc Test(a, value) {{ a[] = value; return a; }}");
        assert_eq!(
            call(&source, &[Value::Array(vec![Value::Int(1)]), Value::Int(2)])
                .expect("strict array append succeeds"),
            Value::Array(vec![Value::Int(1), Value::Int(2)]),
            "#strict {strict_level}"
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
            #strict 1
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
        #strict 1
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
    let source = "#strict 1\nfunc Test(value) { value[] = 1; }";
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
fn nested_array_append_target_errors_before_evaluating_rhs() {
    let source = r#"
        #strict 1
        static target, calls;
        func Init() {
            target = [];
            calls = 0;
        }
        func SideEffect() {
            calls++;
            return 7;
        }
        func Fail() { target[][0] = SideEffect(); }
        func Inspect() { return [calls, target]; }
    "#;
    let mut engine = Engine::new();
    engine.set_global_variables(lc_script::new_global_variables());
    engine.load_script(source).expect("script loads");
    engine.call("Init", &[]).expect("state initialized");

    engine
        .call("Fail", &[])
        .expect_err("nested access into the new nil slot fails");
    assert_eq!(
        engine.call("Inspect", &[]).expect("state remains readable"),
        Value::Array(vec![Value::Nil, Value::Array(vec![Value::Nil])])
    );
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
