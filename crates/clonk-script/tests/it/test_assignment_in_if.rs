// Test for assignment in if-condition

crate::support::compile_case!(
    assignment_in_if_condition,
    r#"func Test() { var x; if (x = 5) return x; }"#
);

crate::support::compile_case!(
    assignment_in_if_with_block,
    r#"func Test() { var obj; if (obj = FindObj()) { } }"#
);

crate::support::compile_case!(
    fbrg_line_11_pattern,
    r#"func Test() { var iChkEff; if (iChkEff = CheckEffect("Test", 0, 150)) return (iChkEff!=-1 && RemoveObject()); }"#
);
