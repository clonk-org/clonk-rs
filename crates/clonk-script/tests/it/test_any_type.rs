// Test for 'any' type annotation support

// Exact pattern from INDI line 818
crate::support::compile_case!(
    indi_line_818_pattern,
    r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { }"#
);

// func Test(any x)
crate::support::compile_case!(any_type_first_param, r#"func Test(any x) { }"#);

// func Test(int x, any y, string z)
crate::support::compile_case!(
    any_type_middle_param,
    r#"func Test(int x, any y, string z) { }"#
);

// func Test(any x, any y)
crate::support::compile_case!(multiple_any_params, r#"func Test(any x, any y) { }"#);

// Pattern from JungleClonk, Inuk, Trapper (same as INDI)
crate::support::compile_case!(
    jungle_clonk_pattern,
    r#"func ControlCommandFinished (string CommandName, object Target, any Tx, int Ty, object Target2, any Data) { return(1); }"#
);

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

// Regression test: ensure existing type annotations work
crate::support::compile_case!(
    existing_types_still_work,
    r#"func Test(int x, bool y, string z, object obj) { }"#
);
