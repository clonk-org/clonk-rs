// Test for empty argument support in function calls

// Exact pattern from WTOW line 116
crate::support::compile_cases! {
    wtow_line_116_pattern: r#"func Test() { SetColorDw(, pObj); }"#;

// func(, x)
    empty_first_arg: r#"func Test() { SomeFunc(, 42); }"#;

// func(x, , y)
    empty_middle_arg: r#"func Test() { SomeFunc(1, , 3); }"#;

// Contents(, , true)
    multiple_empty_args: r#"func Test() { Contents(, , true); }"#;

// func(, )
    all_empty_args: r#"func Test() { SomeFunc(, ); }"#;

// func(x, )
    trailing_empty_arg: r#"func Test() { SomeFunc(42, ); }"#;

// Regression test: func(x, y)
    normal_args_still_work: r#"func Test() { SomeFunc(1, 2, 3); }"#;

// Mix of empty and complex expressions
    empty_args_with_expressions: r#"func Test() { SomeFunc(, GetX() + 5, , "test"); }"#;

// Pattern from JungleClonk: Contents(, , true)
    jungle_clonk_pattern: r#"func Test() { return(DefinitionCall(GetID(Contents(, , true)), "IsSpear")); }"#;
}
