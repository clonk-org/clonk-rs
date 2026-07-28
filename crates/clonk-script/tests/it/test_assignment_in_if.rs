// Test for assignment in if-condition

#[test]
fn assignment_in_if_condition() {
    let source = r#"func Test() { var x; if (x = 5) return x; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn assignment_in_if_with_block() {
    let source = r#"func Test() { var obj; if (obj = FindObj()) { } }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn fbrg_line_11_pattern() {
    let source = r#"func Test() { var iChkEff; if (iChkEff = CheckEffect("Test", 0, 150)) return (iChkEff!=-1 && RemoveObject()); }"#;
    crate::support::assert_compiles(source);
}
