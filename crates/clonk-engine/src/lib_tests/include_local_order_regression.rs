use super::*;

#[test]
fn sibling_include_local_names_follow_cpp_push_front_order() {
    // Parse_Script push_front makes textual A,B resolve B then A;
    // AppendTo adds each parent's LocalNamed entries in declaration
    // order, and AddName retains the first slot for duplicates
    // (C4AulParse.cpp:1456; C4AulLink.cpp:84-94,145-157;
    // C4ValueMap.cpp:406-427).
    let mut engine = Engine::new();
    crate::TestValueExt::test_value(engine.register_definition(test_definition(
        "INCA",
        "A",
        "local a0, shared, a1;",
    )));
    crate::TestValueExt::test_value(engine.register_definition(test_definition(
        "INCB",
        "B",
        "local b0, shared, b1;",
    )));
    crate::TestValueExt::test_value(engine.register_definition(test_definition(
        "CHLD",
        "Child",
        "#include INCA\n#include INCB\nlocal c0, c1;",
    )));

    crate::TestValueExt::test_value(engine.resolve_includes());
    crate::TestValueExt::test_value(engine.resolve_includes());

    let names = crate::TestValueExt::test_value(engine.definitions.get("CHLD"))
        .script
        .local_variable_names()
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        ["c0", "c1", "b0", "shared", "b1", "a0", "a1"],
        "child locals precede last-declared include B, then include A"
    );
}
