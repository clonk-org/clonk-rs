//! Bounded malformed-input campaign over the legacy network decoders
//! (clonk-org/clonk-rs#960).
//!
//! These bytes arrive from peers *before* the session can decide whether to
//! trust the peer at all, so the decoders in `clonk-network::legacy` are the
//! first thing a hostile or broken client reaches. A panic there takes down a
//! host; an unbounded allocation does the same more slowly.
//!
//! The contract is therefore three things, not one:
//!
//! * arbitrary, truncated, concatenated and oversized bytes never panic — every
//!   rejection is a typed `LegacyControlError`;
//! * decoded work stays in proportion to the input. A control list is a
//!   sequence of at-least-one-byte entries terminated by `PID_None`, so a
//!   frame can never hold more controls than the payload has bytes. That is the
//!   invariant a forged count or length field would break;
//! * the prefix decoders never claim to have consumed more than they were
//!   given, because their callers (`CtrlRec.c4b` chunk walking) advance by that
//!   number and would read out of the buffer or loop forever.
//!
//! Decoding is also asserted to be deterministic: the same bytes must produce
//! the same outcome twice, since a lockstep peer depends on every client
//! reading a packet identically.
//!
//! This runs in the ordinary suite so the contract holds on every change
//! without the fuzzing engine; `fuzz/fuzz_targets/legacy_network.rs` carries the
//! libFuzzer target for long campaigns.

use clonk_network::{
    decode_control_entry_payload, decode_control_entry_prefix, decode_control_list_prefix,
    decode_control_payload, decode_init_scenario_player_control_entry_payload,
    decode_join_data_envelope, decode_join_game_parameters_envelope,
    decode_player_info_update_payload,
};

/// Inputs are capped so a case measures the decoders rather than the mutator.
const MAX_INPUT: usize = 4096;

/// Verbatim C++ serializer output (see
/// `tests/fixtures/cpp_control_goldens/README.md`). Seeding from these rather
/// than from Rust-encoded bytes keeps the campaign anchored to shapes the real
/// wire produces, including their delivery-byte prefix.
const CPP_GOLDENS: &[(&str, &[u8])] = &[
    (
        "synchronize",
        include_bytes!("fixtures/cpp_control_goldens/synchronize_delivery_1.bin"),
    ),
    (
        "client_update_activate",
        include_bytes!("fixtures/cpp_control_goldens/client_update_activate_delivery_1.bin"),
    ),
    (
        "client_join",
        include_bytes!("fixtures/cpp_control_goldens/client_join_delivery_2.bin"),
    ),
    (
        "client_remove",
        include_bytes!("fixtures/cpp_control_goldens/client_remove_delivery_1.bin"),
    ),
    (
        "player_info_minimal",
        include_bytes!("fixtures/cpp_control_goldens/player_info_minimal_delivery_2.bin"),
    ),
    (
        "player_info_update_request_add",
        include_bytes!("fixtures/cpp_control_goldens/player_info_update_request_add.bin"),
    ),
    (
        "join_player_embedded",
        include_bytes!("fixtures/cpp_control_goldens/join_player_embedded_client_4_tick_64.bin"),
    ),
    (
        "join_player_resource",
        include_bytes!("fixtures/cpp_control_goldens/join_player_resource_client_4_tick_7.bin"),
    ),
    (
        "join_player_resource_sha",
        include_bytes!("fixtures/cpp_control_goldens/join_player_resource_sha_client_4_tick_7.bin"),
    ),
    (
        "player_info_minimal_frame",
        include_bytes!("fixtures/cpp_control_goldens/player_info_minimal_client_4_tick_7.bin"),
    ),
    (
        "player_info_resource_frame",
        include_bytes!("fixtures/cpp_control_goldens/player_info_resource_client_4_tick_7.bin"),
    ),
];

/// Every decoder is driven with the same bytes: which entry point a peer
/// reaches is chosen by the packet id the transport already read, and a hostile
/// peer picks that id, so any payload can arrive at any of them.
fn drive_all(input: &[u8], what: &str) -> bool {
    let mut decoded_anything = false;

    if decode_join_data_envelope(input).is_ok() {
        decoded_anything = true;
    }
    if decode_join_game_parameters_envelope(input).is_ok() {
        decoded_anything = true;
    }
    if decode_init_scenario_player_control_entry_payload(input).is_ok() {
        decoded_anything = true;
    }
    if decode_player_info_update_payload(input).is_ok() {
        decoded_anything = true;
    }
    if decode_control_entry_payload(input).is_ok() {
        decoded_anything = true;
    }

    if let Ok(frame) = decode_control_payload(input) {
        assert!(
            frame.controls.len() <= input.len(),
            "{what}: {} controls decoded from {} bytes",
            frame.controls.len(),
            input.len()
        );
        decoded_anything = true;
    }

    if let Ok((control, consumed)) = decode_control_entry_prefix(input) {
        assert!(
            consumed <= input.len(),
            "{what}: entry prefix reports {consumed} consumed of {}",
            input.len()
        );
        // A caller that advances by a zero-length entry never terminates.
        assert!(
            consumed > 0,
            "{what}: entry prefix consumed nothing while decoding {control:?}"
        );
        decoded_anything = true;
    }

    if let Ok((controls, consumed)) = decode_control_list_prefix(input) {
        assert!(
            consumed <= input.len(),
            "{what}: list prefix reports {consumed} consumed of {}",
            input.len()
        );
        // The terminating PID_None is always consumed, so a successful list is
        // never zero-length in bytes even when it holds no controls.
        assert!(consumed > 0, "{what}: list prefix consumed nothing");
        assert!(
            controls.len() < consumed,
            "{what}: {} controls from {consumed} consumed bytes",
            controls.len()
        );
        decoded_anything = true;
    }

    decoded_anything
}

/// The same bytes must decode the same way twice; lockstep depends on it.
fn assert_deterministic(input: &[u8], what: &str) {
    assert_eq!(
        decode_control_payload(input).is_ok(),
        decode_control_payload(input).is_ok(),
        "{what}: control payload decode is not deterministic"
    );
    assert_eq!(
        decode_control_list_prefix(input)
            .map(|(controls, consumed)| (controls.len(), consumed))
            .ok(),
        decode_control_list_prefix(input)
            .map(|(controls, consumed)| (controls.len(), consumed))
            .ok(),
        "{what}: control list decode is not deterministic"
    );
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        if bound == 0 {
            0
        } else {
            (self.next() % bound as u64) as usize
        }
    }
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..=rng.below(6) {
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes[at] = (rng.next() & 0xff) as u8;
            }
            1 if bytes.len() >= 4 => {
                // Counts and lengths are 32-bit; the interesting values are the
                // boundaries, which single bit flips rarely reach.
                let at = rng.below(bytes.len() - 3);
                let value = match rng.below(5) {
                    0 => 0u32,
                    1 => u32::MAX,
                    2 => 0x7fff_ffff,
                    3 => 0x8000_0000,
                    _ => (rng.next() & 0xffff_ffff) as u32,
                };
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            2 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            3 => {
                let at = rng.below(bytes.len() + 1);
                bytes.insert(at, (rng.next() & 0xff) as u8);
            }
            4 if !bytes.is_empty() => {
                // Concatenation: a suffix-tolerant decoder and a prefix decoder
                // must disagree about these in a defined way.
                let span = bytes.clone();
                bytes.extend_from_slice(&span);
            }
            _ if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(32);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            _ => {}
        }
        if bytes.len() > MAX_INPUT {
            bytes.truncate(MAX_INPUT);
        }
    }
    bytes
}

#[test]
fn every_cpp_golden_decodes_or_reports_a_typed_error() {
    for (name, bytes) in CPP_GOLDENS {
        assert!(
            bytes.len() <= MAX_INPUT,
            "{name} is {} bytes, above the campaign cap",
            bytes.len()
        );
        // The delivery byte belongs to the transport, so both framings are fed:
        // with it (what a confused caller passes) and without (what the real
        // entry points receive).
        drive_all(bytes, name);
        drive_all(&bytes[1..], name);
        assert_deterministic(bytes, name);
    }
}

/// The `fuzz/corpus/legacy_network/` seeds the libFuzzer target starts from.
///
/// This does not re-decode them — the campaign above already drives the same
/// bytes through every entry point. What it checks is that the corpus still
/// exists and stays inside the target's input cap, so a long campaign starts
/// from real wire shapes instead of from nothing.
#[test]
fn the_shipped_network_corpus_is_present_and_within_the_input_cap() {
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/legacy_network");
    let entries = std::fs::read_dir(&corpus).unwrap_or_else(|error| {
        panic!(
            "network corpus at {} is readable: {error}",
            corpus.display()
        )
    });
    let mut seen = 0;
    for entry in entries {
        let path = entry.expect("corpus entry").path();
        if !path.is_file() {
            continue;
        }
        let len = std::fs::metadata(&path).expect("corpus entry stats").len() as usize;
        assert!(
            len <= MAX_INPUT,
            "{} is {len} bytes, past the target's {MAX_INPUT}-byte cap, so the \
             target would skip it",
            path.display()
        );
        seen += 1;
    }
    assert_eq!(
        seen,
        CPP_GOLDENS.len(),
        "the network corpus and the seeds this campaign drives have diverged"
    );
}

#[test]
fn arbitrary_bytes_never_panic_or_decode_unbounded_control() {
    let mut rng = Rng(0x5EED_0960);
    for round in 0..4_000 {
        let len = rng.below(MAX_INPUT);
        let bytes = (0..len)
            .map(|_| (rng.next() & 0xff) as u8)
            .collect::<Vec<_>>();
        let what = format!("arbitrary round {round}");
        drive_all(&bytes, &what);
        assert_deterministic(&bytes, &what);
    }
}

#[test]
fn mutated_cpp_goldens_never_panic_or_decode_unbounded_control() {
    let mut rng = Rng(0x0960_5EED);
    let mut reached = 0_usize;
    for round in 0..6_000 {
        let (name, seed) = CPP_GOLDENS[rng.below(CPP_GOLDENS.len())];
        let bytes = mutate(seed, &mut rng);
        let what = format!("{name} mutation round {round}");
        if drive_all(&bytes, &what) {
            reached += 1;
        }
        assert_deterministic(&bytes, &what);
    }
    // A campaign that rejects everything at the first byte proves nothing, so
    // record that the mutations genuinely reach the decoders.
    assert!(
        reached > 0,
        "no mutated golden decoded; the campaign never left the entry check"
    );
}

#[test]
fn truncation_at_every_offset_is_safe() {
    for (name, bytes) in CPP_GOLDENS {
        for end in 0..bytes.len() {
            let what = format!("{name} truncated to {end}");
            drive_all(&bytes[..end], &what);
        }
    }
}

#[test]
fn a_control_list_never_decodes_more_entries_than_it_has_bytes() {
    // The shape a forged count would break: every entry costs at least its
    // one-byte id, and the list costs one more for its PID_None terminator.
    let mut rng = Rng(0xC0FF_EE60);
    for round in 0..2_000 {
        let len = rng.below(256);
        let bytes = (0..len)
            .map(|_| (rng.next() & 0xff) as u8)
            .collect::<Vec<_>>();
        if let Ok((controls, consumed)) = decode_control_list_prefix(&bytes) {
            assert!(
                controls.len() < consumed && consumed <= bytes.len(),
                "round {round}: {} controls, {consumed} consumed, {} bytes",
                controls.len(),
                bytes.len()
            );
        }
    }
}
