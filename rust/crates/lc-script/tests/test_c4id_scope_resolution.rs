use lc_script::Script;

#[test]
fn test_c4id_in_scope_resolution() {
    // Test that C4IDs can be used in scope resolution syntax: obj->DEFN::Method()
    let source = r#"
        #strict
        func TestScopeResolution() {
            var obj = this;
            // C4ID used in scope resolution
            obj->CLNK::SomeMethod();
            return nil;
        }
    "#;

    let result = Script::compile(source);
    assert!(result.is_ok(), "Should parse C4ID in scope resolution: {:?}", result.err());
}

#[test]
fn test_c4id_after_arrow_operator() {
    // Test that C4IDs can appear after -> operator
    let source = r#"
        #strict
        func TestArrow() {
            var pAmmo = this;
            // C4ID immediately after ->
            pAmmo->OBRL();
            return nil;
        }
    "#;

    let result = Script::compile(source);
    assert!(result.is_ok(), "Should parse C4ID after -> operator: {:?}", result.err());
}

#[test]
fn test_complex_c4id_scope_resolution() {
    // Test the actual pattern from ACT2 script
    let source = r#"
        #strict
        func TestComplexPattern() {
            var pAmmo = this;
            // Complex pattern: method call with C4ID scope resolution
            pAmmo->OBRL::BarrelDoFill(-pAmmo->OBRL::GetAmount());
            return nil;
        }
    "#;

    let result = Script::compile(source);
    assert!(result.is_ok(), "Should parse complex C4ID scope resolution: {:?}", result.err());
}
