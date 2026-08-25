// Parity: Var(n) / Local(n) are numeric scratch slots, SEPARATE from named
// variables, matching C++.
//
// C++ oracle:
//   - `Var(n)`  -> `Caller->NumVars[n]` (C4Script.cpp:3390), a per-call C4ValueList
//     SEPARATE from the named `var x` storage (`ctx.Vars`, C4AulExec.cpp:411).
//   - `Local(n)` -> `pObj->Local[n]` (C4Script.cpp:3408+), the object's numeric
//     local array, SEPARATE from named `local x` (`pObj->LocalNamed`).
//   - `C4ValueList::GetItem` (C4ValueList.cpp:50) clamps a negative index to 0 and
//     auto-extends (nil-filled) for indices past the end.
//
// The two accessors differ on a NEGATIVE index, and only `Var` reaches the
// clamp: `FnVar` indexes `NumVars` directly, while `FnLocal` returns
// `C4VNull` first (`if (iIndex < 0) return C4VNull;`, C4Script.cpp:3421), so
// `Local(-1)` never aliases `Local(0)`.
//
// These tests pin the separateness, round-trip, auto-extend, and the two
// different negative-index rules.

use clonk_script::{Engine, Value};
use std::collections::HashMap;

eval_cases! {
    var_slot_round_trips:
        "func Test() { Var(0) = 7; return Var(0); }" => Value::Int(7);

    // `var x` is named storage; `Var(0)` is the separate NumVars scratch.
    var_slot_is_separate_from_named_var:
        "func Test() { var x = 5; Var(0) = 99; return x; }" => Value::Int(5);
    unset_var_slot_reads_as_nil:
        "func Test() { return Var(3); }" => Value::Nil;

    // Typed C4ValueInt engine arguments call C4Value::getInt, so a nil loop
    // counter reaches FnLocal as index zero (C4Value.h:159,317-321;
    // C4Script.cpp:3423-3433). SLCR::CountTargets relies on `var i; Local(i)`.
    nil_local_index_converts_to_zero:
        "func Test() { Local(0) = 17; var i; return Local(i); }" => Value::Int(17);

    // C4ValueList::GetItem clamps index < 0 to 0, so Var(-1) aliases Var(0).
    negative_var_index_clamps_to_zero:
        "func Test() { Var(0) = 42; return Var(-1); }" => Value::Int(42);
    // FnLocal returns C4VNull before the C4ValueList clamp can apply
    // (C4Script.cpp:3421), so a negative index reads nil rather than slot 0.
    // Objects.c4d/Environment.c4d/Glint.c4d depends on this: it keys a lookup
    // table by material index and probes it with GetMaterial, which returns -1
    // for sky. Aliasing slot 0 there made the port sparkle where C++ does not
    // and consumed synchronized Random() draws (clonk-org/clonk-rs#1066).
    negative_local_index_reads_nil:
        "func Test() { Local(0) = 8; return Local(-1); }" => Value::Nil;
    // The shape Glint actually uses: the table lookup is a CONDITION, so a
    // clamped -1 silently took the branch keyed at slot 0.
    negative_local_index_is_falsy_in_a_condition:
        // Sentinels are both non-zero: a literal 0 folds to nil below
        // strict 3 and would mask the branch under test.
        "func Test() { Local(0) = 3; if (Local(-1)) { return 1; } return 2; }"
            => Value::Int(2);
    // A write through a negative index must not reach slot 0 either.
    negative_local_index_write_does_not_touch_slot_zero:
        "func Test() { Local(0) = 5; Local(-1) = 9; return Local(0); }" => Value::Int(5);

    // Var/Local slots are function-scoped, not block-scoped (C++ NumVars/Local are
    // flat per-call/object arrays), so a write inside a block persists after it.
    slot_write_inside_block_is_visible_after:
        "func Test() { if (1) { Var(0) = 11; } return Var(0); }" => Value::Int(11);
}

#[test]
fn local_slot_persists_through_local_vars_round_trip() {
    // C++ Local(n) (pObj->Local) persists on the object across calls. The Rust VM
    // round-trips the slots through the object's `local_vars` map (the same path
    // clonk-engine uses). A later call sees the earlier Local(n) write.
    let mut engine = Engine::new();
    engine
        .load_script("func Store() { Local(0) = 42; } func Fetch() { return Local(0); }")
        .expect("loads");
    let (_, locals) = engine
        .call_with_locals("Store", &[], &HashMap::new())
        .expect("Store call");
    let (result, _) = engine
        .call_with_locals("Fetch", &[], &locals)
        .expect("Fetch call");
    assert_eq!(result, Value::Int(42));
}

#[test]
fn var_slot_does_not_persist_across_calls() {
    // Var(n) (C++ NumVars) is per-call scratch, NOT persisted like Local(n).
    let mut engine = Engine::new();
    engine
        .load_script("func Store() { Var(0) = 42; } func Fetch() { return Var(0); }")
        .expect("loads");
    let (_, locals) = engine
        .call_with_locals("Store", &[], &HashMap::new())
        .expect("Store call");
    let (result, _) = engine
        .call_with_locals("Fetch", &[], &locals)
        .expect("Fetch call");
    assert_eq!(result, Value::Nil);
}
