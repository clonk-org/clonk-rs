use lc_script::{Engine, Value};

fn evaluate(strict_level: u8) -> Value {
    let mut engine = Engine::new();
    engine
        .load_script(&format!(
            r#"#strict {strict_level}
func Probe()
{{
    var integer = 0;
    var boolean = false;
    return [0, false, integer, boolean, +0, -0, 1 - 1, 1 == 2];
}}
"#
        ))
        .expect("literal strictness probe compiles");
    engine.call("Probe", &[]).expect("probe executes")
}

#[test]
fn zero_literals_are_nil_only_below_strict_three() {
    let legacy = Value::Array(vec![
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Nil,
        Value::Int(0),
        Value::Int(0),
        Value::Bool(false),
    ]);
    assert_eq!(evaluate(1), legacy);
    assert_eq!(evaluate(2), legacy);

    assert_eq!(
        evaluate(3),
        Value::Array(vec![
            Value::Int(0),
            Value::Bool(false),
            Value::Int(0),
            Value::Bool(false),
            Value::Int(0),
            Value::Int(0),
            Value::Int(0),
            Value::Bool(false),
        ])
    );
}
