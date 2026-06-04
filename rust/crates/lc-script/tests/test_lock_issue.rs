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
fn bracket_annotation_german_chars() {
    // C4Script identifiers are restricted to [0-9A-Za-z_] (C++ lexer,
    // src/C4AulParse.cpp:668-671): a non-ASCII byte terminates the identifier and
    // is then reported as an "unexpected character". German umlauts therefore
    // live in C4Script *string literals* and comments, never in bare identifiers.
    // A bracket annotation whose identifier carries a non-ASCII char must be
    // rejected, matching C++. (The original source for this test had a `\u{FFFD}`
    // replacement char where a `ü` was intended — either way it is non-ASCII and
    // must be rejected.)
    let bad_identifier =
        lc_script::Script::compile("func Test()\n{\n  [Schl\u{FFFD}ssel]\n  return 1;\n}\n");
    assert!(
        bad_identifier.is_err(),
        "a non-ASCII character in identifier position must be rejected (C4Script identifiers are [0-9A-Za-z_])"
    );

    // German characters ARE valid inside string literals — the real way LOCK and
    // other German content carry them.
    let german_string =
        lc_script::Script::compile("func Test()\n{\n  var s = \"Schlüssel\";\n  return s;\n}\n");
    if let Err(e) = &german_string {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(
        german_string.is_ok(),
        "German characters inside a string literal should compile"
    );
}
