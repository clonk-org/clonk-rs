use lc_script::{Engine, Script, Value};

#[test]
fn broken_function_is_quarantined_without_dropping_the_next_function() {
    let source = r#"
func Broken() { , }
func Ok() { return 1; }
"#;
    let mut engine = Engine::new();
    engine
        .load_script(source)
        .expect("recoverable declaration errors do not drop the script");

    assert_eq!(
        engine
            .call("Ok", &[])
            .expect("the later function remains callable"),
        Value::Int(1)
    );
    let error = engine
        .call("Broken", &[])
        .expect_err("the broken function retains an AB_ERR analogue");
    assert!(
        error.to_string().contains("parse error"),
        "unexpected runtime error: {error}"
    );
}

#[test]
fn lexer_error_in_broken_function_does_not_swallow_the_next_function() {
    let script = Script::compile(
        r#"
func Broken() { return "bad\q"; }
func Ok() { return 1; }
"#,
    )
    .expect("the lexer error is quarantined to its function");
    assert!(
        script
            .parse_diagnostics()
            .iter()
            .any(|error| error.message().contains("unknown escape sequence"))
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine
            .call("Ok", &[])
            .expect("the later function remains callable"),
        Value::Int(1)
    );
    let error = engine
        .call("Broken", &[])
        .expect_err("the broken function retains the lexer error");
    assert!(error.to_string().contains("parse error"));
}

#[test]
fn invalid_top_level_forms_are_diagnosed_without_dropping_the_next_declaration() {
    for (source, expected_diagnostic, expected_strict_level) in [
        (
            "#strict 4\nfunc Ok() { return 1; }",
            "unknown strict level",
            Some(1),
        ),
        (
            "#unknown\nfunc Ok() { return 1; }",
            "unknown directive",
            None,
        ),
        (
            ";\nfunc Ok() { return 1; }",
            "expected function declaration",
            None,
        ),
    ] {
        let script = Script::compile(source).expect("top-level recovery keeps loading the script");
        assert!(
            script
                .parse_diagnostics()
                .iter()
                .any(|error| error.message().contains(expected_diagnostic)),
            "missing {expected_diagnostic:?} diagnostic for {source:?}"
        );
        assert_eq!(script.strict_level(), expected_strict_level);

        let mut engine = Engine::new();
        engine.add_script(script);
        assert_eq!(
            engine.call("Ok", &[]).expect("the next declaration parses"),
            Value::Int(1)
        );
    }
}

#[test]
fn broken_old_style_function_is_quarantined_at_the_next_label() {
    let script = Script::compile(
        r#"
Broken:
  ,
Ok:
  return(2);
"#,
    )
    .expect("the later old-style declaration survives the broken body");
    let mut engine = Engine::new();
    engine.add_script(script);

    let error = engine
        .call("Broken", &[])
        .expect_err("the broken old-style function retains a parse-error sentinel");
    assert!(error.to_string().contains("parse error"));
    assert_eq!(
        engine
            .call("Ok", &[])
            .expect("the next old-style function remains callable"),
        Value::Int(2)
    );
}
