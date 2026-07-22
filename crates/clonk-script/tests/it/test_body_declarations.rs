use std::collections::HashMap;

use clonk_script::{new_global_variables, Engine, Script, Value, VarDeclKind};

#[test]
fn nested_body_declarations_register_before_execution_and_across_functions() {
    let script = Script::compile(
        r#"#strict 3
func NeverCalled() {
    if (false) {
        local first, second;
        static total;
    }
}
func Probe() {
    first = 3;
    second = 4;
    total = 5;
    return [first, second, total];
}
"#,
    )
    .expect("body declarations compile");
    assert!(script.parse_diagnostics().is_empty());
    assert_eq!(
        script
            .var_decls()
            .iter()
            .map(|declaration| (declaration.kind, declaration.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (VarDeclKind::Local, "first"),
            (VarDeclKind::Local, "second"),
            (VarDeclKind::Static, "total"),
        ]
    );

    let globals = new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals.clone());
    engine.add_script(script);
    assert_eq!(
        engine.local_variable_names().collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert_eq!(
        globals
            .borrow()
            .get("total")
            .map(|cell| cell.borrow().clone()),
        Some(Value::Nil),
        "the dead branch registers its static before either function runs"
    );

    let (value, locals) = engine
        .call_with_locals("Probe", &[], &HashMap::new())
        .expect("the other function sees the hoisted declarations");
    assert_eq!(
        value,
        Value::Array(vec![Value::Int(3), Value::Int(4), Value::Int(5)])
    );
    assert_eq!(locals.get("first"), Some(&Value::Int(3)));
    assert_eq!(locals.get("second"), Some(&Value::Int(4)));
    assert_eq!(
        globals
            .borrow()
            .get("total")
            .map(|cell| cell.borrow().clone()),
        Some(Value::Int(5))
    );
}

#[test]
fn old_style_body_declarations_are_script_wide() {
    let script = Script::compile(
        r#"#strict
Declarations:
    if (false) {
        local old_local;
        static old_static;
    }
    return(0);
Probe:
    old_local = 6;
    old_static = 7;
    return([old_local, old_static]);
"#,
    )
    .expect("old-style body declarations compile");
    assert!(
        script.parse_diagnostics().is_empty(),
        "unexpected diagnostics: {:?}",
        script.parse_diagnostics()
    );
    assert_eq!(
        script
            .var_decls()
            .iter()
            .map(|declaration| (declaration.kind, declaration.name.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (VarDeclKind::Local, "old_local"),
            (VarDeclKind::Static, "old_static"),
        ]
    );

    let globals = new_global_variables();
    let mut engine = Engine::new();
    engine.set_global_variables(globals);
    engine.add_script(script);
    let (value, locals) = engine
        .call_with_locals("Probe", &[], &HashMap::new())
        .expect("old-style declarations are visible from the next function");
    assert_eq!(value, Value::Array(vec![Value::Int(6), Value::Int(7)]));
    assert_eq!(locals.get("old_local"), Some(&Value::Int(6)));
}

#[test]
fn local_and_static_initializers_are_rejected_in_every_declaration_position() {
    for (label, source) in [
        ("top-level local", "local value = 5;"),
        ("top-level static", "static value = 5;"),
        ("new local", "func Bad() { local value = 5; }"),
        ("new static", "func Bad() { static value = 5; }"),
        ("old local", "#strict\nBad:\n local value = 5;"),
        ("old static", "#strict\nBad:\n static value = 5;"),
    ] {
        let script = Script::compile(source).expect("invalid declaration is quarantined");
        assert!(
            script
                .parse_diagnostics()
                .iter()
                .any(|error| error.message().contains("',' or ';'")),
            "{label} must retain the C++ list-terminator diagnostic: {:?}",
            script.parse_diagnostics()
        );
    }
}

#[test]
fn global_old_style_local_reports_without_poisoning_the_function() {
    let script = Script::compile_global(
        r#"#strict
global Broken:
    local forbidden;
    return(9);
global Healthy:
    return(7);
"#,
    )
    .expect("the global script recovers after the preparser diagnostic");
    assert!(
        script.parse_diagnostics().iter().any(|error| {
            error
                .message()
                .contains("'local' variable declaration in global script")
        }),
        "unexpected diagnostics: {:?}",
        script.parse_diagnostics()
    );
    assert_eq!(
        script
            .var_decls()
            .iter()
            .map(|declaration| (declaration.kind, declaration.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(VarDeclKind::Local, "forbidden")],
        "preparse recovery retries the declaration and registers its names"
    );
    let mut engine = Engine::new();
    engine.add_script(script);
    assert_eq!(
        engine
            .call("Broken", &[])
            .expect("the later parser pass retains the function"),
        Value::Int(9)
    );

    let new_style = Script::compile_global(
        "#strict\nglobal func Allowed() { local declared; return(1); }",
    )
    .expect("new-style global function compiles");
    assert!(
        new_style.parse_diagnostics().is_empty(),
        "the ownerless-local restriction is old-style only: {:?}",
        new_style.parse_diagnostics()
    );
    assert_eq!(
        new_style
            .var_decls()
            .iter()
            .map(|declaration| (declaration.kind, declaration.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(VarDeclKind::Local, "declared")]
    );

    let malformed = Script::compile_global(
        "#strict\nglobal Invalid:\n local value = 5;\n return(9);",
    )
    .expect("the malformed declaration is quarantined");
    assert!(
        malformed.parse_diagnostics().iter().any(|error| {
            error
                .message()
                .contains("'local' variable declaration in global script")
        }),
        "the preparser diagnostic survives the later parser error: {:?}",
        malformed.parse_diagnostics()
    );
    assert!(
        malformed
            .parse_diagnostics()
            .iter()
            .any(|error| error.message().contains("',' or ';'")),
        "the parser pass still rejects an initializer: {:?}",
        malformed.parse_diagnostics()
    );
    assert_eq!(
        malformed
            .var_decls()
            .iter()
            .map(|declaration| (declaration.kind, declaration.name.as_str()))
            .collect::<Vec<_>>(),
        vec![(VarDeclKind::Local, "value")],
        "preparse registers the name before rejecting its delimiter"
    );
    let mut engine = Engine::new();
    engine.add_script(malformed);
    engine
        .call("Invalid", &[])
        .expect_err("a parser-pass declaration error still poisons the function");
}
