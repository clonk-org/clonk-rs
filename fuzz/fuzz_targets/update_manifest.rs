//! The update manifest and the plan built from it (clonk-org/clonk-rs#965).
//!
//! A manifest is fetched over the network and authorizes replacing the
//! executables of an installed build, so it is parsed and planned as if it
//! were attacker-supplied. The contract is that arbitrary bytes produce a
//! typed error or a bounded value, and that planning stays total over whatever
//! parsed — including target triples no publisher would emit.
//!
//! The archive half of #965 is not driven from here: extraction writes to the
//! filesystem, and a fuzzing engine that leaves a tree behind per input is its
//! own problem. It runs against a disposable root in the ordinary suite
//! instead — see `crates/clonk-update/tests/update_fuzz.rs`, which also
//! enforces this target's contract without the fuzzing engine.

#![no_main]

use clonk_update::{decide_for_this_build, InstalledState, Manifest};
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there. The
/// transport caps a manifest at `MANIFEST_MAX_BYTES` before it is parsed, so
/// an unbounded input is not a shape this code ever meets.
const MAX_INPUT: usize = 32_768;

const TRIPLES: [&str; 4] = [
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "",
    "\u{202e}reversed",
];

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    let Ok(manifest) = Manifest::parse(data) else {
        return;
    };
    for triple in TRIPLES {
        let _ = decide_for_this_build(&manifest, &None, triple);
        let _ = decide_for_this_build(&manifest, &Some(InstalledState::default()), triple);
    }
});
