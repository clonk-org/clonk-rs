//! Engine-registered script constants (`RegisterGlobalConstant`,
//! C4Aul.cpp / C4Script.cpp:6581 + the C4ScriptConstMap table at :6208):
//! bare identifiers like `DIR_Right` resolve to engine-provided values when
//! no variable of that name exists; locals shadow them.

use lc_script::{Engine, Script, Value};

#[test]
fn registered_constants_resolve_as_identifiers() {
    let mut engine = Engine::new();
    engine.register_constant("DIR_Right", Value::Int(1));
    engine.register_constant("NO_OWNER", Value::Int(-1));
    engine.add_script(
        Script::compile(
            r#"
            global func Probe() { return DIR_Right + NO_OWNER; }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(engine.call("Probe", &[]).expect("call succeeds"), Value::Int(0));
}

#[test]
fn local_variables_shadow_constants() {
    let mut engine = Engine::new();
    engine.register_constant("DIR_Right", Value::Int(1));
    engine.add_script(
        Script::compile(
            r#"
            global func Probe() { var DIR_Right = 9; return DIR_Right; }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(engine.call("Probe", &[]).expect("call succeeds"), Value::Int(9));
}

#[test]
fn old_style_constant_calls_yield_the_constant_below_strict2() {
    // Below #strict 2 a global constant used as `OCF_Chop()` parses as the
    // constant with the call parens ignored ("old-style usage",
    // C4AulParse.cpp:2838-2860) — Objects.c4d grass relies on it.
    let mut engine = Engine::new();
    engine.register_constant("OCF_Chop", Value::Int(256));
    engine.add_script(
        Script::compile(
            r#"
            #strict
            global func Probe() { return OCF_Chop(); }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(256)
    );

    let mut strict2 = Engine::new();
    strict2.register_constant("OCF_Chop", Value::Int(256));
    strict2.add_script(
        Script::compile(
            r#"
            #strict 2
            global func Probe() { return OCF_Chop(); }
            "#,
        )
        .expect("script compiles"),
    );
    assert!(
        strict2.call("Probe", &[]).is_err(),
        "#strict 2 scripts may not call constants"
    );
}

#[test]
fn unknown_identifiers_still_error() {
    let mut engine = Engine::new();
    engine.add_script(
        Script::compile(r#"global func Probe() { return NoSuchConstant; }"#)
            .expect("script compiles"),
    );
    assert!(engine.call("Probe", &[]).is_err());
}
