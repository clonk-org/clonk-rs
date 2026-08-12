use clonk_engine::compat;
use clonk_script::{Engine, Value};

fn evaluate(strict_level: u8, body: &str) -> Value {
    let mut engine = Engine::new();
    compat::register_host_functions(&mut engine);
    let strict = if strict_level == 1 {
        "#strict".to_owned()
    } else {
        format!("#strict {strict_level}")
    };
    crate::support::TestValueExt::test_value(
        engine.load_script(&format!("{strict}\nfunc Probe() {{ {body} }}")),
    );
    crate::support::TestValueExt::test_value(engine.call("Probe", &[]))
}

#[test]
fn get_type_of_falsy_values_matches_cpp_strictness_vectors() {
    // The first four values are already nil below strict 3 because of C++
    // literal handling. The computed Int(0)/Bool(false) pair stays concrete
    // until FnGetType applies its caller-strictness rule.
    let body = "var x = 0; var y = false; var unset; return [GetType(0), GetType(false), GetType(x), GetType(y), GetType(1 - 1), GetType(1 == 2), GetType(unset)];";

    let legacy = Value::Array(vec![
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
        Value::Int(0),
    ]);
    assert_eq!(evaluate(1, body), legacy);
    assert_eq!(evaluate(2, body), legacy);
    assert_eq!(
        evaluate(3, body),
        Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(2),
            Value::Int(1),
            Value::Int(2),
            Value::Int(0),
        ])
    );
}

#[test]
fn legacy_format_receives_nil_for_zero_literals() {
    assert_eq!(
        evaluate(1, r#"return [Format("%v", 0), Format("%v", false)];"#),
        Value::Array(vec![Value::String("0".into()), Value::String("0".into())])
    );
}
