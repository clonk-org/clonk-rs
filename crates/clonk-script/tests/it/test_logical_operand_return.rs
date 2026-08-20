// Parity: at #strict 2, && and || return the operand value (Lua-style),
// not a coerced bool.
//
// C++ oracle: src/C4AulExec.cpp:999-1021 + C4AulParse.cpp:3003
//   AB_JUMPAND/AB_JUMPOR (STRICT2 only): the surviving operand keeps its
//   original type. Below STRICT2 the EAGER AB_And/AB_Or opcodes run:
//   both sides always evaluate and the result coerces to bool
//   (C4AulExec.cpp:733-748).

use clonk_script::Value;

eval_cases! {
    // 5 && 3 -> 3 (left truthy: pop, eval+leave right)
    and_returns_right_operand_when_left_truthy:
        "#strict 2\nfunc Test() { return 5 && 3; }" => Value::Int(3);

    // Below strict 3 the literal 0 is nil; short-circuiting leaves that nil.
    and_returns_left_operand_when_left_falsy:
        "#strict 2\nfunc Test() { return 0 && 3; }" => Value::Nil;

    // 5 || 7 -> 5 (left truthy: short-circuit, leave left)
    or_returns_left_operand_when_left_truthy:
        "#strict 2\nfunc Test() { return 5 || 7; }" => Value::Int(5);

    // 0 || 7 -> 7 (left falsy: pop, eval+leave right)
    or_returns_right_operand_when_left_falsy:
        "#strict 2\nfunc Test() { return 0 || 7; }" => Value::Int(7);
}

#[test]
fn logical_result_flows_into_arithmetic() {
    // (5 && 3) + 1 -> 4: only correct if && yields int 3, not bool true.
    assert_eq!(
        crate::support::eval("#strict 2\nfunc Test() { return (5 && 3) + 1; }"),
        Value::Int(4)
    );
    // (0 || 10) * 2 -> 20
    assert_eq!(
        crate::support::eval("#strict 2\nfunc Test() { return (0 || 10) * 2; }"),
        Value::Int(20)
    );
}
