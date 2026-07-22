// Test for assignment in if-condition

#[test]
fn assignment_in_if_condition() {
    let source = r#"func Test() { var x; if (x = 5) return x; }"#;
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
fn assignment_in_if_with_block() {
    let source = r#"func Test() { var obj; if (obj = FindObj()) { } }"#;
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
fn fbrg_line_11_pattern() {
    let source = r#"func Test() { var iChkEff; if (iChkEff = CheckEffect("Test", 0, 150)) return (iChkEff!=-1 && RemoveObject()); }"#;
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
