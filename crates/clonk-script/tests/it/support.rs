//! Helpers shared by the compile-only parser tests.

/// Compile `source` and assert it parsed, reporting the parser's own
/// line/column/message when it did not.
///
/// Every parser test spelled this out inline as a `compile` binding, an
/// `eprintln!` of the error fields and a bare `assert!(result.is_ok())`.
/// Raising the same fields as the panic message keeps the diagnosis without
/// needing captured stdout to read it.
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
