//! C4Aul varargs: `func F(...)` ends the parameter list (C4AulParse.cpp:1642-1648),
//! `G(...)` at a call site forwards every unnamed parameter of the current
//! function starting at its named-parameter count (C4AulParse.cpp:2293-2306),
//! and `Par(i)` reads the current function's parameter slot, nil when out of
//! range (C4AulExec.cpp:1127-1140). planet/System.c4g/Helpers.c relies on all
//! three (`SetActionKeepPhase(...)`, `ScheduleCall`'s `Par(i + 4)`).

use lc_script::{Engine, Script, Value};

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
