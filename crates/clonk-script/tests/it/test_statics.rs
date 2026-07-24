//! C4Aul `static` variables live in ONE engine-global named table
//! (C4AulScriptEngine::GlobalNamed): every script host reads and writes
//! the same values. Identifier resolution finds them after locals and
//! before global constants (C4AulParse.cpp:2836-2839 — "global constants
//! have lowest priority").

use clonk_script::{Engine, Script, Value};

#[test]
fn statics_share_one_table_across_script_hosts() {
    let table = clonk_script::new_global_variables();
    let mut writer = Engine::new();
    writer.set_global_variables(table.clone());
    writer.add_script(
        Script::compile(
            "static counter;\n\
             public func Bump() { counter = counter + 1; return counter; }",
        )
        .expect("compiles"),
    );
    let mut reader = Engine::new();
    reader.set_global_variables(table.clone());
    reader.add_script(Script::compile("public func Read() { return counter; }").expect("compiles"));

    assert_eq!(writer.call("Bump", &[]).expect("bump"), Value::Int(1));
    assert_eq!(writer.call("Bump", &[]).expect("bump"), Value::Int(2));
    assert_eq!(
        reader.call("Read", &[]).expect("read"),
        Value::Int(2),
        "the second host sees the first host's writes"
    );
}

#[test]
fn statics_do_not_become_object_locals() {
    // Without the shared table a `static` falls back to the old per-host
    // var_decl behavior; with it, the name must NOT be duplicated into the
    // per-object locals (each object would get its own copy).
    let table = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(table.clone());
    engine.add_script(
        Script::compile("static shared;\npublic func Set(v) { shared = v; return shared; }")
            .expect("compiles"),
    );
    let locals = std::collections::HashMap::new();
    let (value, finals) = engine
        .call_with_locals("Set", &[Value::Int(9)], &locals)
        .expect("call succeeds");
    assert_eq!(value, Value::Int(9));
    assert!(
        !finals.contains_key("shared"),
        "statics never serialize into object locals"
    );
    assert_eq!(
        table
            .borrow()
            .get("shared")
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(9))
    );
}
