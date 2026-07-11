// Parity: `==` / `!=` semantics depend on the script's #strict level.
//
// C++ oracle: C4Value::Equals (C4Value.cpp:823):
//   NONSTRICT / STRICT1 -> _getRaw() == other._getRaw()  (raw Data compare)
//   STRICT2             -> operator==                     (cross-type numeric)
//   STRICT3             -> type-checked: different types are never equal
// The raw / cross-type levels make the numeric coincidences equal (0 == nil,
// 1 == true, 0 == false), because Int/Bool/nil share the integer Data slot.
// STRICT3 treats them as distinct types. In Rust, Value is a value type (no
// pointer identity), so NONSTRICT/STRICT1/STRICT2 collapse to one "lenient"
// rule (Int/Bool/nil compared by integer value, everything else by content),
// and STRICT3 keeps the type-checked rule. The VM previously ignored #strict
// and always used the type-checked rule.

use lc_script::{Engine, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("loads");
    engine.call("Test", &[]).expect("call")
}

#[test]
fn nonstrict_treats_zero_and_nil_as_equal() {
    assert_eq!(eval("func Test() { return 0 == nil; }"), Value::Bool(true));
}

#[test]
fn nonstrict_treats_one_and_true_as_equal() {
    assert_eq!(eval("func Test() { return 1 == true; }"), Value::Bool(true));
    assert_eq!(
        eval("func Test() { return 0 == false; }"),
        Value::Bool(true)
    );
}

#[test]
fn strict3_distinguishes_types() {
    assert_eq!(
        eval("#strict 3\nfunc Test() { return 0 == nil; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 3\nfunc Test() { return 1 == true; }"),
        Value::Bool(false)
    );
}

#[test]
fn strict3_not_equal_is_inverse() {
    assert_eq!(
        eval("#strict 3\nfunc Test() { return 0 != nil; }"),
        Value::Bool(true)
    );
}

#[test]
fn same_type_equality_holds_at_all_levels() {
    assert_eq!(eval("func Test() { return 5 == 5; }"), Value::Bool(true));
    assert_eq!(
        eval("#strict 3\nfunc Test() { return 5 == 5; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("func Test() { return nil == nil; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 3\nfunc Test() { return 7 != 8; }"),
        Value::Bool(true)
    );
}
