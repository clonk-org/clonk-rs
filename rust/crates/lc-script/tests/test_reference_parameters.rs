// Test for reference parameters (&param)

#[test]
fn simple_reference_parameter() {
    let source = r#"func SetValues(&x, &y) { x = 10; y = 20; }"#;
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
fn reference_with_type_annotation() {
    let source = r#"func SetValues(int &x, int &y) { x = 10; y = 20; }"#;
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
fn mixed_reference_and_value_parameters() {
    let source = r#"func GetSum(a, b, &result) { result = a + b; }"#;
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
fn mgwp_pattern() {
    // The actual pattern from MGWP script
    let source = r#"private func GetWarpPosition(&x, &y) { x = 10; y = 20; }"#;
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
fn reference_parameter_with_object_type() {
    let source = r#"func GetObject(object &obj) { obj = FindObject(CLNK); }"#;
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
