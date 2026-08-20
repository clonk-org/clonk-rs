//! `??` / `??=` — the nil-coalescing operators (C4ScriptOpMap: priority 3
//! `AB_NilCoalescing` between `||` and the assignments, and the priority-2
//! `AB_NilCoalescingIt` setter, C4AulParse.cpp:464,477). Coalescing is on
//! NIL only: 0 and false are kept, unlike `||`. Real content depends on it
//! (Fantasy.c4d/.../Goblet.c4d/Script.c:69).

use clonk_script::{Engine, Value};

fn run(source: &str, function: &str) -> Value {
    crate::support::run(source, function, &[])
}

#[test]
fn nil_coalescing_keeps_falsy_non_nil_values() {
    let source = r#"
        #strict 3
        global func KeepZero() { return 0 ?? 7; }
        global func KeepFalse() { return false ?? 7; }
        global func TakeRight() { var a; return a ?? 7; }
    "#;
    assert_eq!(run(source, "KeepZero"), Value::Int(0), "0 is not nil");
    assert_eq!(
        run(source, "KeepFalse"),
        Value::Bool(false),
        "false is not nil"
    );
    assert_eq!(
        run(source, "TakeRight"),
        Value::Int(7),
        "nil takes the right side"
    );
}

#[test]
fn nil_coalescing_short_circuits_the_right_side() {
    // AB_JUMPNOTNIL skips the right operand when the left is non-nil
    // (C4AulParse.cpp:1050-1056).
    let source = r#"
        local hits;
        func Bump() { hits = hits + 1; return 99; }
        func Probe() { return 1 ?? Bump(); }
        func Hits() { return hits; }
    "#;
    let mut engine = Engine::new();
    crate::support::load_script(&mut engine, source);
    let locals = std::collections::HashMap::new();
    let (value, locals) = engine
        .call_with_locals("Probe", &[], &locals)
        .expect("call succeeds");
    assert_eq!(value, Value::Int(1));
    assert_eq!(
        locals.get("hits").cloned().unwrap_or(Value::Nil),
        Value::Nil,
        "the right side never ran"
    );
}

#[test]
fn nil_coalescing_binds_looser_than_or() {
    // Priority 3 vs 4: `a || b ?? c` parses as `(a || b) ?? c`.
    let source = r#"
        global func Probe() { return false || false ?? 42; }
    "#;
    // (false || false) is false — non-nil — so ?? keeps it.
    assert_eq!(run(source, "Probe"), Value::Bool(false));
}

#[test]
fn nil_coalescing_assignment_sets_only_when_nil() {
    let source = r#"
        #strict 3
        global func FillsNil() { var a; a ??= 5; return a; }
        global func KeepsZero() { var a = 0; a ??= 5; return a; }
        global func KeepsValue() { var a = 3; a ??= 5; return a; }
    "#;
    assert_eq!(run(source, "FillsNil"), Value::Int(5));
    assert_eq!(run(source, "KeepsZero"), Value::Int(0), "0 is not nil");
    assert_eq!(run(source, "KeepsValue"), Value::Int(3));
}
