use std::collections::HashMap;

use clonk_script::{Engine, ScriptError, Value};

const VALUE_STACK_OVERFLOW: &str = "internal error: value stack overflow!";

fn repeated_values(count: usize) -> String {
    vec!["1"; count].join(",")
}

fn map_entries(count: usize) -> String {
    (0..count)
        .map(|index| format!("key{index}=1"))
        .collect::<Vec<_>>()
        .join(",")
}

fn assert_value_stack_overflow(error: ScriptError) {
    let ScriptError::Runtime(error) = error else {
        panic!("expected runtime value-stack overflow, got {error}");
    };
    assert_eq!(error.message(), VALUE_STACK_OVERFLOW);
}

#[test]
fn vm_enforces_cpp_1024_value_stack_limit_for_large_array_literal() {
    // An ordinary engine-to-script entry already owns ten C4Aul parameter
    // slots. The flat literal may therefore retain 1,014 operands, while the
    // next operand would require value-stack slot 1,025.
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func Fits() {{ return [{}]; }}
            func Overflows() {{ return [{}]; }}
            "#,
            repeated_values(1014),
            repeated_values(1015),
        ))
        .expect("array-boundary script loads");

    let Value::Array(values) = engine.call("Fits", &[]).expect("1,014 operands fit") else {
        panic!("Fits must return an array");
    };
    assert_eq!(values.len(), 1014);

    assert_value_stack_overflow(
        engine
            .call("Overflows", &[])
            .expect_err("the 1,015th operand must overflow"),
    );
}

#[test]
fn direct_exec_uses_all_1024_value_stack_slots() {
    // Native DirectExec supplies no ten-slot parameter frame, so its flat
    // aggregate boundary is the full C4AulExec::Values capacity.
    let engine = Engine::new();
    let locals = HashMap::new();
    let (value, _) = engine
        .direct_exec_with_locals_and_this_at_strict(
            &format!("[{}]", repeated_values(1024)),
            &locals,
            Value::Nil,
            Some(3),
        )
        .expect("1,024 DirectExec operands fit");
    let Value::Array(values) = value else {
        panic!("DirectExec must return an array");
    };
    assert_eq!(values.len(), 1024);

    assert_value_stack_overflow(
        engine
            .direct_exec_with_locals_and_this_at_strict(
                &format!("[{}]", repeated_values(1025)),
                &locals,
                Value::Nil,
                Some(3),
            )
            .expect_err("DirectExec operand 1,025 must overflow"),
    );
}

#[test]
fn eval_reentry_shares_the_suspended_callers_value_stack() {
    let mut engine = Engine::new();
    engine
        .load_script("#strict 3\nfunc Run(code) { return eval(code); }")
        .expect("eval stack-boundary script loads");

    assert!(matches!(
        engine
            .call(
                "Run",
                &[Value::String(
                    format!("[{}]", repeated_values(1013)).into(),
                )],
            )
            .expect("caller frame plus eval parameter leaves 1,013 slots"),
        Value::Array(values) if values.len() == 1013
    ));
    assert_value_stack_overflow(
        engine
            .call(
                "Run",
                &[Value::String(
                    format!("[{}]", repeated_values(1014)).into(),
                )],
            )
            .expect_err("DirectExec must retain its suspended caller and eval parameter"),
    );
}

#[test]
fn map_and_nested_expression_operands_share_the_value_stack() {
    // A map retains key and value slots for every entry. A nested binary
    // expression likewise retains its left side while evaluating its right;
    // neither receives an independent per-container/per-expression budget.
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func MapFits() {{ return {{ {} }}; }}
            func MapOverflows() {{ return {{ {} }}; }}
            func NestedFits() {{ return [{}, 1 + 2]; }}
            func NestedOverflows() {{ return [{}, 1 + 2]; }}
            "#,
            map_entries(507),
            map_entries(508),
            repeated_values(1012),
            repeated_values(1013),
        ))
        .expect("map and nested-expression boundary script loads");

    let Value::Proplist(entries) = engine.call("MapFits", &[]).expect("507 map pairs fit") else {
        panic!("MapFits must return a map");
    };
    assert_eq!(entries.len(), 507);
    assert_value_stack_overflow(
        engine
            .call("MapOverflows", &[])
            .expect_err("map pair 508 must overflow"),
    );

    let Value::Array(values) = engine
        .call("NestedFits", &[])
        .expect("the nested binary peak fits exactly")
    else {
        panic!("NestedFits must return an array");
    };
    assert_eq!(values.len(), 1013);
    assert_eq!(values.last(), Some(&Value::Int(3)));
    assert_value_stack_overflow(
        engine
            .call("NestedOverflows", &[])
            .expect_err("the nested right operand must overflow"),
    );
}

#[test]
fn only_a_direct_string_index_uses_cpp_embedded_operand_optimization() {
    // AB_MAPA_R/V embeds a raw string token in the bytecode. Parentheses make
    // the same literal an ordinary dynamic index and therefore require one
    // additional live value-stack slot.
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func EmbeddedFits() {{ var map = {{ key = 1 }}; return [{}, map["key"]]; }}
            func ParenthesizedOverflows() {{ var map = {{ key = 1 }}; return [{}, map[("key")]]; }}
            "#,
            repeated_values(1012),
            repeated_values(1012),
        ))
        .expect("string-index boundary script loads");

    assert!(matches!(
        engine
            .call("EmbeddedFits", &[])
            .expect("the embedded string operand fits exactly"),
        Value::Array(values) if values.len() == 1013 && values.last() == Some(&Value::Int(1))
    ));
    assert_value_stack_overflow(
        engine
            .call("ParenthesizedOverflows", &[])
            .expect_err("a parenthesized string index must consume one dynamic slot"),
    );
}

#[test]
fn value_stack_releases_statement_results_and_unwinds_after_error() {
    let statements = "1;".repeat(2048);
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func Overflows() {{ return [{}]; }}
            func ManySmallExpressions() {{ {statements} return 7; }}
            "#,
            repeated_values(1015),
        ))
        .expect("unwind script loads");

    assert_value_stack_overflow(
        engine
            .call("Overflows", &[])
            .expect_err("large aggregate must overflow"),
    );
    assert_eq!(
        engine
            .call("ManySmallExpressions", &[])
            .expect("a later call sees a clean stack"),
        Value::Int(7),
        "the budget tracks simultaneously live values, not cumulative evaluations",
    );
}

#[test]
fn surplus_call_arguments_remain_live_until_cpp_arity_balancing() {
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func Target() {{ return 9; }}
            func Fits() {{ return Target({}); }}
            func Overflows() {{ return Target({}); }}
            "#,
            repeated_values(1014),
            repeated_values(1015),
        ))
        .expect("surplus-argument boundary script loads");

    assert_eq!(
        engine.call("Fits", &[]).expect("1,014 arguments fit"),
        Value::Int(9),
    );
    assert_value_stack_overflow(
        engine
            .call("Overflows", &[])
            .expect_err("argument 1,015 must overflow before dispatch"),
    );
}

#[test]
fn hoisted_locals_foreach_and_short_circuit_share_the_dynamic_budget() {
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict 3
            func LocalFits() {{ var local; return [{}]; }}
            func LocalOverflows() {{ var local; return [{}]; }}
            func ForeachFits() {{ for (var item in [1]) {{ return [{}]; }} }}
            func ForeachOverflows() {{ for (var item in [1]) {{ return [{}]; }} }}
            func SkipsOversizedRhs() {{ return false && [{}]; }}
            "#,
            repeated_values(1013),
            repeated_values(1014),
            repeated_values(1011),
            repeated_values(1012),
            repeated_values(1015),
        ))
        .expect("locals/foreach/short-circuit script loads");

    assert!(matches!(
        engine.call("LocalFits", &[]).expect("one local leaves 1,013 operand slots"),
        Value::Array(values) if values.len() == 1013
    ));
    assert_value_stack_overflow(
        engine
            .call("LocalOverflows", &[])
            .expect_err("hoisted locals stay live through the expression"),
    );
    assert!(matches!(
        engine.call("ForeachFits", &[]).expect("foreach body fits at its exact peak"),
        Value::Array(values) if values.len() == 1011
    ));
    assert_value_stack_overflow(
        engine
            .call("ForeachOverflows", &[])
            .expect_err("foreach cursor slots stay live through the body"),
    );
    assert_eq!(
        engine
            .call("SkipsOversizedRhs", &[])
            .expect("an unexecuted oversized branch consumes no slots"),
        Value::Bool(false),
    );
}
