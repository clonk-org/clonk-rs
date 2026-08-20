//! Scripted C4Effect callbacks have a legacy exception to ordinary
//! script-call parameter conversion. These regressions exercise the Check
//! callback path that first required it in Rust.

use std::collections::HashMap;
use std::sync::Arc;

use clonk_script::{Engine, LocalCells, Script, ScriptError, Value};

fn runtime_message(error: ScriptError) -> String {
    match error {
        ScriptError::Runtime(error) => error.to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

#[test]
fn effect_check_warns_for_strict2_type_mismatch_without_replacing_the_value() {
    // C4Effect::Check executes the retained Fx<Name>Effect function with
    // nonStrict3WarnConversionOnly enabled (src/C4Effect.cpp:271-287).
    // C4AulScriptFunc::Exec therefore lets a pre-#strict-3 callback run after
    // a failed conversion (src/C4AulExec.cpp:1621-1648), retaining the
    // original C4Value rather than a coerced replacement.
    let mut engine = Engine::new();
    crate::support::load_script(
        &mut engine,
        r#"
        #strict 2
        func FxResearchEffect(string effect_name, object target, int number, int new_number, id definition) {
            return definition;
        }
        func FxNestedEffect(string effect_name, object target, int number, int new_number, id definition) {
            return RequireId(definition);
        }
        func RequireId(id definition) {
            return definition;
        }
    "#,
    );
    let args = vec![
        Value::String("IntOverlayAction".into()),
        Value::Object(1),
        Value::Int(0),
        Value::Nil,
        Value::String("Door".into()),
    ];
    let cells = LocalCells::from_local_vars(&HashMap::new());

    assert_eq!(
        engine
            .call_effect_callback_with_cells_and_this(
                "FxResearchEffect",
                &args,
                &cells,
                Value::Nil,
            )
            .expect("effect checker keeps running after the warning"),
        Value::String("Door".into()),
        "the checker sees C++'s original incompatible string value"
    );
    assert_eq!(
        runtime_message(
            engine
                .call_effect_callback_with_cells_and_this(
                    "FxNestedEffect",
                    &args,
                    &cells,
                    Value::Nil,
                )
                .expect_err("a nested ordinary #strict 2 call still rejects string -> id")
        ),
        r#"call to "RequireId" parameter 1: got "string", but expected "id"!"#
    );
    assert_eq!(
        runtime_message(
            engine
                .call("FxResearchEffect", &args)
                .expect_err("ordinary #strict 2 calls still reject string -> id")
        ),
        r#"call to "FxResearchEffect" parameter 5: got "string", but expected "id"!"#
    );
}

#[test]
fn effect_check_warns_for_pre_strict3_reference_mismatch_but_not_strict3() {
    // `CheckConvertFunctionParameters` applies its warning-only result to the
    // same C4Value::ConvertTo(C4V_pC4Value) failure as any other parameter
    // type (src/C4AulExec.cpp:1364-1397; src/C4Value.cpp:488-620). The
    // exception stops at #strict 3 (src/C4AulExec.cpp:1621-1648).
    let cells = LocalCells::from_local_vars(&HashMap::new());
    let args = vec![Value::String("Door".into())];

    let mut strict2 = Engine::new();
    crate::support::load_script(
        &mut strict2,
        r#"
        #strict 2
        func FxReferenceEffect(&value) { return value; }
    "#,
    );
    assert_eq!(
        strict2
            .call_effect_callback_with_cells_and_this(
                "FxReferenceEffect",
                &args,
                &cells,
                Value::Nil,
            )
            .expect("effect checker keeps running after a reference warning"),
        Value::String("Door".into()),
        "the checker keeps C++'s incompatible non-reference value"
    );

    let mut strict3 = Engine::new();
    crate::support::load_script(
        &mut strict3,
        r#"
        #strict 3
        func FxStrictEffect(id definition) { return definition; }
    "#,
    );
    assert_eq!(
        runtime_message(
            strict3
                .call_effect_callback_with_cells_and_this(
                    "FxStrictEffect",
                    &args,
                    &cells,
                    Value::Nil,
                )
                .expect_err("#strict 3 must keep conversion failures fatal")
        ),
        r#"call to "FxStrictEffect" parameter 1: got "string", but expected "id"!"#
    );
}

#[test]
fn exact_global_effect_check_passes_values_to_strict3_reference_parameters() {
    // C4Effect::Check constructs an initializer-list C4AulParSet from owned
    // C4Values; unlike C4Material's explicit GetRef calls, none of those
    // arguments is a C4V_pC4Value (src/C4Effect.cpp:271-287;
    // src/C4Material.cpp:814-815). A strict-3 `&` checker must therefore
    // reject the first value parameter.
    let globals = Script::compile(
        r#"
            #strict 3
            global func FxStrictEffect(&new_name) { return new_name; }
        "#,
    )
    .expect("strict-3 global checker compiles");
    let mut engine = Engine::new();
    engine.set_global_functions(Some(Arc::new(globals.functions().clone())));

    assert_eq!(
        runtime_message(
            engine
                .call_global_for_effect_callback(
                    "FxStrictEffect",
                    &[Value::String("Pending".into())],
                )
                .expect_err("C4Effect value arguments do not satisfy a strict-3 reference")
        ),
        r#"call to "FxStrictEffect" parameter 1: got "string", but expected "&"!"#
    );
}
