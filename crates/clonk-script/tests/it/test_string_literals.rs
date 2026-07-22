use clonk_script::{Engine, Script, Value};

fn compile_and_call(source: &str) -> (Script, Value) {
    let script = Script::compile(source).expect("string-literal script compiles");
    let mut engine = Engine::new();
    engine.add_script(script.clone());
    let value = engine
        .call("Test", &[])
        .expect("the string-literal function remains callable");
    (script, value)
}

#[test]
fn unknown_escapes_preserve_the_backslash_and_warn_once() {
    for (source, expected, escape) in [
        (r#"func Test() { return "a\nb"; }"#, r"a\nb", 'n'),
        (r#"func Test() { return "\d"; }"#, r"\d", 'd'),
    ] {
        let (script, value) = compile_and_call(source);

        assert_eq!(value, Value::String(expected.into()));
        assert_eq!(
            script.parse_diagnostics().len(),
            1,
            "each unknown escape emits exactly one warning for {source:?}"
        );
        assert_eq!(
            script.parse_diagnostics()[0].message(),
            format!("unknown escape: {escape}")
        );
    }
}

#[test]
fn quote_and_backslash_escapes_keep_their_special_meaning_without_warnings() {
    let (script, value) = compile_and_call(r#"func Test() { return "a\"b\\c"; }"#);

    assert_eq!(value, Value::String("a\"b\\c".into()));
    assert!(
        script.parse_diagnostics().is_empty(),
        "recognized escapes must not warn: {:?}",
        script.parse_diagnostics()
    );
}

#[test]
fn long_string_below_strict3_truncates_to_1024_and_warns() {
    let literal = "x".repeat(2000);
    let source = format!("#strict 2\nfunc Test() {{ return \"{literal}\"; }}");
    let (script, value) = compile_and_call(&source);

    assert_eq!(value, Value::String("x".repeat(1024).into()));
    assert_eq!(
        script.parse_diagnostics().len(),
        1,
        "one truncation warning is retained: {:?}",
        script.parse_diagnostics()
    );
    assert_eq!(script.parse_diagnostics()[0].message(), "string too long");
}

#[test]
fn long_string_at_strict3_quarantines_only_its_function() {
    let literal = "x".repeat(2000);
    let source =
        format!("#strict 3\nfunc Broken() {{ return \"{literal}\"; }}\nfunc Ok() {{ return 7; }}");
    let script = Script::compile(&source).expect("strict string error is recoverable");
    assert!(
        script
            .parse_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message() == "string too long"),
        "the strict parse failure is retained: {:?}",
        script.parse_diagnostics()
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    let error = engine
        .call("Broken", &[])
        .expect_err("the strict3 function containing the long string is quarantined");
    assert!(
        error.to_string().contains("string too long"),
        "unexpected runtime error: {error}"
    );
    assert_eq!(
        engine
            .call("Ok", &[])
            .expect("the following function remains callable"),
        Value::Int(7)
    );
}
