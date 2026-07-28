// Test for method slot assignments (obj->LocalN("key") = value)

#[test]
fn obj_localn_assignment() {
    let source = r#"func Test() { var obj; obj->LocalN("pNext") = GetValue(); }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn obj_local_assignment() {
    let source = r#"func Test() { var obj; obj->Local(0) = 42; }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn fbrg_pattern() {
    let source = r#"func Test() {
        var pNext, pLast;
        pNext->LocalN("pLast") = pLast;
        pLast->LocalN("pNext") = pNext;
    }"#;
    crate::support::assert_compiles(source);
}

#[test]
fn nested_obj_localn_assignment() {
    let source = r#"func Test() { GetObject()->LocalN("key") = value; }"#;
    crate::support::assert_compiles(source);
}
