// Test for multi-variable declarations like: var a, b, c;

use clonk_script::Value;

eval_cases! {
    multi_variable_declaration_should_work: r#"
        global func Test() {
            var a, b, c;
            a = 10;
            b = 20;
            c = 30;
            return c;
        }
    "# => Value::Int(30);

    multi_variable_declaration_in_function_should_initialize_all_vars: r#"
        global func Test() {
            var iX, iY, iDir;
            iDir = 5;
            iX = iDir * 2;
            return iX;
        }
    "# => Value::Int(10);

    multi_variable_declaration_with_init_should_work: r#"
        global func Test() {
            var a = 1, b = 2, c;
            c = a + b;
            return c;
        }
    "# => Value::Int(3);
}
