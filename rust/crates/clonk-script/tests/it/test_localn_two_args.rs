// Test for LocalN/Var with two arguments as assignment target

#[test]
fn localn_two_args_assignment() {
    // LocalN("key", obj) = value pattern
    let source = r#"func Test() { var obj; LocalN("active", obj) = false; }"#;
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
fn localn_two_args_with_index() {
    // LocalN(0, obj) = value pattern
    let source = r#"func Test() { var obj; LocalN(0, obj) = 42; }"#;
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
fn var_two_args_assignment() {
    // Var("key", obj) = value pattern
    let source = r#"func Test() { var obj; Var("data", obj) = 123; }"#;
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
fn warp_line_30_pattern() {
    // Exact pattern from WARP line 30
    let source = r#"func Test() { var pEnd; LocalN("active", pEnd) = false; }"#;
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
fn local_two_args_assignment() {
    // Local(index, obj) = value pattern
    let source = r#"func Test() { var obj; Local(0, obj) = 99; }"#;
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
