// M_Mov_Intro.c:16 declares `var g_pIntroHorse` inside a function while a
// file-level `static g_pIntroHorse;` exists — CR compiles and runs it (the
// var shadows the static for the function body).

run_cases! {
    var_shadowing_a_static_compiles_and_runs: r#"#strict
static g_pTest;
func Trigger() {
    var g_pTest = 5;
    return(g_pTest);
}
"#, "Trigger", &[] => clonk_script::Value::Int(5);
}
