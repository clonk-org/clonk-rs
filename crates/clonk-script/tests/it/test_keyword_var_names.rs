//! C4Aul keywords are contextual in variable declarations too: the C++
//! tokenizer emits plain ATT_IDTF for every word and `Parse_Var` takes the
//! identifier as-is, so `var func, objhgt = ...` is legal — real content
//! relies on it (planet/System.c4g/Commits.c:269 `var func, objhgt=...`).

use clonk_script::{Engine, Script, Value};

#[test]
fn func_keyword_is_a_valid_var_name() {
    // Commits.c declares `var func, objhgt=...` and never touches `func`
    // again; the declaration alone must compile and the sibling initializer
    // must work.
    let source = r#"
        global func Probe() {
            var func, objhgt = 5;
            return objhgt + 2;
        }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("keyword var name compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(7)
    );
}
