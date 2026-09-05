use clonk_script::{Engine, LocalCells, RuntimeError, ScriptCallOutcome, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug)]
struct PauseRequest;

#[test]
fn direct_exec_continuation_keeps_the_live_local_cells() {
    // C++ oracle: C4AulExec.cpp:1658-1707 retains the one DirectExec frame
    // across a native callback instead of replaying its expression.
    let mut engine = Engine::new();
    engine
        .load_script("local base;")
        .expect("DirectExec host should declare the object local");
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });

    let cells = LocalCells::from_local_vars(&HashMap::from([("base".to_string(), Value::Int(2))]));
    let suspension = match engine
        .direct_exec_with_cells_and_this_at_strict_in_context_diagnostics_with_continuation(
            "base + Pause()",
            &cells,
            Value::Object(7),
            Some(3),
            "DirectExec test",
            false,
        )
        .expect("DirectExec should suspend at the host callback")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("DirectExec completed before Pause"),
    };

    let result = engine
        .resume_script_continuation_with_value(suspension, Value::Int(5))
        .expect("DirectExec suffix should resume")
        .complete_value();
    assert_eq!(result, Value::Int(7));
    assert_eq!(cells.snapshot().get("base"), Some(&Value::Int(2)));
}

#[test]
fn nested_eval_hook_keeps_its_child_suspension_and_parent_suffix() {
    // C++ oracle: C4Script.cpp:4507-4520 selects FnEval's receiver before
    // C4AulExec.cpp:1658-1707 runs the temporary child expression.
    let mut nested = Engine::new();
    nested.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    let nested = Rc::new(RefCell::new(nested));

    let nested_for_hook = Rc::clone(&nested);
    let mut engine = Engine::new();
    engine.register_eval_direct_exec_continuation_hook(Rc::new(
        move |_source, cells, this, strict_level, _depth| {
            let nested = nested_for_hook.borrow();
            Some(
                nested.eval_direct_exec_with_cells_and_this_at_strict_with_continuation(
                    "Pause() + 2",
                    cells,
                    this,
                    strict_level,
                    _depth,
                ),
            )
        },
    ));

    let suspension = match engine
        .direct_exec_with_cells_and_this_at_strict_in_context_diagnostics_with_continuation(
            "eval(\"ignored\") + 1",
            &LocalCells::default(),
            Value::Object(7),
            Some(3),
            "DirectExec test",
            false,
        )
        .expect("nested eval should suspend")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("nested eval completed before Pause"),
    };

    let result = engine
        .resume_script_continuation_with_value(suspension, Value::Int(4))
        .expect("nested eval child should resume")
        .complete_value();
    assert_eq!(result, Value::Int(7));
}

trait CompleteValue {
    fn complete_value(self) -> Value;
}

impl CompleteValue for ScriptCallOutcome {
    fn complete_value(self) -> Value {
        match self {
            ScriptCallOutcome::Complete(value) => value,
            ScriptCallOutcome::Suspended(_) => panic!("continuation suspended again"),
        }
    }
}
