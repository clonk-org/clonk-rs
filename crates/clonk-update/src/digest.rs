//! Streaming SHA-256 over a downloaded component.
//!
//! With no manifest signature, the digest recorded per component *is* the
//! integrity check, so it has to hold for the largest realistic payload as well
//! as the smallest: `content` is ~299 MB and is hashed in fixed-size chunks,
//! never buffered whole.
//!
//! Comparison folds the whole digest before testing it, so it does not return
//! early on the first differing byte. Timing does not plausibly leak anything
//! here — the expected value arrives in a public manifest — but the cost is nil
//! and it removes the question. (`ring`'s own `verify_slices_are_equal` is
//! deprecated as not-for-external-use, so the fold is written out.)

use ring::digest::{Context, SHA256};
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Bytes pulled from the reader per iteration. Large enough that syscall
/// overhead disappears next to the hash, small enough to stay off the radar of
/// a machine already holding a 299 MB download on disk.
pub const DIGEST_CHUNK: usize = 64 * 1024;

const SHA256_HEX_LEN: usize = 64;

#[derive(Debug, Error)]
pub enum DigestError {
    #[error("failed to read {path} while hashing it: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read the component while hashing it: {0}")]
    Stream(#[source] std::io::Error),
    #[error("expected digest {expected:?} is not a SHA-256")]
    MalformedExpected { expected: String },
    #[error("component digest mismatch: expected {expected}, got {actual}")]
    Mismatch { expected: String, actual: String },
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(hex: &str) -> Option<[u8; SHA256_HEX_LEN / 2]> {
    (hex.len() == SHA256_HEX_LEN)
        .then(|| {
            hex.as_bytes()
                .chunks(2)
                .map(|pair| {
                    std::str::from_utf8(pair)
                        .ok()
                        .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                })
                .collect::<Option<Vec<u8>>>()
        })
        .flatten()
        .and_then(|bytes| bytes.try_into().ok())
}

/// Compares two digests without short-circuiting on the first difference.
fn digests_equal(left: &[u8; SHA256_HEX_LEN / 2], right: &[u8; SHA256_HEX_LEN / 2]) -> bool {
    left.iter()
        .zip(right.iter())
        .fold(0u8, |difference, (left, right)| difference | (left ^ right))
        == 0
}

/// Hashes everything the reader yields, in [`DIGEST_CHUNK`] steps.
pub fn sha256_reader<R: Read>(mut reader: R) -> Result<String, DigestError> {
    let mut context = Context::new(&SHA256);
    let mut buffer = vec![0u8; DIGEST_CHUNK];
    loop {
        let filled = reader.read(&mut buffer).map_err(DigestError::Stream)?;
        if filled == 0 {
            return Ok(to_hex(context.finish().as_ref()));
        }
        context.update(&buffer[..filled]);
    }
}

pub fn sha256_file(path: &Path) -> Result<String, DigestError> {
    let file = std::fs::File::open(path).map_err(|source| DigestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    sha256_reader(std::io::BufReader::new(file)).map_err(|error| match error {
        DigestError::Stream(source) => DigestError::Read {
            path: path.to_path_buf(),
            source,
        },
        other => other,
    })
}

/// Hashes the reader and refuses anything that is not exactly `expected`.
///
/// Hex case is not significant; anything that is not 64 hex digits is a
/// malformed expectation rather than a mismatch, because the two call for
/// different responses — retry the download versus distrust the manifest.
pub fn verify_reader<R: Read>(reader: R, expected: &str) -> Result<(), DigestError> {
    let wanted = decode_hex(&expected.to_ascii_lowercase()).ok_or_else(|| {
        DigestError::MalformedExpected {
            expected: expected.to_string(),
        }
    })?;
    let actual = sha256_reader(reader)?;
    let found = decode_hex(&actual).ok_or_else(|| DigestError::MalformedExpected {
        expected: actual.clone(),
    })?;
    digests_equal(&wanted, &found)
        .then_some(())
        .ok_or(DigestError::Mismatch {
            expected: expected.to_ascii_lowercase(),
            actual,
        })
}

/// Verifies a component archive already on disk.
pub fn verify_file(path: &Path, expected: &str) -> Result<(), DigestError> {
    let file = std::fs::File::open(path).map_err(|source| DigestError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    verify_reader(std::io::BufReader::new(file), expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Read};

    /// FIPS 180-2 example: SHA-256 of "abc".
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    /// Records the largest slice it was ever asked to fill, so a test can show
    /// the reader is consumed incrementally.
    struct WidestRead<'a> {
        remaining: &'a [u8],
        widest: usize,
    }

    impl Read for WidestRead<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.widest = self.widest.max(buf.len());
            let taken = buf.len().min(self.remaining.len());
            buf[..taken].copy_from_slice(&self.remaining[..taken]);
            self.remaining = &self.remaining[taken..];
            Ok(taken)
        }
    }

    #[test]
    fn a_known_vector_hashes_to_its_published_digest() {
        assert_eq!(sha256_reader(Cursor::new(b"abc")).expect("hash"), ABC);
    }

    #[test]
    fn a_component_is_never_read_into_memory_whole() {
        // The largest component is 299 MB. Buffering it to hash it would cost
        // more memory than the game itself, on the machines least able to
        // spare it.
        let payload = vec![7u8; 4 * 1024 * 1024];
        let mut reader = WidestRead {
            remaining: &payload,
            widest: 0,
        };
        sha256_reader(&mut reader).expect("hash");
        assert!(
            reader.widest <= DIGEST_CHUNK,
            "read {} bytes at once, over the {DIGEST_CHUNK}-byte chunk",
            reader.widest
        );
    }

    #[test]
    fn a_file_hashes_to_the_same_digest_as_its_bytes() {
        let directory = tempfile::TempDir::new().expect("directory");
        let path = directory.path().join("component.zip");
        std::fs::write(&path, b"abc").expect("write");
        assert_eq!(sha256_file(&path).expect("hash"), ABC);
    }

    #[test]
    fn a_matching_digest_verifies_whatever_its_case() {
        verify_reader(Cursor::new(b"abc"), ABC).expect("lowercase digest");
        verify_reader(Cursor::new(b"abc"), &ABC.to_uppercase()).expect("uppercase digest");
    }

    #[test]
    fn a_component_whose_bytes_differ_is_rejected() {
        assert!(matches!(
            verify_reader(Cursor::new(b"abd"), ABC),
            Err(DigestError::Mismatch { .. })
        ));
    }

    #[test]
    fn an_expected_digest_that_is_not_a_sha256_is_rejected() {
        // A truncated or mistyped manifest digest must never be treated as
        // "close enough"; it fails the same way a corrupt download does.
        for malformed in ["", "ba7816bf", &"zz".repeat(32), &ABC[1..]] {
            assert!(
                matches!(
                    verify_reader(Cursor::new(b"abc"), malformed),
                    Err(DigestError::MalformedExpected { .. })
                ),
                "{malformed:?} should not be usable as an expected digest"
            );
        }
    }
}
