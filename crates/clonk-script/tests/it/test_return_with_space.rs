// Test for return with empty parentheses and space

// return ();
crate::support::compile_case!(
    return_empty_parens_with_space,
    r#"func Test() { return (); }"#
);

// if condition with return ();
crate::support::compile_case!(
    return_empty_parens_with_space_in_if,
    r#"func Test() { var x; if (x == 1) return (); }"#
);

// Nested if with return ();
crate::support::compile_case!(
    return_empty_parens_with_space_nested_if,
    r#"func Test() { var x; if (x) if (x == 2) return (); }"#
);

// Exact pattern from OILP line 8
crate::support::compile_case!(
    oilp_line_8_pattern,
    r#"func Test() { var OilCnt; if (OilCnt == -1) return (); }"#
);

// Exact pattern from OILP line 9
crate::support::compile_case!(
    oilp_line_9_pattern,
    r#"func Test() { var OilCnt; if (OilCnt >= 100) return (); }"#
);

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

// Make sure we don't break: return (expr) + other
crate::support::compile_case!(
    return_with_space_and_expression_after,
    r#"func Test() { return (42) + 10; }"#
);

// Multiple return () in same function
crate::support::compile_case!(
    multiple_return_empty_with_space,
    r#"
    func Test() {
        var x;
        if (x == 1) return ();
        if (x == 2) return ();
        return ();
    }"#,
);
