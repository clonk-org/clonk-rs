//! Bounded malformed-input campaign over the classic league record-stream
//! decoder (clonk-org/clonk-rs#966).
//!
//! These bytes arrive from a league server or from a peer's recording. The
//! container is a zlib envelope around a chunk sequence whose frame deltas,
//! chunk types, filenames and packed lengths are all attacker-controlled, and
//! C++ deliberately *recovers* from a malformed or interrupted suffix rather
//! than rejecting (`C4Playback::ReadBinary`; `StreamToRecord` ignores its own
//! failure result). "Does it reject bad input" is therefore the wrong
//! question: the contract is that arbitrary bytes produce a typed error or a
//! bounded value, never a panic and never unbounded expansion.
//!
//! This runs in the ordinary suite so the contract holds on every change
//! without the fuzzing engine, which is also what lets a discovered reproducer
//! be retained as a plain test. `fuzz/fuzz_targets/record_stream.rs` calls the
//! same entry point for longer campaigns.

use std::io::Write;

use clonk_network::{decode_classic_record_stream, CLASSIC_RECORD_STREAM_MAX_INFLATED};
use flate2::write::ZlibEncoder;
use flate2::Compression;

/// Bounded because a record stream is read from a file or a socket whose size
/// the transport already caps; an unbounded case would measure the mutator.
const MAX_INPUT: usize = 65_536;

fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(bytes).expect("encode zlib fixture");
    encoder.finish().expect("finish zlib fixture")
}

/// `RCT_File` — the chunk type the stream's leading initial save uses.
const FILE_CHUNK: u8 = 0x30;

fn packed_u32(mut value: u32, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

fn file_chunk(delta: u8, name: &[u8], data: &[u8]) -> Vec<u8> {
    let mut chunk = vec![delta, FILE_CHUNK];
    chunk.extend_from_slice(name);
    chunk.push(0);
    packed_u32(data.len() as u32, &mut chunk);
    chunk.extend_from_slice(data);
    chunk
}

/// The shapes a real stream is built from, plus the boundaries C++ tolerates:
/// a leading initial save, file overlays, control and frame chunks, an End
/// chunk, a clean EOF without one, forged lengths and truncated headers.
fn seeds() -> Vec<Vec<u8>> {
    let initial = file_chunk(0, b"Record.c4s", b"\x00\x01\x02\x03initial-save");
    let overlay = file_chunk(3, b"Objects.c4d", b"overlay");
    let mut inflated_variants: Vec<Vec<u8>> = vec![
        initial.clone(),
        [initial.clone(), overlay.clone()].concat(),
        // Control, frame and end chunks after the initial save.
        [initial.clone(), vec![1, 0x02], vec![7, 0x03]].concat(),
        [initial.clone(), vec![255, 0x03]].concat(),
    ];
    // A debug chunk (>= 0x80) with its four-byte type and packed length.
    let mut debug = initial.clone();
    debug.extend_from_slice(&[2, 0x81, b'D', b'B', b'G', b'0']);
    packed_u32(4, &mut debug);
    debug.extend_from_slice(b"data");
    inflated_variants.push(debug);
    // A repeated initial save, and an overlay repeating the same filename.
    inflated_variants.push([initial.clone(), initial.clone()].concat());
    inflated_variants.push([initial.clone(), overlay.clone(), overlay.clone()].concat());
    // A forged length far past the chunk: the decoder must stop, not read on.
    let mut forged = vec![0_u8, FILE_CHUNK];
    forged.extend_from_slice(b"Record.c4s\0");
    packed_u32(u32::MAX, &mut forged);
    forged.extend_from_slice(b"short");
    inflated_variants.push(forged);
    // A header cut mid-filename, and a chunk header with no payload at all.
    inflated_variants.push(vec![0, FILE_CHUNK, b'R', b'e', b'c']);
    inflated_variants.push(vec![0, FILE_CHUNK]);
    inflated_variants.push(vec![0]);
    inflated_variants.push(Vec::new());

    let mut seeds: Vec<Vec<u8>> = inflated_variants.iter().map(|bytes| zlib(bytes)).collect();
    // Suffix bytes after a complete zlib stream, a truncated envelope, and raw
    // bytes that are not zlib at all.
    let whole = zlib(&[initial.clone(), overlay].concat());
    seeds.push([whole.clone(), b"trailing".to_vec()].concat());
    seeds.push(whole[..whole.len() / 2].to_vec());
    seeds.push(initial);
    seeds.push(b"\x78\x9c".to_vec());
    seeds.push(Vec::new());
    seeds
}

/// SplitMix64. A fixed seed keeps any failure reproducible from the test name
/// alone; the long campaigns live in `fuzz/`.
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

fn mutate(seed: &[u8], corpus: &[Vec<u8>], rng: &mut Rng) -> Vec<u8> {
    let mut bytes = seed.to_vec();
    for _ in 0..=rng.below(8) {
        if bytes.len() >= MAX_INPUT {
            bytes.truncate(MAX_INPUT / 2);
        }
        match rng.below(6) {
            0 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes[at] = (rng.next() & 0xff) as u8;
            }
            1 if !bytes.is_empty() => {
                let at = rng.below(bytes.len());
                bytes.remove(at);
            }
            2 => {
                let at = rng.below(bytes.len() + 1);
                bytes.insert(at, (rng.next() & 0xff) as u8);
            }
            3 if !bytes.is_empty() => {
                // Duplicate a span: repeated chunks and overlapping file
                // overlays are what this container is most permissive about.
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(256);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            4 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            _ => {
                bytes.extend_from_slice(&corpus[rng.below(corpus.len())]);
            }
        }
    }
    bytes.truncate(MAX_INPUT);
    bytes
}

#[test]
fn arbitrary_bytes_never_panic_in_the_record_stream_decoder() {
    let corpus = seeds();
    let mut rng = Rng(0x5eed_9966_0102_0304);
    for round in 0..20_000 {
        let seed = &corpus[rng.below(corpus.len())];
        let input = mutate(seed, &corpus, &mut rng);
        let Ok(stream) = decode_classic_record_stream(&input) else {
            continue;
        };
        let produced = stream.initial_group.len()
            + stream.control_record.len()
            + stream
                .files
                .iter()
                .map(|file| file.filename.as_bytes().len() + file.data.len())
                .sum::<usize>();
        // Everything handed back is carved out of the one inflated buffer, so
        // the decoder's ceiling bounds the caller's view of it too.
        assert!(
            produced <= CLASSIC_RECORD_STREAM_MAX_INFLATED,
            "{produced} bytes produced from {} compressed in round {round}",
            input.len()
        );
    }
}

/// The shipped corpus under `fuzz/corpus/record_stream/`, which the cargo-fuzz
/// target starts from. Reading it here keeps the two in step: a seed that stops
/// decoding cleanly fails the ordinary suite rather than quietly degrading the
/// campaign.
#[test]
fn the_shipped_fuzz_corpus_decodes_without_panicking() {
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/record_stream");
    let entries = std::fs::read_dir(&corpus)
        .unwrap_or_else(|error| panic!("fuzz corpus at {} is readable: {error}", corpus.display()));
    let mut seen = 0;
    for entry in entries {
        let path = entry.expect("corpus entry").path();
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("corpus entry reads");
        if let Ok(stream) = decode_classic_record_stream(&bytes) {
            let produced = stream.initial_group.len()
                + stream.control_record.len()
                + stream
                    .files
                    .iter()
                    .map(|file| file.filename.as_bytes().len() + file.data.len())
                    .sum::<usize>();
            assert!(
                produced <= CLASSIC_RECORD_STREAM_MAX_INFLATED,
                "{} produced {produced} bytes",
                path.display()
            );
        }
        seen += 1;
    }
    assert!(seen > 0, "the shipped corpus is not empty");
}

/// Every seed shape decodes or fails cleanly on its own, before mutation.
/// A seed that stops being decodable would quietly degrade the campaign into
/// mutating something the decoder rejects at its first byte.
#[test]
fn every_seed_shape_decodes_or_fails_cleanly() {
    let mut decoded = 0;
    for seed in seeds() {
        if decode_classic_record_stream(&seed).is_ok() {
            decoded += 1;
        }
    }
    assert!(
        decoded >= 6,
        "only {decoded} seed shapes still decode; the corpus has drifted"
    );
}
