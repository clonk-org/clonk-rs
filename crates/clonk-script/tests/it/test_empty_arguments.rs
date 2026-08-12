// Test for empty argument support in function calls

// Exact pattern from WTOW line 116
crate::support::compile_case!(
    wtow_line_116_pattern,
    r#"func Test() { SetColorDw(, pObj); }"#
);

// func(, x)
crate::support::compile_case!(empty_first_arg, r#"func Test() { SomeFunc(, 42); }"#);

// func(x, , y)
crate::support::compile_case!(empty_middle_arg, r#"func Test() { SomeFunc(1, , 3); }"#);

// Contents(, , true)
crate::support::compile_case!(
    multiple_empty_args,
    r#"func Test() { Contents(, , true); }"#
);

// func(, )
crate::support::compile_case!(all_empty_args, r#"func Test() { SomeFunc(, ); }"#);

// func(x, )
crate::support::compile_case!(trailing_empty_arg, r#"func Test() { SomeFunc(42, ); }"#);

// Regression test: func(x, y)
crate::support::compile_case!(
    normal_args_still_work,
    r#"func Test() { SomeFunc(1, 2, 3); }"#
);

// Mix of empty and complex expressions
crate::support::compile_case!(
    empty_args_with_expressions,
    r#"func Test() { SomeFunc(, GetX() + 5, , "test"); }"#
);

// Pattern from JungleClonk: Contents(, , true)
crate::support::compile_case!(
    jungle_clonk_pattern,
    r#"func Test() { return(DefinitionCall(GetID(Contents(, , true)), "IsSpear")); }"#
);
