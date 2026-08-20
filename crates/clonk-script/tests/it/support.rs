//! Helpers shared by the parser and runtime integration tests.

use clonk_script::{Engine, ScriptError, Value};

#[track_caller]
pub fn load_script(engine: &mut Engine, source: &str) {
    engine.load_script(source).expect("test script loads");
}

#[track_caller]
pub fn engine(source: &str) -> Engine {
    let mut engine = Engine::new();
    load_script(&mut engine, source);
    engine
}

#[track_caller]
pub fn call(engine: &Engine, function: &str, args: &[Value]) -> Value {
    engine.call(function, args).expect("test function runs")
}

#[track_caller]
pub fn run(source: &str, function: &str, args: &[Value]) -> Value {
    call(&engine(source), function, args)
}

#[track_caller]
pub fn eval(source: &str) -> Value {
    run(source, "Test", &[])
}

#[track_caller]
pub fn try_eval(source: &str, args: &[Value]) -> Result<Value, ScriptError> {
    engine(source).call("Test", args)
}

#[track_caller]
pub fn runtime_error(source: &str, args: &[Value]) -> String {
    match try_eval(source, args).expect_err("test function must fail") {
        ScriptError::Runtime(error) => error.message().to_string(),
        other => panic!("expected runtime error, got {other}"),
    }
}

macro_rules! eval_test {
    ($name:ident { $($source:expr => $expected:expr $(, $message:expr)?;)+ }) => {
        #[test]
        fn $name() {
            $(assert_eq!($crate::support::eval($source), $expected $(, $message)?);)+
        }
    };
}

macro_rules! eval_cases {
    ($($name:ident: $source:expr => $expected:expr $(, $message:expr)?;)+) => {
        $(eval_test!($name { $source => $expected $(, $message)?; });)+
    };
}

macro_rules! run_cases {
    ($($name:ident: $source:expr, $function:expr, $args:expr => $expected:expr $(, $message:expr)?;)+) => {
        $(
            #[test]
            fn $name() {
                assert_eq!($crate::support::run($source, $function, $args), $expected $(, $message)?);
            }
        )+
    };
}

/// Compile `source`, retaining the parser's location and message on failure.
#[track_caller]
pub fn assert_compiles(source: &str) {
    if let Err(error) = clonk_script::Script::compile(source) {
        panic!(
            "compile failed: line {}, col {}: {}",
            error.line(),
            error.column(),
            error.message()
        );
    }
}

macro_rules! compile_case {
    ($name:ident, $source:expr $(,)?) => {
        #[test]
        fn $name() {
            $crate::support::assert_compiles($source);
        }
    };
}

macro_rules! compile_cases {
    ($($name:ident: $source:expr;)+) => {
        $(compile_case!($name, $source);)+
    };
}

pub(crate) use {compile_case, compile_cases};

#[cfg(test)]
mod tests {
    use super::assert_compiles;

    /// `Script::compile` is error-recovering: it collects parse problems as
    /// diagnostics and still returns `Ok`, so the sites this helper replaced
    /// asserted only that parsing did not panic. Pin that, so tightening the
    /// helper into a diagnostics check cannot silently change the meaning of
    /// every caller at once.
    #[test]
    fn assert_compiles_keeps_the_recovering_parser_contract() {
        assert_compiles("func Test() { return 1 2 3 ) ) ) ; }");
    }
}
