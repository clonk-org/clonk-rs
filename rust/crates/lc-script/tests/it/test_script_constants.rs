//! Engine-registered script constants (`RegisterGlobalConstant`,
//! C4Aul.cpp / C4Script.cpp:6581 + the C4ScriptConstMap table at :6208):
//! bare identifiers like `DIR_Right` resolve to engine-provided values when
//! no variable of that name exists; locals shadow them.

use lc_script::{Engine, Script, Value};

#[test]
fn registered_constants_resolve_as_identifiers() {
    let mut engine = Engine::new();
    engine.register_constant("DIR_Right", Value::Int(1));
    engine.register_constant("NO_OWNER", Value::Int(-1));
    engine.add_script(
        Script::compile(
            r#"
            global func Probe() { return DIR_Right + NO_OWNER; }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(engine.call("Probe", &[]).expect("call succeeds"), Value::Int(0));
}

#[test]
fn local_variables_shadow_constants() {
    let mut engine = Engine::new();
    engine.register_constant("DIR_Right", Value::Int(1));
    engine.add_script(
        Script::compile(
            r#"
            global func Probe() { var DIR_Right = 9; return DIR_Right; }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(engine.call("Probe", &[]).expect("call succeeds"), Value::Int(9));
}

#[test]
fn old_style_constant_calls_yield_the_constant_below_strict2() {
    // Below #strict 2 a global constant used as `OCF_Chop()` parses as the
    // constant with the call parens ignored ("old-style usage",
    // C4AulParse.cpp:2838-2860) — Objects.c4d grass relies on it.
    let mut engine = Engine::new();
    engine.register_constant("OCF_Chop", Value::Int(256));
    engine.add_script(
        Script::compile(
            r#"
            #strict
            global func Probe() { return OCF_Chop(); }
            "#,
        )
        .expect("script compiles"),
    );
    assert_eq!(
        engine.call("Probe", &[]).expect("call succeeds"),
        Value::Int(256)
    );

    let mut strict2 = Engine::new();
    strict2.register_constant("OCF_Chop", Value::Int(256));
    strict2.add_script(
        Script::compile(
            r#"
            #strict 2
            global func Probe() { return OCF_Chop(); }
            "#,
        )
        .expect("script compiles"),
    );
    assert!(
        strict2.call("Probe", &[]).is_err(),
        "#strict 2 scripts may not call constants"
    );
}

#[test]
fn unknown_identifiers_still_error() {
    let mut engine = Engine::new();
    engine.add_script(
        Script::compile(r#"global func Probe() { return NoSuchConstant; }"#)
            .expect("script compiles"),
    );
    assert!(engine.call("Probe", &[]).is_err());
}

#[test]
fn script_static_consts_are_callable_across_hosts_below_strict2() {
    // `static const` names are ENGINE-GLOBAL constants (the preparser
    // registers them via RegisterGlobalConstant, C4Aul.cpp:484); below
    // #strict 2 a constant used as `NAME()` yields the constant with the
    // empty call parens consumed ("old-style usage", C4AulParse.cpp:
    // 2834-2864) — MagiClonk.c4d/Script.c:76 reads
    // `GetPlrExtraData(iPlayer, MCLK_ComboExtraDataName())` where the
    // constant is declared in MagiClonk's script but the call runs in the
    // MAGE host that merged it.
    let globals = lc_script::new_global_variables();
    let consts = lc_script::new_global_variables();

    let mut declarer = Engine::new();
    declarer.set_global_variables(globals.clone());
    declarer.set_global_constants(consts.clone());
    declarer.add_script(
        Script::compile(
            "#strict\n\nstatic const MCLK_ComboExtraDataName = \"MCLK_PrefCombo\";\n",
        )
        .expect("declaring script compiles"),
    );
    declarer.adopt_statics_into_globals();

    let mut caller = Engine::new();
    caller.set_global_variables(globals.clone());
    caller.set_global_constants(consts.clone());
    caller.add_script(
        Script::compile(
            "#strict\n\nfunc Probe() { return(MCLK_ComboExtraDataName()); }\n",
        )
        .expect("calling script compiles"),
    );
    caller.adopt_statics_into_globals();

    assert_eq!(
        caller.call("Probe", &[]).expect("constant call succeeds"),
        Value::String("MCLK_PrefCombo".to_string().into())
    );
}

#[test]
fn later_static_const_declarations_overwrite_the_shared_value() {
    // C4AulScriptEngine::RegisterGlobalConstant reuses the existing name
    // index and assigns the new value (C4Aul.cpp:484-492). Existing hosts
    // keep seeing the same shared cell, now with the replacement value.
    let globals = lc_script::new_global_variables();
    let consts = lc_script::new_global_variables();

    let mut caller = Engine::new();
    caller.set_global_variables(globals.clone());
    caller.set_global_constants(consts.clone());
    caller.add_script(
        Script::compile("#strict\nfunc Probe() { return(SHARED_VALUE()); }\n")
            .expect("caller compiles"),
    );
    caller.adopt_statics_into_globals();

    for value in [17, 42] {
        let mut declarer = Engine::new();
        declarer.set_global_variables(globals.clone());
        declarer.set_global_constants(consts.clone());
        declarer.add_script(
            Script::compile(&format!("#strict\nstatic const SHARED_VALUE = {value};\n"))
                .expect("declarer compiles"),
        );
        declarer.adopt_statics_into_globals();
        assert_eq!(
            caller.call("Probe", &[]).expect("constant call succeeds"),
            Value::Int(value)
        );
    }
}

#[test]
fn signed_static_consts_are_registered_and_not_assignable() {
    let globals = lc_script::new_global_variables();
    let consts = lc_script::new_global_variables();

    let mut declarer = Engine::new();
    declarer.set_global_variables(globals.clone());
    declarer.set_global_constants(consts.clone());
    declarer.add_script(
        Script::compile("#strict\nstatic const FM_Error = -1;\n")
            .expect("declaration compiles"),
    );
    declarer.adopt_statics_into_globals();

    let mut caller = Engine::new();
    caller.set_global_variables(globals.clone());
    caller.set_global_constants(consts.clone());
    caller.add_script(
        Script::compile(
            "#strict\nfunc Read() { return FM_Error; }\n\
             func Rewrite() { FM_Error = 7; }\n",
        )
        .expect("caller compiles"),
    );
    caller.adopt_statics_into_globals();

    assert_eq!(
        caller.call("Read", &[]).expect("constant resolves"),
        Value::Int(-1)
    );
    assert!(
        caller.call("Rewrite", &[]).is_err(),
        "RegisterGlobalConstant values are not writable GlobalNamed cells"
    );
    assert!(globals.borrow().get("FM_Error").is_none());
    assert_eq!(
        caller.call("Read", &[]).expect("constant remains intact"),
        Value::Int(-1)
    );
}

#[test]
fn plain_static_variables_are_not_callable() {
    // Only CONSTANTS resolve through the call idiom (the C++ parser's
    // constant fallback checks GetGlobalConstant, C4AulParse.cpp:2834;
    // a `static` variable followed by parens never parses as a call).
    let globals = lc_script::new_global_variables();
    let consts = lc_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine.set_global_constants(consts.clone());
    engine.add_script(
        Script::compile("#strict\n\nstatic someVar;\n\nfunc Probe() { return(someVar()); }\n")
            .expect("script compiles"),
    );
    engine.adopt_statics_into_globals();
    assert!(
        engine.call("Probe", &[]).is_err(),
        "static variables never resolve as functions"
    );
}

#[test]
fn constant_calls_reject_parameters_like_cpp() {
    // "parameters not allowed in functional usage of constants" —
    // C4AulParse.cpp:2860 requires the immediate ')' after '('.
    let mut engine = Engine::new();
    engine.register_constant("OCF_Chop", Value::Int(256));
    engine.add_script(
        Script::compile(
            r#"
            #strict
            global func Probe() { return OCF_Chop(5); }
            "#,
        )
        .expect("script compiles"),
    );
    let error = engine.call("Probe", &[]).expect_err("parameters are rejected");
    assert!(
        error.to_string().contains("parameters not allowed"),
        "unexpected error: {error}"
    );
}

#[test]
fn zero_valued_registered_constants_fold_at_each_use_site() {
    for strict_level in [1, 2, 3] {
        let mut engine = Engine::new();
        engine.register_constant("ZERO_VALUE", Value::Int(0));
        engine.register_constant("FALSE_VALUE", Value::Bool(false));
        let directive = if strict_level == 1 {
            "#strict".to_string()
        } else {
            format!("#strict {strict_level}")
        };
        engine
            .load_script(&format!(
                "{directive}\n\
                 func Direct() {{ return ZERO_VALUE; }}\n\
                 func Probe() {{ return [ZERO_VALUE, FALSE_VALUE]; }}"
            ))
            .expect("constant strictness probe compiles");

        let expected_zero = if strict_level < 3 {
            Value::Nil
        } else {
            Value::Int(0)
        };
        let expected_false = if strict_level < 3 {
            Value::Nil
        } else {
            Value::Bool(false)
        };
        assert_eq!(engine.call("Direct", &[]).unwrap(), expected_zero.clone());
        assert_eq!(
            engine.call("Probe", &[]).unwrap(),
            Value::Array(vec![expected_zero, expected_false])
        );
    }
}

#[test]
fn zero_valued_script_constants_fold_but_runtime_statics_do_not() {
    for strict_level in [1, 2, 3] {
        let globals = lc_script::new_global_variables();
        let constants = lc_script::new_global_variables();
        let mut engine = Engine::new();
        engine.set_global_variables(globals);
        engine.set_global_constants(constants);
        let directive = if strict_level == 1 {
            "#strict".to_string()
        } else {
            format!("#strict {strict_level}")
        };
        let old_style_call = if strict_level == 1 {
            "func Called() { return [ZERO_VALUE(), FALSE_VALUE()]; }\n"
        } else {
            ""
        };
        engine.add_script(
            Script::compile(&format!(
                "{directive}\n\
                 static const ZERO_VALUE = 0;\n\
                 static const FALSE_VALUE = false;\n\
                 static slot;\n\
                 func Direct() {{ return ZERO_VALUE; }}\n\
                 {old_style_call}\
                 func Probe() {{ slot = 1 - 1; return [ZERO_VALUE, FALSE_VALUE, slot]; }}"
            ))
            .expect("script constant strictness probe compiles"),
        );
        engine.adopt_statics_into_globals();

        let (expected_zero, expected_false) = if strict_level < 3 {
            (Value::Nil, Value::Nil)
        } else {
            (Value::Int(0), Value::Bool(false))
        };
        assert_eq!(engine.call("Direct", &[]).unwrap(), expected_zero.clone());
        assert_eq!(
            engine.call("Probe", &[]).unwrap(),
            Value::Array(vec![
                expected_zero.clone(),
                expected_false.clone(),
                Value::Int(0),
            ])
        );
        if strict_level == 1 {
            assert_eq!(
                engine.call("Called", &[]).unwrap(),
                Value::Array(vec![expected_zero, expected_false])
            );
        }
    }
}
