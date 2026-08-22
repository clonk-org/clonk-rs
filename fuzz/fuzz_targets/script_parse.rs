//! The C4Script front end (clonk-org/clonk-rs#962).
//!
//! Scripts come from scenarios and definition packs, and `DirectExec` also
//! accepts developer and message-board text. The parser deliberately *recovers*
//! from many C++ syntax errors rather than rejecting, so the contract is richer
//! than "does it reject bad input": arbitrary bytes must not panic, must not
//! recurse into a stack overflow, and must not produce diagnostics out of
//! proportion to the input.
//!
//! Every strictness level is reached, because the grammar genuinely differs
//! between them — `nil`, array literals, map literals and dot access are each
//! admitted at a different level.

#![no_main]

use clonk_script::Script;
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 4096;

const STRICTNESS: [&str; 4] = ["", "#strict\n", "#strict 2\n", "#strict 3\n"];

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    // The lexer takes &str; a lossy decode is what a native-encoded script looks
    // like after the group layer has read it.
    let body = String::from_utf8_lossy(data);

    for level in STRICTNESS {
        let source = format!("{level}{body}");
        let ceiling = 4 * (source.len() + 16);
        let mut diagnostics = 0;
        for compile in [
            Script::compile,
            Script::compile_global,
            Script::compile_c4_string,
            Script::compile_global_c4_string,
        ] {
            if let Ok(script) = compile(&source) {
                diagnostics += script.parse_diagnostics().len();
            }
        }
        assert!(
            diagnostics <= ceiling,
            "{diagnostics} diagnostics from {} bytes",
            source.len()
        );
    }
});
