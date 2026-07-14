// Test for comma operator support

use lc_script::{Engine, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

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
        Value::Int(0),
        "pre-#strict-2 return(first, unused...) returns its first value"
    );
    assert_eq!(
        side_effects.load(Ordering::SeqCst),
        1,
        "legacy unused return parameters still execute for side effects"
    );
}

#[test]
fn strict2_adjacent_return_parentheses_uses_the_normal_comma_operator() {
    let mut engine = Engine::new();
    engine
        .load_script("#strict 2\nfunc Probe() { return(0, 42); }")
        .expect("strict-2 comma expression compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("Probe executes"),
        Value::Int(42),
        "#strict 2 has no legacy multi-parameter return hack"
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
fn simple_comma_in_return() {
    // return (expr1, expr2)
    let source = r#"func Test() { return (0, 42); }"#;
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
fn comma_in_variable_initializer() {
    // var x = (expr1, expr2)
    let source = r#"func Test() { var x = (0, 42); }"#;
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
fn comma_in_if_condition() {
    // if ((expr1, expr2))
    let source = r#"func Test() { var x; if ((x = 5, x > 0)) return 1; }"#;
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
fn comma_in_while_condition() {
    // while ((expr1, expr2))
    let source = r#"func Test() { var x; while ((x = x + 1, x < 10)) {} }"#;
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
fn nested_comma_expressions() {
    // (a, (b, c))
    let source = r#"func Test() { return (1, (2, 3)); }"#;
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

    // NOTE (pre-existing parity divergence, see PORT_STATUS.md): the Rust port
    // currently ALSO accepts a parenthesized comma sequence such as
    // `var x = (1, 2)`. C++ does NOT — its `(...)` parser (C4AulParse.cpp:2933)
    // reads exactly one expression then matches `)`. In C++ a comma-sequence is
    // only legal inside a `return (...)` statement (the `multi_params_hack`,
    // C4AulParse.cpp:2069). So `var x = (1, 2)` should eventually be rejected for
    // parity; it is not asserted here to avoid pinning the divergence as correct.
}
