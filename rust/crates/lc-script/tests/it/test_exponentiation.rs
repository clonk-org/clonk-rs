// Test for exponentiation operator (**)

use lc_script::{Engine, Value};

fn eval(expression: &str) -> Value {
    let mut engine = Engine::new();
    engine
        .load_script(&format!("func Test() {{ return {expression}; }}"))
        .expect("exponentiation script loads");
    engine.call("Test", &[]).expect("script call succeeds")
}

#[test]
fn simple_exponentiation() {
    let source = r#"func Test() { return 2**3; }"#;
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
fn exponentiation_is_left_associative_like_cpp() {
    // C4Aul's operator table marks ** as non-right-associative, so the
    // second operator closes the first expression: (2**3)**2 = 8**2 = 64.
    assert_eq!(eval("2**3**2"), Value::Int(64));
}

#[test]
fn exponentiation_with_parentheses() {
    assert_eq!(eval("2**(3**2)"), Value::Int(512));
}

#[test]
fn exponentiation_with_variable() {
    let source = r#"func Test() { var iAlpha = 2; return iAlpha**5; }"#;
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
fn exponentiation_with_negative_base() {
    let source = r#"func Test() { return (-2)**3; }"#;
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
fn frca_pattern() {
    // The actual pattern from FRCA script: Sqrt(Sqrt(iAlpha**5))
    let source = r#"func Test() { var iAlpha; return Sqrt(Sqrt(iAlpha**5)); }"#;
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
fn exponentiation_precedence_higher_than_multiply() {
    assert_eq!(eval("2**3*2"), Value::Int(16));
    assert_eq!(eval("2*3**2"), Value::Int(18));
}

#[test]
fn unary_precedence_is_higher_than_exponentiation() {
    assert_eq!(eval("-2**2"), Value::Int(4));
}
