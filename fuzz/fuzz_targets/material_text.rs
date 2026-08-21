//! `Material.txt`, the material enumeration and `TexMap.txt`
//! (clonk-org/clonk-rs#963).
//!
//! These three share a corpus in practice — a material name list and a texture
//! map are both bare line-oriented files the loader distinguishes only by
//! filename — so fuzzing them together is what actually happens on disk.

#![no_main]

use clonk_resources::material::{MaterialEnumeration, MaterialLibrary};
use clonk_resources::texmap::TextureMap;
use libfuzzer_sys::fuzz_target;

const MAX_INPUT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let _ = MaterialLibrary::parse_bytes(data);
    if let Ok(enumeration) = MaterialEnumeration::parse(data) {
        // Every enumerated name is one line of the input, so the count cannot
        // exceed the line count.
        assert!(
            enumeration.names().len() <= data.iter().filter(|byte| **byte == b'\n').count() + 1,
            "more enumerated materials than input lines"
        );
    }
    let map = TextureMap::parse_bytes(data);
    // The texmap is indexed by a single byte; nothing may create more.
    assert!(
        (0..=u8::MAX)
            .filter(|index| map.entry(*index).is_some())
            .count()
            <= 256,
        "texture map exceeded its byte index space"
    );
    let _ = TextureMap::parse_flags_bytes(data);
});
