// Test C4Aul assignment precedence around the unary NOT operator.

use clonk_script::{Engine, Script, Value};

fn global_value(globals: &clonk_script::GlobalVariables, name: &str) -> Value {
    globals
        .borrow()
        .get(name)
        .map(|value| value.borrow().clone())
        .unwrap_or(Value::Nil)
}

#[test]
fn bang_assignment_errors_after_rhs_without_writing_or_continuing() {
    // C4AulParse.cpp:2702-2966 leaves `!x` as the assignment's lhs;
    // AB_Set then evaluates the chained rhs before rejecting bool -> reference
    // (C4AulExec.cpp:858-864). The failed outer write must not reach x.
    let source = r#"
        #strict 2
        static x;
        static y;
        static rhs_calls;
        static after;
        func Mark() { ++rhs_calls; return 42; }
        func Test() { !x = y = Mark(); after = 1; }
    "#;
    let script = Script::compile(source).expect("runtime-invalid assignment still compiles");
    assert!(
        script.parse_diagnostics().is_empty(),
        "the C++-valid expression must not be quarantined: {:?}",
        script.parse_diagnostics()
    );

    let globals = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine.add_script(script);
    engine.adopt_statics_into_globals();

    let error = engine
        .call("Test", &[])
        .expect_err("assigning through the bool result of !x must fail");
    assert!(
        error.to_string().contains("operator \"=\" left side")
            && error.to_string().contains("bool"),
        "unexpected runtime error: {error}"
    );

    assert_eq!(
        global_value(&globals, "x"),
        Value::Nil,
        "the invalid outer lhs is never written"
    );
    assert_eq!(
        global_value(&globals, "y"),
        Value::Int(42),
        "the nested rhs assignment runs"
    );
    assert_eq!(
        global_value(&globals, "rhs_calls"),
        Value::Int(1),
        "the rhs runs exactly once"
    );
    assert_eq!(
        global_value(&globals, "after"),
        Value::Nil,
        "the runtime error aborts the function"
    );
}

#[test]
fn bang_compound_assignment_remains_a_runtime_reference_error() {
    let source = r#"
        #strict 2
        static x;
        static rhs_calls;
        static after;
        func Mark() { ++rhs_calls; return 42; }
        func Test() { !x += Mark(); after = 1; }
    "#;
    let script = Script::compile(source).expect("runtime-invalid compound assignment compiles");
    assert!(
        script.parse_diagnostics().is_empty(),
        "the C++-valid compound expression must not be quarantined: {:?}",
        script.parse_diagnostics()
    );

    let globals = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine.add_script(script);
    engine.adopt_statics_into_globals();

    let error = engine
        .call("Test", &[])
        .expect_err("the bool result of !x is not a += reference");
    assert!(
        error.to_string().contains("operator \"+=\" left side")
            && error.to_string().contains("bool"),
        "unexpected runtime error: {error}"
    );
    assert_eq!(global_value(&globals, "x"), Value::Nil);
    assert_eq!(global_value(&globals, "rhs_calls"), Value::Int(1));
    assert_eq!(global_value(&globals, "after"), Value::Nil);
}

#[test]
fn bang_nil_coalescing_assignment_short_circuits_before_reference_check() {
    let source = r#"
        #strict 2
        static x;
        static rhs_calls;
        func Mark() { ++rhs_calls; return 42; }
        func Test() { return !x ??= Mark(); }
    "#;
    let script = Script::compile(source).expect("nil-coalescing assignment compiles");
    assert!(script.parse_diagnostics().is_empty());

    let globals = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine.add_script(script);
    engine.adopt_statics_into_globals();

    assert_eq!(
        engine.call("Test", &[]).expect("non-nil lhs skips AB_Set"),
        Value::Bool(true)
    );
    assert_eq!(global_value(&globals, "x"), Value::Nil);
    assert_eq!(global_value(&globals, "rhs_calls"), Value::Nil);
}

// Exact pattern from DYNB line 57: if(!iCount = GetComponent(...))
crate::support::compile_cases! {
    dynb_line_57_pattern:
    r#"
        func Test() {
            var iCount;
            if(!iCount = GetComponent()) {
                return 1;
            }
        }
    "#;

// Simple: !x = y is C++-valid syntax with a runtime-invalid bool lhs.
    not_with_assignment: r#"func Test() { var x; if(!x = 42) return 1; }"#;

// Precedence preservation: !a + b should still be (!a) + b, not !(a + b)
    not_with_addition_preserved: r#"func Test() { var a = 1; var b = 2; return !a + b; }"#;

// Baseline: !func() should continue to work
    not_with_function_call: r#"func Test() { return !GetFlag(); }"#;

// Control: !(x = y) with explicit parens should work
    not_with_parenthesized_assignment: r#"func Test() { var x; if(!(x = 42)) return 1; }"#;
}

#[test]
fn increment_not_affected() {
    // ++x = y should still be invalid (pre-increment doesn't return lvalue in this context)
    // This ensures our fix doesn't break increment/decrement behavior
    let source = r#"func Test() { var x; ++x = 42; }"#;
    let script = clonk_script::Script::compile(source)
        .expect("the invalid function body is quarantined instead of aborting the script");
    assert!(!script.parse_diagnostics().is_empty());

    let mut engine = clonk_script::Engine::new();
    engine.add_script(script);
    let error = engine
        .call("Test", &[])
        .expect_err("calling the quarantined function must surface its parse error");
    assert!(error.to_string().contains("parse error"));
}

// Complex RHS: !x = a + b
crate::support::compile_cases! {
    not_with_complex_assignment: r#"func Test() { var x, a = 1, b = 2; if(!x = a + b) return 1; }"#;

// Ensure ~ (bitwise NOT) still has normal precedence with addition
    bitwise_not_precedence_preserved: r#"func Test() { var a = 5; return ~a + 1; }"#;

// Ensure - (unary negate) still has normal precedence with addition
    negate_precedence_preserved: r#"func Test() { var a = 5; return -a + 1; }"#;
}
