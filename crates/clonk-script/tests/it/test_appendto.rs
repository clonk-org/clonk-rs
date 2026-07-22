//! `#appendto` linking primitive: C4AulScript::AppendTo with bHighPrio
//! (C4AulLink.cpp:114-141) — the appended script's functions OVERRIDE the
//! target's same-name functions (the original becomes the `inherited`
//! target), global functions are not appended (:127), and the appended
//! script's locals join the target's declarations.

use clonk_script::{Engine, LocalCells, Script, Value};

#[test]
fn appended_functions_override_and_reach_the_original_via_inherited() {
    let mut target = Engine::new();
    target.add_script(Script::compile("public func Probe() { return 1; }").expect("compiles"));

    let mut append = Engine::new();
    append.add_script(
        Script::compile("#strict\npublic func Probe() { return 10 + inherited(); }")
            .expect("compiles"),
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

#[test]
fn direct_exec_uses_target_strictness_while_eval_keeps_appended_origin() {
    // C4Object::MenuCommand supplies the destination definition script's own
    // strictness to DirectExec. A stricter appended function does not change
    // that host field, while eval inside the copied function still uses its
    // original script's strictness (C4Object.cpp:3757-3761;
    // C4Script.cpp:4501-4513).
    let mut target = Engine::new();
    target.add_script(Script::compile("func Own() { return 1; }").expect("target compiles"));

    let mut append = Engine::new();
    append.add_script(
        Script::compile(
            "#strict 3\n\
             func AppendedEval() { return eval(\"1 == true\"); }",
        )
        .expect("append compiles"),
    );
    target.append_overrides_from(&append);

    assert_eq!(
        target
            .call("AppendedEval", &[])
            .expect("appended eval runs"),
        Value::Bool(false),
        "eval keeps the appended function's strict-3 origin"
    );

    let cells = LocalCells::default();
    assert_eq!(
        target
            .direct_exec_with_cells_and_this(
                r#"eval("1 == true")"#,
                &cells,
                Value::Nil,
            )
            .expect("menu-style DirectExec runs"),
        Value::Bool(true),
        "default DirectExec uses the nonstrict destination host"
    );
    assert_eq!(
        target
            .direct_exec_with_cells_and_this_at_strict(
                r#"eval("1 == true")"#,
                &cells,
                Value::Nil,
                Some(3),
            )
            .expect("explicit-strict DirectExec runs"),
        Value::Bool(false),
        "synchronized controls retain their explicit strictness"
    );
}
