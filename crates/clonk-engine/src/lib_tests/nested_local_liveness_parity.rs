//! A call that re-enters an object whose script is already running reads that
//! object's named locals live.
//!
//! C++ keeps named locals on the `C4Object` itself (`Local[]`), so re-entering
//! it mid-call reads whatever the outer frame has written so far — there is no
//! per-call copy to diverge from.
//!
//! This is **characterisation**, not a regression fix. `compat/profile.json`
//! used to carry `sim-script-nested-local-snapshot`, which said such a call
//! starts from the pre-call snapshot (clonk-org/clonk-rs#1094). It does not,
//! and the entry has been removed.
//!
//! The mechanism is `session_local_cells`: one cell table per object, shared
//! for as long as a session is registered. Both nested-call sites consult it
//! *before* the snapshot (`contexts.rs:2890`, `:3177`), so the snapshot only
//! ever seeds the first registration — whenever a session exists its live
//! cells win and the snapshot is discarded.
//!
//! For the entry to be real, some path would have to run an object's script
//! without registering. None does:
//!
//! - every object-script entry in `definition.rs` funnels through
//!   `exec_in_object_context_for_definition`, which registers before invoking;
//! - the nested-call paths register when they create a session;
//! - the remaining `with_cells` sites are definition- or global-level calls
//!   with `this = Nil` and no object locals, and `Eval()`
//!   (`eval_direct_exec_hook`) passes the caller's live cells straight through.
//!
//! The three tests below cover the three branches of `nested_call_prep`
//! (`contexts.rs:6117-6146`) — re-entry of the running scope, of a dormant
//! scope, and of a freshly built one — which are the only places the snapshot
//! could have reached a call.

use super::*;
use crate::lib_test_support::spawn_fixture;

/// Not a value any uninitialised local could hold, so reading the snapshot
/// cannot pass by coincidence.
const WRITTEN: i32 = 41;

fn register(engine: &mut Engine, id: &str, source: &str) {
    crate::TestValueExt::test_value(engine.register_script_definition(id, id, source));
}

fn call(engine: &mut Engine, object: ObjectId, function: &str) -> Value {
    let index = crate::TestValueExt::test_value(engine.find_object_index(object));
    crate::TestValueExt::test_value(engine.call_object_function(index, function, Vec::new()))
}

#[test]
fn re_entering_a_running_object_reads_its_locals_live() {
    let mut engine = Engine::new();
    // HOME writes a local, then leaves its own frame through AWAY, which calls
    // straight back in. The re-entrant Read must see the write.
    register(
        &mut engine,
        "HOME",
        &format!(
            "#strict 3\n\
             local mark;\n\
             func Outer(pAway) {{\n\
             \x20 mark = {WRITTEN};\n\
             \x20 return ObjectCall(pAway, \"Bounce\", this());\n\
             }}\n\
             func Read() {{ return mark; }}\n"
        ),
    );
    register(
        &mut engine,
        "AWAY",
        "#strict 3\n\
         func Bounce(pHome) { return ObjectCall(pHome, \"Read\"); }\n",
    );
    engine.resolve_appends();
    crate::TestValueExt::test_value(engine.resolve_includes());

    let home = spawn_fixture!(engine, "HOME");
    let away = spawn_fixture!(engine, "AWAY");

    let index = crate::TestValueExt::test_value(engine.find_object_index(home));
    let seen = crate::TestValueExt::test_value(engine.call_object_function(
        index,
        "Outer",
        vec![crate::object_reference_value(away)],
    ));

    assert_eq!(
        seen,
        Value::Int(WRITTEN),
        "the re-entrant read must see the outer frame's write, not a pre-call copy"
    );
    // And the write is still there once the outer frame returns.
    assert_eq!(call(&mut engine, home, "Read"), Value::Int(WRITTEN));
}

/// `AddEffect` runs `Fx*Start` synchronously inside the adding call
/// (`C4Effect::C4Effect` -> `CallStart`, C4Effect.cpp:106-130), so the
/// callback re-enters an object whose own script is still in flight. C++
/// reads `Local[]` off the live `C4Object`, so the callback must see the
/// outer frame's write.
#[test]
fn an_effect_start_callback_reads_the_adding_frames_local_write() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "EFHM",
        &format!(
            "#strict 3\n\
             local mark, seen;\n\
             func Outer() {{\n\
             \x20 mark = {WRITTEN};\n\
             \x20 AddEffect(\"Probe\", this(), 1, 0, this());\n\
             \x20 return seen;\n\
             }}\n\
             func FxProbeStart(pTarget, iNumber) {{ seen = mark; }}\n"
        ),
    );
    engine.resolve_appends();
    crate::TestValueExt::test_value(engine.resolve_includes());

    let home = spawn_fixture!(engine, "EFHM");

    assert_eq!(
        call(&mut engine, home, "Outer"),
        Value::Int(WRITTEN),
        "the effect start callback must read the adding frame's write, not a pre-call copy"
    );
}

/// The third entry path into `nested_call_prep`: a call whose target is the
/// scope already on top (`contexts.rs:6117-6122`), rather than a dormant
/// scope or a fresh one. C++ has no per-call copy here either.
#[test]
fn calling_straight_back_into_the_running_scope_reads_its_local_live() {
    let mut engine = Engine::new();
    register(
        &mut engine,
        "SELF",
        &format!(
            "#strict 3\n\
             local mark;\n\
             func Outer() {{\n\
             \x20 mark = {WRITTEN};\n\
             \x20 return ObjectCall(this(), \"Read\");\n\
             }}\n\
             func Read() {{ return mark; }}\n"
        ),
    );
    engine.resolve_appends();
    crate::TestValueExt::test_value(engine.resolve_includes());

    let home = spawn_fixture!(engine, "SELF");

    assert_eq!(
        call(&mut engine, home, "Outer"),
        Value::Int(WRITTEN),
        "a self re-entry must read the running frame's write, not a pre-call copy"
    );
}
