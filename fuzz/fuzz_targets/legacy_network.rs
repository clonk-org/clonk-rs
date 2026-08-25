//! The legacy network decoders (clonk-org/clonk-rs#960).
//!
//! These bytes arrive from peers *before* the session can decide whether to
//! trust the peer, so this is the first code a hostile or broken client
//! reaches. A panic here takes down a host; an unbounded allocation does the
//! same more slowly.
//!
//! Every decoder is driven with the same input because the entry point a peer
//! reaches is selected by the packet id the transport already read, and a
//! hostile peer picks that id — so any payload can arrive at any of them.
//!
//! The bounds asserted here are the ones a forged count or length field would
//! break, and they mirror `crates/clonk-network/tests/legacy_network_fuzz.rs`
//! so a finding here reproduces in the ordinary suite.

#![no_main]

use clonk_network::{
    decode_control_entry_payload, decode_control_entry_prefix, decode_control_list_prefix,
    decode_control_payload, decode_init_scenario_player_control_entry_payload,
    decode_join_data_envelope, decode_join_game_parameters_envelope,
    decode_player_info_update_payload,
};
use libfuzzer_sys::fuzz_target;

/// Matches the ordinary suite's cap, so a finding here reproduces there.
const MAX_INPUT: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.len() > MAX_INPUT {
        return;
    }

    let _ = decode_join_data_envelope(data);
    let _ = decode_join_game_parameters_envelope(data);
    let _ = decode_init_scenario_player_control_entry_payload(data);
    let _ = decode_player_info_update_payload(data);
    let _ = decode_control_entry_payload(data);

    if let Ok(frame) = decode_control_payload(data) {
        // A control costs at least its one-byte id, so a frame can never hold
        // more of them than the payload has bytes.
        assert!(
            frame.controls.len() <= data.len(),
            "{} controls decoded from {} bytes",
            frame.controls.len(),
            data.len()
        );
    }

    if let Ok((_, consumed)) = decode_control_entry_prefix(data) {
        // Callers advance by this number; past the end reads out of the buffer,
        // and zero never terminates.
        assert!(
            consumed > 0 && consumed <= data.len(),
            "entry prefix consumed {consumed} of {}",
            data.len()
        );
    }

    if let Ok((controls, consumed)) = decode_control_list_prefix(data) {
        assert!(
            consumed > 0 && consumed <= data.len(),
            "list prefix consumed {consumed} of {}",
            data.len()
        );
        // The terminating PID_None costs a byte of its own.
        assert!(
            controls.len() < consumed,
            "{} controls from {consumed} consumed bytes",
            controls.len()
        );
    }
});
