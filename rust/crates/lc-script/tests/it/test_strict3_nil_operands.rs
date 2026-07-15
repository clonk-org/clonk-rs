//! STRICT3 nil-operand checks for C4Aul's numeric binary and compound
//! operators. The one-operand `CheckOpPar` wrapper accidentally keeps its
//! default `allowAny=true`, so unary `-`/`~` and `++`/`--` deliberately retain
//! the legacy nil-to-zero coercion even at STRICT3.

use lc_script::{Engine, ScriptError, Value};

fn call(source: &str) -> Result<Value, ScriptError> {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script loads");
    engine.call("Test", &[])
}

fn runtime_error(source: &str) -> String {
    match call(source).expect_err("nil operand must raise a runtime error") {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn strict3_nil_binary_operator_families_error() {
    let cases = [
        (
            "var a; return a + 1;",
            "operator \"+\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return 1 + a;",
            "operator \"+\" right side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a ** 2;",
            "operator \"**\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a - 1;",
            "operator \"-\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a * 1;",
            "operator \"*\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a / 1;",
            "operator \"/\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a % 1;",
            "operator \"%\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a << 1;",
            "operator \"<<\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a >> 1;",
            "operator \">>\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a & 1;",
            "operator \"&\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a | 1;",
            "operator \"|\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a ^ 1;",
            "operator \"^\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a < 5;",
            "operator \"<\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a <= 5;",
            "operator \"<=\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a > 5;",
            "operator \">\" left side: got nil, but expected \"int\"!",
        ),
        (
            "var a; return a >= 5;",
            "operator \">=\" left side: got nil, but expected \"int\"!",
        ),
    ];

    for (body, expected) in cases {
        let source = format!("#strict 3\nfunc Test() {{ {body} }}");
        assert_eq!(runtime_error(&source), expected, "{body}");
    }
}

#[test]
fn strict3_nil_supported_compound_assignments_error() {
    let cases = [
        (
            "var a; a += 1; return a;",
            "left side: got nil, but expected \"int\"!",
        ),
        (
            "var a = 1, b; a += b; return a;",
            "right side: got nil, but expected \"int\"!",
        ),
        (
            "var a; a <<= 1; return a;",
            "left side: got nil, but expected \"int\"!",
        ),
    ];

    for (body, expected) in cases {
        let source = format!("#strict 3\nfunc Test() {{ {body} }}");
        let error = runtime_error(&source);
        assert!(error.contains(expected), "{body}: {error}");
    }
}

#[test]
fn below_strict3_nil_numeric_operators_coerce_to_zero() {
    let expected = Value::Array(vec![
        Value::Int(1),
        Value::Bool(true),
        Value::Int(0),
        Value::Int(4),
        Value::Int(0),
        Value::Int(-1),
        Value::Int(0),
        Value::Int(1),
        Value::Int(2),
    ]);

    for strict_prefix in ["", "#strict 1\n", "#strict 2\n"] {
        let source = format!(
            r#"{strict_prefix}
            func Test() {{
                var add, comparison, shift, bit, neg, invert;
                var increment, compound;
                var old_increment = increment++;
                compound += 2;
                return [add + 1, comparison < 5, shift << 1,
                        bit | 4, -neg, ~invert, old_increment, increment, compound];
            }}
            "#
        );
        assert_eq!(call(&source).expect("legacy coercions succeed"), expected);
    }
}

#[test]
fn strict3_nil_tolerant_operators_remain_tolerant() {
    let source = r#"
        #strict 3
        func Test() {
            var value, assigned;
            assigned ??= 4;
            return [value == nil, value != nil, value S= "", value eq "",
                    value ?? 7, value && 8, value || 9, !value, assigned];
        }
    "#;

    assert_eq!(
        call(source).expect("nil-tolerant operators succeed"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
            Value::Int(7),
            Value::Nil,
            Value::Int(9),
            Value::Bool(true),
            Value::Int(4),
        ])
    );
}

#[test]
fn strict3_nil_unary_and_counters_keep_cpp_wrapper_coercion() {
    let source = r#"
        #strict 3
        func Test() {
            var neg, invert, post_inc, post_dec, pre_inc, pre_dec;
            var old_inc = post_inc++;
            var old_dec = post_dec--;
            return [-neg, ~invert, old_inc, post_inc, old_dec, post_dec,
                    ++pre_inc, --pre_dec];
        }
    "#;

    assert_eq!(
        call(source).expect("the C++ unary wrapper permits nil"),
        Value::Array(vec![
            Value::Int(0),
            Value::Int(-1),
            Value::Int(0),
            Value::Int(1),
            Value::Int(0),
            Value::Int(-1),
            Value::Int(1),
            Value::Int(-1),
        ])
    );
}
