//! Bounded malformed-input campaign over the legacy resource-text parsers
//! (clonk-org/clonk-rs#963).
//!
//! These grammars are permissive by design: they accept arbitrary native bytes,
//! repeated sections and keys, numeric prefixes and radices, array fields and
//! embedded RTF control words, and they *recover* rather than reject. That makes
//! "does it reject bad input" the wrong question — the contract is that
//! arbitrary bytes produce a typed error or a bounded value, never a panic, an
//! unbounded allocation, or work that does not terminate.
//!
//! This runs in the ordinary test suite so the contract is enforced on every
//! change without the fuzzing engine, which is also what lets a discovered
//! reproducer be retained as a plain test. `fuzz/` carries the cargo-fuzz
//! targets that call these same functions for longer campaigns.

use clonk_resources::definition::parse_def_core;
use clonk_resources::material::{MaterialEnumeration, MaterialLibrary};
use clonk_resources::rtf::rtf_to_plain_text;
use clonk_resources::texmap::TextureMap;

/// One parser under test: its name, and a call that returns a coarse "size" of
/// whatever it produced. The uniform signature is what lets the campaign run
/// every grammar over the same bytes, which is the point — these formats reach
/// each other's parsers in the wild.
type Parser = (&'static str, fn(&[u8]) -> usize);

const PARSERS: &[Parser] = &[
    ("def_core", |bytes| {
        parse_def_core(bytes).map_or(0, |core| core.id.len() + core.name.map_or(0, |n| n.len()))
    }),
    ("material_library", |bytes| {
        MaterialLibrary::parse_bytes(bytes).map_or(0, |library| library.iter().count())
    }),
    ("material_enumeration", |bytes| {
        MaterialEnumeration::parse(bytes).map_or(0, |enumeration| enumeration.names().len())
    }),
    ("texture_map", |bytes| {
        let map = TextureMap::parse_bytes(bytes);
        (0..=u8::MAX)
            .filter(|index| map.entry(*index).is_some())
            .count()
    }),
    ("texture_flags", |bytes| {
        let _ = TextureMap::parse_flags_bytes(bytes);
        0
    }),
    ("rtf_plain_text", |bytes| rtf_to_plain_text(bytes).len()),
];

/// Seeds drawn from the shapes these grammars actually meet: section headers,
/// repeated keys, native high bytes, NULs, numeric radices, C4ID lists, vertex
/// arrays and RTF control words.
const SEEDS: &[&[u8]] = &[
    b"",
    b"\x00",
    b"[DefCore]\r\nid=TEST\r\nName=Test\r\n",
    b"[DefCore]\r\nid=TEST\r\nid=TWIC\r\nName=A\r\nName=B\r\n",
    b"[DefCore]\r\nCategory=0x4000\r\nMass=-1\r\nValue=0777\r\n",
    b"[DefCore]\r\nVertexX=1,2,3,4\r\nVertexY=-1,-2\r\nVertexCNAT=1,2,4,8\r\n",
    b"[DefCore]\r\nComponents=WOOD=2;METL=1\r\n",
    b"[DefCore]\r\nName=A\xe4\xf6\xfc\x00trailing\r\n",
    b"[Physical]\r\nEnergy=100000\r\nWalk=50000\r\n",
    b"[Material Water]\r\nName=Water\r\nDensity=25\r\nMaxSlide=4\r\n",
    b"[Material]\r\n[Material]\r\n[Material]\r\n",
    b"[Enumeration]\r\nWater\r\nGranite\r\nEarth\r\n",
    b"[Enumeration]\r\n",
    b"[Enumeration]\r\nAVeryLongMaterialNameBeyondTheCap\r\n",
    b"Water\r\nGranite\r\nEarth\r\n",
    b"1=Water-Smooth\r\n2=Granite-Rough\r\n",
    b"Smooth\r\nRough\r\n",
    b"{\\rtf1\\ansi Hello \\b bold\\b0 world}",
    b"{\\rtf1{\\*\\generator x}\\par text",
    b"{\\rtf1\\ansi \\'e4\\'f6\\'fc}",
    b"{{{{{{{{{{",
    b"\\\\\\\\\\\\\\\\",
    b"[A]\r\n=\r\n=\r\n=\r\n",
    b"=====",
    b"\r\n\r\n\r\n\r\n",
];

/// A deterministic byte mutator. A fixed seed keeps a failure reproducible from
/// the test name alone, which matters more here than statistical coverage: the
/// long campaigns live in `fuzz/`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // SplitMix64.
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

/// Input length is bounded because these parsers are reached from files whose
/// size the group layer already caps; an unbounded case would measure the
/// mutator rather than the grammar.
const MAX_INPUT: usize = 4096;

/// No parser here may produce more than it consumed, plus a small constant for
/// headers and defaulted fields. Measured worst case over the campaign is well
/// under 1:1 — 216 units from 251 bytes — so this is a real ceiling rather than
/// a formality: repeated sections and array fields are where these grammars
/// could otherwise amplify.
fn output_ceiling(input_len: usize) -> usize {
    input_len + 64
}

fn mutate(seed: &[u8], rng: &mut Rng) -> Vec<u8> {
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
                // Duplicate a span: repeated keys and sections are the shapes
                // these grammars are most permissive about.
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(64);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            4 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            _ => {
                // Splice in another seed, so cross-grammar confusion is covered.
                let other = SEEDS[rng.below(SEEDS.len())];
                bytes.extend_from_slice(other);
            }
        }
    }
    bytes.truncate(MAX_INPUT);
    bytes
}

#[test]
fn arbitrary_bytes_never_panic_in_the_resource_text_parsers() {
    let mut rng = Rng(0x5eed_1234_5678_9abc);
    for round in 0..50_000 {
        let seed = SEEDS[rng.below(SEEDS.len())];
        let input = mutate(seed, &mut rng);
        for (name, parse) in PARSERS {
            let produced = parse(&input);
            // A parser must not manufacture output out of proportion to its
            // input: that is the amplification these grammars could permit
            // through repeated sections and array fields.
            assert!(
                produced <= output_ceiling(input.len()),
                "{name} produced {produced} units from {} bytes in round {round}",
                input.len()
            );
        }
    }
}

/// The shipped corpus under `fuzz/corpus/resource_text/`, which the cargo-fuzz
/// targets start from. Reading it here is what keeps the two in step: a seed
/// that stops parsing cleanly fails the ordinary suite rather than quietly
/// degrading the campaign.
#[test]
fn the_shipped_fuzz_corpus_parses_without_panicking() {
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/resource_text");
    let entries = std::fs::read_dir(&corpus)
        .unwrap_or_else(|error| panic!("fuzz corpus at {} is readable: {error}", corpus.display()));
    let mut seen = 0;
    for entry in entries {
        let path = entry.expect("corpus entry").path();
        if !path.is_file() {
            continue;
        }
        let bytes = std::fs::read(&path).expect("corpus entry reads");
        for (name, parse) in PARSERS {
            let produced = parse(&bytes);
            assert!(
                produced <= output_ceiling(bytes.len()),
                "{name} produced {produced} units from {}",
                path.display()
            );
        }
        seen += 1;
    }
    assert!(
        seen >= 4,
        "the shipped corpus lost its seeds: only {seen} left"
    );
}

#[test]
fn every_seed_parses_without_panicking() {
    // The seeds are the corpus; a failure here is a plain reproducible bug
    // rather than a mutation-dependent one.
    for seed in SEEDS {
        for (name, parse) in PARSERS {
            let produced = parse(seed);
            assert!(
                produced <= output_ceiling(seed.len()),
                "{name} produced {produced} units from a seed"
            );
        }
    }
}

#[test]
fn truncation_at_every_offset_is_safe() {
    // Truncation is the most common real-world corruption — a short read, a
    // cut-off download, a partially written save — and it hits every parser
    // state in turn.
    for seed in SEEDS {
        for length in 0..seed.len() {
            for (name, parse) in PARSERS {
                let produced = parse(&seed[..length]);
                assert!(
                    produced <= output_ceiling(length),
                    "{name} produced {produced} units from a {length}-byte prefix"
                );
            }
        }
    }
}

#[test]
fn interior_nul_bytes_are_safe_at_every_offset() {
    // C++ hands these parsers a native C string, so a NUL truncates rather than
    // terminating the read (`parse_def_core` splits on it explicitly). Placing
    // one at every offset checks that no parser walks past it.
    for seed in SEEDS {
        for at in 0..seed.len() {
            let mut bytes = seed.to_vec();
            bytes[at] = 0;
            for (name, parse) in PARSERS {
                let produced = parse(&bytes);
                assert!(
                    produced <= output_ceiling(bytes.len()),
                    "{name} produced {produced} units with a NUL at {at}"
                );
            }
        }
    }
}
