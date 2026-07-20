//! Runtime parity for typed script parameters. C4Aul checks and converts every
//! argument slot before entering the callee (`CheckConvertFunctionParameters`,
//! C4AulExec.cpp:1364-1397).

use lc_script::{Engine, Script, TypeAnnotation, Value, ValueMap};

fn eval(source: &str) -> Result<Value, lc_script::ScriptError> {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[])
}

fn runtime_message(error: lc_script::ScriptError) -> String {
    match error {
        lc_script::ScriptError::Runtime(error) => error.to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn param_with_object_type() {
    let source = r#"func Test(object pMage) { return 1; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn param_with_int_type() {
    let source = r#"func Test(int value) { return value; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn map_parameter_carries_map_annotation_and_checks_runtime_type() {
    let script = Script::compile("#strict 3\nfunc Accept(map value) { return value; }")
        .expect("map parameter compiles");
    let parameter = &script.functions()["Accept"].params[0];
    assert_eq!(parameter.name, "value");
    assert_eq!(parameter.type_annotation, Some(TypeAnnotation::Map));

    let mut engine = Engine::new();
    engine.add_script(script);
    let map = Value::Proplist(ValueMap::from([("answer", Value::Int(42))]));
    assert_eq!(
        engine
            .call("Accept", std::slice::from_ref(&map))
            .expect("a C4V_Map argument is accepted"),
        map
    );
    assert_eq!(
        engine
            .call("Accept", &[Value::Nil])
            .expect("nil converts to every non-reference C4Value type"),
        Value::Nil
    );
    assert_eq!(
        runtime_message(
            engine
                .call("Accept", &[Value::Array(Vec::new())])
                .expect_err("an array is not a map")
        ),
        r#"call to "Accept" parameter 1: got "array", but expected "map"!"#
    );
}

#[test]
fn map_parameter_type_parses_after_an_int_parameter() {
    let script = Script::compile("func F(int first, map second) { return second; }")
        .expect("map annotation compiles after an int annotation");
    let params = &script.functions()["F"].params;
    assert_eq!(params[0].type_annotation, Some(TypeAnnotation::Int));
    assert_eq!(params[1].name, "second");
    assert_eq!(params[1].type_annotation, Some(TypeAnnotation::Map));
}

#[test]
fn lone_map_type_word_remains_an_untyped_parameter_named_map() {
    let script = Script::compile("func F(map) { return map; }")
        .expect("a parameter named map still compiles");
    let parameter = &script.functions()["F"].params[0];
    assert_eq!(parameter.name, "map");
    assert_eq!(parameter.type_annotation, None);

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine
            .call("F", &[Value::Int(7)])
            .expect("the untyped parameter accepts an integer"),
        Value::Int(7)
    );
}

#[test]
fn strict2_string_argument_to_int_parameter_reports_exact_error() {
    let error = eval(
        r#"
            #strict 2
            func T(int x) { return x; }
            func Test() { return T("abc"); }
        "#,
    )
    .expect_err("string -> int must fail at the call boundary");

    assert_eq!(
        runtime_message(error),
        r#"call to "T" parameter 1: got "string", but expected "int"!"#
    );
}

#[test]
fn int_argument_is_mutated_to_id_before_the_callee_runs() {
    assert_eq!(
        eval("func T(id x) { return x; } func Test() { return T(1000); }")
            .expect("0..=9999 converts to id"),
        Value::C4Id("1000".into())
    );
}

#[test]
fn int_to_id_conversion_rejects_values_outside_the_legacy_range() {
    for value in ["-1", "10000"] {
        let error = eval(&format!(
            "func T(id x) {{ return x; }} func Test() {{ return T({value}); }}"
        ))
        .expect_err("out-of-range int must not convert to id");
        assert_eq!(
            runtime_message(error),
            r#"call to "T" parameter 1: got "int", but expected "id"!"#,
            "value {value}"
        );
    }
}

#[test]
fn pre_strict3_callers_bridge_nil_to_typed_int_and_bool_zeroes() {
    for caller_directive in ["", "#strict 2\n"] {
        let mut engine = Engine::new();
        engine.add_script(
            Script::compile(
                "#strict 3\nfunc Accept(int number, bool flag) { return [number, flag]; }",
            )
            .expect("strict-3 callee compiles"),
        );
        engine.add_script(
            Script::compile(&format!(
                "{caller_directive}func Test() {{ var number, flag; return Accept(number, flag); }}"
            ))
            .expect("caller compiles"),
        );

        assert_eq!(
            engine.call("Test", &[]).expect("bridge call succeeds"),
            Value::Array(vec![Value::Int(0), Value::Bool(false)]),
            "caller directive {caller_directive:?}"
        );
    }
}

#[test]
fn strict3_caller_leaves_nil_typed_parameters_as_nil() {
    assert_eq!(
        eval(
            r#"
                #strict 3
                func Accept(int number, bool flag) { return [number, flag]; }
                func Test() { return Accept(nil, nil); }
            "#,
        )
        .expect("nil is accepted for every non-reference C4Value type"),
        Value::Array(vec![Value::Nil, Value::Nil])
    );
}

#[test]
fn engine_entry_converts_all_ten_slots_before_named_and_par_reads() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
                #strict 3
                func Accept(id definition, int missing, bool flag) {
                    return [definition, missing, flag, Par(0), Par(1), Par(2)];
                }
            "#,
        )
        .expect("script loads");

    let id = Value::C4Id("1000".into());
    assert_eq!(
        engine
            .call("Accept", &[Value::Int(1000)])
            .expect("engine entry converts parameters"),
        Value::Array(vec![
            id.clone(),
            Value::Int(0),
            Value::Bool(false),
            id,
            Value::Int(0),
            Value::Bool(false),
        ])
    );
}

#[test]
fn pre_strict3_engine_entry_normalizes_every_raw_zero_to_nil() {
    let mut engine = Engine::new();
    engine
        .load_script("func Accept(value) { return value; }")
        .expect("script loads");

    for zero in [
        Value::Int(0),
        Value::Bool(false),
        Value::Object(0),
        Value::C4Id("NONE".into()),
        Value::C4Id("0000".into()),
    ] {
        assert_eq!(
            engine.call("Accept", &[zero]).expect("call succeeds"),
            Value::Nil
        );
    }
}

#[test]
fn caller_source_strictness_wins_over_link_destination_owner() {
    let mut destination = Engine::new();
    destination.add_script(
        Script::compile("func Accept(value) { return value; }")
            .expect("nonstrict destination compiles"),
    );

    let mut strict_source = Engine::new();
    strict_source.add_script(
        Script::compile("#strict 3\nfunc Test() { return Accept(0); }")
            .expect("strict source compiles"),
    );
    destination.merge_from(&strict_source);

    assert_eq!(
        destination.call("Test", &[]).expect("linked call succeeds"),
        Value::Int(0),
        "HasStrictNil follows pOrgScript, not the destination script owner"
    );
}
