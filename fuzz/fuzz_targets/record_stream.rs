//! The classic league record-stream container (clonk-org/clonk-rs#966).
//!
//! These bytes arrive from a league server or from a peer's recording, and the
//! decoder is a zlib envelope wrapped around a chunk sequence whose lengths
//! are all attacker-controlled. C++ recovers from a malformed or interrupted
//! suffix rather than rejecting (`C4Playback::ReadBinary`), so the contract is
//! not "does it reject bad input": arbitrary bytes must produce a typed error
//! or a bounded value, never a panic and never unbounded expansion.
//!
//! The same contract runs without the fuzzing engine in
//! `crates/clonk-network/tests/record_stream_fuzz.rs`.

#![no_main]

use clonk_network::decode_classic_record_stream;
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 65_536;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }
    if let Ok(stream) = decode_classic_record_stream(data) {
        // The decoder's own bound is on the inflated buffer; what reaches the
        // caller is carved out of it, so nothing it hands back may exceed the
        // ceiling that bound allows for this input.
        let produced = stream.initial_group.len()
            + stream.control_record.len()
            + stream
                .files
                .iter()
                .map(|file| file.filename.as_bytes().len() + file.data.len())
                .sum::<usize>();
        assert!(
            produced <= clonk_network::CLASSIC_RECORD_STREAM_MAX_INFLATED,
            "{produced} bytes produced from {} compressed",
            data.len()
        );
    }
});
