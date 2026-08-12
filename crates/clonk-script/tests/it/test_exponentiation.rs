// Test for exponentiation operator (**)

use clonk_script::{Engine, ScriptError, Value};

fn eval(expression: &str) -> Value {
    let mut engine = Engine::new();
    engine
        .load_script(&format!("func Test() {{ return {expression}; }}"))
        .expect("exponentiation script loads");
    engine.call("Test", &[]).expect("script call succeeds")
}

fn eval_strict1(expression: &str) -> Value {
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            "#strict\nfunc Test() {{ var empty; return {expression}; }}"
        ))
        .expect("STRICT1 exponentiation script loads");
    engine.call("Test", &[]).expect("script call succeeds")
}

fn runtime_error(expression: &str) -> String {
    let mut engine = Engine::new();
    engine
        .load_script(&format!("func Test() {{ return {expression}; }}"))
        .expect("exponentiation script loads");
    match engine
        .call("Test", &[])
        .expect_err("non-integer exponentiation operand must fail")
    {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

crate::support::compile_case!(simple_exponentiation, r#"func Test() { return 2**3; }"#);

#[test]
fn exponentiation_is_left_associative_like_cpp() {
    // C4Aul's operator table marks ** as non-right-associative, so the
    // second operator closes the first expression: (2**3)**2 = 8**2 = 64.
    assert_eq!(eval("2**3**2"), Value::Int(64));
}

#[test]
fn exponentiation_with_parentheses() {
    assert_eq!(eval("2**(3**2)"), Value::Int(512));
}

crate::support::compile_case!(
    exponentiation_with_variable,
    r#"func Test() { var iAlpha = 2; return iAlpha**5; }"#
);

crate::support::compile_case!(
    exponentiation_with_negative_base,
    r#"func Test() { return (-2)**3; }"#
);

// The actual pattern from FRCA script: Sqrt(Sqrt(iAlpha**5))
crate::support::compile_case!(
    frca_pattern,
    r#"func Test() { var iAlpha; return Sqrt(Sqrt(iAlpha**5)); }"#
);

#[test]
fn exponentiation_precedence_higher_than_multiply() {
    assert_eq!(eval("2**3*2"), Value::Int(16));
    assert_eq!(eval("2*3**2"), Value::Int(18));
}

#[test]
fn unary_precedence_is_higher_than_exponentiation() {
    assert_eq!(eval("-2**2"), Value::Int(4));
}

#[test]
fn exponentiation_edge_semantics_match_cpp() {
    assert_eq!(
        eval_strict1("[2 ** -1, empty ** 3, 2 ** empty, true ** true, 2 ** 40, 10 ** 10]",),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(0),
            Value::Int(1),
            Value::Int(1),
            Value::Int(0),
            Value::Int(1_410_065_408),
        ])
    );
}

#[test]
fn exponentiation_rejects_non_coercible_operands() {
    assert_eq!(
        runtime_error(r#""a" ** 2"#),
        "cannot apply '**' to operands of type string and int"
    );
}

#[test]
fn exponentiation_bool_coercion_and_overflow_match_cpp() {
    assert_eq!(
        eval_strict1("[5 ** -2, true ** 3, (1 << 30) ** 2]"),
        Value::Array(vec![Value::Int(0), Value::Int(1), Value::Int(0)])
    );
}

#[test]
fn exponentiation_compound_assignment_updates_the_retained_reference() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"#strict 3
func Test() {
    var values = [2], index = 0;
    var result = (values[index++] **= 3);
    return [values[0], index, result];
}
"#,
        )
        .expect("power-assignment script loads");
    assert_eq!(
        engine.call("Test", &[]).expect("power assignment runs"),
        Value::Array(vec![Value::Int(8), Value::Int(1), Value::Int(8)])
    );
}

#[test]
fn shift_counts_are_masked_like_cpp() {
    assert_eq!(
        eval_strict1("[1 << 32, 64 >> 32, 1 << -1, 64 >> -1, -8 >> 33]"),
        Value::Array(vec![
            Value::Int(1),
            Value::Int(64),
            Value::Int(i32::MIN),
            Value::Int(0),
            Value::Int(-4),
        ])
    );
}
