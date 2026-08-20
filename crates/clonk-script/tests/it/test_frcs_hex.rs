// Test for FRCS hex literal fix

use clonk_script::Value;

crate::support::compile_cases! {
    frcs_hex_literal_with_missing_comma_space: r#"func Test() { CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff); }"#;
    frcs_full_context: r#"func Test() {
        Sound("MgWind*", false, obj, 50, 0, false, true, 300);
        CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff);
    }"#;
}

run_cases! {
    c4_integer_literal_edges_execute_end_to_end:
            r#"
            #strict 3
            func Probe() {
                return [0xffffffff, 4294967295, 0xa0c0ff, 0XFF, 1_AA, 12_A, 0x];
            }
            "#,
        "Probe", &[] =>
        Value::Array(vec![
            Value::Int(-1),
            Value::Int(-1),
            Value::Int(0xa0c0ff),
            Value::C4Id("0XFF".to_string()),
            Value::C4Id("1_AA".to_string()),
            Value::C4Id("12_A".to_string()),
            Value::Int(0),
        ]);
}
