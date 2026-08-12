// Test for LocalN/Var with two arguments as assignment target

// LocalN("key", obj) = value pattern
crate::support::compile_case!(
    localn_two_args_assignment,
    r#"func Test() { var obj; LocalN("active", obj) = false; }"#
);

// LocalN(0, obj) = value pattern
crate::support::compile_case!(
    localn_two_args_with_index,
    r#"func Test() { var obj; LocalN(0, obj) = 42; }"#
);

// Var("key", obj) = value pattern
crate::support::compile_case!(
    var_two_args_assignment,
    r#"func Test() { var obj; Var("data", obj) = 123; }"#
);

// Exact pattern from WARP line 30
crate::support::compile_case!(
    warp_line_30_pattern,
    r#"func Test() { var pEnd; LocalN("active", pEnd) = false; }"#
);

// Local(index, obj) = value pattern
crate::support::compile_case!(
    local_two_args_assignment,
    r#"func Test() { var obj; Local(0, obj) = 99; }"#
);
