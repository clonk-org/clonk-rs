// C++ S=/eq/ne run CheckOpPars with String operands before comparing the raw
// bytes (C4AulExec.cpp:289-299,691-707; C4AulParse.cpp:456-458).

use lc_script::{Engine, ScriptError, Value};

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
fn falsy_operands_compare_as_empty_below_strict_three() {
    for expression in [
        "nil S= \"\"",
        "0 S= \"\"",
        "false S= \"\"",
        "\"\" S= \"\"",
        "nil eq \"\"",
    ] {
        assert_eq!(
            eval(&format!("func Test() {{ return {expression}; }}")),
            Value::Bool(true),
            "{expression}"
        );
    }
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
fn truthy_non_strings_raise_operator_type_errors_at_every_strict_level() {
    for prefix in ["", "#strict 1\n", "#strict 2\n", "#strict 3\n"] {
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
fn strict_three_keeps_typed_zero_and_false_but_nil_is_still_empty() {
    assert_eq!(
        eval("#strict 3\nfunc Test() { return nil S= \"\"; }"),
        Value::Bool(true)
    );
    assert!(eval_error("#strict 3\nfunc Test() { return 0 S= \"\"; }")
        .contains("got \"int\""));
    assert!(eval_error("#strict 3\nfunc Test() { return false S= \"\"; }")
        .contains("got \"bool\""));
}

#[test]
fn textual_ne_compares_nil_as_empty_string() {
    assert_eq!(
        eval("func Test() { return nil ne \"x\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("func Test() { return nil ne \"\"; }"),
        Value::Bool(false)
    );
}

#[test]
fn keyword_string_equality_accepts_a_real_host_returned_string() {
    let mut engine = Engine::new();
    engine.register_host_function("GetAction", |_| Ok(Value::String("Walk".to_string())));
    engine
        .load_script("func Test() { return GetAction() eq \"Walk\"; }")
        .expect("script loads");

    assert_eq!(
        engine.call("Test", &[]).expect("comparison succeeds"),
        Value::Bool(true)
    );
}
