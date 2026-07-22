// Test for reference return functions (func &)

use clonk_script::{Engine, Value};

#[test]
fn private_func_ref_no_params() {
    // private func & FuncName()
    let source = r#"private func & GetValue() { return(Local(0)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn public_func_ref_no_params() {
    // public func & FuncName()
    let source = r#"public func & GetData() { return(Var(0)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn func_ref_with_params() {
    // func & with parameters
    let source = r#"func & GetSlot(int index) { return(Local(index)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn race_line_47_pattern() {
    // Exact pattern from RACE line 47
    let source = r#"private func & PlayerDeaths(int iPlr) { return(Local(iPlr*2)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn race_line_48_pattern() {
    // Exact pattern from RACE line 48
    let source = r#"private func & TeamDeaths(int iTeam) { return(Local(iTeam*2+1)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn multiple_ref_functions() {
    // Multiple reference return functions in same script
    let source = r#"
    private func & GetA() { return(Local(0)); }
    private func & GetB() { return(Local(1)); }
    public func & GetC() { return(Var(0)); }
    "#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn ref_func_used_as_lvalue() {
    // Reference return function used in assignment (lvalue)
    let source = r#"
    private func & GetSlot(int i) { return(Local(i)); }
    func Test() { GetSlot(0) = 42; }
    "#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn ref_func_with_increment() {
    // Reference return function with increment
    let source = r#"
    private func & Counter() { return(Local(0)); }
    func Test() { ++Counter(); }
    "#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn protected_func_ref() {
    // protected func & pattern
    let source = r#"protected func & GetInternal() { return(Local(5)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn global_func_ref() {
    // global func & pattern
    let source = r#"global func & GetGlobal() { return(Var()); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn func_ref_no_access_modifier() {
    // func & without explicit access modifier (defaults to public)
    let source = r#"func & DefaultAccess() { return(Local()); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn ref_func_multiple_params() {
    // func & with multiple parameters
    let source = r#"private func & GetValue(int x, int y, object obj) { return(Local(x + y)); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn effect_callback_without_ref_return() {
    // Make sure regular functions still work
    let source = r#"global func FxFireStart(effect, target) { return effect + target; }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn reference_return_mutates_local_slot() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            func & Slot(int index) { return Local(index); }
            func Test() {
                Slot(0) = 42;
                return Local(0);
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(42));
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
