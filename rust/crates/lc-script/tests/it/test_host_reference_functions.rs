//! Native `C4Value *` parameters retain caller lvalues while non-lvalues
//! arrive as null references (FnSimFlight, C4Script.cpp:5309-5312). This file
//! freezes that VM/host boundary without implementing SimFlight itself.

use lc_script::{Engine, RuntimeError, Value};

#[test]
fn reference_aware_host_writes_through_a_local_lvalue() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("SetRef", [0], |args| {
        let target = args
            .first()
            .ok_or_else(|| RuntimeError::new("SetRef expects one argument"))?;
        assert!(target.is_reference());
        assert_eq!(target.read()?, Value::Int(3));
        assert!(target.write(Value::Int(9))?);
        Ok(Value::Bool(true))
    });
    engine
        .load_script(
            r#"
            func Test() {
                var value = 3;
                SetRef(value);
                return value;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(9));
}

#[test]
fn reference_aware_host_writes_through_property_and_index_lvalues() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("SetRef", [0], |args| {
        let target = args
            .first()
            .ok_or_else(|| RuntimeError::new("SetRef expects a target"))?;
        let value = args
            .get(1)
            .ok_or_else(|| RuntimeError::new("SetRef expects a value"))?
            .read()?;
        assert!(target.is_reference());
        assert!(!args[1].is_reference());
        assert!(target.write(value)?);
        Ok(Value::Bool(true))
    });
    engine
        .load_script(
            r#"
            func Test() {
                var data = { score = 4 };
                var values = [1, 2];
                SetRef(data.score, 8);
                SetRef(values[0], 10);
                return data.score + values[0];
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(18));
}

#[test]
fn ordinary_host_arguments_remain_copied_values() {
    let mut engine = Engine::new();
    engine.register_host_function("Observe", |args| {
        assert_eq!(args, [Value::Int(3)]);
        Ok(Value::Int(9))
    });
    engine
        .load_script(
            r#"
            func Test() {
                var value = 3;
                Observe(value);
                return value;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(3));
}

#[test]
fn declared_reference_non_lvalue_is_observable_without_forced_writeback() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("RequireRefs", [0, 1], |args| {
        assert!(args[0].is_reference());
        assert!(!args[1].is_reference());
        assert_eq!(args[1].read()?, Value::Int(3));
        assert!(!args[1].write(Value::Int(7))?);
        // FnSimFlight validates every pointer before simulation and therefore
        // leaves even valid earlier refs untouched when one is null
        // (C4Script.cpp:5311-5312).
        Ok(Value::Nil)
    });
    engine
        .load_script(
            r#"
            func Three() { return 3; }
            func Test() {
                var value = 5;
                var result = RequireRefs(value, Three());
                return [value, result];
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(
        engine.call("Test", &[]).unwrap(),
        Value::Array(vec![Value::Int(5), Value::Nil])
    );
}

#[test]
fn inherited_reaches_reference_aware_host_with_the_callers_lvalue() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("SetRef", [0], |args| {
        assert!(args[0].write(Value::Int(12))?);
        Ok(Value::Bool(true))
    });
    engine
        .load_script(
            r#"
            func SetRef(&value) { return inherited(value); }
            func Test() {
                var value = 2;
                SetRef(value);
                return value;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(12));
}

#[test]
fn engine_scope_unqualified_call_preserves_reference_metadata() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("SetRef", [0], |args| {
        assert!(args[0].write(Value::Int(14))?);
        Ok(Value::Bool(true))
    });
    engine
        .load_script(
            r#"
            global func Test() {
                var value = 4;
                SetRef(value);
                return value;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(14));
}

#[test]
fn common_host_removal_surface_clears_reference_aware_registration() {
    let mut engine = Engine::new();
    engine.register_host_reference_function("SetRef", [0], |_| Ok(Value::Nil));
    assert!(engine.has_host_function("SetRef"));

    assert!(engine.remove_host_function("SetRef").is_none());
    assert!(!engine.has_host_function("SetRef"));
    assert!(engine.call("SetRef", &[]).is_err());
}
