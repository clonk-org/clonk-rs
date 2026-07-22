// Test for empty argument support in function calls

#[test]
fn wtow_line_116_pattern() {
    // Exact pattern from WTOW line 116
    let source = r#"func Test() { SetColorDw(, pObj); }"#;
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
fn empty_first_arg() {
    // func(, x)
    let source = r#"func Test() { SomeFunc(, 42); }"#;
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
fn empty_middle_arg() {
    // func(x, , y)
    let source = r#"func Test() { SomeFunc(1, , 3); }"#;
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
fn multiple_empty_args() {
    // Contents(, , true)
    let source = r#"func Test() { Contents(, , true); }"#;
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
fn all_empty_args() {
    // func(, )
    let source = r#"func Test() { SomeFunc(, ); }"#;
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
fn trailing_empty_arg() {
    // func(x, )
    let source = r#"func Test() { SomeFunc(42, ); }"#;
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
fn normal_args_still_work() {
    // Regression test: func(x, y)
    let source = r#"func Test() { SomeFunc(1, 2, 3); }"#;
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
fn empty_args_with_expressions() {
    // Mix of empty and complex expressions
    let source = r#"func Test() { SomeFunc(, GetX() + 5, , "test"); }"#;
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
fn jungle_clonk_pattern() {
    // Pattern from JungleClonk: Contents(, , true)
    let source = r#"func Test() { return(DefinitionCall(GetID(Contents(, , true)), "IsSpear")); }"#;
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
