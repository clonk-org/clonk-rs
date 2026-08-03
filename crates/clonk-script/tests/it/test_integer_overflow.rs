// Parity: integer arithmetic wraps on 32-bit two's-complement overflow.
//
// C++ oracle: src/C4AulExec.cpp:546-553
//   case AB_Sub: pPar1->SetInt(pPar1->_getInt() - pPar2->_getInt());
// C4ValueInt is int32_t, so the difference wraps silently and the script keeps
// running. A checked Rust `-` panics instead in every profile that keeps
// overflow checks on (dev, play, test), aborting the engine where C++ carries
// on — and a panic on a script-reachable path is forbidden outright.

use clonk_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[]).expect("call succeeds")
}

#[test]
fn subtraction_below_int_min_wraps_like_cpp() {
    // -2147483647 - 2 == 2147483647 (C4AulExec.cpp:550)
    assert_eq!(
        eval("func Test() { var a = -2147483647; return a - 2; }"),
        Value::Int(i32::MAX)
    );
}

#[test]
fn subtraction_above_int_max_wraps_like_cpp() {
    // 2147483647 - -2 == -2147483647 (C4AulExec.cpp:550)
    assert_eq!(
        eval("func Test() { var a = 2147483647; return a - -2; }"),
        Value::Int(i32::MIN + 1)
    );
}

#[test]
fn compound_subtraction_below_int_min_wraps_like_cpp() {
    // `-=` lowers to the same AB_Sub arm (parser.rs Symbol::MinusEqual).
    assert_eq!(
        eval("func Test() { var a = -2147483647; a -= 2; return a; }"),
        Value::Int(i32::MAX)
    );
}

#[test]
fn nonoverflowing_subtraction_is_unaffected() {
    assert_eq!(eval("func Test() { return 17 - 5; }"), Value::Int(12));
    assert_eq!(
        eval("func Test() { var a = 17; a -= 5; return a; }"),
        Value::Int(12)
    );
}
