// Test for LOCK script array literal / annotation issue

// Line 32 from LOCK with ASCII-only: annotation without dollar signs
crate::support::compile_case!(
    lock_simple_annotation_ascii,
    r#"
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
    "#,
);

// Minimal test with bracket annotation
crate::support::compile_case!(
    simple_bracket_annotation,
    r#"
func Test()
{
  [Some text]
  return 1;
}
    "#,
);

#[test]
fn function_description_accepts_german_chars() {
    // This bracket block is not an identifier context: C4AulParse.cpp:1825-1853
    // bypasses the tokenizer and raw-scans function descriptions through their
    // balanced closing bracket. Localized non-ASCII text is therefore valid.
    let german_description =
        clonk_script::Script::compile("func Test()\n{\n  [Schlüssel]\n  return 1;\n}\n");
    assert!(
        german_description.is_ok(),
        "localized function descriptions are raw text"
    );

    // German characters ARE valid inside string literals — the real way LOCK and
    // other German content carry them.
    let german_string =
        clonk_script::Script::compile("func Test()\n{\n  var s = \"Schlüssel\";\n  return s;\n}\n");
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
