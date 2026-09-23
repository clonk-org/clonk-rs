// Parity: at #strict 2, && and || return the operand value (Lua-style),
// not a coerced bool.
//
// C++ oracle: src/C4AulExec.cpp:999-1021 + C4AulParse.cpp:3003
//   AB_JUMPAND/AB_JUMPOR (STRICT2 only): the surviving operand keeps its
//   original type. Below STRICT2 the EAGER AB_And/AB_Or opcodes run:
//   both sides always evaluate and the result coerces to bool
//   (C4AulExec.cpp:733-748).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use clonk_script::{Engine, Value};

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

#[test]
fn an_or_with_a_falsy_left_assigns_through_its_right_operand() {
    // `||` binds tighter than `=`, and C4Aul checks no assignment target while
    // it parses. `||` drops its left operand's reference
    // (C4AulParse.cpp:2998-3000). AB_JUMPOR pops a falsy left operand
    // (C4AulExec.cpp:1011-1021), and the right operand keeps its reference for
    // AB_Set. S2Tower's DTUtility.c defines `Isolate` with
    // `obj || obj = this;`.
    let mut engine = Engine::new();
    crate::support::load_script(
        &mut engine,
        "#strict 2\nfunc Default(value) { value || value = 5; return value; }",
    );
    assert_eq!(
        engine.call("Default", &[Value::Nil]).expect("Default runs"),
        Value::Int(5)
    );
}

#[test]
fn an_or_with_a_truthy_left_leaves_no_reference_to_assign() {
    // AB_JUMPOR keeps the truthy left operand, which is a plain value, so
    // AB_Set rejects it after its right side ran (C4AulExec.cpp:266-275,
    // 858-865, 1011-1021).
    let calls = Arc::new(AtomicUsize::new(0));
    let mut engine = Engine::new();
    crate::support::load_script(
        &mut engine,
        "#strict 2\nfunc Default(value) { value || value = RightSide(); return value; }",
    );
    {
        let calls = Arc::clone(&calls);
        engine.register_host_function("RightSide", move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Int(5))
        });
    }
    let error = engine
        .call("Default", &[Value::Int(3)])
        .expect_err("a value is no reference to assign through");
    assert!(
        error
            .to_string()
            .contains(r#"operator "=" left side: got "int", but expected "&"!"#),
        "got: {error}"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the right side ran first");
}

#[test]
fn and_and_nil_coalescing_assign_through_the_right_operand_they_select() {
    // `&&` and `??` drop their left operand's reference too, and AB_JUMPAND
    // and AB_JUMPNOTNIL pop the left operand when they go on to the right one
    // (C4AulParse.cpp:2998-3027; C4AulExec.cpp:999-1009, 1032-1042).
    for (source, argument) in [
        (
            "#strict 2\nfunc Update(value) { value && value = 5; return value; }",
            Value::Int(3),
        ),
        (
            "#strict 2\nfunc Update(value) { value ?? value = 5; return value; }",
            Value::Nil,
        ),
    ] {
        let mut engine = Engine::new();
        crate::support::load_script(&mut engine, source);
        assert_eq!(
            engine.call("Update", &[argument]).expect("Update runs"),
            Value::Int(5),
            "{source}"
        );
    }
}

#[test]
fn an_or_below_strict_two_leaves_a_bool_that_cannot_be_assigned() {
    // Below #strict 2 `||` is the eager AB_Or, whose bool result AB_Set
    // rejects (C4AulExec.cpp:266-275, 733-748, 858-865).
    let mut engine = Engine::new();
    crate::support::load_script(
        &mut engine,
        "#strict\nfunc Default(value) { value || value = 5; return value; }",
    );
    let error = engine
        .call("Default", &[Value::Nil])
        .expect_err("a bool is no reference to assign through");
    assert!(
        error
            .to_string()
            .contains(r#"operator "=" left side: got "bool", but expected "&"!"#),
        "got: {error}"
    );
}
