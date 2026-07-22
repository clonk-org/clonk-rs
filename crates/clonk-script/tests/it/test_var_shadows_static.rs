// M_Mov_Intro.c:16 declares `var g_pIntroHorse` inside a function while a
// file-level `static g_pIntroHorse;` exists — CR compiles and runs it (the
// var shadows the static for the function body).

#[test]
fn var_shadowing_a_static_compiles_and_runs() {
    let source = r#"#strict
static g_pTest;
func Trigger() {
    var g_pTest = 5;
    return(g_pTest);
}
"#;
    let mut engine = clonk_script::Engine::new();
    engine.load_script(source).expect("shadowing loads");
    let result = engine.call("Trigger", &[]).expect("runs");
    assert_eq!(result, clonk_script::Value::Int(5));
}
