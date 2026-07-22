// Test for multi-variable declarations like: var a, b, c;

use clonk_script::{Engine, Script, Value};

#[test]
fn multi_variable_declaration_should_work() {
    let source = r#"
        global func Test() {
            var a, b, c;
            a = 10;
            b = 20;
            c = 30;
            return c;
        }
    "#;

    let script = Script::compile(source).expect("should parse");

    let mut engine = Engine::new();
    engine.add_script(script);

    let result = engine.call("Test", &[]).expect("should execute");
    assert_eq!(result, Value::Int(30));
}

#[test]
fn multi_variable_declaration_in_function_should_initialize_all_vars() {
    let source = r#"
        global func Test() {
            var iX, iY, iDir;
            iDir = 5;
            iX = iDir * 2;
            return iX;
        }
    "#;

    let script = Script::compile(source).expect("should parse");

    let mut engine = Engine::new();
    engine.add_script(script);

    let result = engine.call("Test", &[]).expect("should execute");
    assert_eq!(result, Value::Int(10));
}

#[test]
fn multi_variable_declaration_with_init_should_work() {
    let source = r#"
        global func Test() {
            var a = 1, b = 2, c;
            c = a + b;
            return c;
        }
    "#;

    let script = Script::compile(source).expect("should parse");

    let mut engine = Engine::new();
    engine.add_script(script);

    let result = engine.call("Test", &[]).expect("should execute");
    assert_eq!(result, Value::Int(3));
}
