//! The update manifest a client fetches before downloading anything.
//!
//! # Trust model
//!
//! Clients read `releases/latest/download/manifest.json` anonymously over TLS
//! (rustls with bundled roots) and verify every component against the SHA-256
//! the manifest records. That covers tampering in transit and a corrupted or
//! substituted component archive.
//!
//! It deliberately does **not** cover a malicious manifest published by
//! whoever controls the repository. Signing was considered and dropped: with
//! automated daily releases the private key would have to live in CI, so an
//! attacker who could publish a release could equally invoke the workflow that
//! signs — the signature would guard almost nothing while adding a key that,
//! once lost, would permanently stop shipped clients from updating. Offline
//! signing is the variant that would actually help, and it is incompatible
//! with hands-off releases.
//!
//! The manifest is a plain document with a detached-signature-shaped future:
//! adding `manifest.json.sig` later needs no schema change here.

use crate::components::{ComponentId, EmittedComponent};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Bumped only when an older client must refuse to read a newer manifest.
pub const MANIFEST_SCHEMA: u32 = 1;

/// One downloadable unit.
///
/// `BTreeMap` and declaration order throughout, so the serialised bytes do not
/// depend on hash iteration order — a client caching by digest would otherwise
/// see spurious changes.
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
    /// `definition_requires_newer_engine` silently *prunes* definitions that
    /// declare a newer engine rather than reporting an error.
    pub engine_version: [i32; 5],
    pub released_at: String,
    pub components: Vec<ComponentEntry>,
}

impl Manifest {
    /// Serialises to the exact bytes that get published.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
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

pub fn write_manifest(directory: &Path, manifest: &Manifest) -> Result<()> {
    let bytes = manifest.to_bytes()?;
    let manifest_path = directory.join("manifest.json");
    std::fs::write(&manifest_path, &bytes)
        .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ComponentId;
    use std::path::PathBuf;
    use tempfile::TempDir;

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
        // A client caches by digest, so iteration-order dependence would look
        // like the manifest changing when nothing had.
        let first = manifest().to_bytes().expect("serialise");
        let second = manifest().to_bytes().expect("serialise");
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
    fn every_component_records_the_digest_a_client_verifies() {
        // With no signature, the recorded hash is the whole integrity story
        // for the payload; an entry without one would be unverifiable.
        for entry in manifest().components {
            assert_eq!(entry.sha256.len(), 64, "{} needs a full SHA-256", entry.name);
            assert!(entry.size > 0, "{} needs a size", entry.name);
            assert!(!entry.archive.is_empty(), "{} needs an archive", entry.name);
        }
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
            content
                .install
                .get("x86_64-unknown-linux-gnu")
                .map(String::as_str),
            Some("content")
        );
    }

    #[test]
    fn the_engine_component_unpacks_at_the_install_root() {
        // Its archive already carries `bin/…` or `Contents/…`, so it needs no
        // destination prefix of its own.
        let manifest = manifest();
        let engine = manifest
            .components
            .iter()
            .find(|entry| entry.name == "engine")
            .expect("engine entry");
        assert!(engine.install.values().all(|destination| destination.is_empty()));
    }

    #[test]
    fn a_written_manifest_round_trips() {
        let out = TempDir::new().expect("output");
        write_manifest(out.path(), &manifest()).expect("write manifest");

        let bytes = std::fs::read(out.path().join("manifest.json")).expect("read manifest");
        let parsed: Manifest = serde_json::from_slice(&bytes).expect("parse manifest");
        assert_eq!(parsed, manifest());
    }
}
