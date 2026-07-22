//! Engine-registered script constants (`RegisterGlobalConstant`,
//! C4Aul.cpp / C4Script.cpp:6581 + the C4ScriptConstMap table at :6208):
//! bare identifiers like `DIR_Right` resolve to engine-provided values when
//! no variable of that name exists; locals shadow them.

use clonk_script::{Engine, Script, Value};

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
    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();

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
fn prior_static_constants_resolve_in_declaration_order() {
    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals);
    engine.set_global_constants(consts);
    engine.register_constant("ENGINE_VALUE", Value::Int(9));
    engine.add_script(
        Script::compile(
            "#strict 3\n\
             static const BASE_VALUE = 41, DERIVED_VALUE = BASE_VALUE, ENGINE_ALIAS = ENGINE_VALUE;\n\
             func Probe() { return [DERIVED_VALUE, ENGINE_ALIAS]; }",
        )
        .expect("ordered static constants compile"),
    );
    engine.adopt_statics_into_globals();

    assert_eq!(
        engine
            .call("Probe", &[])
            .expect("derived constant resolves"),
        Value::Array(vec![Value::Int(41), Value::Int(9)])
    );
}

#[test]
fn static_const_rejects_nonliteral_nonconstant_initializer() {
    for initializer in [
        "RuntimeCall()",
        "1 + 2",
        "(1)",
        "-RuntimeValue",
        "!false",
        "[1]",
        "{ value = 1 }",
    ] {
        let script = Script::compile(&format!(
            "#strict 3\n\
             static const BAD_VALUE = {initializer};\n\
             func Good() {{ return 7; }}"
        ))
        .expect("recovering preparser returns the partial script");

        assert!(
            !script.parse_diagnostics().is_empty(),
            "initializer unexpectedly accepted: {initializer}"
        );
        assert!(
            script.functions().contains_key("Good"),
            "recovery lost the following function for: {initializer}"
        );

        let globals = clonk_script::new_global_variables();
        let consts = clonk_script::new_global_variables();
        let registration = clonk_script::register_global_declarations(
            script.var_decls(),
            &globals,
            Some(&consts),
        );
        match initializer {
            "RuntimeCall()" => {
                let error = registration.expect_err("unknown call prefix must fail linking");
                assert_eq!(error.initializer(), "RuntimeCall");
                assert!(consts.borrow().get("BAD_VALUE").is_none());
            }
            "1 + 2" => {
                registration.expect("literal prefix still registers before delimiter failure");
                let value = consts
                    .borrow()
                    .get("BAD_VALUE")
                    .expect("native preparser retains the literal prefix")
                    .borrow()
                    .clone();
                assert_eq!(value, Value::Int(1));
            }
            _ => {
                registration.expect("initializer failed before creating a declaration");
                assert!(consts.borrow().get("BAD_VALUE").is_none());
            }
        }
    }
}

#[test]
fn signed_hex_static_constants_follow_cpp_token_boundary() {
    for initializer in ["+0x1", "-0x1"] {
        let script = Script::compile(&format!(
            "#strict 3\nstatic const BAD_VALUE = {initializer};\n"
        ))
        .expect("recovering preparser returns the partial script");
        assert!(
            !script.parse_diagnostics().is_empty(),
            "signed hexadecimal initializer unexpectedly accepted: {initializer}"
        );
        let globals = clonk_script::new_global_variables();
        let consts = clonk_script::new_global_variables();
        clonk_script::register_global_declarations(script.var_decls(), &globals, Some(&consts))
            .expect("signed decimal prefix is a valid native constant token");
        let value = consts
            .borrow()
            .get("BAD_VALUE")
            .expect("native preparser registers the signed zero prefix")
            .borrow()
            .clone();
        assert_eq!(value, Value::Int(0));
    }

    let direct = Script::compile("#strict 3\nstatic const HEX_VALUE = 0x1;")
        .expect("direct hexadecimal constant compiles");
    assert!(direct.parse_diagnostics().is_empty());
}

#[test]
fn unresolved_static_const_name_stops_registration_with_a_link_error() {
    let script = Script::compile(
        "#strict 3\n\
         static const FIRST_VALUE = 1, BAD_VALUE = MISSING_VALUE, AFTER_VALUE = 2;\n\
         static const RECOVERED_VALUE = 3;",
    )
    .expect("named initializer syntax compiles");
    assert!(script.parse_diagnostics().is_empty());

    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();
    let error =
        clonk_script::register_global_declarations(script.var_decls(), &globals, Some(&consts))
            .expect_err("unknown named initializer must fail linking");

    assert_eq!(error.declaration(), "BAD_VALUE");
    assert_eq!(error.initializer(), "MISSING_VALUE");
    assert_eq!(
        consts
            .borrow()
            .get("FIRST_VALUE")
            .expect("earlier declaration remains registered")
            .borrow()
            .clone(),
        Value::Int(1)
    );
    assert!(consts.borrow().get("BAD_VALUE").is_none());
    assert!(consts.borrow().get("AFTER_VALUE").is_none());
    assert_eq!(
        consts
            .borrow()
            .get("RECOVERED_VALUE")
            .expect("later declaration resumes after the failed group")
            .borrow()
            .clone(),
        Value::Int(3)
    );
}

#[test]
fn mutable_static_cannot_satisfy_constant_initializer_in_fallback_table() {
    let script = Script::compile(
        "#strict 3\nstatic mutable_value;\nstatic const BAD_VALUE = mutable_value;",
    )
    .expect("declaration syntax compiles");
    let fallback = clonk_script::new_global_variables();

    let error = clonk_script::register_global_declarations(script.var_decls(), &fallback, None)
        .expect_err("GlobalNamed mutable cell is not a constant");

    assert_eq!(error.initializer(), "mutable_value");
    assert!(fallback.borrow().contains_key("mutable_value"));
    assert!(!fallback.borrow().contains_key("BAD_VALUE"));
}

#[test]
fn fallback_table_preserves_constants_across_registration_calls() {
    let fallback = clonk_script::new_global_variables();
    for source in [
        "#strict 3\nstatic const FIRST_VALUE = 12;",
        "#strict 3\nstatic const SECOND_VALUE = FIRST_VALUE;",
        "#strict 3\nstatic const THIRD_VALUE = FIRST_VALUE;\nstatic FIRST_VALUE;",
    ] {
        let script = Script::compile(source).expect("declaration syntax compiles");
        clonk_script::register_global_declarations(script.var_decls(), &fallback, None)
            .expect("prior fallback constant resolves");
    }

    assert_eq!(
        fallback
            .borrow()
            .get("SECOND_VALUE")
            .expect("derived fallback constant registered")
            .borrow()
            .clone(),
        Value::Int(12)
    );
    assert_eq!(
        fallback
            .borrow()
            .get("THIRD_VALUE")
            .expect("alias resolves before a later mutable declaration")
            .borrow()
            .clone(),
        Value::Int(12)
    );
}

#[test]
fn later_static_const_declarations_overwrite_the_shared_value() {
    // C4AulScriptEngine::RegisterGlobalConstant reuses the existing name
    // index and assigns the new value (C4Aul.cpp:484-492). Existing hosts
    // keep seeing the same shared cell, now with the replacement value.
    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();

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
    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();

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
    let globals = clonk_script::new_global_variables();
    let consts = clonk_script::new_global_variables();
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
        let globals = clonk_script::new_global_variables();
        let constants = clonk_script::new_global_variables();
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
