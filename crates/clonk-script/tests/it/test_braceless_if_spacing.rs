// Test for specific spacing patterns in brace-less if

// Note the space before the closing paren: "() )"
crate::support::compile_case!(
    if_with_space_before_closing_paren,
    r#"func Test() { var dir; if(dir == GetValue() ) dir = -1; }"#
);

// Exact pattern: function call with () followed by space and )
crate::support::compile_case!(
    if_with_double_parens_and_space,
    r#"func Test() { var x; if(x == Func() ) x = 1; }"#
);

// Line 55 pattern: x = y = -1;
crate::support::compile_case!(
    chained_assignment_in_if_body,
    r#"func Test() { var x, y; if(1) x = y = -1; }"#
);

// Most exact reproduction of line 34
crate::support::compile_case!(
    dir_left_exact_pattern,
    r#"
func Test() {
  var dir;
  if(dir == DIR_Left() ) dir = -1;
}
"#,
);
