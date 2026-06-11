//! `#appendto` linking primitive: C4AulScript::AppendTo with bHighPrio
//! (C4AulLink.cpp:114-141) — the appended script's functions OVERRIDE the
//! target's same-name functions (the original becomes the `inherited`
//! target), global functions are not appended (:127), and the appended
//! script's locals join the target's declarations.

use lc_script::{Engine, Script, Value};

#[test]
fn appended_functions_override_and_reach_the_original_via_inherited() {
    let mut target = Engine::new();
    target.add_script(Script::compile("public func Probe() { return 1; }").expect("compiles"));

    let mut append = Engine::new();
    append.add_script(
        Script::compile("public func Probe() { return 10 + inherited(); }").expect("compiles"),
    );

    target.append_overrides_from(&append);
    assert_eq!(
        target.call("Probe", &[]).expect("call succeeds"),
        Value::Int(11),
        "appendto wins; the original is its inherited target"
    );
}

#[test]
fn appended_scripts_bring_new_functions_but_not_globals() {
    let mut target = Engine::new();
    target.add_script(Script::compile("public func Own() { return 1; }").expect("compiles"));

    let mut append = Engine::new();
    append.add_script(
        Script::compile(
            "public func SetAI(name, interval) { return 7; }\n\
             global func Helper() { return 9; }",
        )
        .expect("compiles"),
    );

    target.append_overrides_from(&append);
    assert_eq!(
        target.call("SetAI", &[]).expect("call succeeds"),
        Value::Int(7)
    );
    assert!(
        !target.has_function("Helper"),
        "global funcs live in the global table, never in appends (C4AulLink.cpp:127)"
    );
}

#[test]
fn appended_locals_resolve_on_the_target() {
    let mut target = Engine::new();
    target.add_script(Script::compile("// empty\n").expect("compiles"));

    let mut append = Engine::new();
    append.add_script(
        Script::compile(
            "local iState;\n\
             public func Bump() { iState = iState + 1; return iState; }",
        )
        .expect("compiles"),
    );

    target.append_overrides_from(&append);
    let locals = std::collections::HashMap::new();
    let (value, finals) = target
        .call_with_locals("Bump", &[], &locals)
        .expect("call succeeds");
    assert_eq!(value, Value::Int(1));
    assert_eq!(finals.get("iState"), Some(&Value::Int(1)));
}
