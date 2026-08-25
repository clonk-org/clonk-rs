//! Bounded malformed-input campaign over the update manifest and the archive
//! extractor (clonk-org/clonk-rs#965).
//!
//! A manifest is fetched over the network and authorizes replacing executables;
//! the archive it names is assumed hostile, because whoever can publish a
//! manifest can publish its digest too. The contract is that arbitrary bytes
//! produce a typed error or a bounded value — never a panic, and never a write
//! outside the destination this call owns.
//!
//! The curated cases live beside the code they check (34 in `extract.rs`, 9 in
//! `manifest.rs`, 17 in `decide.rs`); what this adds is the *arbitrary* half,
//! run in the ordinary suite so the contract holds on every change without the
//! fuzzing engine. `fuzz/fuzz_targets/update_manifest.rs` calls the same entry
//! points for longer campaigns.

use std::io::Write;
use std::path::Path;

use clonk_update::{decide_for_this_build, extract_archive, InstalledState, Manifest};

/// Bounded because the transport caps a manifest before it is parsed and a
/// component before it is opened; an unbounded case would measure the mutator.
const MAX_INPUT: usize = 32_768;

/// What the extractor is allowed to write for one archive in this campaign.
const UNPACKED_BUDGET: u64 = 1024 * 1024;

const VALID_MANIFEST: &str = r#"{
  "schema": 1,
  "version": "0.4.0",
  "engine_version": [
    4,
    9,
    11,
    0,
    362
  ],
  "released_at": "2026-07-28T10:00:00Z",
  "components": [
    {
      "name": "engine",
      "targets": {
        "aarch64-apple-darwin": {
          "archive": "update-engine-0.4.0-aarch64-apple-darwin.zip",
          "sha256": "cc00000000000000000000000000000000000000000000000000000000000000",
          "size": 18000000,
          "install": ""
        },
        "x86_64-pc-windows-msvc": {
          "archive": "update-engine-0.4.0-x86_64-pc-windows-msvc.zip",
          "sha256": "aa00000000000000000000000000000000000000000000000000000000000000",
          "size": 24000000,
          "install": ""
        }
      }
    }
  ]
}"#;

fn manifest_seeds() -> Vec<Vec<u8>> {
    let with_second_component = VALID_MANIFEST.replace(
        "\"components\": [",
        "\"components\": [\n    {\"name\": \"content\", \"targets\": {}},",
    );
    vec![
        VALID_MANIFEST.as_bytes().to_vec(),
        with_second_component.into_bytes(),
        // A schema outside the supported one, refused before the body is
        // trusted.
        VALID_MANIFEST
            .replace("\"schema\": 1", "\"schema\": 99")
            .into_bytes(),
        // An install path that would climb out of the destination, and one
        // that only differs from another by case.
        VALID_MANIFEST
            .replace(
                "\"install\": \"\"",
                "\"install\": \"Contents/../../escape\"",
            )
            .into_bytes(),
        VALID_MANIFEST
            .replace("\"install\": \"\"", "\"install\": \"CONTENTS/resources\"")
            .into_bytes(),
        // A size no download can honour, and a component list that is not a
        // list at all.
        VALID_MANIFEST
            .replace("\"size\": 18000000", "\"size\": 18446744073709551615")
            .into_bytes(),
        br#"{"schema":1,"components":{}}"#.to_vec(),
        // Deep nesting, unterminated strings, and bare fragments.
        b"{\"schema\":1,\"version\":\"".to_vec(),
        b"[[[[[[[[[[[[[[[[[[[[".to_vec(),
        b"null".to_vec(),
        b"".to_vec(),
    ]
}

/// Build a ZIP whose entries are named exactly as given, bypassing any
/// sanitising a well-behaved writer would apply. These are the shapes the
/// extractor's guards exist for.
fn archive_with_names(names: &[&str]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for name in names {
        if writer.start_file(*name, options).is_err() {
            continue;
        }
        let _ = writer.write_all(b"payload");
    }
    writer
        .finish()
        .map(std::io::Cursor::into_inner)
        .unwrap_or_default()
}

fn archive_seeds() -> Vec<Vec<u8>> {
    vec![
        archive_with_names(&["bin/clonk"]),
        // Traversal, absolute, and drive-qualified names.
        archive_with_names(&["../escaped"]),
        archive_with_names(&["a/../../escaped"]),
        archive_with_names(&["/etc/passwd"]),
        archive_with_names(&["C:\\Windows\\system32"]),
        archive_with_names(&["a\\..\\..\\escaped"]),
        // Names a folding filesystem would alias, and a file/directory clash.
        archive_with_names(&["Bin/Clonk", "bin/clonk"]),
        archive_with_names(&["bin", "bin/clonk"]),
        // Empty, dotted and reserved components.
        archive_with_names(&["", ".", "..", "a//b", "CON", "nul.txt"]),
        // A large highly compressible entry, and an empty archive.
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            let options: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("big", options).expect("start entry");
            writer
                .write_all(&vec![0_u8; (UNPACKED_BUDGET * 4) as usize])
                .expect("write entry");
            writer
                .finish()
                .map(std::io::Cursor::into_inner)
                .expect("finish archive")
        },
        archive_with_names(&[]),
        b"PK\x03\x04truncated".to_vec(),
        b"".to_vec(),
    ]
}

/// SplitMix64. A fixed seed keeps any failure reproducible from the test name.
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
                let at = rng.below(bytes.len());
                let len = rng.below(bytes.len() - at).min(128);
                let span = bytes[at..at + len].to_vec();
                bytes.extend_from_slice(&span);
            }
            4 if !bytes.is_empty() => {
                bytes.truncate(rng.below(bytes.len()));
            }
            _ => bytes.extend_from_slice(&corpus[rng.below(corpus.len())]),
        }
    }
    bytes.truncate(MAX_INPUT);
    bytes
}

#[test]
fn arbitrary_bytes_never_panic_in_manifest_parsing_or_planning() {
    let corpus = manifest_seeds();
    let mut rng = Rng(0x5eed_0965_0001_0002);
    let mut parsed = 0;
    for _ in 0..20_000 {
        let input = mutate(&corpus[rng.below(corpus.len())], &corpus, &mut rng);
        let Ok(manifest) = Manifest::parse(&input) else {
            continue;
        };
        parsed += 1;
        // Planning is the half that reaches the installed state and the
        // target triple, and it must stay total over whatever parsed.
        for triple in [
            "aarch64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "",
            "\u{202e}reversed",
        ] {
            let _ = decide_for_this_build(&manifest, &None, triple);
            let _ = decide_for_this_build(&manifest, &Some(InstalledState::default()), triple);
        }
    }
    // A campaign where nothing parses would assert planning against a planner
    // that never ran.
    assert!(
        parsed > 0,
        "no mutated manifest reached planning; the seeds have drifted"
    );
}

/// The shipped corpus under `fuzz/corpus/update_manifest/`, which the
/// cargo-fuzz target starts from. Reading it here keeps the two in step: a
/// seed that stops parsing cleanly fails the ordinary suite rather than
/// quietly degrading the campaign.
#[test]
fn the_shipped_fuzz_corpus_parses_without_panicking() {
    let corpus =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fuzz/corpus/update_manifest");
    let entries = std::fs::read_dir(&corpus)
        .unwrap_or_else(|error| panic!("fuzz corpus at {} is readable: {error}", corpus.display()));
    let mut parsed = 0;
    let mut seen = 0;
    for entry in entries {
        let path = entry.expect("corpus entry").path();
        if !path.is_file() {
            continue;
        }
        seen += 1;
        let bytes = std::fs::read(&path).expect("corpus entry reads");
        if let Ok(manifest) = Manifest::parse(&bytes) {
            parsed += 1;
            let _ = decide_for_this_build(&manifest, &None, "aarch64-apple-darwin");
        }
    }
    assert!(seen > 0, "the shipped corpus is not empty");
    assert!(
        parsed >= 4,
        "only {parsed} of {seen} corpus manifests still parse; the corpus has drifted"
    );
}

/// Nothing an archive can say may put a byte outside the directory the
/// extractor was handed.
#[test]
fn arbitrary_archives_never_escape_the_destination() {
    let corpus = archive_seeds();
    let mut rng = Rng(0x5eed_0965_0003_0004);
    let scratch = tempfile::tempdir().expect("campaign scratch directory");
    // Two siblings of the destination: the extractor may create neither, and a
    // `..` that got through would land in one of them.
    let outside = scratch.path().join("outside");
    std::fs::create_dir(&outside).expect("sibling directory");
    let sentinel = outside.join("untouched");
    std::fs::write(&sentinel, b"sentinel").expect("sentinel file");

    let mut extracted = 0;
    for round in 0..2_000 {
        let input = mutate(&corpus[rng.below(corpus.len())], &corpus, &mut rng);
        let archive = scratch.path().join("candidate.zip");
        std::fs::write(&archive, &input).expect("stage the candidate archive");
        let destination = scratch.path().join(format!("dest-{round}"));
        std::fs::create_dir(&destination).expect("destination directory");

        if let Ok(summary) = extract_archive(&archive, &destination, UNPACKED_BUDGET) {
            extracted += 1;
            assert!(
                summary.bytes <= UNPACKED_BUDGET,
                "round {round} wrote {} bytes against a {UNPACKED_BUDGET}-byte budget",
                summary.bytes
            );
            assert!(
                written_bytes_under(&destination) <= UNPACKED_BUDGET,
                "round {round} left more than the budget under the destination"
            );
        }

        assert_eq!(
            std::fs::read(&sentinel).expect("the sentinel still exists"),
            b"sentinel",
            "round {round} wrote through the destination"
        );
        assert_eq!(
            std::fs::read_dir(&outside)
                .expect("sibling readable")
                .count(),
            1,
            "round {round} created something beside the destination"
        );
        let _ = std::fs::remove_dir_all(&destination);
    }
    // A campaign where nothing ever extracts would assert containment against
    // an extractor that never ran.
    assert!(
        extracted > 0,
        "no mutated archive reached extraction; the corpus has drifted"
    );
}

fn written_bytes_under(root: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => written_bytes_under(&entry.path()),
            Ok(_) => entry.metadata().map(|meta| meta.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}
