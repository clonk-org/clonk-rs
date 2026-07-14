// Test for assignment expressions as operands to unary NOT operator

#[test]
fn dynb_line_57_pattern() {
    // Exact pattern from DYNB line 57: if(!iCount = GetComponent(...))
    let source = r#"
        func Test() {
            var iCount;
            if(!iCount = GetComponent()) {
                return 1;
            }
        }
    "#;
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
fn not_with_assignment() {
    // Simple: !x = y should parse as !(x = y)
    let source = r#"func Test() { var x; if(!x = 42) return 1; }"#;
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
fn not_with_addition_preserved() {
    // Precedence preservation: !a + b should still be (!a) + b, not !(a + b)
    let source = r#"func Test() { var a = 1; var b = 2; return !a + b; }"#;
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
fn not_with_function_call() {
    // Baseline: !func() should continue to work
    let source = r#"func Test() { return !GetFlag(); }"#;
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
fn not_with_parenthesized_assignment() {
    // Control: !(x = y) with explicit parens should work
    let source = r#"func Test() { var x; if(!(x = 42)) return 1; }"#;
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
fn increment_not_affected() {
    // ++x = y should still be invalid (pre-increment doesn't return lvalue in this context)
    // This ensures our fix doesn't break increment/decrement behavior
    let source = r#"func Test() { var x; ++x = 42; }"#;
    let script = lc_script::Script::compile(source)
        .expect("the invalid function body is quarantined instead of aborting the script");
    assert!(!script.parse_diagnostics().is_empty());

    let mut engine = lc_script::Engine::new();
    engine.add_script(script);
    let error = engine
        .call("Test", &[])
        .expect_err("calling the quarantined function must surface its parse error");
    assert!(error.to_string().contains("parse error"));
}

#[test]
fn not_with_complex_assignment() {
    // Complex RHS: !x = a + b
    let source = r#"func Test() { var x, a = 1, b = 2; if(!x = a + b) return 1; }"#;
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
fn bitwise_not_precedence_preserved() {
    // Ensure ~ (bitwise NOT) still has normal precedence with addition
    let source = r#"func Test() { var a = 5; return ~a + 1; }"#;
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
fn negate_precedence_preserved() {
    // Ensure - (unary negate) still has normal precedence with addition
    let source = r#"func Test() { var a = 5; return -a + 1; }"#;
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
