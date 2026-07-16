// Parity: `..` is the C4Script string/array/map concatenation operator.
//
// C++ oracle: AB_Concat / AB_ConcatIt (C4AulExec.cpp:594-657), priority 10 in
// C4ScriptOpMap (looser than `+`=13, tighter than `==`=9). `..` joins the string
// forms of its operands (so `5 .. 3` is "53", never 8), appends arrays, and
// merges maps. `..=` is the compound form. The Rust lexer previously rejected
// `..` as an error, so any content using it failed to parse.

use lc_script::{Engine, ScriptError, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[]).expect("call succeeds")
}

fn runtime_error(source: &str, args: &[Value]) -> String {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    match engine
        .call("Test", args)
        .expect_err("unsupported concatenation must fail")
    {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
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
fn concat_scalar_conversion_matches_cpp_to_string() {
    for (expression, expected) in [
        (r#""x" .. true"#, "x1"),
        (r#""x" .. false"#, "x0"),
        (r#""x" .. 5"#, "x5"),
        (r#""x" .. TREE"#, "xTREE"),
    ] {
        assert_eq!(
            eval(&format!(
                "#strict 3\nfunc Test() {{ return {expression}; }}"
            )),
            Value::String(expected.into())
        );
    }
    assert_eq!(
        eval(r#"#strict 3
func Test() { var value = "x"; value ..= true; return value; }"#),
        Value::String("x1".into())
    );
}

#[test]
fn concat_rejects_values_without_cpp_string_conversion() {
    for (expression, value_type) in [
        (r#""x" .. nil"#, "any"),
        (r#""x" .. [1]"#, "array"),
        (r#""x" .. {}"#, "map"),
    ] {
        assert_eq!(
            runtime_error(
                &format!("func Test() {{ return {expression}; }}"),
                &[]
            ),
            format!(
                "operator \"..\" right side: can not convert \"{value_type}\" to \"string\"!"
            )
        );
    }

    assert_eq!(
        runtime_error(r#"func Test(obj) { return "x" .. obj; }"#, &[Value::Object(7)]),
        "operator \"..\" right side: can not convert \"object\" to \"string\"!"
    );
    assert_eq!(
        runtime_error(r#"func Test(obj) { return obj .. "x"; }"#, &[Value::Object(7)]),
        "operator \"..\" left side: can not convert \"object\" to \"string\", \"array\" or \"map\"!"
    );
    assert_eq!(
        runtime_error(
            r#"#strict 3
func Test() { return "x" .. nil; }"#,
            &[]
        ),
        "operator \"..\" right side: got nil, but expected \"any\"!"
    );
    assert_eq!(
        runtime_error(
            r#"func Test() { var value = "x"; value ..= [1]; return value; }"#,
            &[]
        ),
        "operator \"..=\" right side: can not convert \"array\" to \"string\"!"
    );
    assert_eq!(
        runtime_error(
            "#strict 2\nfunc Test() { return \"x\" .. (1 == 2); }",
            &[]
        ),
        "operator \"..\" right side: can not convert \"any\" to \"string\"!"
    );
}

#[test]
fn concat_nil_left_side_reports_cpp_conversion_error() {
    assert_eq!(
        runtime_error(r#"func Test() { return nil .. "a"; }"#, &[]),
        "operator \"..\" left side: can not convert \"any\" to \"string\", \"array\" or \"map\"!"
    );
}

#[test]
fn strict3_concat_assign_checks_nil_left_side_first() {
    assert_eq!(
        runtime_error(
            r#"#strict 3
func Test() { var value; value ..= nil; return value; }"#,
            &[]
        ),
        "operator \"..=\" left side: got nil, but expected \"&\"!"
    );
}

#[test]
fn concat_two_ints_is_string_not_addition() {
    // 5 .. 3 == "53" (concat), not 8 (`+` would add).
    assert_eq!(
        eval("func Test() { return 5 .. 3; }"),
        Value::String("53".into())
    );
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
fn concat_maps_merges_with_right_side_winning() {
    assert_eq!(
        eval(
            "func Test() { var merged = { a = 1, b = 2 } .. { b = 3, c = 4 }; return [merged.a, merged.b, merged.c]; }"
        ),
        Value::Array(vec![Value::Int(1), Value::Int(3), Value::Int(4)])
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
