//! The signed update manifest a client fetches before downloading anything.
//!
//! Clients read `releases/latest/download/manifest.json` anonymously, verify
//! its Ed25519 signature against a key compiled into the binary, and only then
//! parse it. Verifying the exact bytes that were downloaded — rather than a
//! re-serialisation of a parsed structure — means a parser disagreement can
//! never become a signature bypass, and removes every canonicalisation
//! question (key order, whitespace, number formatting) from the trust path.

use crate::components::{ComponentId, EmittedComponent};
use anyhow::{bail, Context, Result};
// `public_key()` is a trait method, not inherent.
use ring::signature::KeyPair as _;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Bumped only when an older client must refuse to read a newer manifest.
pub const MANIFEST_SCHEMA: u32 = 1;

/// Domain separation, so a signature over some other clonk-rs artifact can
/// never be replayed as a manifest signature.
const SIGNING_DOMAIN: &[u8] = b"clonk-rs/update-manifest/v1\0";

const SIGNATURE_MAGIC: &str = "clonk-rs-signature v1";

/// One downloadable unit.
///
/// `BTreeMap` and declaration order throughout: the serialised bytes are the
/// signed artifact, so they must not depend on hash iteration order.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentEntry {
    pub name: String,
    pub archive: String,
    pub sha256: String,
    pub size: u64,
    /// Where the archive's contents land, relative to the install root, per
    /// target triple. Shared components differ per platform only in
    /// destination, never in bytes.
    pub install: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    /// The release this manifest describes (`PORT_VERSION`).
    pub version: String,
    /// `ENGINE_VERSION`, the C4XVer tuple — deliberately *not* the release
    /// version. A client whose engine is older must refuse `content`, because
    /// `definition_requires_newer_engine` would silently prune definitions
    /// that declare a newer engine rather than report an error.
    pub engine_version: [i32; 5],
    pub released_at: String,
    pub components: Vec<ComponentEntry>,
}

impl Manifest {
    /// Serialises to the exact bytes that get signed and published.
    pub fn to_signed_bytes(&self) -> Result<Vec<u8>> {
        let mut bytes = serde_json::to_vec_pretty(self).context("failed to serialise manifest")?;
        bytes.push(b'\n');
        Ok(bytes)
    }
}

/// Destination of a component's contents per target triple.
///
/// macOS keeps game data inside the bundle, so the same prefix-free archive
/// unpacks to a different place there.
fn install_targets(component: ComponentId, triples: &[&str]) -> BTreeMap<String, String> {
    triples
        .iter()
        .map(|triple| {
            let macos = triple.contains("apple-darwin");
            let destination = match component {
                // The engine archive already carries its platform layout.
                ComponentId::Engine => String::new(),
                ComponentId::Planet if macos => "Contents/Resources/planet".to_string(),
                ComponentId::Planet => "planet".to_string(),
                ComponentId::Content if macos => "Contents/Resources/content".to_string(),
                ComponentId::Content => "content".to_string(),
            };
            ((*triple).to_string(), destination)
        })
        .collect()
}

pub fn build_manifest(
    version: &str,
    engine_version: [i32; 5],
    released_at: &str,
    emitted: &[EmittedComponent],
    triples: &[&str],
) -> Manifest {
    let mut components: Vec<ComponentEntry> = emitted
        .iter()
        .map(|component| ComponentEntry {
            name: component.id.name().to_string(),
            archive: component
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sha256: component.sha256.clone(),
            size: component.size,
            install: install_targets(component.id, triples),
        })
        .collect();
    // Apply order, not emit order: data first, executables last, so an
    // interrupted apply leaves an older-but-working binary that can retry.
    components.sort_by_key(|entry| match entry.name.as_str() {
        "content" => 0,
        "planet" => 1,
        _ => 2,
    });

    Manifest {
        schema: MANIFEST_SCHEMA,
        version: version.to_string(),
        engine_version,
        released_at: released_at.to_string(),
        components,
    }
}

/// Detached signature file: three lines, human-inspectable, trivially parsed.
pub fn signature_document(key_id: &str, signature: &[u8]) -> String {
    format!(
        "{SIGNATURE_MAGIC}\nkeyid: {key_id}\nsig: {}\n",
        crate::components::hex_digest(signature)
    )
}

pub fn parse_signature_document(document: &str) -> Result<(String, Vec<u8>)> {
    let mut key_id = None;
    let mut signature = None;
    let mut lines = document.lines();
    match lines.next() {
        Some(first) if first.trim() == SIGNATURE_MAGIC => {}
        other => bail!("unrecognised signature header: {:?}", other.unwrap_or("")),
    }
    for line in lines {
        if let Some(rest) = line.strip_prefix("keyid:") {
            key_id = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("sig:") {
            signature = Some(decode_hex(rest.trim())?);
        }
    }
    match (key_id, signature) {
        (Some(key_id), Some(signature)) => Ok((key_id, signature)),
        _ => bail!("signature document is missing a keyid or sig line"),
    }
}

fn decode_hex(text: &str) -> Result<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        bail!("hex payload has an odd length");
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&text[index..index + 2], 16)
                .with_context(|| format!("invalid hex at offset {index}"))
        })
        .collect()
}

/// The key identity a client logs, derived so rotation stays interoperable.
pub fn key_id(public_key: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, public_key);
    crate::components::hex_digest(&digest.as_ref()[..8])
}

pub fn sign_manifest(pkcs8: &[u8], manifest_bytes: &[u8]) -> Result<Vec<u8>> {
    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|error| anyhow::anyhow!("update signing key is not a valid Ed25519 pkcs8: {error}"))?;
    let mut payload = SIGNING_DOMAIN.to_vec();
    payload.extend_from_slice(manifest_bytes);
    Ok(pair.sign(&payload).as_ref().to_vec())
}

pub fn verify_manifest(public_key: &[u8], manifest_bytes: &[u8], signature: &[u8]) -> Result<()> {
    let mut payload = SIGNING_DOMAIN.to_vec();
    payload.extend_from_slice(manifest_bytes);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(&payload, signature)
        .map_err(|_| anyhow::anyhow!("manifest signature does not verify against the trusted key"))
}

pub fn write_manifest(directory: &Path, manifest: &Manifest, pkcs8: &[u8]) -> Result<()> {
    let bytes = manifest.to_signed_bytes()?;
    let manifest_path = directory.join("manifest.json");
    std::fs::write(&manifest_path, &bytes)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;

    let pair = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8)
        .map_err(|error| anyhow::anyhow!("update signing key is not a valid Ed25519 pkcs8: {error}"))?;
    let signature = sign_manifest(pkcs8, &bytes)?;
    let document = signature_document(&key_id(pair.public_key().as_ref()), &signature);
    let signature_path = directory.join("manifest.json.sig");
    std::fs::write(&signature_path, document)
        .with_context(|| format!("failed to write {}", signature_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ComponentId;
    use ring::signature::KeyPair;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn keypair() -> (Vec<u8>, Vec<u8>) {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("generate key");
        let pair =
            ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse key");
        (pkcs8.as_ref().to_vec(), pair.public_key().as_ref().to_vec())
    }

    fn emitted() -> Vec<EmittedComponent> {
        vec![
            EmittedComponent {
                id: ComponentId::Engine,
                path: PathBuf::from("clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"),
                sha256: "aa".repeat(32),
                size: 24_000_000,
            },
            EmittedComponent {
                id: ComponentId::Content,
                path: PathBuf::from("content-bb.zip"),
                sha256: "bb".repeat(32),
                size: 250_000_000,
            },
        ]
    }

    fn manifest() -> Manifest {
        build_manifest(
            "0.4.0",
            [4, 9, 11, 0, 362],
            "2026-07-28T10:00:00Z",
            &emitted(),
            &["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"],
        )
    }

    #[test]
    fn manifest_bytes_are_stable_across_runs() {
        // The bytes are the signed artifact, so any iteration-order dependence
        // would make a signature unreproducible.
        let first = manifest().to_signed_bytes().expect("serialise");
        let second = manifest().to_signed_bytes().expect("serialise");
        assert_eq!(first, second);
    }

    #[test]
    fn manifest_carries_the_engine_tuple_not_the_release_version() {
        let manifest = manifest();
        assert_eq!(manifest.engine_version, [4, 9, 11, 0, 362]);
        assert_eq!(manifest.version, "0.4.0");
    }

    #[test]
    fn components_are_ordered_data_first_and_executables_last() {
        // An interrupted apply should leave the old binary in place, able to
        // retry, rather than a new binary beside stale data.
        let names: Vec<_> = manifest()
            .components
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        assert_eq!(names, ["content", "engine"]);
    }

    #[test]
    fn macos_installs_shared_components_inside_the_bundle() {
        let manifest = manifest();
        let content = manifest
            .components
            .iter()
            .find(|entry| entry.name == "content")
            .expect("content entry");
        assert_eq!(
            content.install.get("aarch64-apple-darwin").map(String::as_str),
            Some("Contents/Resources/content")
        );
        assert_eq!(
            content.install.get("x86_64-unknown-linux-gnu").map(String::as_str),
            Some("content")
        );
    }

    #[test]
    fn a_signature_verifies_over_the_exact_manifest_bytes() {
        let (pkcs8, public) = keypair();
        let bytes = manifest().to_signed_bytes().expect("serialise");
        let signature = sign_manifest(&pkcs8, &bytes).expect("sign");
        verify_manifest(&public, &bytes, &signature).expect("verify");
    }

    #[test]
    fn a_single_flipped_byte_fails_verification() {
        let (pkcs8, public) = keypair();
        let bytes = manifest().to_signed_bytes().expect("serialise");
        let signature = sign_manifest(&pkcs8, &bytes).expect("sign");

        let mut tampered = bytes.clone();
        let last = tampered.len() - 2;
        tampered[last] ^= 0x01;
        verify_manifest(&public, &tampered, &signature)
            .expect_err("a tampered manifest must not verify");
    }

    #[test]
    fn another_keys_signature_is_refused() {
        let (pkcs8, _) = keypair();
        let (_, other_public) = keypair();
        let bytes = manifest().to_signed_bytes().expect("serialise");
        let signature = sign_manifest(&pkcs8, &bytes).expect("sign");
        verify_manifest(&other_public, &bytes, &signature)
            .expect_err("an untrusted key must not verify");
    }

    #[test]
    fn a_signature_over_a_different_domain_is_refused() {
        // Guards replay of a signature made over some other clonk-rs artifact.
        let (pkcs8, public) = keypair();
        let bytes = manifest().to_signed_bytes().expect("serialise");
        let pair = ring::signature::Ed25519KeyPair::from_pkcs8(&pkcs8).expect("parse key");
        let foreign = pair.sign(&bytes).as_ref().to_vec();
        verify_manifest(&public, &bytes, &foreign)
            .expect_err("a signature without our domain prefix must not verify");
    }

    #[test]
    fn the_signature_document_round_trips() {
        let (pkcs8, public) = keypair();
        let bytes = manifest().to_signed_bytes().expect("serialise");
        let signature = sign_manifest(&pkcs8, &bytes).expect("sign");
        let document = signature_document(&key_id(&public), &signature);

        let (parsed_id, parsed_sig) = parse_signature_document(&document).expect("parse");
        assert_eq!(parsed_id, key_id(&public));
        assert_eq!(parsed_sig, signature);
    }

    #[test]
    fn a_garbled_signature_document_is_an_error_not_a_panic() {
        parse_signature_document("not a signature").expect_err("header is checked");
        parse_signature_document("clonk-rs-signature v1\nkeyid: aa\n")
            .expect_err("a missing sig line is an error");
        parse_signature_document("clonk-rs-signature v1\nkeyid: aa\nsig: xyz\n")
            .expect_err("non-hex payloads are rejected");
    }

    #[test]
    fn writing_emits_a_manifest_and_a_detached_signature_that_verify_together() {
        let (pkcs8, public) = keypair();
        let out = TempDir::new().expect("output");
        write_manifest(out.path(), &manifest(), &pkcs8).expect("write manifest");

        let bytes = std::fs::read(out.path().join("manifest.json")).expect("read manifest");
        let document =
            std::fs::read_to_string(out.path().join("manifest.json.sig")).expect("read signature");
        let (_, signature) = parse_signature_document(&document).expect("parse signature");
        verify_manifest(&public, &bytes, &signature).expect("published pair verifies");
    }
}
