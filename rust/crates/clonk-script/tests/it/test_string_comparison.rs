// C++ S=/eq/ne run CheckOpPars with String operands before comparing the raw
// bytes (C4AulExec.cpp:289-299,691-707; C4AulParse.cpp:456-458).

use clonk_script::{c4_string_from_bytes, Engine, ScriptError, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script loads");
    engine.call("Test", &[]).expect("script call succeeds")
}

fn eval_error(source: &str) -> String {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script loads");
    match engine
        .call("Test", &[])
        .expect_err("comparison must reject the operand")
    {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn falsy_operands_compare_as_empty_below_strict_two() {
    for expression in [
        "empty S= \"\"",
        "0 S= \"\"",
        "false S= \"\"",
        "\"\" S= \"\"",
        "empty eq \"\"",
    ] {
        assert_eq!(
            eval(&format!(
                "func Test() {{ var empty; return {expression}; }}"
            )),
            Value::Bool(true),
            "{expression}"
        );
    }

    let mut engine = Engine::new();
    engine.register_host_function("ZeroId", |_| Ok(Value::C4Id("00000".into())));
    engine
        .load_script("func Test() { return ZeroId() S= \"\"; }")
        .expect("script loads");
    assert_eq!(engine.call("Test", &[]).expect("comparison runs"), Value::Bool(true));
}

#[test]
fn string_comparison_is_case_sensitive_and_uses_raw_text() {
    assert_eq!(
        eval("func Test() { return \"a\" S= \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("func Test() { return \"a\" eq \"A\"; }"),
        Value::Bool(false)
    );
}

#[test]
fn string_comparison_stops_at_the_native_nul_terminator() {
    let left = c4_string_from_bytes(b"same\0left");
    let right = c4_string_from_bytes(b"same\0right");
    let mut engine = Engine::new();
    engine.register_host_function("Left", move |_| Ok(Value::String(left.clone().into())));
    engine.register_host_function("Right", move |_| Ok(Value::String(right.clone().into())));
    engine
        .load_script(
            "func Equal() { return Left() eq Right(); }\n\
             func NotEqual() { return Left() ne Right(); }",
        )
        .expect("script loads");

    assert_eq!(
        engine.call("Equal", &[]).expect("comparison succeeds"),
        Value::Bool(true)
    );
    assert_eq!(
        engine.call("NotEqual", &[]).expect("comparison succeeds"),
        Value::Bool(false)
    );
}

#[test]
fn truthy_non_strings_raise_operator_type_errors_below_strict_two() {
    for prefix in ["", "#strict\n"] {
        assert_eq!(
            eval_error(&format!(
                "{prefix}func Test() {{ return 5 S= \"5\"; }}"
            )),
            "operator \"S=\" left side: got \"int\", but expected \"string\"!"
        );
    }

    assert_eq!(
        eval_error("func Test() { return \"5\" S= 5; }"),
        "operator \"S=\" right side: got \"int\", but expected \"string\"!"
    );
    assert_eq!(
        eval_error("func Test() { return 5 eq \"5\"; }"),
        "operator \"eq\" left side: got \"int\", but expected \"string\"!"
    );
    assert_eq!(
        eval_error("func Test() { return \"5\" ne 5; }"),
        "operator \"ne\" right side: got \"int\", but expected \"string\"!"
    );
}

#[test]
fn strict_two_treats_adjacent_s_operators_as_identifier_s() {
    assert_eq!(
        eval("#strict 2\nfunc Test() { var S; S=1; return [S, S!=2, S<5]; }"),
        Value::Array(vec![Value::Int(1), Value::Bool(true), Value::Bool(true)])
    );
}

#[test]
fn textual_ne_compares_nil_as_empty_string() {
    assert_eq!(
        eval("func Test() { var empty; return empty ne \"x\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("func Test() { var empty; return empty ne \"\"; }"),
        Value::Bool(false)
    );
}

#[test]
fn keyword_string_equality_accepts_a_real_host_returned_string() {
    let mut engine = Engine::new();
    engine.register_host_function("GetAction", |_| Ok(Value::String("Walk".to_string().into())));
    engine
        .load_script("func Test() { return GetAction() eq \"Walk\"; }")
        .expect("script loads");

    assert_eq!(
        engine.call("Test", &[]).expect("comparison succeeds"),
        Value::Bool(true)
    );
}
