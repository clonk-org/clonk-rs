// Test for brace-less if/while with assignment in then/body

crate::support::compile_case!(
    braceless_if_with_assignment,
    r#"func Test() { var dir; if(dir == 0) dir = -1; }"#
);

// WARP pattern: if(dir == DIR_Left() ) dir = -1;
crate::support::compile_case!(
    braceless_if_with_function_call_then_assignment,
    r#"func Test() { var dir; if(dir == GetDir()) dir = -1; }"#
);

crate::support::compile_case!(
    braceless_while_with_assignment,
    r#"func Test() { var i = 0; while(i < 10) i = i + 1; }"#
);

// Exact pattern from WARP script line 34
crate::support::compile_case!(
    warp_exact_pattern,
    r#"func Test() { var dir; if(dir == DIR_Left() ) dir = -1; }"#
);

crate::support::compile_case!(
    braceless_if_with_compound_assignment,
    r#"func Test() { var x = 5; if(x > 0) x += 10; }"#
);
