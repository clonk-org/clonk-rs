//! Runtime parity for typed script parameters. C4Aul checks and converts every
//! argument slot before entering the callee (`CheckConvertFunctionParameters`,
//! C4AulExec.cpp:1364-1397).

use clonk_script::{Engine, Script, TypeAnnotation, Value, ValueMap};

fn eval(source: &str) -> Result<Value, clonk_script::ScriptError> {
    let mut engine = Engine::new();
    engine.load_script(source).expect("script should load");
    engine.call("Test", &[])
}

fn runtime_message(error: clonk_script::ScriptError) -> String {
    match error {
        clonk_script::ScriptError::Runtime(error) => error.to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn param_with_object_type() {
    let source = r#"func Test(object pMage) { return 1; }"#;
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
fn param_with_int_type() {
    let source = r#"func Test(int value) { return value; }"#;
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
fn map_parameter_carries_map_annotation_and_checks_runtime_type() {
    let script = Script::compile("#strict 3\nfunc Accept(map value) { return value; }")
        .expect("map parameter compiles");
    let parameter = &script.functions()["Accept"].params[0];
    assert_eq!(parameter.name, "value");
    assert_eq!(parameter.type_annotation, Some(TypeAnnotation::Map));

    let mut engine = Engine::new();
    engine.add_script(script);
    let map = Value::Proplist(ValueMap::from([("answer", Value::Int(42))]));
    assert_eq!(
        engine
            .call("Accept", std::slice::from_ref(&map))
            .expect("a C4V_Map argument is accepted"),
        map
    );
    assert_eq!(
        engine
            .call("Accept", &[Value::Nil])
            .expect("nil converts to every non-reference C4Value type"),
        Value::Nil
    );
    assert_eq!(
        runtime_message(
            engine
                .call("Accept", &[Value::Array(Vec::new())])
                .expect_err("an array is not a map")
        ),
        r#"call to "Accept" parameter 1: got "array", but expected "map"!"#
    );
}

#[test]
fn map_parameter_type_parses_after_an_int_parameter() {
    let script = Script::compile("func F(int first, map second) { return second; }")
        .expect("map annotation compiles after an int annotation");
    let params = &script.functions()["F"].params;
    assert_eq!(params[0].type_annotation, Some(TypeAnnotation::Int));
    assert_eq!(params[1].name, "second");
    assert_eq!(params[1].type_annotation, Some(TypeAnnotation::Map));
}

#[test]
fn lone_cpp_type_words_warn_and_become_untyped_parameters_before_strict_two() {
    for strict_prefix in ["", "#strict\n"] {
        for type_name in [
            "int", "bool", "id", "object", "string", "array", "map", "any",
        ] {
            let source = format!("{strict_prefix}func F({type_name}) {{ return {type_name}; }}");
            let script = Script::compile(&source).expect("the legacy diagnostic is nonfatal");
            assert_eq!(script.parse_diagnostics().len(), 1, "source: {source}");
            assert_eq!(
                script.parse_diagnostics()[0].message(),
                format!("parameter has the same name as type {type_name}"),
                "source: {source}"
            );

            let parameter = &script.functions()["F"].params[0];
            assert_eq!(parameter.name, type_name, "source: {source}");
            assert_eq!(parameter.type_annotation, None, "source: {source}");
            assert!(!parameter.is_reference, "source: {source}");
        }
    }
}

#[test]
fn lone_legacy_type_reference_warning_clears_the_reference_flag() {
    for strict_prefix in ["", "#strict\n"] {
        let source = format!("{strict_prefix}func F(int &) {{ return int; }}");
        let script = Script::compile(&source).expect("the legacy diagnostic is nonfatal");
        assert_eq!(script.parse_diagnostics().len(), 1, "source: {source}");
        assert_eq!(
            script.parse_diagnostics()[0].message(),
            "parameter has the same name as type int",
            "source: {source}"
        );

        let parameter = &script.functions()["F"].params[0];
        assert_eq!(parameter.name, "int", "source: {source}");
        assert_eq!(parameter.type_annotation, None, "source: {source}");
        assert!(!parameter.is_reference, "source: {source}");
    }
}

#[test]
fn strict_two_rejects_parameter_named_only_by_type() {
    for strict_level in [2, 3] {
        for type_name in [
            "int", "bool", "id", "object", "string", "array", "map", "any",
        ] {
            let source = format!(
                "#strict {strict_level}\nfunc Broken({type_name}) {{ return 1; }}\n\
                 func Healthy() {{ return 7; }}"
            );
            let script = Script::compile(&source).expect("the bad declaration is recovered");
            assert!(
                script.parse_diagnostics().iter().any(|error| {
                    error.message() == format!("parameter has the same name as type {type_name}")
                }),
                "missing lone-type diagnostic for {source:?}: {:?}",
                script.parse_diagnostics()
            );
            assert!(
                script.functions().contains_key("Healthy"),
                "the next declaration must survive recovery: {source}"
            );
        }
    }
}

#[test]
fn nonlegacy_parameter_type_extensions_are_rejected() {
    for (strict_prefix, strict_level) in [
        ("", 0),
        ("#strict\n", 1),
        ("#strict 2\n", 2),
        ("#strict 3\n", 3),
    ] {
        for parameters in [
            "proplist value",
            "effect value",
            "nil value",
            "object|nil value",
            "int|string value",
        ] {
            let source = format!(
                "{strict_prefix}func Broken({parameters}) {{ return 1; }}\n\
                 func Healthy() {{ return 7; }}"
            );
            let script = Script::compile(&source).expect("the bad declaration is recovered");
            assert!(
                !script.parse_diagnostics().is_empty(),
                "extension must be rejected: {source}"
            );
            if strict_level >= 2 && parameters.contains('|') {
                assert_eq!(
                    script.parse_diagnostics()[0].message(),
                    "unexpected character '|' found",
                    "C++ disables operator lexing after a parameter type: {source}"
                );
            }
            assert!(
                script.functions().contains_key("Healthy"),
                "the next declaration must survive recovery: {source}"
            );
        }
    }

    let ordinary_names = Script::compile(
        "#strict 3\nfunc Names(proplist, effect, Int) { return 1; }",
    )
    .expect("non-type words remain ordinary parameter names");
    assert!(
        ordinary_names.parse_diagnostics().is_empty(),
        "unexpected ordinary-name diagnostic: {:?}",
        ordinary_names.parse_diagnostics()
    );
    let parameters = &ordinary_names.functions()["Names"].params;
    assert_eq!(
        parameters
            .iter()
            .map(|parameter| parameter.name.as_str())
            .collect::<Vec<_>>(),
        ["proplist", "effect", "Int"]
    );
    assert!(parameters
        .iter()
        .all(|parameter| parameter.type_annotation.is_none()));

    let contextual_nil = Script::compile("#strict 2\nfunc Name(nil) { return nil; }")
        .expect("nil remains an identifier below STRICT3");
    assert!(contextual_nil.parse_diagnostics().is_empty());
    assert_eq!(contextual_nil.functions()["Name"].params[0].name, "nil");
    assert_eq!(
        contextual_nil.functions()["Name"].params[0].type_annotation,
        None
    );
}

#[test]
fn legacy_pipe_parameter_names_are_not_union_annotations() {
    for strict_prefix in ["", "#strict\n"] {
        let source = format!(
            "{strict_prefix}func F(object|nil, object|123_AbC, &|value, |plain) {{ return 1; }}"
        );
        let script = Script::compile(&source).expect("legacy pipe identifiers compile");
        assert!(
            script.parse_diagnostics().is_empty(),
            "unexpected legacy diagnostic: {:?}",
            script.parse_diagnostics()
        );
        let parameters = &script.functions()["F"].params;
        assert_eq!(parameters[0].name, "|nil");
        assert_eq!(parameters[0].type_annotation, Some(TypeAnnotation::Object));
        assert!(!parameters[0].is_reference);
        assert_eq!(parameters[1].name, "|123_AbC");
        assert_eq!(parameters[1].type_annotation, Some(TypeAnnotation::Object));
        assert!(!parameters[1].is_reference);
        assert_eq!(parameters[2].name, "|value");
        assert_eq!(parameters[2].type_annotation, None);
        assert!(parameters[2].is_reference);
        assert_eq!(parameters[3].name, "|plain");
        assert_eq!(parameters[3].type_annotation, None);
        assert!(!parameters[3].is_reference);

        let separated = format!(
            "{strict_prefix}func Broken(|@name) {{ return 1; }}\n\
             func Healthy() {{ return 7; }}"
        );
        let script = Script::compile(&separated).expect("the split legacy name is recovered");
        assert!(!script.parse_diagnostics().is_empty());
        assert!(script.functions().contains_key("Healthy"));
    }

    for declaration in [
        "|nil",
        "int|nil",
        "int &|nil",
        "int||nil",
        "int|=nil",
    ] {
        let source = format!("#strict 2\nfunc Broken({declaration}) {{ return 1; }}");
        let script = Script::compile(&source).expect("the invalid pipe spelling is recovered");
        assert_eq!(
            script.parse_diagnostics()[0].message(),
            "unexpected character '|' found",
            "source: {source}"
        );
    }

    let over_cap = Script::compile(
        "#strict 2\nfunc Broken(a,b,c,d,e,f,g,h,i,j,|name) { return 1; }",
    )
    .expect("the invalid eleventh token is recovered");
    assert_eq!(
        over_cap.parse_diagnostics()[0].message(),
        "unexpected character '|' found"
    );

    for declaration in ["int &=value", "int =value", "int & =value"] {
        let source = format!("#strict 2\nfunc Broken({declaration}) {{ return 1; }}");
        let amp_equal =
            Script::compile(&source).expect("the disabled equals spelling is recovered");
        assert_eq!(
            amp_equal.parse_diagnostics()[0].message(),
            "unexpected character '=' found",
            "source: {source}"
        );
    }

    let over_cap_equal = Script::compile(
        "#strict 2\nfunc Broken(a,b,c,d,e,f,g,h,i,j,=value) { return 1; }",
    )
    .expect("the invalid eleventh token is recovered");
    assert_eq!(
        over_cap_equal.parse_diagnostics()[0].message(),
        "unexpected character '=' found"
    );
}

#[test]
fn boolean_literals_are_not_parameter_names() {
    for declaration in ["true", "false", "int true", "int false"] {
        let source = format!(
            "#strict 3\nfunc Broken({declaration}) {{ return 1; }}\n\
             func Healthy() {{ return 7; }}"
        );
        let script = Script::compile(&source).expect("the bad declaration is recovered");
        assert!(
            !script.parse_diagnostics().is_empty(),
            "boolean literal must not bind as a parameter: {source}"
        );
        assert!(script.functions().contains_key("Healthy"), "source: {source}");
    }
}

#[test]
fn strict2_string_argument_to_int_parameter_reports_exact_error() {
    let error = eval(
        r#"
            #strict 2
            func T(int x) { return x; }
            func Test() { return T("abc"); }
        "#,
    )
    .expect_err("string -> int must fail at the call boundary");

    assert_eq!(
        runtime_message(error),
        r#"call to "T" parameter 1: got "string", but expected "int"!"#
    );
}

#[test]
fn int_argument_is_mutated_to_id_before_the_callee_runs() {
    assert_eq!(
        eval("func T(id x) { return x; } func Test() { return T(1000); }")
            .expect("0..=9999 converts to id"),
        Value::C4Id("1000".into())
    );
}

#[test]
fn int_to_id_conversion_rejects_values_outside_the_legacy_range() {
    for value in ["-1", "10000"] {
        let error = eval(&format!(
            "func T(id x) {{ return x; }} func Test() {{ return T({value}); }}"
        ))
        .expect_err("out-of-range int must not convert to id");
        assert_eq!(
            runtime_message(error),
            r#"call to "T" parameter 1: got "int", but expected "id"!"#,
            "value {value}"
        );
    }
}

#[test]
fn pre_strict3_callers_bridge_nil_to_typed_int_and_bool_zeroes() {
    for caller_directive in ["", "#strict 2\n"] {
        let mut engine = Engine::new();
        engine.add_script(
            Script::compile(
                "#strict 3\nfunc Accept(int number, bool flag) { return [number, flag]; }",
            )
            .expect("strict-3 callee compiles"),
        );
        engine.add_script(
            Script::compile(&format!(
                "{caller_directive}func Test() {{ var number, flag; return Accept(number, flag); }}"
            ))
            .expect("caller compiles"),
        );

        assert_eq!(
            engine.call("Test", &[]).expect("bridge call succeeds"),
            Value::Array(vec![Value::Int(0), Value::Bool(false)]),
            "caller directive {caller_directive:?}"
        );
    }
}

#[test]
fn strict3_caller_leaves_nil_typed_parameters_as_nil() {
    assert_eq!(
        eval(
            r#"
                #strict 3
                func Accept(int number, bool flag) { return [number, flag]; }
                func Test() { return Accept(nil, nil); }
            "#,
        )
        .expect("nil is accepted for every non-reference C4Value type"),
        Value::Array(vec![Value::Nil, Value::Nil])
    );
}

#[test]
fn engine_entry_converts_all_ten_slots_before_named_and_par_reads() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
                #strict 3
                func Accept(id definition, int missing, bool flag) {
                    return [definition, missing, flag, Par(0), Par(1), Par(2)];
                }
            "#,
        )
        .expect("script loads");

    let id = Value::C4Id("1000".into());
    assert_eq!(
        engine
            .call("Accept", &[Value::Int(1000)])
            .expect("engine entry converts parameters"),
        Value::Array(vec![
            id.clone(),
            Value::Int(0),
            Value::Bool(false),
            id,
            Value::Int(0),
            Value::Bool(false),
        ])
    );
}

#[test]
fn pre_strict3_engine_entry_normalizes_every_raw_zero_to_nil() {
    let mut engine = Engine::new();
    engine
        .load_script("func Accept(value) { return value; }")
        .expect("script loads");

    for zero in [
        Value::Int(0),
        Value::Bool(false),
        Value::Object(0),
        Value::C4Id("NONE".into()),
        Value::C4Id("0000".into()),
    ] {
        assert_eq!(
            engine.call("Accept", &[zero]).expect("call succeeds"),
            Value::Nil
        );
    }
}

#[test]
fn caller_source_strictness_wins_over_link_destination_owner() {
    let mut destination = Engine::new();
    destination.add_script(
        Script::compile("func Accept(value) { return value; }")
            .expect("nonstrict destination compiles"),
    );

    let mut strict_source = Engine::new();
    strict_source.add_script(
        Script::compile("#strict 3\nfunc Test() { return Accept(0); }")
            .expect("strict source compiles"),
    );
    destination.merge_from(&strict_source);

    assert_eq!(
        destination.call("Test", &[]).expect("linked call succeeds"),
        Value::Int(0),
        "HasStrictNil follows pOrgScript, not the destination script owner"
    );
}
