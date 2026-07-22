// Test for return with empty parentheses and space

#[test]
fn return_empty_parens_with_space() {
    // return ();
    let source = r#"func Test() { return (); }"#;
    let result = clonk_script::Script::compile(source);
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
fn return_empty_parens_with_space_in_if() {
    // if condition with return ();
    let source = r#"func Test() { var x; if (x == 1) return (); }"#;
    let result = clonk_script::Script::compile(source);
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
fn return_empty_parens_with_space_nested_if() {
    // Nested if with return ();
    let source = r#"func Test() { var x; if (x) if (x == 2) return (); }"#;
    let result = clonk_script::Script::compile(source);
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
fn oilp_line_8_pattern() {
    // Exact pattern from OILP line 8
    let source = r#"func Test() { var OilCnt; if (OilCnt == -1) return (); }"#;
    let result = clonk_script::Script::compile(source);
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
fn oilp_line_9_pattern() {
    // Exact pattern from OILP line 9
    let source = r#"func Test() { var OilCnt; if (OilCnt >= 100) return (); }"#;
    let result = clonk_script::Script::compile(source);
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
fn return_with_space_vs_without_space() {
    // Both should work
    let source1 = r#"func Test1() { return(); }"#;
    let source2 = r#"func Test2() { return (); }"#;

    let result1 = clonk_script::Script::compile(source1);
    let result2 = clonk_script::Script::compile(source2);

    if let Err(e) = &result1 {
        eprintln!(
            "Error in source1: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    if let Err(e) = &result2 {
        eprintln!(
            "Error in source2: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }

    assert!(result1.is_ok());
    assert!(result2.is_ok());
}

#[test]
fn return_with_space_and_expression_after() {
    // Make sure we don't break: return (expr) + other
    let source = r#"func Test() { return (42) + 10; }"#;
    let result = clonk_script::Script::compile(source);
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
fn multiple_return_empty_with_space() {
    // Multiple return () in same function
    let source = r#"
    func Test() {
        var x;
        if (x == 1) return ();
        if (x == 2) return ();
        return ();
    }"#;
    let result = clonk_script::Script::compile(source);
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
