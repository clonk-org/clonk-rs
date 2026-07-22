use clonk_script::{Engine, Script, Value};

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
fn c4_comment_whitespace_edges_raw_newline_is_confined_to_its_function() {
    let source = "func Broken() { return \"a\nb\"; }\nfunc Ok() { return 7; }";
    let script = Script::compile(source).expect("raw string newline is recoverable");
    assert_eq!(script.parse_diagnostics().len(), 1);
    assert_eq!(script.parse_diagnostics()[0].message(), "string not closed");

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine.call("Ok", &[]).expect("later function survives"),
        Value::Int(7)
    );
    let error = engine
        .call("Broken", &[])
        .expect_err("broken function retains its parse-error sentinel");
    assert!(error.to_string().contains("string not closed"), "{error}");
}

#[test]
fn unknown_escape_warns_without_quarantining_the_function() {
    let script = Script::compile(
        r#"
func Broken() { return "bad\q"; }
func Ok() { return 1; }
"#,
    )
    .expect("an unknown escape warns without rejecting the script");
    assert_eq!(
        script.parse_diagnostics().len(),
        1,
        "one unknown escape produces one warning"
    );
    assert_eq!(script.parse_diagnostics()[0].message(), "unknown escape: q");

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine
            .call("Ok", &[])
            .expect("the later function remains callable"),
        Value::Int(1)
    );
    assert_eq!(
        engine
            .call("Broken", &[])
            .expect("the function with an unknown escape remains callable"),
        Value::String(r"bad\q".into())
    );
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

#[test]
fn stray_loop_control_warns_and_is_a_noop_below_strict_two() {
    for strict_prefix in ["", "#strict\n"] {
        for (keyword, expected_message) in [
            ("break", "'break' is only allowed inside loops"),
            ("continue", "'continue' is only allowed inside loops"),
        ] {
            let source = format!("{strict_prefix}func Probe() {{ {keyword}; return 7; }}");
            let script = Script::compile(&source).expect("stray loop control only warns");
            assert_eq!(script.parse_diagnostics().len(), 1, "source: {source}");
            assert_eq!(
                script.parse_diagnostics()[0].message(),
                expected_message,
                "source: {source}"
            );

            let mut engine = Engine::new();
            engine.add_script(script);
            assert_eq!(
                engine
                    .call("Probe", &[])
                    .expect("warning leaves function executable"),
                Value::Int(7),
                "source: {source}"
            );
        }
    }
}

#[test]
fn stray_loop_control_is_a_deferred_function_error_at_strict_two_or_higher() {
    for strict_prefix in ["#strict 2\n", "#strict 3\n"] {
        for (keyword, expected_message) in [
            ("break", "'break' is only allowed inside loops"),
            ("continue", "'continue' is only allowed inside loops"),
        ] {
            let source = format!(
                "{strict_prefix}func Broken() {{ {keyword}; return 7; }}\nfunc Ok() {{ return 9; }}"
            );
            let script = Script::compile(&source).expect("the broken function is quarantined");
            assert_eq!(script.parse_diagnostics().len(), 1, "source: {source}");
            assert_eq!(
                script.parse_diagnostics()[0].message(),
                expected_message,
                "source: {source}"
            );

            let mut engine = Engine::new();
            engine.add_script(script);
            assert_eq!(engine.call("Ok", &[]).unwrap(), Value::Int(9));
            let error = engine
                .call("Broken", &[])
                .expect_err("strict loop control must reach the parse-error sentinel");
            assert!(error.to_string().contains(expected_message), "{error}");
        }
    }
}

#[test]
fn loop_control_inside_each_loop_shape_is_unchanged() {
    let script = Script::compile(
        r#"
#strict 3
func Probe()
{
    var i = 0, sum = 0;
    while (1) break;
    for (var declared = 0; declared < 2; ++declared)
    {
        if (declared == 0) continue;
        sum = sum + declared;
    }
    for (i = 0; i < 3; ++i) if (i == 1) break;
    for (var value in [3, 4])
    {
        sum = sum + value;
        break;
    }
    return [sum, i];
}
"#,
    )
    .expect("loop controls parse in loop bodies");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        script.parse_diagnostics()
    );

    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine.call("Probe", &[]).unwrap(),
        Value::Array(vec![Value::Int(4), Value::Int(1)])
    );
}
