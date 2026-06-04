// Parity: && and || return the operand value (Lua-style), not a coerced bool.
//
// C++ oracle: src/C4AulExec.cpp:999-1021
//   AB_JUMPAND: if (!a) jump (leave a on stack) else pop a, eval b (leave b)
//   AB_JUMPOR:  if ( a) jump (leave a on stack) else pop a, eval b (leave b)
// So `a && b` == (a falsy ? a : b) and `a || b` == (a truthy ? a : b), and the
// surviving value keeps its original type (int/object/...), it is NOT converted
// to a bool. The Rust VM previously returned Value::Bool, diverging whenever the
// result flowed into arithmetic or a comparison.

use lc_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[]).expect("call succeeds")
}

#[test]
fn and_returns_right_operand_when_left_truthy() {
    // 5 && 3 -> 3 (left truthy: pop, eval+leave right)
    assert_eq!(eval("func Test() { return 5 && 3; }"), Value::Int(3));
}

#[test]
fn and_returns_left_operand_when_left_falsy() {
    // 0 && 3 -> 0 (left falsy: short-circuit, leave left)
    assert_eq!(eval("func Test() { return 0 && 3; }"), Value::Int(0));
}

#[test]
fn or_returns_left_operand_when_left_truthy() {
    // 5 || 7 -> 5 (left truthy: short-circuit, leave left)
    assert_eq!(eval("func Test() { return 5 || 7; }"), Value::Int(5));
}

#[test]
fn or_returns_right_operand_when_left_falsy() {
    // 0 || 7 -> 7 (left falsy: pop, eval+leave right)
    assert_eq!(eval("func Test() { return 0 || 7; }"), Value::Int(7));
}

#[test]
fn logical_result_flows_into_arithmetic() {
    // (5 && 3) + 1 -> 4: only correct if && yields int 3, not bool true.
    assert_eq!(eval("func Test() { return (5 && 3) + 1; }"), Value::Int(4));
    // (0 || 10) * 2 -> 20
    assert_eq!(eval("func Test() { return (0 || 10) * 2; }"), Value::Int(20));
}
