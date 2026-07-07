//! Host functions reach the CALLING script function's numbered `Var(n)`
//! slots — the `cthr->Caller->NumVars` seam. FnFindConstructionSite reads
//! and writes them (C4Script.cpp:1958-1981); the planet System.c4g
//! FindConstructionSiteX wrapper stages coordinates through Var(0)/Var(1)
//! (Commits.c:384-390).

use lc_script::{Engine, RuntimeError, Script, Value};

#[test]
fn host_functions_read_and_write_the_callers_var_slots() {
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| {
        let slots = lc_script::caller_var_slots()
            .ok_or_else(|| RuntimeError::new("host fn must see its script caller"))?;
        // Caller->NumVars[0] reads the staged value...
        let seen = slots.get(0);
        // ...and writes land back in the caller's slot (V2 = C4VInt(v2),
        // C4Script.cpp:1978).
        slots.set(1, Value::Int(99));
        Ok(seen)
    });
    engine.add_script(
        Script::compile(
            "#strict\n\nfunc Run() { Var(0) = 42; if (Probe() != 42) return(-1); return(Var(1)); }\n",
        )
        .expect("script compiles"),
    );
    assert_eq!(engine.call("Run", &[]).expect("run succeeds"), Value::Int(99));
}

#[test]
fn host_functions_without_a_script_caller_see_none() {
    // `if (!cthr->Caller) return {}` (C4Script.cpp:1966): engine-driven
    // host calls have no caller var space.
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| {
        Ok(Value::Bool(lc_script::caller_var_slots().is_some()))
    });
    engine.add_script(Script::compile("#strict\n").expect("compiles"));
    assert_eq!(
        engine.call("Probe", &[]).expect("direct host call runs"),
        Value::Bool(false)
    );
}

#[test]
fn unset_caller_var_slots_read_nil() {
    // C4ValueList::GetItem: out-of-range/unset slots are nil.
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| {
        let slots = lc_script::caller_var_slots()
            .ok_or_else(|| RuntimeError::new("host fn must see its script caller"))?;
        Ok(Value::Bool(slots.get(7) == Value::Nil))
    });
    engine.add_script(
        Script::compile("#strict\n\nfunc Run() { return(Probe()); }\n").expect("compiles"),
    );
    assert_eq!(engine.call("Run", &[]).expect("runs"), Value::Bool(true));
}
