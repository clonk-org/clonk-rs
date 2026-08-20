//! `LocalN("name")` returns a reference to the executing object's named
//! local variable (FnLocalN, C4Script.cpp:4591-4605 — pObj defaults to
//! cthr->Obj). ArrowPack's PackCount() reads `LocalN("iUsedItems")`;
//! WaterTower assigns through it (`LocalN("iWater", pObj) += x`).

use clonk_script::Value;

run_cases! {
    // Named locals persist on the object between engine callbacks
    // (clonk-engine round-trips local_vars); within the VM a write in a helper
    // is visible to LocalN in the caller.
    localn_reads_a_named_object_local: r#"
        local iUsedItems;
        global func Prime() { LocalN("iUsedItems") = 7; return 0; }
        global func Probe() { Prime(); return LocalN("iUsedItems"); }
    "#, "Probe", &[] => Value::Int(7);

    // C++ LocalNamed.GetItem miss -> C4VNull.
    localn_unset_local_is_nil: r#"
        global func Probe() { return LocalN("never_set"); }
    "#, "Probe", &[] => Value::Nil;

    // FnLocalN returns pVarN->GetRef() (C4Script.cpp:4604): writes through.
    localn_is_assignable_like_a_reference: r#"
        local iWater;
        global func Probe() {
            LocalN("iWater") = 5;
            LocalN("iWater") += 2;
            // Read back through LocalN too: a global func may not name the
            // declaring host's local directly (C4AulParse.cpp:2000-2004).
            return LocalN("iWater");
        }
    "#, "Probe", &[] => Value::Int(7);
}
