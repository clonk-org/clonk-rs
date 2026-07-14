// Parity: `==` / `!=` semantics depend on the script's #strict level.
//
// C++ oracle: C4Value::Equals (C4Value.cpp:823):
//   NONSTRICT / STRICT1 -> _getRaw() == other._getRaw()  (raw Data compare)
//   STRICT2             -> operator==                     (cross-type numeric)
//   STRICT3             -> type-checked: different types are never equal
// The raw / cross-type levels make the numeric coincidences equal (0 == nil,
// 1 == true, 0 == false), because Int/Bool/nil share the integer Data slot.
// Pointer-backed strings/arrays/maps compare by raw identity below STRICT2,
// by content at STRICT2, and with matching outer types at STRICT3.

use lc_script::{Engine, Script, Value};

fn eval(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.load_script(source).expect("loads");
    engine.call("Test", &[]).expect("call")
}

fn eval_with_format(source: &str) -> Value {
    let mut engine = Engine::new();
    engine.register_host_function("Format", |_| Ok(Value::String("a".into())));
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
fn strict1_strings_compare_raw_identity() {
    assert_eq!(
        eval_with_format("#strict 1\nfunc Test() { return Format(\"%s\", \"a\") == \"a\"; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var s = \"a\"; return s == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format(
            "#strict 1\nfunc Test() { var s = Format(\"%s\", \"a\"); var t = s; return s == t; }"
        ),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Same(string s) { return s == \"a\"; } func Test() { return Same(\"a\"); }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Literal() { return \"a\"; } func Test() { return Literal() == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict 1\nfunc Identity(s) { return s; } func Test() { var s = Format(\"%s\", \"a\"); return Identity(s) == s; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict 1\nfunc Test() { var s = Format(\"%s\", \"a\"); var t = s ?? \"fallback\"; return t == s; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict 1\nfunc SetLiteral(&s) { s = \"a\"; } func Test() { var s = Format(\"%s\", \"a\"); SetLiteral(s); return s == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = [\"a\"]; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval_with_format("#strict 1\nfunc Test() { var a = [Format(\"%s\", \"a\")]; return a[0] == a[0] && a[0] != \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = [nil]; a[0] = \"a\"; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = [\"a\"] .. []; return a[0] == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { SetLocal(0, \"a\"); return Local(0) == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { return SetLocal(0, \"a\") == \"a\"; }"),
        Value::Bool(true)
    );
    let mut engine = Engine::new();
    engine
        .load_script("#strict 1\nfunc Test(target) { return target->SetLocal(0, \"a\") == \"a\"; }")
        .expect("loads");
    assert_eq!(
        engine.call("Test", &[Value::Object(1)]).expect("call"),
        Value::Bool(true)
    );
}

#[test]
fn strict1_arrays_compare_raw_identity() {
    assert_eq!(
        eval("#strict 1\nfunc Test() { return [1] == [1]; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = [1]; var b = a; return a == b; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Same(&a) { return a == a; } func Test() { var a = [1]; return Same(a); }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = [1]; var b = a; b[0] = 1; return a == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Touch(&value) {} func Test() { var a = [1]; var b = a; Touch(a[0]); return a == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Touch(&value) {} func Test() { var inner = [1]; var a = [inner]; var b = inner; Touch(a[0][0]); return a[0] == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { var a = { x = 1 }; var b = a; b.x = 1; return a == b; }"),
        Value::Bool(false)
    );
    assert_eq!(
        eval("#strict 1\nfunc Test() { Local(0) = [1]; var old = Local(0); SetLocal(0, \"x\"); return Local(0) == old; }"),
        Value::Bool(false)
    );
}

#[test]
fn strict2_pointer_values_keep_content_equality() {
    assert_eq!(
        eval_with_format("#strict 2\nfunc Test() { return Format(\"%s\", \"a\") == \"a\"; }"),
        Value::Bool(true)
    );
    assert_eq!(
        eval("#strict 2\nfunc Test() { return [1] == [1]; }"),
        Value::Bool(true)
    );
}

#[test]
fn strict1_shared_cells_and_string_constants_keep_identity() {
    let globals = lc_script::new_global_variables();
    let constants = lc_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals);
    engine.set_global_constants(constants);
    engine.register_host_function("Format", |_| Ok(Value::String("a".into())));
    engine.add_script(
        Script::compile(
            "#strict 1\nstatic s; static const S = \"a\";\nfunc Test() { s = Format(\"%s\", \"a\"); var t = s; return [s == s, s == t, S == \"a\", S() == \"a\"]; }",
        )
        .expect("compiles"),
    );
    engine.adopt_statics_into_globals();

    assert_eq!(
        engine.call("Test", &[]).expect("call"),
        Value::Array(vec![
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
        ])
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
