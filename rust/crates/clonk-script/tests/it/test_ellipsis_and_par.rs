//! C4Aul varargs: `func F(...)` ends the parameter list (C4AulParse.cpp:1642-1648),
//! `G(...)` at a call site forwards every unnamed parameter of the current
//! function starting at its named-parameter count (C4AulParse.cpp:2293-2306),
//! and `Par(i)` reads the current function's parameter slot, nil when out of
//! range (C4AulExec.cpp:1127-1140). planet/System.c4g/Helpers.c relies on all
//! three (`SetActionKeepPhase(...)`, `ScheduleCall`'s `Par(i + 4)`).

use clonk_script::{Engine, Script, Value};

#[test]
fn ellipsis_parameter_list_compiles() {
    let source = r#"
        global func TakeAnything(...) { return 7; }
        global func Probe() { return TakeAnything(1, 2, 3); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("ellipsis param list compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(7)
    );
}

#[test]
fn function_declaration_rejects_eleventh_parameter() {
    // The limit is a direct parse error at every strictness level
    // (C4AulParse.cpp:1624-1640), not a call-time truncation rule.
    for strict_prefix in ["", "#strict 3\n"] {
        let source = format!(
            "{strict_prefix}func TooMany(a, b, c, d, e, f, g, h, i, j, k) {{ return 1; }}\n\
             func Healthy() {{ return 7; }}"
        );
        let script = Script::compile(&source).expect("top-level recovery retains the script");
        assert!(
            script.parse_diagnostics().iter().any(|error| {
                error.message() == "'func' parameter list: too many parameters (max 10)"
            }),
            "missing parameter-limit diagnostic for {source:?}: {:?}",
            script.parse_diagnostics()
        );
        assert!(
            !script.functions().contains_key("TooMany"),
            "the rejected declaration must not be registered"
        );
        assert!(
            script.functions().contains_key("Healthy"),
            "the next declaration must survive preparse recovery"
        );
    }
}

#[test]
fn function_parameter_limit_preserves_cpp_boundary_order() {
    // The native loop checks ')' first, the syntactic count second, and
    // ellipsis third. Duplicate names still consume syntactic entries even
    // though their C4ValueMap slot is reused.
    for source in [
        "func Ten(a, b, c, d, e, f, g, h, i, j) {}",
        "func NineVariadic(a, b, c, d, e, f, g, h, i, ...) {}",
        "func BareVariadic(...) {}",
        "func TenTrailingComma(a, b, c, d, e, f, g, h, i, j,) {}",
    ] {
        let script = Script::compile(source).expect("boundary declaration compiles");
        assert!(
            script.parse_diagnostics().is_empty(),
            "unexpected diagnostic for {source:?}: {:?}",
            script.parse_diagnostics()
        );
    }

    for source in [
        "func TenThenVariadic(a, b, c, d, e, f, g, h, i, j, ...) {}",
        "func DuplicateEleven(a, a, a, a, a, a, a, a, a, a, a) {}",
    ] {
        let script = Script::compile(source).expect("rejected declaration is diagnosed");
        assert!(
            script.parse_diagnostics().iter().any(|error| {
                error.message() == "'func' parameter list: too many parameters (max 10)"
            }),
            "syntactic limit must reject {source:?}: {:?}",
            script.parse_diagnostics()
        );
    }
}

#[test]
fn par_reads_current_function_arguments() {
    let source = r#"
        global func PickSecond(...) { return Par(1); }
        global func Probe() { return PickSecond(10, 20, 30); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(20)
    );
}

#[test]
fn par_out_of_range_is_nil() {
    // C4AulExec.cpp:1138 Set0() when the index is outside ParCnt.
    let source = r#"
        global func PickFar(...) { return Par(9); }
        global func Probe() { return PickFar(1); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Nil
    );
}

#[test]
fn ellipsis_call_forwards_all_args_of_varargs_function() {
    // SetActionKeepPhase pattern: zero named params, so `Inner(...)`
    // forwards Par(0).. (C4AulParse.cpp:2297 starts at ParNamed.iSize).
    let source = r#"
        global func Inner(a, b) { return a + b; }
        global func Outer(...) { return Inner(...); }
        global func Probe() { return Outer(2, 3); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(5)
    );
}

#[test]
fn ellipsis_call_forwards_only_unnamed_parameters() {
    // With one named parameter, forwarding starts at Par(1).
    let source = r#"
        global func Inner(a, b) { return a * 10 + b; }
        global func Outer(first, ...) { return Inner(...); }
        global func Probe() { return Outer(9, 1, 2); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(12)
    );
}

#[test]
fn par_works_with_named_parameters_too() {
    // Named parameters land in Pars[] like positional ones; Par(0) reads the
    // first regardless of naming (C4AulExec Pars are one flat array).
    let source = r#"
        global func Named(alpha, beta) { return Par(0) + beta; }
        global func Probe() { return Named(40, 2); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(42)
    );
}

#[test]
fn par_is_a_writable_reference_to_the_ten_slot_call_frame() {
    // AB_PAR_R returns a reference into C4AulContext::Pars. Named parameters
    // share those cells, and omitted slots remain writable (C4AulExec.cpp:
    // 1127-1140). Magic.c's ReduceAlchem relies on `Par(1)=this()`.
    let source = r#"
        #strict
        global func Fill(first)
        {
            Par(0) = 5;
            if (!Par(1)) Par(1) = 7;
            return [first, Par(0), Par(1)];
        }
        global func Probe() { return Fill(1); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Array(vec![Value::Int(5), Value::Int(5), Value::Int(7)])
    );
}
