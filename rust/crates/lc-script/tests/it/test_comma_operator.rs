// C4Script has no generic comma operator. Commas remain delimiters, including
// the legacy pre-STRICT2 `return(first, unused...)` compatibility form.

use lc_script::{Engine, Script, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

fn assert_function_quarantined(source: &str, function: &str) {
    let script = Script::compile(source).expect("recoverable parse error keeps the script");
    assert!(
        !script.parse_diagnostics().is_empty(),
        "expected a parse diagnostic for {source:?}"
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    let error = engine
        .call(function, &[])
        .expect_err("invalid comma expression must quarantine its function");
    assert!(error.to_string().contains("parse error"));
}

#[test]
fn legacy_adjacent_return_parentheses_returns_first_and_evaluates_the_rest() {
    let side_effects = Arc::new(AtomicUsize::new(0));
    let observed_side_effects = Arc::clone(&side_effects);
    let mut engine = Engine::new();
    engine.register_host_function("SideEffect", move |_| {
        observed_side_effects.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(42))
    });
    engine
        .load_script("#strict\nfunc Probe() { return(0, SideEffect()); }")
        .expect("legacy adjacent return syntax compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("Probe executes"),
        Value::Nil,
        "pre-#strict-2 return(first, unused...) returns its first value"
    );
    assert_eq!(
        side_effects.load(Ordering::SeqCst),
        1,
        "legacy unused return parameters still execute for side effects"
    );
}

#[test]
fn legacy_spaced_return_parentheses_returns_first_and_evaluates_the_rest() {
    let evaluation_order = Arc::new(Mutex::new(Vec::new()));
    let observed_order = Arc::clone(&evaluation_order);
    let mut engine = Engine::new();
    engine.register_host_function("Mark", move |args| {
        let Some(Value::Int(marker)) = args.first() else {
            panic!("Mark requires one integer")
        };
        observed_order.lock().expect("order lock").push(*marker);
        Ok(Value::Int(*marker))
    });
    engine
        .load_script(
            "#strict\n\
             func One() { return (Mark(1), Mark(2)); }\n\
             func Zero() { return (0, Mark(7)); }",
        )
        .expect("legacy spaced return syntax compiles");

    assert_eq!(
        engine.call("One", &[]).expect("One executes"),
        Value::Int(1),
        "tokenizer whitespace must not disable the legacy return-parameter path"
    );
    assert_eq!(
        *evaluation_order.lock().expect("order lock"),
        vec![1, 2],
        "both operands execute from left to right"
    );

    evaluation_order.lock().expect("order lock").clear();
    assert_eq!(
        engine.call("Zero", &[]).expect("Zero executes"),
        Value::Nil,
        "the first falsy value remains the return value"
    );
    assert_eq!(
        *evaluation_order.lock().expect("order lock"),
        vec![7],
        "the unused return parameter still executes exactly once"
    );
}

#[test]
fn comma_nested_inside_a_call_does_not_trigger_legacy_return_parameters() {
    let mut engine = Engine::new();
    engine.register_host_function("Second", |args| {
        Ok(args.get(1).cloned().unwrap_or(Value::Nil))
    });
    engine
        .load_script("#strict\nfunc Probe() { return (Second(1, 2)); }")
        .expect("nested call comma compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("Probe executes"),
        Value::Int(2),
        "only commas directly inside the return parentheses are legacy parameters"
    );
}

#[test]
fn strict2_does_not_enter_the_legacy_multi_parameter_return_path() {
    assert_function_quarantined(
        "#strict 2\nfunc Probe() { return(0, 42); }",
        "Probe",
    );
}

#[test]
fn mgsm_line_24_pattern() {
    // Exact pattern from MGSM line 24
    let source = r#"func Test() { if (!SetAction("Wait")) return (0, RemoveObject()); }"#;
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
fn nonstrict_spaced_return_parentheses_return_the_first_value() {
    let mut engine = Engine::new();
    engine
        .load_script("func Test() { return (1, 2); }")
        .expect("nonstrict legacy return syntax loads");

    assert_eq!(
        engine.call("Test", &[]).expect("Test executes"),
        Value::Int(1),
        "nonstrict spaced return parameters keep the first value"
    );
}

#[test]
fn comma_with_three_expressions() {
    // return (expr1, expr2, expr3)
    let source = r#"func Test() { return (1, 2, 3); }"#;
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
fn comma_with_function_calls() {
    // return (1, Message(...), Sound(...))
    let source = r#"func Test() { return (1, Message("test"), Sound("Click")); }"#;
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
fn comma_with_assignment() {
    // return (1, var = 0)
    let source = r#"func Test() { var x; return (1, x = 42); }"#;
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
fn generic_comma_expressions_in_assignments_are_rejected() {
    assert_function_quarantined("func Test() { var x; x = (1, 2); }", "Test");
    assert_function_quarantined("func Test() { var x = (0, 42); }", "Test");
}

#[test]
fn comma_in_if_condition() {
    assert_function_quarantined(
        "func Test() { var x; if ((x = 5, x > 0)) return 1; }",
        "Test",
    );
}

#[test]
fn comma_in_while_condition() {
    assert_function_quarantined(
        "func Test() { var x; while ((x = x + 1, x < 10)) {} }",
        "Test",
    );
}

#[test]
fn nested_comma_expressions() {
    // The outer comma is the legacy return delimiter; the nested comma is
    // still an invalid generic expression.
    assert_function_quarantined("func Test() { return (1, (2, 3)); }", "Test");
}

#[test]
fn lock_pattern() {
    // Pattern from Lock.c4d scripts
    let source = r#"func Test() { return (1, Message("test"), Sound("Error")); }"#;
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
fn kingdoms_pattern() {
    // Pattern from Kingdoms scripts
    let source = r#"func Test() { var clonk; if (!clonk) return (0, RemoveObject()); }"#;
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
fn comma_in_var_decl_without_parens_is_rejected_like_cpp() {
    // C4Script has NO comma operator (it is absent from C4ScriptOpMap,
    // src/C4AulParse.cpp:423). Inside a `var` declaration the comma is a
    // *declarator separator* (`var a = 1, b = 2;` declares two variables), so
    // C++ `Parse_Var` (src/C4AulParse.cpp:3252) parses the initializer with
    // `Parse_Expression()` — which stops at the comma — and then expects another
    // variable NAME. `var x = 1, 2;` therefore fails in C++ ("variable name"
    // expected, finding the int `2`). The Rust port must reject it identically.
    let rejected = lc_script::Script::compile(r#"func Test() { var x = 1, 2; }"#)
        .expect("the invalid function body is quarantined instead of aborting the script");
    assert!(
        !rejected.parse_diagnostics().is_empty(),
        "unparenthesized comma in a var declaration must produce a parse diagnostic"
    );
    let mut engine = Engine::new();
    engine.add_script(rejected);
    let error = engine
        .call("Test", &[])
        .expect_err("calling the quarantined function must surface its parse error");
    assert!(error.to_string().contains("parse error"));

    // The standard multi-declarator form must keep compiling: here the comma is
    // a declarator separator (`var a = 1` then `b = 2`), which C++ Parse_Var
    // supports directly.
    assert!(
        lc_script::Script::compile(r#"func Test() { var a = 1, b = 2; return a + b; }"#).is_ok(),
        "standard multi-declarator var should compile (comma is a declarator separator)"
    );
}
