//! `inherited(...)` calls the function this one overloaded
//! (Fn->OwnerOverloaded, C4AulParse.cpp:2775-2798); `_inherited` is the safe
//! spelling that yields nil instead of an error when no parent exists
//! (C4AUL_SafeInherited, C4AulParse.cpp:55-56). Overloads arise when a later
//! script redefines a name (C4AulScriptEngine link order) or when an
//! #include'd parent defines the same function.

use lc_script::{Engine, Script, Value};

#[test]
fn safe_inherited_without_parent_is_nil() {
    let source = r#"
        global func Construction() { return _inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Construction", &[]).expect("call succeeds"),
        Value::Nil
    );
}

#[test]
fn plain_inherited_without_parent_errors() {
    let source = r#"
        global func Construction() { return inherited(); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert!(engine.call("Construction", &[]).is_err());
}

#[test]
fn later_script_overloads_earlier_and_reaches_it_via_inherited() {
    let mut engine = Engine::new();
    engine.add_script(Script::compile("global func F() { return 1; }").expect("first compiles"));
    engine.add_script(
        Script::compile("global func F() { return inherited() + 10; }").expect("second compiles"),
    );
    assert_eq!(
        engine.call("F", &[]).expect("call succeeds"),
        Value::Int(11)
    );
}

#[test]
fn inherited_forwards_arguments() {
    let mut engine = Engine::new();
    engine
        .add_script(Script::compile("global func F(a, b) { return a + b; }").expect("compiles"));
    engine.add_script(
        Script::compile("global func F(a, b) { return inherited(a, b) * 2; }").expect("compiles"),
    );
    assert_eq!(
        engine.call("F", &[Value::Int(2), Value::Int(3)]).expect("call succeeds"),
        Value::Int(10)
    );
}

#[test]
fn include_parent_function_is_reachable_via_inherited() {
    // The #include seam: the child keeps its own function and the parent's
    // becomes its overload target (C4AulLink include handling).
    let mut parent = Engine::new();
    parent.add_script(Script::compile("global func F() { return 5; }").expect("compiles"));
    let mut child = Engine::new();
    child.add_script(
        Script::compile("global func F() { return _inherited() + 1; }").expect("compiles"),
    );
    child.merge_from(&parent);
    assert_eq!(
        child.call("F", &[]).expect("call succeeds"),
        Value::Int(6)
    );
}
