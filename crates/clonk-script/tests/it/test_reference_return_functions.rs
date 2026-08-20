// Test for reference return functions (func &)

use clonk_script::{Engine, Value};

// private func & FuncName()
crate::support::compile_cases! {
    private_func_ref_no_params: r#"private func & GetValue() { return(Local(0)); }"#;

// public func & FuncName()
    public_func_ref_no_params: r#"public func & GetData() { return(Var(0)); }"#;

// func & with parameters
    func_ref_with_params: r#"func & GetSlot(int index) { return(Local(index)); }"#;

// Exact pattern from RACE line 47
    race_line_47_pattern: r#"private func & PlayerDeaths(int iPlr) { return(Local(iPlr*2)); }"#;

// Exact pattern from RACE line 48
    race_line_48_pattern: r#"private func & TeamDeaths(int iTeam) { return(Local(iTeam*2+1)); }"#;

// Multiple reference return functions in same script
    multiple_ref_functions:
    r#"
    private func & GetA() { return(Local(0)); }
    private func & GetB() { return(Local(1)); }
    public func & GetC() { return(Var(0)); }
    "#;

// Reference return function used in assignment (lvalue)
    ref_func_used_as_lvalue:
    r#"
    private func & GetSlot(int i) { return(Local(i)); }
    func Test() { GetSlot(0) = 42; }
    "#;

// Reference return function with increment
    ref_func_with_increment:
    r#"
    private func & Counter() { return(Local(0)); }
    func Test() { ++Counter(); }
    "#;

// protected func & pattern
    protected_func_ref: r#"protected func & GetInternal() { return(Local(5)); }"#;

// global func & pattern
    global_func_ref: r#"global func & GetGlobal() { return(Var()); }"#;

// func & without explicit access modifier (defaults to public)
    func_ref_no_access_modifier: r#"func & DefaultAccess() { return(Local()); }"#;

// func & with multiple parameters
    ref_func_multiple_params: r#"private func & GetValue(int x, int y, object obj) { return(Local(x + y)); }"#;

// Make sure regular functions still work
    effect_callback_without_ref_return: r#"global func FxFireStart(effect, target) { return effect + target; }"#;
}

run_cases! {
    reference_return_mutates_local_slot:
            r#"
            func & Slot(int index) { return Local(index); }
            func Test() {
                Slot(0) = 42;
                return Local(0);
            }
            "#,
        "Test", &[] => Value::Int(42);
}

#[test]
fn reference_return_mutates_array_and_proplist_elements() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            local Data;

            func & ArraySlot(int index) { return Data[index]; }
            func & Score() { return Data.score; }

            func TestArray() {
                Data = [1, 2];
                ArraySlot(1) = 9;
                return Data[0] + Data[1];
            }

            func TestProplist() {
                Data = { score = 3 };
                Score() = 11;
                return Data.score;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("TestArray", &[]).unwrap(), Value::Int(10));
    assert_eq!(engine.call("TestProplist", &[]).unwrap(), Value::Int(11));
}

#[test]
fn arrow_reference_return_call_is_an_assignment_lvalue() {
    // C4AulParse.cpp:3154-3245 leaves AB_CALL's result reference intact for
    // the AB_Set lvalue; C4AulExec.cpp:1054-1067 preserves it when the
    // callee is `func &`. Kingdoms' THRN uses this exact call-lvalue shape.
    let source = r#"
        local sacrifice_made;
        public func & SacrificeMade() { return sacrifice_made; }
        public func Mark(target) { target->SacrificeMade() = 1; }
    "#;

    clonk_script::Script::compile(source).expect("arrow func-& result is assignable");
}
