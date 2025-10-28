// Test for comma operator support

#[test]
fn mgsm_line_24_pattern() {
    // Exact pattern from MGSM line 24
    let source = r#"func Test() { if (!SetAction("Wait")) return (0, RemoveObject()); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn simple_comma_in_return() {
    // return (expr1, expr2)
    let source = r#"func Test() { return (0, 42); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_with_three_expressions() {
    // return (expr1, expr2, expr3)
    let source = r#"func Test() { return (1, 2, 3); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_with_function_calls() {
    // return (1, Message(...), Sound(...))
    let source = r#"func Test() { return (1, Message("test"), Sound("Click")); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_with_assignment() {
    // return (1, var = 0)
    let source = r#"func Test() { var x; return (1, x = 42); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_in_variable_initializer() {
    // var x = (expr1, expr2)
    let source = r#"func Test() { var x = (0, 42); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_in_if_condition() {
    // if ((expr1, expr2))
    let source = r#"func Test() { var x; if ((x = 5, x > 0)) return 1; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_in_while_condition() {
    // while ((expr1, expr2))
    let source = r#"func Test() { var x; while ((x = x + 1, x < 10)) {} }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn nested_comma_expressions() {
    // (a, (b, c))
    let source = r#"func Test() { return (1, (2, 3)); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn lock_pattern() {
    // Pattern from Lock.c4d scripts
    let source = r#"func Test() { return (1, Message("test"), Sound("Error")); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn kingdoms_pattern() {
    // Pattern from Kingdoms scripts
    let source = r#"func Test() { var clonk; if (!clonk) return (0, RemoveObject()); }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn comma_in_var_decl_without_parens() {
    // var x = 1, 2 is valid - evaluates to var x = 2
    let source = r#"func Test() { var x = 1, 2; }"#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}
