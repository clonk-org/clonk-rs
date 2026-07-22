// Test for specific spacing patterns in brace-less if

#[test]
fn if_with_space_before_closing_paren() {
    // Note the space before the closing paren: "() )"
    let source = r#"func Test() { var dir; if(dir == GetValue() ) dir = -1; }"#;
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
fn if_with_double_parens_and_space() {
    // Exact pattern: function call with () followed by space and )
    let source = r#"func Test() { var x; if(x == Func() ) x = 1; }"#;
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
fn chained_assignment_in_if_body() {
    // Line 55 pattern: x = y = -1;
    let source = r#"func Test() { var x, y; if(1) x = y = -1; }"#;
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
fn dir_left_exact_pattern() {
    // Most exact reproduction of line 34
    let source = r#"
func Test() {
  var dir;
  if(dir == DIR_Left() ) dir = -1;
}
"#;
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
