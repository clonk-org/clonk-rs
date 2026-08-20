// Test for COWB script argument count flexibility issue
// Action callbacks may pass extra arguments that scripts ignore
// Runtime behavior tested via integration test

// Throwing function from COWB that expects 1 arg
// Action system will call it with 2 args at runtime
crate::support::compile_cases! {
    cowb_throwing_pattern:
    r#"
private func Throwing(pObj) {
    if(!pObj) return(0);
    return(1);
}
    "#;

    // Simple function with one parameter
    // Should compile successfully
    function_with_one_param: r#"
func Test(pObj) {
    return(pObj);
}
    "#;

    // Function with no parameters
    // Should compile successfully
    function_with_no_params: r#"
func Test() {
    return(42);
}
    "#;
}
