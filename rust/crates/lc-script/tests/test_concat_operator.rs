// Parity: `..` is the C4Script string/array/map concatenation operator.
//
// C++ oracle: AB_Concat / AB_ConcatIt (C4AulExec.cpp:594-657), priority 10 in
// C4ScriptOpMap (looser than `+`=13, tighter than `==`=9). `..` joins the string
// forms of its operands (so `5 .. 3` is "53", never 8), appends arrays, and
// merges maps. `..=` is the compound form. The Rust lexer previously rejected
// `..` as an error, so any content using it failed to parse.

use lc_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[]).expect("call succeeds")
}

#[test]
fn concat_two_strings() {
    assert_eq!(
        eval(r#"func Test() { return "Hello, " .. "World"; }"#),
        Value::String("Hello, World".into())
    );
}

#[test]
fn concat_string_and_int() {
    assert_eq!(
        eval(r#"func Test() { return "n=" .. 42; }"#),
        Value::String("n=42".into())
    );
}

#[test]
fn concat_two_ints_is_string_not_addition() {
    // 5 .. 3 == "53" (concat), not 8 (`+` would add).
    assert_eq!(eval("func Test() { return 5 .. 3; }"), Value::String("53".into()));
}

#[test]
fn concat_assign_operator() {
    assert_eq!(
        eval(r#"func Test() { var s = "a"; s ..= "b"; return s; }"#),
        Value::String("ab".into())
    );
}

#[test]
fn concat_arrays_appends() {
    assert_eq!(
        eval("func Test() { return [1, 2] .. [3]; }"),
        Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
}

#[test]
fn concat_binds_looser_than_addition() {
    // "x" .. 1 + 2  ==  "x" .. (1 + 2)  ==  "x3"
    assert_eq!(
        eval(r#"func Test() { return "x" .. 1 + 2; }"#),
        Value::String("x3".into())
    );
}

#[test]
fn ellipsis_still_lexes_as_varargs() {
    // `...` must still tokenize as the varargs forwarder, not `..` + `.`.
    let mut engine = Engine::new();
    engine
        .load_script("func Inner(a, b) { return a + b; } func Test() { return Inner(2, 3, ...); }")
        .expect("script with ... should still parse");
    assert_eq!(engine.call("Test", &[]).expect("call"), Value::Int(5));
}
