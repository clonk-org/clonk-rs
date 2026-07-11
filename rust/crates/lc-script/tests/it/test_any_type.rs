// Test for 'any' type annotation support

#[test]
fn indi_line_818_pattern() {
    // Exact pattern from INDI line 818
    let source = r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { }"#;
    let result = lc_script::Script::compile(source);
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
fn any_type_first_param() {
    // func Test(any x)
    let source = r#"func Test(any x) { }"#;
    let result = lc_script::Script::compile(source);
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
fn any_type_middle_param() {
    // func Test(int x, any y, string z)
    let source = r#"func Test(int x, any y, string z) { }"#;
    let result = lc_script::Script::compile(source);
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
fn multiple_any_params() {
    // func Test(any x, any y)
    let source = r#"func Test(any x, any y) { }"#;
    let result = lc_script::Script::compile(source);
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
fn jungle_clonk_pattern() {
    // Pattern from JungleClonk, Inuk, Trapper (same as INDI)
    let source = r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { return(1); }"#;
    let result = lc_script::Script::compile(source);
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
fn any_with_other_types() {
    // Mix of all types including any
    let source = r#"func Test(int a, bool b, string c, object d, id e, array f, proplist g, effect h, any i) { }"#;
    let result = lc_script::Script::compile(source);
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
fn existing_types_still_work() {
    // Regression test: ensure existing type annotations work
    let source = r#"func Test(int x, bool y, string z, object obj) { }"#;
    let result = lc_script::Script::compile(source);
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
