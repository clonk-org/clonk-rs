// Parity: integer division/modulo by zero must yield 0, not an error.
//
// C++ oracle: src/C4AulExec.cpp:500-528
//   case AB_Div: if (pPar2->_getInt()) pPar1->SetInt(a / b); else pPar1->Set0();
//   case AB_Mod: if (pPar2->_getInt()) pPar1->SetInt(a % b); else pPar1->Set0();
// The C4Script VM silently produces 0 when the divisor is zero. The Rust VM
// previously threw a runtime error ("division by zero" / "modulo by zero"),
// which aborts the calling script instead of continuing with 0 — a divergence
// from the golden engine on every div/mod-by-zero, breaking lockstep.

use clonk_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[]).expect("call succeeds")
}

#[test]
fn division_by_zero_yields_zero_like_cpp() {
    // 5 / 0 == 0 (C4AulExec.cpp:507 pPar1->Set0())
    assert_eq!(eval("func Test() { return 5 / 0; }"), Value::Int(0));
}

#[test]
fn modulo_by_zero_yields_zero_like_cpp() {
    // 5 % 0 == 0 (C4AulExec.cpp:526 pPar1->Set0())
    assert_eq!(eval("func Test() { return 5 % 0; }"), Value::Int(0));
}

#[test]
fn division_by_zero_via_variable_yields_zero() {
    assert_eq!(
        eval("func Test() { var d = 0; return 42 / d; }"),
        Value::Int(0)
    );
}

#[test]
fn modulo_by_zero_via_variable_yields_zero() {
    assert_eq!(
        eval("func Test() { var d = 0; return 42 % d; }"),
        Value::Int(0)
    );
}

#[test]
fn nonzero_division_is_unaffected() {
    assert_eq!(eval("func Test() { return 17 / 5; }"), Value::Int(3));
    assert_eq!(eval("func Test() { return 17 % 5; }"), Value::Int(2));
}
