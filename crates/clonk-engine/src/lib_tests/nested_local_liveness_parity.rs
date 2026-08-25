//! A call that re-enters an object whose script is already running reads that
//! object's named locals live.
//!
//! C++ keeps named locals on the `C4Object` itself (`Local[]`), so re-entering
//! it mid-call reads whatever the outer frame has written so far — there is no
//! per-call copy to diverge from.
//!
//! This is **characterisation**, not a regression fix. `compat/profile.json`
//! carries `sim-script-nested-local-snapshot`, which says a call into an object
//! whose script is already in flight starts from the pre-call snapshot
//! (clonk-org/clonk-rs#1094). The most direct construction of that claim — an
//! object writing a named local, leaving its frame through a second object, and
//! being called straight back into — does **not** reproduce it: the port agrees
//! with C++ here, because `session_local_cells` shares one cell table per object
//! for as long as a session is registered, and the ordinary entry paths register
//! one.
//!
//! So this pins the behaviour that does hold, and records that the divergence
//! needs a reproducing case naming the entry path that fails to register before
//! it can be closed or reclassified. It is deliberately *not* grounds for
//! removing the profile entry.

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
