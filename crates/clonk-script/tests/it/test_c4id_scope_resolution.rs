use clonk_script::{Engine, Script, Value};

fn compile_without_diagnostics(source: &str) -> Script {
    let script = Script::compile(source).expect("script loads");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        script.parse_diagnostics()
    );
    script
}

#[test]
fn test_c4id_in_scope_resolution() {
    // Test that C4IDs can be used in scope resolution syntax: obj->DEFN::Method()
    let source = r#"
        #strict
        func TestScopeResolution() {
            var obj = this;
            // C4ID used in scope resolution
            obj->CLNK::SomeMethod();
            return 0;
        }
    "#;

    compile_without_diagnostics(source);
}

#[test]
fn test_c4id_after_arrow_operator() {
    // Test that C4IDs can appear after -> operator
    let source = r#"
        #strict
        func TestArrow() {
            var pAmmo = this;
            // C4ID immediately after ->
            pAmmo->OBRL();
            return 0;
        }
    "#;

    let script = Script::compile(source).expect("script loads with a legacy warning");
    assert!(
        script
            .parse_diagnostics()
            .iter()
            .any(|error| error.message() == "stupid func label: OBRL"),
        "C++ warns when a C4ID-shaped name is used as a function label"
    );
}

#[test]
fn test_complex_c4id_scope_resolution() {
    // Test the actual pattern from ACT2 script
    let source = r#"
        #strict
        func TestComplexPattern() {
            var pAmmo = this;
            // Complex pattern: method call with C4ID scope resolution
            pAmmo->OBRL::BarrelDoFill(-pAmmo->OBRL::GetAmount());
            return 0;
        }
    "#;

    compile_without_diagnostics(source);
}

#[test]
fn c4id_shaped_names_are_quarantined_without_panicking_or_dropping_later_code() {
    for (bad_declaration, expected_message, broken_is_callable) in [
        ("func TEST () { return 1; }", "function name", false),
        ("func Broken() { var ABCD; }", "variable name", true),
        (
            "#strict\nfunc Broken() { for (var COIN in []) {} }",
            "variable name",
            true,
        ),
        (
            "#strict 3\nfunc Broken() { var x = {}; x.FLNT = 1; }",
            "property name",
            true,
        ),
        ("static ABCD;", "variable name", false),
        ("static const FLNT = 1;", "variable name", false),
    ] {
        let source = format!("{bad_declaration}\nfunc Good() {{ return 7; }}");
        let script = Script::compile(&source)
            .unwrap_or_else(|error| panic!("recoverable name error aborted the script: {error}"));
        assert!(
            script
                .parse_diagnostics()
                .iter()
                .any(|error| error.message().contains(expected_message)),
            "missing {expected_message:?} diagnostic for {bad_declaration:?}: {:?}",
            script.parse_diagnostics()
        );

        let mut engine = Engine::new();
        engine.add_script(script);
        assert_eq!(
            engine
                .call("Good", &[])
                .expect("the later declaration remains callable"),
            Value::Int(7)
        );
        if broken_is_callable {
            let error = engine
                .call("Broken", &[])
                .expect_err("the broken function retains a parse-error sentinel");
            assert!(error.to_string().contains("parse error"));
        }
    }
}

#[test]
fn adjacent_c4id_shaped_function_names_and_c4id_values_remain_valid() {
    let mut engine = Engine::new();
    engine
        .load_script("func TEST() { return FLNT; }")
        .expect("the delimiter-sensitive identifier form loads");
    assert_eq!(
        engine.call("TEST", &[]).expect("TEST executes"),
        Value::C4Id("FLNT".to_string())
    );
}
