//! Packed C4Group archives through the production in-memory reader
//! (clonk-org/clonk-rs#959).
//!
//! Scenario, definition, player, save, replay and update content all enter
//! through a group container, and a player join carries one as raw bytes over
//! the wire (`C4ControlJoinPlayer`, C4Control.cpp:731-744), so this reader sees
//! attacker-shaped input before any higher-level parser gets a say.
//!
//! The amplification here is numeric: a 204-byte header names an entry count
//! and each 316-byte record names a payload size and may itself be a nested
//! group. Those are numbers that multiply, so the target asserts proportion —
//! entries against the bytes that must carry them, extracted payload against
//! the image, nesting against an explicit depth bound — rather than only
//! absence of panics.
//!
//! `Group::from_memory` is the entry point that sniffs the gzip envelope and
//! parses the table, which is what the join path uses.

#![no_main]

use std::path::PathBuf;

use clonk_resources::Group;
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 16 * 1024;

/// `GROUP_HEADER_SIZE` / `GROUP_ENTRY_SIZE` in `group.rs`: the reader's own
/// record sizes, not limits invented here.
const GROUP_HEADER_SIZE: usize = 204;
const GROUP_ENTRY_SIZE: usize = 316;

/// A child group is an entry whose payload is another group image, so nesting
/// costs at least a header plus a record per level. Anything deeper than the
/// input can encode means the walk is not consuming input.
const MAX_DEPTH: usize = 64;

/// `C4GROUP_GZ_MAGIC` / `GZ_MAGIC` in `group.rs`.
const GZ_MAGICS: [[u8; 2]; 2] = [[0x1E, 0x8C], [0x1F, 0x8B]];

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    let Ok(group) = Group::from_memory(PathBuf::from("fuzz.c4g"), data.to_vec()) else {
        return;
    };
    let Ok(entries) = group.entries() else {
        return;
    };

    // A gzip envelope expands, so the parsed image is no longer the input and
    // the proportion bounds below would be measuring the wrong number. Those
    // inputs still exercise the reader for panics; the decompression ratio is
    // the gz layer's own contract, not this reader's.
    let wrapped = GZ_MAGICS.iter().any(|magic| data.starts_with(magic));
    if !wrapped {
        let image = data.len();
        assert!(
            entries.len() <= image.saturating_sub(GROUP_HEADER_SIZE) / GROUP_ENTRY_SIZE + 1,
            "{} entries from a {image}-byte image",
            entries.len()
        );

        let mut extracted = 0usize;
        for entry in &entries {
            if let Ok(bytes) = group.read_entry_bytes_exact(entry) {
                extracted += bytes.len();
            }
            assert!(
                extracted <= image,
                "extracted {extracted} bytes from a {image}-byte image"
            );
        }
    } else {
        for entry in &entries {
            let _ = group.read_entry_bytes_exact(entry);
        }
    }

    // Nested children: every level must consume input, so the walk terminates.
    let mut level = vec![group];
    for depth in 0..MAX_DEPTH {
        let mut next = Vec::new();
        for parent in &level {
            let Ok(children) = parent.entries() else {
                continue;
            };
            for entry in &children {
                if let Ok(child) = parent.open_child_entry_exact(entry) {
                    next.push(child);
                }
            }
        }
        if next.is_empty() {
            return;
        }
        assert!(depth + 1 < MAX_DEPTH, "child nesting failed to terminate");
        level = next;
    }
});
