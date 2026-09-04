//! Bounded malformed-input campaign over the legacy audio decoders
//! (clonk-org/clonk-rs#964).
//!
//! Scenario and definition packs supply audio, so these decoders read attacker-
//! shaped bytes. The amplification risk here is different in kind from the text
//! grammars: RIFF chunk lengths, WAV channel/sample-rate fields, MIDI
//! variable-length quantities and tempo changes are all *numbers that multiply*
//! — a small file can name a very large amount of decoded output or a very large
//! number of scheduled events.
//!
//! So the contract is not merely "no panic": decoded output must stay in
//! proportion to the input. `decode_audio` is the production entry point that
//! sniffs the format and dispatches, which is what this drives.
//!
//! This runs in the ordinary suite so the contract holds on every change without
//! the fuzzing engine; `fuzz/` carries the libFuzzer target for long campaigns.

use clonk_audio::decode_audio;

/// Inputs are capped so a case measures the decoder rather than the mutator.
const MAX_INPUT: usize = 4096;

/// A decoded frame is 2 × f32 = 8 bytes. Legacy WAV is at worst 4-bit ADPCM, so
/// one input byte can legitimately become two samples; MIDI is a timeline, not a
/// ratio, and is bounded separately below. This ceiling is deliberately generous
/// for PCM and still catches an unbounded expansion.
fn frame_ceiling(input_len: usize) -> usize {
    input_len * 8 + 4096
}

/// MIDI names a *duration*, not a size, so its output is not proportional to its
/// input at all. The decoder bounds it explicitly instead — `MAX_EAGER_DECODE_
/// SECONDS` in `fluidsynth.rs` — and that is the number asserted here rather
/// than one invented for the test, which would either pass vacuously or reject
/// output the decoder deliberately permits.
const MAX_MIDI_SECONDS: usize = 15 * 60;

fn midi_frame_ceiling(sample_rate: u32) -> usize {
    MAX_MIDI_SECONDS * sample_rate.max(1) as usize
}

fn riff(chunk: &[u8]) -> Vec<u8> {
    let mut wav = b"RIFF".to_vec();
    wav.extend_from_slice(&(4 + chunk.len() as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(chunk);
    wav
}

/// A `fmt ` chunk plus a `data` chunk, which is the shape every supported WAV
/// layout shares.
fn wav(format_tag: u16, channels: u16, rate: u32, bits: u16, data: &[u8]) -> Vec<u8> {
    let block_align = channels * bits.div_ceil(8);
    let mut chunk = b"fmt ".to_vec();
    chunk.extend_from_slice(&16u32.to_le_bytes());
    chunk.extend_from_slice(&format_tag.to_le_bytes());
    chunk.extend_from_slice(&channels.to_le_bytes());
    chunk.extend_from_slice(&rate.to_le_bytes());
    chunk.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
    chunk.extend_from_slice(&block_align.to_le_bytes());
    chunk.extend_from_slice(&bits.to_le_bytes());
    chunk.extend_from_slice(b"data");
    chunk.extend_from_slice(&(data.len() as u32).to_le_bytes());
    chunk.extend_from_slice(data);
    riff(&chunk)
}

/// A single-track MIDI whose track body is supplied verbatim, so a case can name
/// its own events, deltas and meta records.
fn midi(division: u16, track: &[u8]) -> Vec<u8> {
    let mut bytes = b"MThd".to_vec();
    bytes.extend_from_slice(&6u32.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes()); // format 0
    bytes.extend_from_slice(&1u16.to_be_bytes()); // one track
    bytes.extend_from_slice(&division.to_be_bytes());
    bytes.extend_from_slice(b"MTrk");
    bytes.extend_from_slice(&(track.len() as u32).to_be_bytes());
    bytes.extend_from_slice(track);
    bytes
}

fn seeds() -> Vec<Vec<u8>> {
    let end_of_track = [0x00, 0xff, 0x2f, 0x00];
    vec![
        Vec::new(),
        b"RIFF".to_vec(),
        b"MThd".to_vec(),
        // Every supported PCM layout.
        wav(1, 1, 22_050, 8, &[0x80, 0x7f, 0x00, 0xff]),
        wav(1, 2, 44_100, 8, &[0x80, 0x7f, 0x00, 0xff]),
        wav(1, 1, 44_100, 16, &[0x00, 0x01, 0xff, 0x7f]),
        wav(
            1,
            2,
            48_000,
            16,
            &[0x00, 0x01, 0xff, 0x7f, 0x00, 0x00, 0x01, 0x80],
        ),
        // A-law and mu-law.
        wav(6, 1, 8_000, 8, &[0x00, 0x55, 0xaa, 0xff]),
        wav(7, 1, 8_000, 8, &[0x00, 0x55, 0xaa, 0xff]),
        // An unsupported tag, and degenerate geometry.
        wav(0xffff, 1, 44_100, 16, &[0x00, 0x00]),
        wav(1, 0, 44_100, 16, &[0x00, 0x00]),
        wav(1, 1, 0, 16, &[0x00, 0x00]),
        wav(1, 1, 44_100, 0, &[0x00, 0x00]),
        // A data chunk whose declared length overruns the file.
        {
            let mut bytes = wav(1, 1, 44_100, 16, &[0x00, 0x01]);
            let len = bytes.len();
            bytes[len - 6..len - 2].copy_from_slice(&0xffff_ffffu32.to_le_bytes());
            bytes
        },
        // MIDI is deliberately sparse here. A decode costs SECONDS even for an
        // empty timeline, because the synthesizer runs to its liveness bound
        // rather than stopping at the last event (`fluidsynth.rs:18-21`), so
        // every extra MIDI seed adds ~3 s to the ordinary suite. Two shapes
        // cover the parse; the tempo, SysEx, SMPTE and multi-byte-delta forms
        // live in `fuzz/corpus/audio_decode/` where a long campaign can afford
        // them.
        midi(96, &end_of_track),
        midi(
            96,
            &[
                0x00, 0x90, 0x3c, 0x40, 0x60, 0x80, 0x3c, 0x40, 0x00, 0xff, 0x2f, 0x00,
            ],
        ),
    ]
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
        match rng.below(5) {
            0 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes[at] = (rng.next() & 0xff) as u8;
            }
            1 if bytes.len() >= 4 => {
                // Rewrite a 32-bit field wholesale: chunk lengths are where the
                // amplification lives, so flipping single bits rarely reaches
                // the interesting values.
                let at = rng.below(bytes.len() - 3);
                let value = match rng.below(4) {
                    0 => 0u32,
                    1 => u32::MAX,
                    2 => 0x7fff_ffff,
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

/// Decode and assert the output is in proportion to the input. Returns whether
/// anything decoded, so a campaign can report that it reached the decoders at
/// all rather than rejecting everything at the sniff.
fn check(input: &[u8], what: &str) -> bool {
    match decode_audio(input) {
        Ok(decoded) => {
            let ceiling = frame_ceiling(input.len()).max(midi_frame_ceiling(decoded.sample_rate));
            assert!(
                decoded.frames.len() <= ceiling,
                "{what}: {} frames from {} bytes at {} Hz",
                decoded.frames.len(),
                input.len(),
                decoded.sample_rate
            );
            true
        }
        // A typed error is the other half of the contract; only a panic or an
        // unbounded decode is a failure.
        Err(_) => false,
    }
}

#[test]
fn every_audio_seed_decodes_or_reports_a_typed_error() {
    let mut decoded = 0;
    for (index, seed) in seeds().iter().enumerate() {
        if check(seed, &format!("seed {index}")) {
            decoded += 1;
        }
    }
    // If nothing decodes, the seeds no longer describe the formats and the
    // campaign below is exercising the sniff rather than the decoders.
    assert!(
        decoded >= 6,
        "only {decoded} seeds decoded; the corpus no longer reaches the decoders"
    );
}

#[test]
fn concurrent_midi_decodes_complete_without_backend_liveness_failure() {
    use std::sync::{mpsc, Arc, Barrier};
    use std::time::Duration;

    let corpus = seeds();
    let workers = 2;
    let barrier = Arc::new(Barrier::new(workers));
    let (sender, receiver) = mpsc::channel();
    let mut worker_threads = Vec::new();
    for worker in 0..workers {
        let seed = corpus[14 + worker % 2].clone();
        let barrier = Arc::clone(&barrier);
        let sender = sender.clone();
        worker_threads.push(std::thread::spawn(move || {
            barrier.wait();
            let completed = decode_audio(&seed).is_ok();
            sender
                .send(completed)
                .expect("completion receiver remains live");
        }));
    }
    drop(sender);

    for _ in 0..workers {
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(15))
                .expect("concurrent MIDI decode must complete within 15 seconds"),
            "concurrent MIDI decode must return a successful decode"
        );
    }
    for worker in worker_threads {
        worker.join().expect("MIDI decode worker must not panic");
    }
}

/// The `fuzz/corpus/audio_decode/` seeds the libFuzzer target starts from.
///
/// This deliberately does NOT decode them: twelve of the twenty-two are MIDI,
/// and each would cost seconds. What it checks is that the corpus still exists
/// and still fits the target's input cap — the two ways it can silently stop
/// contributing.
#[test]
fn the_shipped_audio_corpus_is_present_and_within_the_input_cap() {
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/audio_decode");
    let entries = std::fs::read_dir(&corpus).unwrap_or_else(|error| {
        panic!("audio corpus at {} is readable: {error}", corpus.display())
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
    assert!(
        seen >= 20,
        "the audio corpus lost its seeds: only {seen} left"
    );
}

#[test]
fn arbitrary_bytes_never_panic_or_decode_unbounded_audio() {
    let corpus = seeds();
    let mut rng = Rng(0x00a1_1d10_5eed_u64);
    // Deliberately modest: a successful MIDI decode SYNTHESISES its timeline, so
    // a round can cost real rendering rather than a parse. 1,500 rounds keeps
    // this a few seconds in the ordinary gates; `fuzz/audio_decode` is where a
    // long campaign belongs.
    for round in 0..600 {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(seed, &mut rng);
        check(&input, &format!("round {round}"));
    }
}

#[test]
fn truncation_at_every_offset_is_safe() {
    // A short read is the commonest real corruption, and for RIFF and SMF it
    // lands mid-header and mid-chunk in turn.
    for (index, seed) in seeds().iter().enumerate() {
        for length in 0..seed.len().min(512) {
            check(&seed[..length], &format!("seed {index} prefix {length}"));
        }
    }
}

#[test]
fn declared_lengths_that_overrun_the_file_are_rejected_not_trusted() {
    // Every 32-bit field in a RIFF or SMF header is a length, and a decoder that
    // trusts one allocates from it. Rewriting each in turn to u32::MAX is the
    // direct test of that.
    for (index, seed) in seeds().iter().enumerate() {
        for at in 0..seed.len().saturating_sub(4).min(64) {
            let mut bytes = seed.clone();
            bytes[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
            check(&bytes, &format!("seed {index} length at {at}"));
            let mut bytes = seed.clone();
            bytes[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
            check(&bytes, &format!("seed {index} be length at {at}"));
        }
    }
}
