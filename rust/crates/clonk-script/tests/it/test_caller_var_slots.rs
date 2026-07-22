//! Host functions reach the CALLING script function's numbered `Var(n)`
//! slots — the `cthr->Caller->NumVars` seam. FnFindConstructionSite reads
//! and writes them (C4Script.cpp:1958-1981); the planet System.c4g
//! FindConstructionSiteX wrapper stages coordinates through Var(0)/Var(1)
//! (Commits.c:384-390).

use std::sync::Arc;

use clonk_script::{Engine, HostCallerStrictness, LocalCells, RuntimeError, Script, Value};

fn caller_strictness_code() -> i32 {
    match clonk_script::caller_strictness() {
        HostCallerStrictness::NoCaller => -1,
        HostCallerStrictness::NonStrict => 0,
        HostCallerStrictness::Strict(level) => i32::from(level),
    }
}

#[test]
fn host_functions_read_and_write_the_callers_var_slots() {
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| {
        let slots = clonk_script::caller_var_slots()
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
    assert_eq!(
        engine.call("Run", &[]).expect("run succeeds"),
        Value::Int(99)
    );
}

#[test]
fn host_functions_without_a_script_caller_see_none() {
    // `if (!cthr->Caller) return {}` (C4Script.cpp:1966): engine-driven
    // host calls have no caller var space.
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| {
        Ok(Value::Bool(clonk_script::caller_var_slots().is_some()))
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
        let slots = clonk_script::caller_var_slots()
            .ok_or_else(|| RuntimeError::new("host fn must see its script caller"))?;
        Ok(Value::Bool(slots.get(7) == Value::Nil))
    });
    engine.add_script(
        Script::compile("#strict\n\nfunc Run() { return(Probe()); }\n").expect("compiles"),
    );
    assert_eq!(engine.call("Run", &[]).expect("runs"), Value::Bool(true));
}

#[test]
fn host_caller_strictness_distinguishes_absent_nonstrict_and_strict_frames() {
    let mut direct = Engine::new();
    direct.register_host_function("Probe", |_args| Ok(Value::Int(caller_strictness_code())));
    assert_eq!(
        direct.call("Probe", &[]).expect("direct host call runs"),
        Value::Int(-1),
        "an engine-driven native call has no script caller"
    );

    for (directive, expected) in [
        ("", 0),
        ("#strict\n", 1),
        ("#strict 2\n", 2),
        ("#strict 3\n", 3),
    ] {
        let mut engine = Engine::new();
        engine.register_host_function("Probe", |_args| Ok(Value::Int(caller_strictness_code())));
        engine.add_script(
            Script::compile(&format!("{directive}func Run() {{ return Probe(); }}\n"))
                .expect("script compiles"),
        );
        assert_eq!(
            engine.call("Run", &[]).expect("script call runs"),
            Value::Int(expected),
            "directive {directive:?}"
        );
    }
}

#[test]
fn strict_engine_scope_global_function_is_the_native_callers_frame() {
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| Ok(Value::Int(caller_strictness_code())));
    engine.add_script(
        Script::compile("global func Run() { return Probe(); }\n")
            .expect("script compiles"),
    );
    assert_eq!(
        engine.call("Run", &[]).expect("global function runs"),
        Value::Int(3),
        "Game.ScriptEngine owns global functions at MAXSTRICT"
    );
}

#[test]
fn linked_function_native_caller_uses_destination_script_strictness() {
    for (destination, source, expected) in [
        (
            "#strict 3\nfunc Own() { return 1; }\n",
            "func Run() { return Probe(); }\n",
            3,
        ),
        (
            "func Own() { return 1; }\n",
            "#strict 3\nfunc Run() { return Probe(); }\n",
            0,
        ),
    ] {
        let mut target = Engine::new();
        target.register_host_function("Probe", |_args| Ok(Value::Int(caller_strictness_code())));
        target.add_script(Script::compile(destination).expect("destination compiles"));

        let mut included = Engine::new();
        included.add_script(Script::compile(source).expect("included script compiles"));
        target.merge_from(&included);

        assert_eq!(
            target.call("Run", &[]).expect("linked function runs"),
            Value::Int(expected),
            "native caller strictness follows the copied function's destination owner"
        );
    }
}

#[test]
fn inherited_native_call_uses_the_overriding_functions_strictness() {
    let mut engine = Engine::new();
    engine.register_host_function("Probe", |_args| Ok(Value::Int(caller_strictness_code())));
    engine.add_script(
        Script::compile("#strict 2\nfunc Probe() { return inherited(); }\n")
            .expect("script compiles"),
    );
    assert_eq!(
        engine.call("Probe", &[]).expect("inherited host call runs"),
        Value::Int(2)
    );
}

#[test]
fn reentrant_vm_restores_the_outer_host_caller_context() {
    let mut outer = Engine::new();
    outer.register_host_function("OuterProbe", |_args| {
        if clonk_script::caller_strictness() != HostCallerStrictness::Strict(2) {
            return Err(RuntimeError::new("outer caller strictness missing"));
        }

        let mut inner = Engine::new();
        inner.register_host_function("InnerProbe", |_args| {
            if clonk_script::caller_strictness() != HostCallerStrictness::NonStrict {
                return Err(RuntimeError::new("inner caller must be NONSTRICT"));
            }
            Ok(Value::Int(17))
        });
        inner.add_script(
            Script::compile("func Run() { return InnerProbe(); }\n")
                .map_err(|error| RuntimeError::new(error.to_string()))?,
        );
        let value = inner
            .call("Run", &[])
            .map_err(|error| RuntimeError::new(error.to_string()))?;

        if clonk_script::caller_strictness() != HostCallerStrictness::Strict(2) {
            return Err(RuntimeError::new("outer caller was not restored"));
        }
        Ok(value)
    });
    outer.add_script(
        Script::compile("#strict 2\nfunc Run() { return OuterProbe(); }\n")
            .expect("script compiles"),
    );
    assert_eq!(
        outer.call("Run", &[]).expect("nested call runs"),
        Value::Int(17)
    );
}

#[test]
fn arrow_bridge_can_preserve_the_suspended_script_caller() {
    let mut outer = Engine::new();
    outer.add_script(
        Script::compile("#strict 3\nfunc Run(target) { return target->Native(); }\n")
            .expect("script compiles"),
    );
    outer.register_method_dispatch(Arc::new(|_args| {
        if clonk_script::caller_strictness() != HostCallerStrictness::Strict(3) {
            return Err(RuntimeError::new(
                "method dispatch must see the suspended script caller",
            ));
        }

        let mut target = Engine::new();
        target.register_host_function("Native", |_args| Ok(Value::Int(caller_strictness_code())));
        let cells = LocalCells::default();
        let ordinary = target
            .call_with_cells_and_this("Native", &[], &cells, Value::Object(7))
            .map_err(|error| RuntimeError::new(error.to_string()))?;
        let preserved = target
            .call_with_cells_and_this_preserving_caller("Native", &[], &cells, Value::Object(7))
            .map_err(|error| RuntimeError::new(error.to_string()))?;
        Ok(Value::Array(vec![ordinary, preserved]))
    }));

    assert_eq!(
        outer
            .call("Run", &[Value::Object(7)])
            .expect("arrow dispatch runs"),
        Value::Array(vec![Value::Int(-1), Value::Int(3)])
    );
}
