// C4Aul's 512-entry context stack shares a separate 1,024-entry value stack.
// Every ordinary script frame retains ten parameter slots, so zero-local
// recursion reaches the value-stack ceiling first (C4AulExec.cpp:62-63,
// 182-221,330-363,1435-1462).

use clonk_script::{Engine, Value};

fn recurse_to(depth: i32) -> Result<Value, clonk_script::ScriptError> {
    let mut engine = Engine::new();
    engine
        .load_script("func Recurse(n) { if (n <= 0) { return 0; } return Recurse(n - 1) + 1; }")
        .expect("loads");
    engine.call("Recurse", &[Value::Int(depth)])
}

#[test]
fn recursion_past_old_limit_of_64_now_runs() {
    // The historical Rust-only 64-frame limit was still too small: C++ can
    // execute this depth before its value-stack ceiling becomes relevant.
    assert_eq!(
        recurse_to(90).expect("90-deep recursion runs"),
        Value::Int(90)
    );
}

#[test]
fn recursion_at_the_value_stack_boundary_runs() {
    // The initial call plus 101 recursive calls retain 1,020 parameter slots;
    // the base-case comparison peaks at 1,022.
    assert_eq!(
        recurse_to(101).expect("the last fitting recursion depth runs"),
        Value::Int(101)
    );
}

#[test]
fn recursion_beyond_the_value_stack_limit_errors_cleanly() {
    let error = recurse_to(102).expect_err("the 103rd active frame must overflow");
    let clonk_script::ScriptError::Runtime(error) = error else {
        panic!("expected runtime value-stack overflow, got {error}");
    };
    assert_eq!(error.message(), "internal error: value stack overflow!");
}
