// Test for method slot assignments (obj->LocalN("key") = value)

crate::support::compile_cases! {
    obj_localn_assignment: r#"func Test() { var obj; obj->LocalN("pNext") = GetValue(); }"#;
    obj_local_assignment: r#"func Test() { var obj; obj->Local(0) = 42; }"#;
    fbrg_pattern:
    r#"func Test() {
        var pNext, pLast;
        pNext->LocalN("pLast") = pLast;
        pLast->LocalN("pNext") = pNext;
    }"#;
    nested_obj_localn_assignment: r#"func Test() { GetObject()->LocalN("key") = value; }"#;
}
