// Test for reference parameters (&param)

use std::sync::Arc;

use clonk_script::{Engine, Script, Value};

#[test]
fn simple_reference_parameter() {
    let source = r#"func SetValues(&x, &y) { x = 10; y = 20; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn reference_with_type_annotation() {
    let source = r#"func SetValues(int &x, int &y) { x = 10; y = 20; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn mixed_reference_and_value_parameters() {
    let source = r#"func GetSum(a, b, &result) { result = a + b; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn mgwp_pattern() {
    // The actual pattern from MGWP script
    let source = r#"private func GetWarpPosition(&x, &y) { x = 10; y = 20; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn reference_parameter_with_object_type() {
    let source = r#"func GetObject(object &obj) { obj = FindObject(CLNK); }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn reference_parameter_mutates_variable() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            func SetValue(&x) { x = 7; }
            func Test() {
                var value = 1;
                SetValue(value);
                return value;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("Test", &[]).unwrap(), Value::Int(7));
}

#[test]
fn reference_parameter_mutates_array_and_proplist_elements() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            func SetValue(&x, value) { x = value; }
            func TestArray() {
                var values = [1, 2];
                SetValue(values[0], 10);
                return values[0] + values[1];
            }
            func TestProplist() {
                var data = { score = 4 };
                SetValue(data.score, 8);
                return data.score;
            }
            "#,
        )
        .expect("script loads");

    assert_eq!(engine.call("TestArray", &[]).unwrap(), Value::Int(12));
    assert_eq!(engine.call("TestProplist", &[]).unwrap(), Value::Int(8));
}

#[test]
fn installed_global_reference_parameter_mutates_object_function_local() {
    // C4Aul resolves the callee before emitting AB_CALL, so a System.c4g
    // global `&` parameter receives the caller's lvalue just like an own-script
    // function (C4AulParse.cpp:2808-2813, 2885-2892). Magic.c's
    // `AlchemBag(&pObject)` relies on this to replace a Clonk with its attached
    // bag before ALC_::Activate calls Transfer on it.
    let globals = Script::compile("global func Rewrite(&value) { value = 7; }")
        .expect("global script compiles");
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            func Probe() {
                var value = 1;
                Rewrite(value);
                return value;
            }
            "#,
        )
        .expect("object script loads");
    engine.set_global_functions(Some(Arc::new(globals.functions().clone())));

    assert_eq!(engine.call("Probe", &[]).unwrap(), Value::Int(7));
}
