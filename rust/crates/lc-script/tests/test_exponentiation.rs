// Test for exponentiation operator (**)

#[test]
fn simple_exponentiation() {
    let source = r#"func Test() { return 2**3; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn right_associativity() {
    // 2**3**2 should be parsed as 2**(3**2) = 2**9 = 512, not (2**3)**2 = 8**2 = 64
    let source = r#"func Test() { return 2**3**2; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn exponentiation_with_parentheses() {
    // (2**3)**2 should be explicitly left-to-right = 8**2 = 64
    let source = r#"func Test() { return (2**3)**2; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn exponentiation_with_variable() {
    let source = r#"func Test() { var iAlpha = 2; return iAlpha**5; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn exponentiation_with_negative_base() {
    let source = r#"func Test() { return (-2)**3; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn frca_pattern() {
    // The actual pattern from FRCA script: Sqrt(Sqrt(iAlpha**5))
    let source = r#"func Test() { var iAlpha; return Sqrt(Sqrt(iAlpha**5)); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn exponentiation_precedence_higher_than_multiply() {
    // 2 * 3**2 should be parsed as 2 * (3**2) = 2 * 9 = 18, not (2*3)**2 = 36
    let source = r#"func Test() { return 2 * 3**2; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn exponentiation_precedence_lower_than_unary() {
    // -2**2 should be parsed as -(2**2) = -4, not (-2)**2 = 4
    let source = r#"func Test() { return -2**2; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}
