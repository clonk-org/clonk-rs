// Test for brace-less if/while with assignment in then/body

#[test]
fn braceless_if_with_assignment() {
    let source = r#"func Test() { var dir; if(dir == 0) dir = -1; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn braceless_if_with_function_call_then_assignment() {
    // WARP pattern: if(dir == DIR_Left() ) dir = -1;
    let source = r#"func Test() { var dir; if(dir == GetDir()) dir = -1; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn braceless_while_with_assignment() {
    let source = r#"func Test() { var i = 0; while(i < 10) i = i + 1; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn warp_exact_pattern() {
    // Exact pattern from WARP script line 34
    let source = r#"func Test() { var dir; if(dir == DIR_Left() ) dir = -1; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn braceless_if_with_compound_assignment() {
    let source = r#"func Test() { var x = 5; if(x > 0) x += 10; }"#;
    crate::support::assert_compiles(source);
}
