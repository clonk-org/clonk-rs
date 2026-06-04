// Parity: the script call-stack depth limit matches C++ MAX_CONTEXT_STACK=512
// (C4AulExec.cpp:62,143-145). A script that recurses 65-511 deep runs in C++ but
// used to error in Rust at the old limit of 64.

use lc_script::{Engine, Value};

fn recurse_to(depth: i32) -> Result<Value, lc_script::ScriptError> {
    let mut engine = Engine::new();
    engine
        .load_script("func Recurse(n) { if (n <= 0) { return 0; } return Recurse(n - 1) + 1; }")
        .expect("loads");
    engine.call("Recurse", &[Value::Int(depth)])
}

#[test]
fn recursion_past_old_limit_of_64_now_runs() {
    // 200 deep: errored at the old 64 limit, must run under C++'s 512.
    assert_eq!(
        recurse_to(200).expect("200-deep recursion runs"),
        Value::Int(200)
    );
}

#[test]
fn recursion_near_the_limit_runs() {
    assert_eq!(
        recurse_to(500).expect("500-deep recursion runs"),
        Value::Int(500)
    );
}

#[test]
fn recursion_far_beyond_the_limit_errors_cleanly() {
    // Well past 512: a clean error, never a crash.
    assert!(recurse_to(5000).is_err());
}
