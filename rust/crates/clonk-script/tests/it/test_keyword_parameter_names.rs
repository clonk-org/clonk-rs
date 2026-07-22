//! C4Aul keywords are contextual: the C++ tokenizer returns plain
//! identifiers (ATT_IDTF) and keyword-ness is a string comparison at each
//! parse position, so reserved words are legal parameter names. Real content
//! relies on it: `func SetPrivateTeleporter(bool private)`
//! (Hazard.c4d/.../Teleporter.c4d/Script.c:238).

use clonk_script::{Engine, Script, Value};

#[test]
fn access_keywords_are_valid_parameter_names() {
    let source = r#"
        global func SetPrivateTeleporter(bool private) {
            return private;
        }
        global func Probe() { return SetPrivateTeleporter(true); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Bool(true)
    );
}

#[test]
fn other_keywords_work_as_parameter_names_too() {
    let source = r#"
        global func Pick(global, local) {
            return global + local;
        }
        global func Probe() { return Pick(2, 3); }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(5)
    );
}
