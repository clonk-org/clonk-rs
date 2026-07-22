//! `++`/`--` on nil: AB_Inc1/AB_Dec1 run CheckOpPar<C4V_Int> which converts
//! nil to 0 before the operation (C4AulExec.cpp:450-458 via C4Value
//! FnCnvGuess, C4Value.cpp:453-466) — `var i; while(i++ < n)` is the standard
//! loop idiom in pre-strict content (e.g. Objects.c4d Loam placer).

use clonk_script::{Engine, Script, Value};

#[test]
fn postfix_increment_on_nil_counts_from_zero() {
    let source = r#"
        global func Probe() {
            var i;
            var hits;
            while (i++ < 3) hits = hits + 1;
            return i;
        }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(4)
    );
}

#[test]
fn prefix_increment_on_nil_yields_one() {
    let source = r#"
        global func Probe() {
            var i;
            return ++i;
        }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(1)
    );
}

#[test]
fn postfix_decrement_on_nil_yields_zero_then_minus_one() {
    let source = r#"
        global func Probe() {
            var i;
            var first = i--;
            return first * 100 + i;
        }
    "#;
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(-1)
    );
}
