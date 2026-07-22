use clonk_script::Script;

#[test]
fn test_game_call_ex_parses() {
    // Test that GameCallEx is recognized during parsing
    // This test should fail until we register GameCallEx as a host function
    let source = r#"
        func Initialize() {
            // GameCallEx should be recognized as a valid function call
            GameCallEx("OnClonkCreation", this());
            return 0;
        }
    "#;

    let result = Script::compile(source);
    // This will fail initially because GameCallEx is not yet registered
    assert!(
        result.is_ok(),
        "Should recognize GameCallEx: {:?}",
        result.err()
    );
}

#[test]
fn test_game_call_ex_with_multiple_params() {
    // Test that GameCallEx accepts multiple parameters
    let source = r#"
        func Test() {
            // Should accept function name plus up to 9 parameters like C++ version
            GameCallEx("SomeFunc", 1, 2, 3);
            GameCallEx("AnotherFunc", this(), 5, "test");
            return 1;
        }
    "#;

    let result = Script::compile(source);
    assert!(
        result.is_ok(),
        "Should handle GameCallEx with parameters: {:?}",
        result.err()
    );
}

#[test]
fn test_game_call_ex_minimal() {
    // Test minimal GameCallEx call with just function name
    let source = r#"
        func Test() {
            GameCallEx("TestFunction");
            return 0;
        }
    "#;

    let result = Script::compile(source);
    assert!(
        result.is_ok(),
        "Should handle minimal GameCallEx call: {:?}",
        result.err()
    );
}
