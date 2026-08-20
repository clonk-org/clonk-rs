//! Assignments retain one lvalue reference before evaluating their RHS.
//! Compound operations reuse that reference for their read and write, and
//! `??=` skips both its RHS and store when the old value is non-nil.

use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use clonk_script::{Engine, Value};

#[test]
fn plain_assignment_resolves_array_index_before_rhs() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed_trace = Arc::clone(&trace);
    let mut engine = Engine::new();
    engine.register_host_function("Trace", move |args| {
        let [Value::Int(marker)] = args else {
            panic!("Trace expects one integer marker, got {args:?}");
        };
        observed_trace.lock().unwrap().push(*marker);
        Ok(Value::Int(if *marker == 1 { 0 } else { 42 }))
    });
    crate::support::load_script(
        &mut engine,
        r#"
        #strict
        func Test() {
            var values = [0];
            values[Trace(1)] = Trace(2);
            return values;
        }
    "#,
    );

    assert_eq!(
        engine.call("Test", &[]).expect("plain assignment succeeds"),
        Value::Array(vec![Value::Int(42)])
    );
    assert_eq!(
        *trace.lock().unwrap(),
        vec![1, 2],
        "the indexed target must resolve before the RHS is evaluated"
    );
}

#[test]
fn assignment_expression_resolves_array_index_before_rhs() {
    let trace = Arc::new(Mutex::new(Vec::new()));
    let observed_trace = Arc::clone(&trace);
    let mut engine = Engine::new();
    engine.register_host_function("Trace", move |args| {
        let [Value::Int(marker)] = args else {
            panic!("Trace expects one integer marker, got {args:?}");
        };
        observed_trace.lock().unwrap().push(*marker);
        Ok(Value::Int(if *marker == 1 { 0 } else { 42 }))
    });
    crate::support::load_script(
        &mut engine,
        r#"
        #strict
        func Test() {
            var values = [0];
            var assigned = values[Trace(1)] = Trace(2);
            return [values, assigned];
        }
    "#,
    );

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("assignment expression succeeds"),
        Value::Array(vec![Value::Array(vec![Value::Int(42)]), Value::Int(42)])
    );
    assert_eq!(
        *trace.lock().unwrap(),
        vec![1, 2],
        "the indexed target must resolve before an expression RHS"
    );
}

#[test]
fn compound_assignment_evaluates_array_index_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let observed_calls = Arc::clone(&calls);
    let mut engine = Engine::new();
    engine.register_host_function("SideEffect", move |args| {
        assert!(args.is_empty());
        observed_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(0))
    });
    crate::support::load_script(
        &mut engine,
        r#"
        #strict
        func Test() {
            var values = [10, 20];
            values[SideEffect()] += 5;
            return values;
        }
    "#,
    );

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("compound assignment succeeds"),
        Value::Array(vec![Value::Int(15), Value::Int(20)])
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the index expression must resolve exactly one lvalue"
    );
}

#[test]
fn compound_assignment_consumes_one_deterministic_random_draw() {
    let draws = Arc::new(AtomicUsize::new(0));
    let observed_draws = Arc::clone(&draws);
    let mut engine = Engine::new();
    engine.register_host_function("Random", move |args| {
        assert_eq!(args, [Value::Int(2)]);
        let draw = observed_draws.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int((draw % 2) as i32))
    });
    crate::support::load_script(
        &mut engine,
        r#"
        #strict
        func Test() {
            var values = [10, 20];
            values[Random(2)] += 1;
            return values;
        }
    "#,
    );

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("Random-index compound assignment succeeds"),
        Value::Array(vec![Value::Int(11), Value::Int(20)])
    );
    assert_eq!(
        draws.load(Ordering::SeqCst),
        1,
        "the deterministic RNG ledger advances once"
    );
}

run_cases! {
    compound_assignment_reads_retained_reference_after_rhs:
            r#"
                func Test() {
                    var value = 1;
                    value += (value = 5);
                    return value;
                }
            "#,
        "Test", &[] => Value::Int(10),
        "the retained lvalue is read after the RHS mutates it"
        ;

    concat_assignment_preserves_nested_array_identity_below_strict_two:
            r#"
                #strict
                func Test() {
                    var inner = [];
                    var values = [inner];
                    values ..= [];
                    return values[0] == inner;
                }
            "#,
        "Test", &[] => Value::Bool(true),
        "..= retains the raw identity of existing nested array elements"
        ;
}

#[test]
fn non_nil_effectvar_coalescing_assignment_skips_rhs_and_write() {
    let reads = Arc::new(AtomicUsize::new(0));
    let writes = Arc::new(AtomicUsize::new(0));
    let rhs_calls = Arc::new(AtomicUsize::new(0));

    let observed_reads = Arc::clone(&reads);
    let observed_writes = Arc::clone(&writes);
    let mut engine = Engine::new();
    engine.register_host_function("EffectVar", move |args| {
        if args.len() == 4 {
            observed_writes.fetch_add(1, Ordering::SeqCst);
            Ok(args[3].clone())
        } else {
            assert_eq!(args, [Value::Int(0), Value::Int(0), Value::Int(0)]);
            observed_reads.fetch_add(1, Ordering::SeqCst);
            Ok(Value::Int(7))
        }
    });
    let observed_rhs_calls = Arc::clone(&rhs_calls);
    engine.register_host_function("MarkRhs", move |args| {
        assert!(args.is_empty());
        observed_rhs_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Value::Int(5))
    });
    crate::support::load_script(
        &mut engine,
        r#"
        #strict 3
        func Test() {
            return EffectVar(0, 0, 0) ??= MarkRhs();
        }
    "#,
    );

    assert_eq!(
        engine
            .call("Test", &[])
            .expect("non-nil coalescing assignment succeeds"),
        Value::Int(7)
    );
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(rhs_calls.load(Ordering::SeqCst), 0, "RHS is skipped");
    assert_eq!(
        writes.load(Ordering::SeqCst),
        0,
        "AB_NilCoalescingIt skips the store for a non-nil reference"
    );
}
