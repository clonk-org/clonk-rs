use clonk_script::{
    lift_native_continuation, Engine, NativeCallOutcome, NativeContinuation, RuntimeError,
    ScriptCallOutcome, ScriptError, ScriptSuspension, Value,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[derive(Debug)]
struct PauseRequest;

fn runtime_error(error: ScriptError) -> RuntimeError {
    match error {
        ScriptError::Runtime(error) => error,
        error => RuntimeError::new(error.to_string()),
    }
}

fn child_engine() -> Engine {
    let mut engine = Engine::new();
    engine.register_host_function("Pause", |_| {
        Err(RuntimeError::host_continuation(PauseRequest, Value::Nil))
    });
    engine
        .load_script("func Child() { Pause(); return 3; }\n")
        .expect("child script loads");
    engine
}

struct NativeSuffix {
    child: Engine,
    first_result: Option<Value>,
    suffix_calls: Arc<AtomicUsize>,
    swept: Arc<AtomicUsize>,
    value_stack_slots: usize,
}

impl NativeContinuation for NativeSuffix {
    fn resume(
        mut self: Box<Self>,
        child_result: Result<Value, RuntimeError>,
    ) -> Result<NativeCallOutcome, RuntimeError> {
        let child_result = child_result?;
        if let Some(first_result) = self.first_result.take() {
            self.suffix_calls.fetch_add(1, Ordering::SeqCst);
            let Value::Int(second_result) = child_result else {
                return Err(RuntimeError::new("child result must be an int"));
            };
            let Value::Int(first_result) = first_result else {
                return Err(RuntimeError::new("child result must be an int"));
            };
            return Ok(NativeCallOutcome::Complete(Value::Int(
                first_result + second_result,
            )));
        }

        self.first_result = Some(child_result);
        let child = self
            .child
            .call_with_continuation("Child", &[])
            .map_err(runtime_error)?;
        Ok(match child {
            ScriptCallOutcome::Complete(value) => {
                let first = self.first_result.take().expect("first result is retained");
                let Value::Int(first) = first else {
                    return Err(RuntimeError::new("child result must be an int"));
                };
                let Value::Int(second) = value else {
                    return Err(RuntimeError::new("child result must be an int"));
                };
                self.suffix_calls.fetch_add(1, Ordering::SeqCst);
                NativeCallOutcome::Complete(Value::Int(first + second))
            }
            ScriptCallOutcome::Suspended(child) => NativeCallOutcome::Suspended {
                child,
                continuation: self,
            },
        })
    }

    fn resume_child(
        &mut self,
        child: ScriptSuspension,
        value: Value,
    ) -> Result<ScriptCallOutcome, RuntimeError> {
        self.child
            .resume_script_continuation_with_value(child, value)
            .map_err(runtime_error)
    }

    fn clear_object_references(&mut self, _object_id: u64) {
        self.swept.fetch_add(1, Ordering::SeqCst);
    }

    fn value_stack_slots(&self) -> usize {
        self.value_stack_slots
    }
}

fn install_native_parent(
    suffix_calls: &Arc<AtomicUsize>,
    swept: &Arc<AtomicUsize>,
    value_stack_slots: &Arc<AtomicUsize>,
    native_calls: &Arc<AtomicUsize>,
) -> Engine {
    let mut engine = Engine::new();
    let suffix_calls = Arc::clone(suffix_calls);
    let swept = Arc::clone(swept);
    let value_stack_slots = Arc::clone(value_stack_slots);
    let native_calls = Arc::clone(native_calls);
    engine.register_host_function("Native", move |_| {
        native_calls.fetch_add(1, Ordering::SeqCst);
        let child = child_engine();
        let outcome = child
            .call_with_continuation("Child", &[])
            .map_err(runtime_error)?;
        lift_native_continuation(
            outcome,
            Box::new(NativeSuffix {
                child,
                first_result: None,
                suffix_calls: Arc::clone(&suffix_calls),
                swept: Arc::clone(&swept),
                value_stack_slots: value_stack_slots.load(Ordering::SeqCst),
            }),
        )
    });
    engine
        .load_script("func Parent() { return Native() + 1; }\n")
        .expect("parent script loads");
    engine
}

#[test]
fn native_continuation_runs_each_child_and_suffix_once() {
    let suffix_calls = Arc::new(AtomicUsize::new(0));
    let swept = Arc::new(AtomicUsize::new(0));
    let value_stack_slots = Arc::new(AtomicUsize::new(2));
    let native_calls = Arc::new(AtomicUsize::new(0));
    let engine = install_native_parent(&suffix_calls, &swept, &value_stack_slots, &native_calls);

    let mut first = match engine
        .call_with_continuation("Parent", &[])
        .expect("first child yields")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("native child completed synchronously"),
    };
    assert!(first.request::<PauseRequest>().is_some());
    assert!(first.attach_value_stack_context().is_ok());
    first.clear_object_references(77);
    assert_eq!(swept.load(Ordering::SeqCst), 1);

    let mut second = match engine
        .resume_script_continuation_with_value(first, Value::Int(10))
        .expect("native suffix starts its second child")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("second child must yield"),
    };
    assert!(second.request::<PauseRequest>().is_some());
    assert_eq!(suffix_calls.load(Ordering::SeqCst), 0);
    assert!(second.attach_value_stack_context().is_ok());
    second.clear_object_references(77);
    assert_eq!(swept.load(Ordering::SeqCst), 2);

    let result = engine
        .resume_script_continuation_with_value(second, Value::Int(20))
        .expect("native suffix completes after the second child")
        .complete_value();
    assert_eq!(result, Value::Int(7));
    assert_eq!(native_calls.load(Ordering::SeqCst), 1);
    assert_eq!(suffix_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn dropping_a_native_suspension_releases_its_aggregate_stack_charge() {
    let suffix_calls = Arc::new(AtomicUsize::new(0));
    let swept = Arc::new(AtomicUsize::new(0));
    let value_stack_slots = Arc::new(AtomicUsize::new(1_000));
    let native_calls = Arc::new(AtomicUsize::new(0));
    let engine = install_native_parent(&suffix_calls, &swept, &value_stack_slots, &native_calls);

    let first = match engine
        .call_with_continuation("Parent", &[])
        .expect("native child yields before the context is attached")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("native child completed synchronously"),
    };
    assert!(first.attach_value_stack_context().is_err());
    drop(first);

    value_stack_slots.store(2, Ordering::SeqCst);
    let second = match engine
        .call_with_continuation("Parent", &[])
        .expect("a dropped suspension must not contaminate the next call")
    {
        ScriptCallOutcome::Suspended(suspension) => suspension,
        ScriptCallOutcome::Complete(_) => panic!("native child completed synchronously"),
    };
    assert!(second.attach_value_stack_context().is_ok());
}

trait CompleteValue {
    fn complete_value(self) -> Value;
}

impl CompleteValue for ScriptCallOutcome {
    fn complete_value(self) -> Value {
        match self {
            ScriptCallOutcome::Complete(value) => value,
            ScriptCallOutcome::Suspended(_) => panic!("native suffix suspended unexpectedly"),
        }
    }
}
