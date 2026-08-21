//! Every legacy resource-text parser over the same bytes
//! (clonk-org/clonk-rs#963).
//!
//! Running them together is deliberate: these grammars are permissive enough
//! that one format's bytes routinely reach another's parser in the wild — a
//! `Names.txt` handed to the material loader, a truncated `DefCore.txt` read as
//! a texture map — and the contract is that every one of them answers with a
//! typed error or a bounded value rather than panicking.

#![no_main]

use clonk_resources::definition::parse_def_core;
use clonk_resources::material::{MaterialEnumeration, MaterialLibrary};
use clonk_resources::rtf::rtf_to_plain_text;
use clonk_resources::texmap::TextureMap;
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's bound, so a finding here reproduces there.
const MAX_INPUT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    let _ = parse_def_core(data);
    let _ = MaterialLibrary::parse_bytes(data);
    let _ = MaterialEnumeration::parse(data);
    let _ = TextureMap::parse_bytes(data);
    let _ = TextureMap::parse_flags_bytes(data);

    // The only parser that returns owned text, and so the only one that can
    // amplify. The ordinary suite pins the ratio; here it just must not hang.
    let plain = rtf_to_plain_text(data);
    assert!(
        plain.len() <= data.len() + 64,
        "rtf_to_plain_text produced {} bytes from {}",
        plain.len(),
        data.len()
    );
});
