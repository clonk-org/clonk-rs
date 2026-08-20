// Test for assignment in if-condition

crate::support::compile_cases! {
    assignment_in_if_condition: r#"func Test() { var x; if (x = 5) return x; }"#;
    assignment_in_if_with_block: r#"func Test() { var obj; if (obj = FindObj()) { } }"#;
    fbrg_line_11_pattern: r#"func Test() { var iChkEff; if (iChkEff = CheckEffect("Test", 0, 150)) return (iChkEff!=-1 && RemoveObject()); }"#;
}
