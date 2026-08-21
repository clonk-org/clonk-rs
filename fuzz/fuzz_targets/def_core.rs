//! `DefCore.txt` alone (clonk-org/clonk-rs#963).
//!
//! Separated from the combined target so a campaign can concentrate on the
//! richest of these grammars: sections, repeated keys, StdCompiler numeric
//! prefixes and radices, boolean spellings, C4ID lists, vertex arrays and
//! native byte preservation past an interior NUL.

#![no_main]

use clonk_resources::definition::parse_def_core;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(core) = parse_def_core(data) {
        // C++ hands the parser a native C string, so nothing past an interior
        // NUL may survive into a field (C4DefCore::Compile).
        let readable = data.split(|byte| *byte == 0).next().unwrap_or_default();
        assert!(
            core.id.len() <= readable.len() + 64,
            "id outgrew the pre-NUL input"
        );
    }
});
