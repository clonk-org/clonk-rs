// Test for return with empty parentheses and space

// return ();
crate::support::compile_cases! {
    return_empty_parens_with_space: r#"func Test() { return (); }"#;

// if condition with return ();
    return_empty_parens_with_space_in_if: r#"func Test() { var x; if (x == 1) return (); }"#;

// Nested if with return ();
    return_empty_parens_with_space_nested_if: r#"func Test() { var x; if (x) if (x == 2) return (); }"#;

// Exact pattern from OILP line 8
    oilp_line_8_pattern: r#"func Test() { var OilCnt; if (OilCnt == -1) return (); }"#;

// Exact pattern from OILP line 9
    oilp_line_9_pattern: r#"func Test() { var OilCnt; if (OilCnt >= 100) return (); }"#;
}

#[test]
fn return_with_space_vs_without_space() {
    // Both should work
    crate::support::assert_compiles(r#"func Test1() { return(); }"#);
    crate::support::assert_compiles(r#"func Test2() { return (); }"#);
}

// Make sure we don't break: return (expr) + other
crate::support::compile_cases! {
    return_with_space_and_expression_after: r#"func Test() { return (42) + 10; }"#;

// Multiple return () in same function
    multiple_return_empty_with_space:
    r#"
    func Test() {
        var x;
        if (x == 1) return ();
        if (x == 2) return ();
        return ();
    }"#;
}
