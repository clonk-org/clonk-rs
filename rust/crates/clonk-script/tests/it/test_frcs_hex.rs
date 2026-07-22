// Test for FRCS hex literal fix

use clonk_script::{Engine, Value};

#[test]
fn frcs_hex_literal_with_missing_comma_space() {
    let source = r#"func Test() { CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff); }"#;
    assert!(clonk_script::Script::compile(source).is_ok());
}

#[test]
fn frcs_full_context() {
    let source = r#"func Test() {
        Sound("MgWind*", false, obj, 50, 0, false, true, 300);
        CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff);
    }"#;
    assert!(clonk_script::Script::compile(source).is_ok());
}

#[test]
fn c4_integer_literal_edges_execute_end_to_end() {
    let mut engine = Engine::new();
    engine
        .load_script(
            r#"
            #strict 3
            func Probe() {
                return [0xffffffff, 4294967295, 0xa0c0ff, 0XFF, 1_AA, 12_A, 0x];
            }
            "#,
        )
        .expect("integer-edge probe compiles");

    assert_eq!(
        engine.call("Probe", &[]).expect("integer-edge probe runs"),
        Value::Array(vec![
            Value::Int(-1),
            Value::Int(-1),
            Value::Int(0xa0c0ff),
            Value::C4Id("0XFF".to_string()),
            Value::C4Id("1_AA".to_string()),
            Value::C4Id("12_A".to_string()),
            Value::Int(0),
        ])
    );
}
