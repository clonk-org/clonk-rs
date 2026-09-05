use clonk_script::{DebuggerHooks, Engine, RuntimeError, ScriptCallOutcome, Value};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

fn load_script(engine: &mut Engine, source: &str) {
    crate::support::load_script(engine, source);
}

#[derive(Debug)]
struct PauseRequest;

#[test]
fn nested_compiled_call_keeps_the_original_host_target_after_relink() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        func Child() {
            return Pause() + 1;
        }
        func Parent() {
            return Child() + 1;
        }
        "#,
    );

    let suspension = match engine
        .call_with_continuation("Parent", &[])
        .expect("Parent suspends through Child")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Parent completed before Pause"),
    };

    // A C++ AB_FUNC stores its selected function pointer in the bytecode. A
    // section relink can therefore replace the name's registration while the
    // suspended Child still resumes the exact call that yielded.
    engine.register_host_reference_function("Pause", [0], |_| Ok(Value::Int(999)));

    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(40))
        .expect("the retained host target resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Child suspended again"),
    };
    assert_eq!(result, Value::Int(42));
}

#[test]
fn nested_compiled_call_preserves_a_second_host_suspension() {
    let mut engine = Engine::new();
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_pause = Arc::clone(&calls);
    engine.register_host_function("Pause", move |_| {
        let call = calls_for_pause.fetch_add(1, Ordering::SeqCst);
        assert!(call < 2, "the fixture should suspend exactly twice");
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        func Child() {
            var first = Pause();
            var second = Pause();
            return first + second;
        }
        func Parent() {
            return Child() + 1;
        }
        "#,
    );

    let first = match engine
        .call_with_continuation("Parent", &[])
        .expect("first Pause suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Parent completed before first Pause"),
    };
    let second = match engine
        .resume_script_continuation_with_value(first, Value::Int(10))
        .expect("first Pause resumes")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("second Pause must suspend"),
    };
    let result = match engine
        .resume_script_continuation_with_value(second, Value::Int(20))
        .expect("second Pause resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Child suspended after its second Pause"),
    };
    assert_eq!(result, Value::Int(31));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn nested_reference_return_keeps_the_caller_alias_after_suspension() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        func &Child(&slot) {
            Pause();
            return slot;
        }
        func Parent(&slot) {
            Child(slot) = 3;
            return slot;
        }
        "#,
    );

    let (suspension, cells) = match engine
        .call_with_ref_args_with_continuation("Parent", &[Value::Int(0)])
        .expect("Parent suspends through a reference-returning Child")
    {
        (ScriptCallOutcome::Suspended(suspension), cells) => (suspension, cells),
        (ScriptCallOutcome::Complete(_), _) => panic!("Parent completed before Pause"),
    };
    let result = match engine
        .resume_script_continuation(suspension)
        .expect("reference-returning Child resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Child suspended twice"),
    };
    assert_eq!(result, Value::Int(3));
    assert_eq!(*cells[0].borrow(), Value::Int(3));
}

#[test]
fn resumes_a_compiled_call_after_a_host_boundary() {
    let marks = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::new();
    let mark_sink = Arc::clone(&marks);
    engine.register_host_function("Mark", move |args| {
        mark_sink.lock().unwrap().push(args.first().cloned());
        Ok(Value::Nil)
    });
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        #strict 3
        func Probe() {
            Mark(1);
            Pause();
            Mark(2);
            return 7;
        }
        "#,
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("Probe suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    assert!(suspension.request::<PauseRequest>().is_some());
    assert_eq!(
        *marks.lock().unwrap(),
        vec![Some(Value::Int(1))],
        "the suffix waits for the host to commit the request"
    );

    let result = match engine
        .resume_script_continuation(suspension)
        .expect("Probe resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(7));
    assert_eq!(
        *marks.lock().unwrap(),
        vec![Some(Value::Int(1)), Some(Value::Int(2))]
    );
}

#[test]
fn resumes_a_host_call_with_the_committed_return_value() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        func Probe() {
            var result = Pause();
            return result;
        }
        "#,
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("Probe suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(42))
        .expect("Probe resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(42));
}

#[test]
fn resumed_direct_this_call_keeps_the_object_receiver() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        #strict 3
        func Probe() {
            Pause();
            return this();
        }
        "#,
    );

    let cells = clonk_script::LocalCells::default();
    let suspension = match engine
        .call_with_cells_and_this_with_continuation("Probe", &[], &cells, Value::Object(42))
        .expect("Probe suspends before this()")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };

    let result = match engine
        .resume_script_continuation(suspension)
        .expect("Probe resumes into this()")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Object(42));
}

#[test]
fn resumes_an_interpreted_reference_call_without_breaking_the_alias() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        func Probe(&slot) {
            slot = 1;
            Pause();
            slot = 2;
            return slot;
        }
        "#,
    );

    let (suspension, cells) = match engine
        .call_with_ref_args_with_continuation("Probe", &[Value::Int(0)])
        .expect("Probe suspends")
    {
        (ScriptCallOutcome::Suspended(suspension), cells) => (suspension, cells),
        (ScriptCallOutcome::Complete(_), _) => panic!("Probe completed before Pause"),
    };
    assert_eq!(*cells[0].borrow(), Value::Int(1));

    let result = match engine
        .resume_script_continuation(suspension)
        .expect("Probe resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(2));
    assert_eq!(*cells[0].borrow(), Value::Int(2));
}

#[test]
fn clears_removed_objects_from_suspended_stack_and_alias_containers() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    engine.register_host_function("ObjectValue", |_| Ok(Value::Object(7)));
    load_script(
        &mut engine,
        r#"
        #strict 3
        func Probe() {
            var old = ObjectValue();
            var values = [ObjectValue()];
            var tail = 19;
            Pause();
            return [old, values[0], tail];
        }
        "#,
    );

    let cells = clonk_script::LocalCells::default();
    let mut suspension = match engine
        .call_with_cells_and_this_with_continuation("Probe", &[], &cells, Value::Object(7))
        .expect("Probe suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };

    // C++ AssignRemoval clears every C4Value in the suspended caller before
    // the section switch; numeric locals survive the sweep (C4Object.cpp:312).
    suspension.clear_object_references(7);
    let result = match engine
        .resume_script_continuation(suspension)
        .expect("Probe resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(
        result,
        Value::Array(vec![Value::Nil, Value::Nil, Value::Int(19)])
    );
}

#[test]
fn interpreted_continuation_keeps_assignment_operands_and_short_circuit_state() {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        #strict 2
        func Probe() {
            var value = 0;
            value = 1 + Pause();
            if (value && Pause()) {
                value += 10;
            }
            return value + 1;
            UnknownAfterReturn();
        }
        "#,
    );

    let first = match engine
        .call_with_continuation("Probe", &[])
        .expect("first Pause suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before assignment Pause"),
    };
    let second = match engine
        .resume_script_continuation_with_value(first, Value::Int(2))
        .expect("assignment resumes")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("short-circuit RHS should suspend"),
    };
    let result = match engine
        .resume_script_continuation_with_value(second, Value::Bool(true))
        .expect("short-circuit resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended after its final call"),
    };
    assert_eq!(result, Value::Int(14));
}

#[test]
fn resumes_an_interpreted_foreach_body_after_each_host_boundary() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut engine = Engine::new();
    let calls_for_pause = Arc::clone(&calls);
    engine.register_host_function("Pause", move |_| {
        let call = calls_for_pause.fetch_add(1, Ordering::SeqCst);
        assert!(call < 2);
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        r#"
        #strict 3
        func Probe() {
            var total = 0;
            for (var item in [1, 2]) {
                total += Pause();
            }
            return total;
        }
        "#,
    );

    let first = match engine
        .call_with_continuation("Probe", &[])
        .expect("first foreach body call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before first Pause"),
    };
    let second = match engine
        .resume_script_continuation_with_value(first, Value::Int(3))
        .expect("first foreach body call resumes")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("second foreach body call must suspend"),
    };
    let result = match engine
        .resume_script_continuation_with_value(second, Value::Int(4))
        .expect("second foreach body call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("foreach suspended after its final call"),
    };
    assert_eq!(result, Value::Int(7));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[test]
fn interpreted_zero_argument_global_call_keeps_its_expression_suffix() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        "#strict 3\nfunc Probe() { return 1 + global->Pause(); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("global call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(2))
        .expect("global call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(3));
}

#[test]
fn interpreted_safe_method_call_keeps_its_expression_suffix() {
    let mut engine = Engine::new();
    engine.register_method_dispatch(Arc::new(|_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    }));
    load_script(
        &mut engine,
        "#strict 3\nfunc Probe(target) { return 1 + target?->Pause(); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[Value::Object(7)])
        .expect("safe method call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before method callback"),
    };
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(2))
        .expect("safe method call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(3));
}

#[test]
fn interpreted_method_slot_target_can_suspend_before_reference_dispatch() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let slot = clonk_script::value_cell(Value::Nil);
    let local_slot = Rc::clone(&slot);
    engine.register_local_cell_hook(Rc::new(move |target, name| {
        (*target == Value::Object(7) && name == "slot").then(|| Rc::clone(&local_slot))
    }));
    load_script(
        &mut engine,
        "#strict 3\nfunc Probe() { return (LocalN(\"slot\", Pause()) = 3); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("method slot target suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Object(7))
        .expect("method slot target resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(3));
    assert_eq!(*slot.borrow(), Value::Int(3));
}

#[test]
fn interpreted_call_retains_the_selected_host_target_across_argument_yield() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Target", 1, |_| Ok(Value::Int(1)));
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        "#strict 3\nfunc Probe() { return Target(Pause()); UnknownAfterReturn(); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("argument call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    engine.register_host_function_with_arity("Target", 1, |_| Ok(Value::Int(2)));
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(9))
        .expect("argument call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(1));
}

#[test]
fn interpreted_call_uses_the_selected_host_arity_after_argument_yield() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Target", 1, |args| {
        Ok(args.first().cloned().unwrap_or(Value::Nil))
    });
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        // Parse_Params fixes the selected native signature before evaluating
        // its operands; a relink during that evaluation cannot truncate the
        // captured call's argument list (C4AulParse.cpp:2311-2344).
        "#strict 3\nfunc Probe() { return Target(Pause()); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("argument call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    engine.register_host_function_with_arity("Target", 0, |_| Ok(Value::Int(2)));
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(9))
        .expect("argument call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(9));
}

#[test]
fn interpreted_global_call_retains_the_selected_host_target_and_context() {
    let mut engine = Engine::new();
    let contexts = Arc::new(Mutex::new(Vec::new()));
    let context_sink = Arc::clone(&contexts);
    engine.register_global_call_context_hook(Arc::new(move |entered| {
        context_sink.lock().expect("context log lock").push(entered);
    }));
    engine.register_host_function_with_arity("Target", 1, |_| Ok(Value::Int(1)));
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    load_script(
        &mut engine,
        "#strict 3\nfunc Probe() { return global->Target(Pause()); UnknownAfterReturn(); }\n",
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("global argument call suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    engine.register_host_function_with_arity("Target", 1, |_| Ok(Value::Int(2)));
    let result = match engine
        .resume_script_continuation_with_value(suspension, Value::Int(9))
        .expect("global argument call resumes")
    {
        ScriptCallOutcome::Complete(value) => value,
        ScriptCallOutcome::Suspended(_) => panic!("Probe suspended twice"),
    };
    assert_eq!(result, Value::Int(1));
    assert_eq!(
        *contexts.lock().expect("context log lock"),
        vec![true, false],
        "the resumed global dispatch retains its C++ global-call context"
    );
}

#[test]
fn ast_continuation_restores_its_frame_budget_before_large_expression() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 1015).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!("#strict 3\nfunc Probe() {{ Pause(); return [{values}]; }}\n"),
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("AST Probe suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let Err(error) = engine.resume_script_continuation(suspension) else {
        panic!("the resumed AST frame must retain its ten parameter slots");
    };
    assert!(
        error
            .to_string()
            .contains("internal error: value stack overflow!"),
        "unexpected resume error: {error}"
    );
}

#[test]
fn compiled_continuation_restores_its_frame_budget_for_nested_large_calls() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 1005).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Child() {{ return [{values}]; }}\nfunc Probe() {{ Pause(); return Child(); }}\n"
        ),
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("compiled Probe suspends")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let Err(error) = engine.resume_script_continuation(suspension) else {
        panic!("the compiled frame must remain live while Child runs");
    };
    assert!(
        error
            .to_string()
            .contains("internal error: value stack overflow!"),
        "unexpected resume error: {error}"
    );
}

#[test]
fn dropping_an_unused_suspension_releases_all_value_stack_reservations() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 1014).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Suspended() {{ return [1, Pause(), 2]; }}\nfunc Fits() {{ return [{values}]; }}\n"
        ),
    );

    let suspension = match engine
        .call_with_continuation("Suspended", &[])
        .expect("Suspended yields")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Suspended completed before Pause"),
    };
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("an unrelated call sees a clean stack while suspension is held"),
        Value::Array(values) if values.len() == 1014
    ));
    drop(suspension);
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("an unrelated call sees a clean stack after suspension drop"),
        Value::Array(values) if values.len() == 1014
    ));
}

#[test]
fn inline_value_stack_context_counts_the_suspended_caller_then_restores_it() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 1014).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Suspended() {{ return [1, Pause(), 2]; }}\nfunc Fits() {{ return [{values}]; }}\n"
        ),
    );

    let mut suspension = match engine
        .call_with_continuation("Suspended", &[])
        .expect("Suspended yields")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Suspended completed before Pause"),
    };
    let context = suspension
        .attach_value_stack_context()
        .expect("the inline context attaches without overflowing itself");
    suspension.clear_object_references(999);
    let nested = engine.call("Fits", &[]);
    drop(context);
    let Err(error) = nested else {
        panic!("an inline nested call must include the suspended caller's slots");
    };
    assert!(error
        .to_string()
        .contains("internal error: value stack overflow!"));
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("the context guard restores the detached budget"),
        Value::Array(values) if values.len() == 1014
    ));
}

#[test]
fn inline_value_stack_context_counts_nested_suspended_frames() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 0, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 995).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Child() {{ Pause(); return 0; }}\nfunc Parent() {{ return Child(); }}\nfunc Fits() {{ return [{values}]; }}\n"
        ),
    );

    let mut suspension = match engine
        .call_with_continuation("Parent", &[])
        .expect("Parent suspends through Child")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Parent completed before Pause"),
    };
    let context = suspension
        .attach_value_stack_context()
        .expect("the nested context attaches without overflowing itself");
    // Section removal can clear references while inline host work is still
    // running; the owned guard holds no borrow of the suspension.
    suspension.clear_object_references(999);
    let nested = engine.call("Fits", &[]);
    drop(context);
    let Err(error) = nested else {
        panic!("an inline nested call must include both suspended frames");
    };
    assert!(error
        .to_string()
        .contains("internal error: value stack overflow!"));
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("the nested context guard restores the detached budget"),
        Value::Array(values) if values.len() == 995
    ));
}

#[test]
fn inline_value_stack_context_counts_suspended_native_host_frame() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 10, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let values = std::iter::repeat_n("0", 985).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Child() {{ Pause(); return 0; }}\nfunc Parent() {{ return Child(); }}\nfunc Fits() {{ return [{values}]; }}\n"
        ),
    );

    let suspension = match engine
        .call_with_continuation("Parent", &[])
        .expect("Parent suspends through the ten-slot host callback")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Parent completed before Pause"),
    };
    let context = suspension
        .attach_value_stack_context()
        .expect("the native callback context attaches");
    let nested = engine.call("Fits", &[]);
    drop(context);
    let Err(error) = nested else {
        panic!("inline work must include the suspended native callback frame");
    };
    assert!(error
        .to_string()
        .contains("internal error: value stack overflow!"));
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("dropping the native context restores the baseline"),
        Value::Array(values) if values.len() == 985
    ));
}

#[test]
fn inline_value_stack_context_counts_suspended_native_host_frame_in_ast() {
    let mut engine = Engine::new();
    engine.register_host_function_with_arity("Pause", 10, |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    // Probe's expression is too large for the compiled path once its ten
    // parameter slots are reserved, so this exercises AST host suspension.
    let probe_values = std::iter::repeat_n("0", 1015).collect::<Vec<_>>().join(",");
    let fits_values = std::iter::repeat_n("0", 995).collect::<Vec<_>>().join(",");
    load_script(
        &mut engine,
        &format!(
            "#strict 3\nfunc Probe() {{ Pause(); return [{probe_values}]; }}\nfunc Fits() {{ return [{fits_values}]; }}\n"
        ),
    );

    let suspension = match engine
        .call_with_continuation("Probe", &[])
        .expect("the AST Probe suspends through the ten-slot host callback")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("Probe completed before Pause"),
    };
    let context = suspension
        .attach_value_stack_context()
        .expect("the AST native callback context attaches");
    let nested = engine.call("Fits", &[]);
    drop(context);
    let Err(error) = nested else {
        panic!("inline AST work must include the suspended native callback frame");
    };
    assert!(error
        .to_string()
        .contains("internal error: value stack overflow!"));
    assert!(matches!(
        engine
            .call("Fits", &[])
            .expect("dropping the AST native context restores the baseline"),
        Value::Array(values) if values.len() == 995
    ));
}

#[test]
fn executes_basic_arithmetic() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func Add(a, b) {
            return a + b;
        }
        func Double(x) {
            var value = Add(x, x);
            return value;
        }
        "#,
    );

    let result = engine
        .call("Add", &[Value::Int(21), Value::Int(21)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(42));

    let double = engine
        .call("Double", &[Value::Int(7)])
        .expect("call succeeds");
    assert_eq!(double, Value::Int(14));
}

#[test]
fn nonstrict_standalone_goto_returns_immediately() {
    fn run(source: &str) -> Value {
        let mut engine = Engine::new();
        engine.register_host_function("goto", |args| {
            Ok(args.first().cloned().unwrap_or(Value::Nil))
        });
        load_script(&mut engine, source);
        engine.call("Probe", &[]).expect("Probe runs")
    }

    for (directive, expected) in [
        ("", Value::Int(41)),
        ("#strict\n", Value::Int(99)),
        ("#strict 2\n", Value::Int(99)),
        ("#strict 3\n", Value::Int(99)),
    ] {
        assert_eq!(
            run(&format!(
                "{directive}func Probe() {{ goto(40 + 1); return 99; }}"
            )),
            expected,
            "only a NONSTRICT bare goto statement returns implicitly"
        );
    }

    assert_eq!(
        run("func Probe() { goto(40 + 1) + 1; return 99; }"),
        Value::Int(41),
        "C++ returns the goto result before evaluating its parsed suffix"
    );
    assert_eq!(
        run("func Probe() { var value = goto(41); return value + 1; }"),
        Value::Int(42),
        "an embedded goto call is an ordinary expression"
    );
    assert_eq!(
        run("func Probe() { (goto(41)); return 99; }"),
        Value::Int(99),
        "a parenthesized goto does not start the legacy statement path"
    );
    assert_eq!(
        run("func Goto(value) { return value; } func Probe() { Goto(41); return 99; }"),
        Value::Int(99),
        "the legacy spelling check is case-sensitive"
    );
    assert_eq!(
        run("func Probe() { var goto; goto(41); return 99; }"),
        Value::Int(99),
        "a named binding takes precedence over the legacy goto hack"
    );
    assert_eq!(
        run("func Probe() { goto(41); return 99; }\n#strict\n"),
        Value::Int(99),
        "the final origin strictness applies even when its directive is later"
    );

    let malformed =
        clonk_script::Script::compile("#strict 2\nfunc Probe() { goto(@); return 99; }")
            .expect("function recovery retains the malformed script");
    assert!(
        malformed
            .parse_diagnostics()
            .iter()
            .any(|error| error.message() == "unexpected character '@'"),
        "the leading-call probe must not consume and hide lexer errors"
    );
}

run_cases! {
    handles_conditionals_and_loops:
        r#"
        global func SumUntil(limit) {
            var acc = 0;
            var current = 1;
            while (current <= limit) {
                acc = acc + current;
                current = current + 1;
            }
            return acc;
        }
        "#,
        "SumUntil", &[Value::Int(5)] => Value::Int(15);

    supports_strings_and_concatenation:
        r#"
        global func Greeting(name) {
            var message = "Hello, " .. name;
            return message .. "!";
        }
        "#,
        "Greeting", &[Value::String("World".into())] => Value::String("Hello, World!".into());

    handles_recursion:
        r#"
        global func Factorial(n) {
            if (n <= 1) {
                return 1;
            }
            return n * Factorial(n - 1);
        }
        "#,
        "Factorial", &[Value::Int(5)] => Value::Int(120);
}

#[test]
fn reports_unknown_function() {
    let engine = Engine::new();
    let error = engine.call("Missing", &[]).unwrap_err();
    assert!(format!("{error}").contains("unknown function"));
}

#[test]
fn host_function_can_be_called_directly() {
    let mut engine = Engine::new();
    engine.register_host_function("HostAdd", |args| {
        let lhs = match args.first() {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostAdd expects first argument to be an int",
                ))
            }
        };
        let rhs = match args.get(1) {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostAdd expects second argument to be an int",
                ))
            }
        };
        Ok(Value::Int(lhs + rhs))
    });

    let result = engine
        .call("HostAdd", &[Value::Int(40), Value::Int(2)])
        .expect("host call succeeds");
    assert_eq!(result, Value::Int(42));
}

#[test]
fn script_can_call_host_function() {
    let mut engine = Engine::new();
    engine.register_host_function("HostMul", |args| {
        let lhs = match args.first() {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostMul expects first argument to be an int",
                ))
            }
        };
        let rhs = match args.get(1) {
            Some(Value::Int(value)) => *value,
            _ => {
                return Err(RuntimeError::new(
                    "HostMul expects second argument to be an int",
                ))
            }
        };
        Ok(Value::Int(lhs * rhs))
    });

    load_script(
        &mut engine,
        r#"
        global func DoubleProduct(a, b) {
            return HostMul(a, b) * 2;
        }
        "#,
    );

    let result = engine
        .call("DoubleProduct", &[Value::Int(3), Value::Int(4)])
        .expect("script call succeeds");
    assert_eq!(result, Value::Int(24));
}

#[test]
fn host_function_errors_propagate() {
    let mut engine = Engine::new();
    engine.register_host_function("HostFail", |_| Err(RuntimeError::new("host failure")));

    let error = engine.call("HostFail", &[]).unwrap_err();
    assert!(format!("{error}").contains("host failure"));
}

run_cases! {
    supports_arrays_and_indexing:
        r#"
        #strict
        global func ThirdElement() {
            var arr = [1, 2, 3, 4];
            return arr[2];
        }
        "#,
        "ThirdElement", &[] => Value::Int(3);

    array_literal_empty_slots_match_cpp:
        r#"
        #strict
        global func EmptySlots() {
            return [[], [,], [,,], [1,], [1,,2], [,1,,], [[,],[2,]], [3,4]];
        }
        "#,
        "EmptySlots", &[] =>
        Value::Array(vec![
            Value::Array(vec![]),
            Value::Array(vec![Value::Nil, Value::Nil]),
            Value::Array(vec![Value::Nil, Value::Nil, Value::Nil]),
            Value::Array(vec![Value::Int(1), Value::Nil]),
            Value::Array(vec![Value::Int(1), Value::Nil, Value::Int(2)]),
            Value::Array(vec![Value::Nil, Value::Int(1), Value::Nil, Value::Nil]),
            Value::Array(vec![
                Value::Array(vec![Value::Nil, Value::Nil]),
                Value::Array(vec![Value::Int(2), Value::Nil]),
            ]),
            Value::Array(vec![Value::Int(3), Value::Int(4)]),
        ]);

    supports_proplists_and_nested_access:
        r#"
        #strict 3
        global func ProplistQuery() {
            var data = { foo = 42, nested = { value = 7 }, numbers = [5, 9] };
            return data.foo + data.nested.value + data.numbers[1];
        }
        "#,
        "ProplistQuery", &[] => Value::Int(58);

    statement_map_literal_evaluates_key_and_value_side_effects:
        r#"
        #strict 3
        static calls;
        func Mark(amount) { calls += amount; return calls; }
        global func StatementMap() {
            calls = 0;
            { [Mark(1)] = Mark(10), nested = { value = Mark(100) } };
            return calls;
        }
        "#,
        "StatementMap", &[] => Value::Int(111);

    assigns_to_proplist_properties:
        r#"
        #strict 3
        global func Mutate() {
            var data = { foo = 1, nested = { value = 2 } };
            data.foo = data.foo + 41;
            data.nested.value = data.foo - 36;
            data.new_field = 3;
            return data.foo + data.nested.value + data.new_field;
        }
        "#,
        "Mutate", &[] => Value::Int(51);
}

#[test]
fn property_assignment_reports_type_errors() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        #strict 3
        global func BadAssign() {
            var value = 5;
            value.foo = 1;
        }
        "#,
    );

    let error = engine.call("BadAssign", &[]).unwrap_err();
    assert!(format!("{error}").contains("cannot assign property 'foo'"));
}

#[test]
fn effect_callbacks_dispatch_via_engine_helper() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func FxFireStart(effect, target) {
            return effect + target;
        }
        "#,
    );

    let effect_result = engine
        .call_effect_callback("Fire", "Start", &[Value::Int(10), Value::Int(5)])
        .expect("effect dispatch succeeds");
    assert_eq!(effect_result, Some(Value::Int(15)));

    let missing = engine
        .call_effect_callback("Fire", "Stop", &[])
        .expect("missing callback returns None");
    assert!(missing.is_none());
}

#[test]
fn debugger_hooks_capture_call_and_return() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func AddOne(value) {
            return value + 1;
        }
        "#,
    );

    let call_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let return_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    let mut hooks = DebuggerHooks::new();
    {
        let call_log = Arc::clone(&call_log);
        hooks.set_on_call(move |name, args| {
            assert_eq!(args, [Value::Int(41)]);
            let mut log = call_log.lock().unwrap();
            log.push(format!("{name}({})", args.len()));
        });
    }
    {
        let return_log = Arc::clone(&return_log);
        hooks.set_on_return(move |name, value| {
            let mut log = return_log.lock().unwrap();
            log.push(format!("{name} -> {value}"));
        });
    }
    engine.set_debugger_hooks(hooks);

    let result = engine
        .call("AddOne", &[Value::Int(41)])
        .expect("call succeeds");
    assert_eq!(result, Value::Int(42));

    let call_entries = call_log.lock().unwrap().clone();
    assert_eq!(call_entries, vec!["AddOne(1)".to_string()]);
    let return_entries = return_log.lock().unwrap().clone();
    assert_eq!(return_entries, vec!["AddOne -> 42".to_string()]);
}

const CANONICAL_SCENARIO: &str = include_str!("../../src/fixtures/canonical/basic.aul");

#[test]
fn canonical_scenario_parity_harness() {
    let mut engine = Engine::new();
    engine
        .load_script(CANONICAL_SCENARIO)
        .expect("canonical script loads");

    let array_sum = engine
        .call("CanonicalArrayCheck", &[])
        .expect("array parity call succeeds");
    assert_eq!(array_sum, Value::Int(21));

    let proplist = engine
        .call("CanonicalProplistCheck", &[])
        .expect("proplist parity call succeeds");
    assert_eq!(proplist, Value::Int(53));

    let effect = engine
        .call_effect_callback("Canonical", "Start", &[Value::Int(7)])
        .expect("effect callback dispatches");
    assert_eq!(effect, Some(Value::Int(7)));
}

run_cases! {
    supports_access_modifiers_on_functions:
        r#"
        private func PrivateHelper() {
            return 10;
        }

        protected func ProtectedHelper() {
            return 20;
        }

        public func PublicHelper() {
            return 30;
        }

        global func GlobalHelper() {
            return 40;
        }

        global func CallAll() {
            return PrivateHelper() + ProtectedHelper() + PublicHelper() + GlobalHelper();
        }
        "#,
        "CallAll", &[] => Value::Int(100);
}

#[test]
fn return_statement_handles_parenthesized_expressions_with_operators() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        global func ReturnParenDivide() {
            return (255*100)/100;
        }

        global func ReturnParenAdd() {
            return (100)+50;
        }

        global func ReturnParenMultiply() {
            return (10)*5;
        }

        global func ReturnComplexExpr() {
            return (255*GetIntensity())/100;
        }

        private func GetIntensity() {
            return 80;
        }
        "#,
    );

    assert_eq!(
        engine
            .call("ReturnParenDivide", &[])
            .expect("call succeeds"),
        Value::Int(255)
    );
    assert_eq!(
        engine.call("ReturnParenAdd", &[]).expect("call succeeds"),
        Value::Int(150)
    );
    assert_eq!(
        engine
            .call("ReturnParenMultiply", &[])
            .expect("call succeeds"),
        Value::Int(50)
    );
    assert_eq!(
        engine
            .call("ReturnComplexExpr", &[])
            .expect("call succeeds"),
        Value::Int(204)
    );
}

/// C4Script arrays are **values**, not handles: `C4ValueArray` is copied when
/// one is assigned to another name or passed to a function, so a write through
/// the second name is invisible to the first
/// (`C4Value.cpp:37-333`, storage and containers).
///
/// This is worth pinning on its own because it is the invariant that any
/// copy-avoidance work has to preserve. clonk-org/clonk-rs#759 proposes making
/// `a[i] = v` stop copying the array when it is uniquely referenced, and the
/// whole risk in that change is turning one of these copies into sharing.
/// Nothing else in the suite fails if aliasing silently appears — the existing
/// array tests all mutate through a single name.
#[test]
fn arrays_are_copied_by_assignment_and_by_argument() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        #strict
        global func Mutate(array taken) {
            taken[0] = 99;
            return taken[0];
        }

        // Assigning to a second name copies: the original keeps its element.
        global func AssignmentCopies() {
            var source = [1, 2, 3];
            var copy = source;
            copy[0] = 99;
            return source[0];
        }

        // Passing to a function copies: the callee's write is not visible.
        global func ArgumentCopies() {
            var source = [1, 2, 3];
            Mutate(source);
            return source[0];
        }

        // The copy is deep enough that a nested element is not shared either.
        global func NestedAssignmentCopies() {
            var source = [[1, 2], [3, 4]];
            var copy = source;
            copy[0][0] = 99;
            return source[0][0];
        }

        // And the callee really did write through its own copy, so these
        // assertions are about isolation rather than the write failing.
        global func CalleeSeesItsOwnWrite() {
            var source = [1, 2, 3];
            return Mutate(source);
        }
        "#,
    );

    for (function, expected) in [
        ("AssignmentCopies", 1),
        ("ArgumentCopies", 1),
        ("NestedAssignmentCopies", 1),
        ("CalleeSeesItsOwnWrite", 99),
    ] {
        assert_eq!(
            engine.call(function, &[]).expect("call succeeds"),
            Value::Int(expected),
            "{function} must observe C4ValueArray value semantics"
        );
    }
}

#[test]
fn array_index_assignment_works() {
    let mut engine = Engine::new();
    load_script(
        &mut engine,
        r#"
        #strict
        global func TestArrayIndexAssignment() {
            var arr = [0, 0, 0];
            arr[0] = 10;
            arr[1] = 20;
            arr[2] = 30;
            return arr[0] + arr[1] + arr[2];
        }

        global func TestNestedArrayAssignment() {
            var matrix = [[0, 0], [0, 0]];
            matrix[0][0] = 1;
            matrix[0][1] = 2;
            matrix[1][0] = 3;
            matrix[1][1] = 4;
            return matrix[0][0] + matrix[0][1] + matrix[1][0] + matrix[1][1];
        }
        "#,
    );

    assert_eq!(
        engine
            .call("TestArrayIndexAssignment", &[])
            .expect("call succeeds"),
        Value::Int(60)
    );
    assert_eq!(
        engine
            .call("TestNestedArrayAssignment", &[])
            .expect("call succeeds"),
        Value::Int(10)
    );
}
