//! RTF description bodies (clonk-org/clonk-rs#963).
//!
//! The only parser in this family that returns owned text, and so the only one
//! that can amplify: control words, nested groups and hex escapes all expand.
//! Deep group nesting is also the natural stack-overflow shape here.

#![no_main]

use clonk_resources::rtf::rtf_to_plain_text;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let plain = rtf_to_plain_text(data);
    assert!(
        plain.len() <= data.len() + 64,
        "rtf_to_plain_text produced {} bytes from {}",
        plain.len(),
        data.len()
    );
});
