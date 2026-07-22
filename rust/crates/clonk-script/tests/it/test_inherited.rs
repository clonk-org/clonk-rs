//! In strict scripts, `inherited(...)` calls the function this one overloaded
//! (Fn->OwnerOverloaded, C4AulParse.cpp:2775-2798); `_inherited` is the safe
//! spelling that yields nil instead of an error when no parent exists
//! (C4AUL_SafeInherited, C4AulParse.cpp:55-56). Overloads arise when a later
//! script redefines a name (C4AulScriptEngine link order) or when an
//! #include'd parent defines the same function.

use clonk_script::{Engine, Script, Value};

fn compile_clean(source: &str) -> Script {
    let script = Script::compile(source).expect("script compiles");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected parse diagnostics: {:?}",
        script.parse_diagnostics()
    );
    script
}

#[test]
fn nonstrict_inherited_is_rejected() {
    for spelling in ["inherited", "_inherited"] {
        let source = format!(
            "func Broken() {{ return {spelling}(); }}\n\
             func Healthy() {{ return 7; }}"
        );
        let script = Script::compile(&source).expect("body error is recovered");
        assert_eq!(
            script.parse_diagnostics().len(),
            1,
            "source: {source}"
        );
        assert_eq!(
            script.parse_diagnostics()[0].message(),
            "inherited disabled; use #strict syntax!",
            "source: {source}"
        );

        let mut engine = Engine::new();
        engine.add_script(script);
        assert_eq!(
            engine.call("Healthy", &[]).expect("recovery keeps sibling functions"),
            Value::Int(7)
        );
        let error = engine
            .call("Broken", &[])
            .expect_err("the rejected function retains a parse-error sentinel");
        assert!(
            error
                .to_string()
                .contains("inherited disabled; use #strict syntax!"),
            "{error}"
        );
    }
}

#[test]
fn safe_inherited_without_parent_is_nil() {
    let source = r#"
        #strict
        global func Construction() { return _inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("Construction", &[]).expect("call succeeds"),
        Value::Nil
    );
}

#[test]
fn safe_inherited_without_parent_evaluates_discarded_arguments() {
    let source = r#"
        #strict
        local calls;
        func SideEffect() { calls++; return 99; }
        func Construction() { return [_inherited(SideEffect()), calls]; }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("Construction", &[]).expect("call succeeds"),
        Value::Array(vec![Value::Nil, Value::Int(1)])
    );
}

#[test]
fn plain_inherited_without_parent_errors() {
    let source = r#"
        #strict
        global func Construction() { return inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert!(engine.call("Construction", &[]).is_err());
}

#[test]
fn later_script_overloads_earlier_and_reaches_it_via_inherited() {
    let mut engine = Engine::new();
    engine.add_script(compile_clean("global func F() { return 1; }"));
    engine.add_script(compile_clean(
        "#strict\nglobal func F() { return inherited() + 10; }",
    ));
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(11)
    );
}

#[test]
fn inherited_forwards_arguments() {
    let mut engine = Engine::new();
    engine.add_script(compile_clean("global func F(a, b) { return a + b; }"));
    engine.add_script(compile_clean(
        "#strict\nglobal func F(a, b) { return inherited(a, b) * 2; }",
    ));
    assert_eq!(
        engine.call("F", &[Value::Int(2), Value::Int(3)]).expect("call succeeds"),
        Value::Int(10)
    );
}

#[test]
fn include_parent_function_is_reachable_via_inherited() {
    // The #include seam: the child keeps its own function and the parent's
    // becomes its overload target (C4AulLink include handling). GLOBAL
    // functions are never copied by includes (C4AulLink.cpp:127 — they
    // live at the engine, where install chaining forms their overloads),
    // so the seam is pinned with public functions.
    let mut parent = Engine::new();
    parent.add_script(compile_clean("public func F() { return 5; }"));
    let mut child = Engine::new();
    child.add_script(compile_clean(
        "#strict\npublic func F() { return _inherited() + 1; }",
    ));
    child.merge_from(&parent);
    assert_eq!(
        child.call("F", &[]).expect("call succeeds"),
        Value::Int(6)
    );

    // A global func in the parent is NOT copied into the child.
    let mut global_parent = Engine::new();
    global_parent.add_script(compile_clean("global func G() { return 5; }"));
    let mut plain_child = Engine::new();
    plain_child.add_script(compile_clean("// empty\n"));
    plain_child.merge_from(&global_parent);
    assert!(
        !plain_child.has_function("G"),
        "includes never copy global funcs (C4AulLink.cpp:127)"
    );
}

#[test]
fn same_script_redefinition_reaches_the_earlier_definition_via_inherited() {
    // C4AulScript::ParseFn links a redefinition in the SAME script to the
    // earlier definition (`Fn->OwnerOverloaded = Fn->Owner->
    // GetOverloadedFunc(Fn)`, C4AulParse.cpp:1404-1406). The Coach.c4d menu
    // idiom relies on it: the implementation is followed by
    // `public func ControlDownDouble(pByObject) { [$TxtGetoff$]
    // return(inherited(pByObject)); }` (Coach.c4d/Script.c) — the wrapper
    // adds the menu description and forwards to the real body.
    let source = r#"
        #strict
        public func F(a) { return a + 1; }
        public func F(a) { return inherited(a) * 10; }
    "#;
    let mut engine = Engine::new();
    engine.add_script(compile_clean(source));
    assert_eq!(
        engine.call("F", &[Value::Int(2)]).expect("call succeeds"),
        Value::Int(30),
        "the later definition wins and inherited() reaches the earlier one"
    );
}
