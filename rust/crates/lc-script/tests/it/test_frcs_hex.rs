// Test for FRCS hex literal fix

#[test]
fn frcs_hex_literal_with_missing_comma_space() {
    let source = r#"func Test() { CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff); }"#;
    assert!(lc_script::Script::compile(source).is_ok());
}

#[test]
fn frcs_full_context() {
    let source = r#"func Test() {
        Sound("MgWind*", false, obj, 50, 0, false, true, 300);
        CreateParticle("NoGravSpark", dx, dy, dvx,dvy, 30, 0xa0c0ff);
    }"#;
    assert!(lc_script::Script::compile(source).is_ok());
}
