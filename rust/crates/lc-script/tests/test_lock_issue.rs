// Test for LOCK script array literal / annotation issue

#[test]
fn lock_simple_annotation_ascii() {
    // Line 32 from LOCK with ASCII-only: annotation without dollar signs
    let source = r#"
func ControlThrow(pClonk)
{
  [Key insert or remove]
  if(Contents())
  {
    Exit(Contents());
    return(1);
  }
  return(1);
}
    "#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn simple_bracket_annotation() {
    // Minimal test with bracket annotation
    let source = r#"
func Test()
{
  [Some text]
  return 1;
}
    "#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}

#[test]
fn bracket_annotation_german_chars() {
    // Test with German characters like in LOCK
    let source = r#"
func Test()
{
  [Schl�ssel]
  return 1;
}
    "#;
    let result = lc_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!("Error: line {}, col {}: {}", e.line(), e.column(), e.message());
    }
    assert!(result.is_ok());
}
