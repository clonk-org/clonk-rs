// Test for 'any' type annotation support

#[test]
fn indi_line_818_pattern() {
    // Exact pattern from INDI line 818
    let source = r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn any_type_first_param() {
    // func Test(any x)
    let source = r#"func Test(any x) { }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn any_type_middle_param() {
    // func Test(int x, any y, string z)
    let source = r#"func Test(int x, any y, string z) { }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn multiple_any_params() {
    // func Test(any x, any y)
    let source = r#"func Test(any x, any y) { }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn jungle_clonk_pattern() {
    // Pattern from JungleClonk, Inuk, Trapper (same as INDI)
    let source = r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { return(1); }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn all_cpp_parameter_types_compile_without_diagnostics() {
    let source = r#"func Test(int a, bool b, id c, object d, string e, array f, map g, any h) { }"#;
    let script = clonk_script::Script::compile(source).expect("all eight C++ types compile");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        script.parse_diagnostics()
    );
}

#[test]
fn existing_types_still_work() {
    // Regression test: ensure existing type annotations work
    let source = r#"func Test(int x, bool y, string z, object obj) { }"#;
    crate::support::assert_compiles(source);
}
