//! `call_with_ref_args` — the host-side C4AulParSet-of-refs pattern
//! (C4Material.cpp:814-815: `parX.GetRef(), ...`): every argument is passed
//! as a reference cell, so callee `&` parameters alias them and their writes
//! are visible in the returned final values; plain parameters receive
//! dereferenced copies (C4Value.cpp:586-597 — refs convert to values for
//! every target type except `C4V_pC4Value`).

use clonk_script::{Engine, Script, Value};

fn engine_with(source: &str) -> Engine {
    let mut engine = Engine::new();
    engine.add_script(Script::compile(source).expect("script compiles"));
    engine
}

#[test]
fn ref_params_write_back_to_the_argument_cells() {
    let engine = engine_with(
        r#"
        global func Adjust(&x, y, &z) {
            x = x + 1;
            y = y + 100; // plain param: a copy, invisible to the caller
            z = "swapped";
            return y;
        }
        "#,
    );
    let (result, finals) = engine
        .call_with_ref_args("Adjust", &[Value::Int(10), Value::Int(20), Value::Int(30)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(120), "the local copy was mutated");
    assert_eq!(finals[0], Value::Int(11), "&x writes back");
    assert_eq!(finals[1], Value::Int(20), "plain y stays the caller's value");
    assert_eq!(finals[2], Value::String("swapped".into()), "&z writes back");
}

#[test]
fn untouched_ref_args_return_their_original_values() {
    let engine = engine_with(
        r#"
        global func Inspect(&x) { return x * 2; }
        "#,
    );
    let (result, finals) = engine
        .call_with_ref_args("Inspect", &[Value::Int(7)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(14));
    assert_eq!(finals, vec![Value::Int(7)]);
}
