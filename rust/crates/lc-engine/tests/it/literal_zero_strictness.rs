use lc_engine::compat;
use lc_script::{Engine, Value};

fn evaluate(strict_level: u8, body: &str) -> Value {
    let mut engine = Engine::new();
    compat::register_host_functions(&mut engine);
    engine
        .load_script(&format!(
            "#strict {strict_level}\nfunc Probe() {{ {body} }}"
        ))
        .expect("strictness probe compiles");
    engine.call("Probe", &[]).expect("probe executes")
}

#[test]
fn get_type_of_zero_literals_matches_cpp_strictness_vectors() {
    let body = "var x = 0; var y = false; return [GetType(0), GetType(false), GetType(x), GetType(y)];";

    let legacy = Value::Array(vec![
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
