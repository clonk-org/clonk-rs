// Test for COWB script argument count flexibility issue
// Action callbacks may pass extra arguments that scripts ignore
// Runtime behavior tested via integration test

#[test]
fn cowb_throwing_pattern() {
    // Throwing function from COWB that expects 1 arg
    // Action system will call it with 2 args at runtime
    let source = r#"
private func Throwing(pObj) {
    if(!pObj) return(0);
    return(1);
}
    "#;
    crate::support::assert_compiles(source);
}

#[test]
fn function_with_one_param() {
    // Simple function with one parameter
    // Should compile successfully
    let source = r#"
func Test(pObj) {
    return(pObj);
}
    "#;
    let result = clonk_script::Script::compile(source);
    assert!(result.is_ok());
}

#[test]
fn function_with_no_params() {
    // Function with no parameters
    // Should compile successfully
    let source = r#"
func Test() {
    return(42);
}
    "#;
    let result = clonk_script::Script::compile(source);
    assert!(result.is_ok());
}
