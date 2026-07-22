// Test for method slot assignments (obj->LocalN("key") = value)

#[test]
fn obj_localn_assignment() {
    let source = r#"func Test() { var obj; obj->LocalN("pNext") = GetValue(); }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn obj_local_assignment() {
    let source = r#"func Test() { var obj; obj->Local(0) = 42; }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn fbrg_pattern() {
    let source = r#"func Test() {
        var pNext, pLast;
        pNext->LocalN("pLast") = pLast;
        pLast->LocalN("pNext") = pNext;
    }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}

#[test]
fn nested_obj_localn_assignment() {
    let source = r#"func Test() { GetObject()->LocalN("key") = value; }"#;
    let result = clonk_script::Script::compile(source);
    if let Err(e) = &result {
        eprintln!(
            "Error: line {}, col {}: {}",
            e.line(),
            e.column(),
            e.message()
        );
    }
    assert!(result.is_ok());
}
