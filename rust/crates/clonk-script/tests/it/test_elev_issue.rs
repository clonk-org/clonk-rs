// Test for ELEV issue - if with !variable followed by return with method call

#[test]
fn elev_pattern_minimal() {
    // Minimal reproduction of ELEV lines 56-57
    let source = r#"
        func Test() {
            var pCase;
            if (!pCase) return(0);
            return (pCase->IsInPermanentMode());
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

#[test]
fn elev_pattern_exact() {
    // Exact pattern from ELEV
    let source = r#"
public func IsInPermanentMode()
{
  if (!pCase) return(0);
  return (pCase->IsInPermanentMode());
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

#[test]
fn braceless_if_with_not() {
    // Braceless if with ! operator
    let source = r#"func Test() { var x; if (!x) return 1; return 0; }"#;
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
