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

/// What one target triple downloads for a component, and where it lands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetArchive {
    pub archive: String,
    pub sha256: String,
    pub size: u64,
    /// Destination relative to the install root. Empty for `engine`, whose
    /// archive already carries `bin/` or `Contents/`.
    pub install: String,
}

/// One downloadable unit, resolved per target triple.
///
/// Keyed by triple even for the shared components, which record the *same*
/// archive under every triple. That redundancy is deliberate: `engine` ships
/// four genuinely different archives, and a single `archive` field could not
/// express which one a given client should fetch — a Windows client would
/// otherwise read whichever archive happened to be recorded last.
///
/// `BTreeMap` and declaration order throughout, so the serialised bytes do not
/// depend on hash iteration order — a client caching by digest would otherwise
/// see spurious changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ComponentEntry {
    pub name: String,
    pub targets: BTreeMap<String, TargetArchive>,
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

/// Where a component's contents land, relative to the install root.
///
/// macOS keeps game data inside the bundle, so the same prefix-free archive
/// unpacks to a different place there.
fn install_destination(component: ComponentId, triple: &str) -> String {
    let macos = triple.contains("apple-darwin");
    match component {
        // The engine archive already carries its platform layout.
        ComponentId::Engine => String::new(),
        ComponentId::Planet if macos => "Contents/Resources/planet".to_string(),
        ComponentId::Planet => "planet".to_string(),
        ComponentId::Content if macos => "Contents/Resources/content".to_string(),
        ComponentId::Content => "content".to_string(),
    }
}

/// Builds a manifest from every emitted archive across all platform passes.
///
/// `emitted` may contain several `engine` archives — one per triple, produced
/// on different runners — each keyed by the triple it was built for.
pub fn build_manifest(
    version: &str,
    engine_version: [i32; 5],
    released_at: &str,
    emitted: &[(String, EmittedComponent)],
    triples: &[&str],
) -> Manifest {
    let mut by_component: BTreeMap<String, BTreeMap<String, TargetArchive>> = BTreeMap::new();

    for (built_for, component) in emitted {
        let archive = component
            .path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // A shared archive is byte-identical everywhere, so it is offered to
        // every triple; an engine archive belongs only to the triple it was
        // built for.
        let claimed: Vec<&str> = if component.id.is_platform_independent() {
            triples.to_vec()
        } else {
            vec![built_for.as_str()]
        };
        for triple in claimed {
            by_component
                .entry(component.id.name().to_string())
                .or_default()
                .insert(
                    triple.to_string(),
                    TargetArchive {
                        archive: archive.clone(),
                        sha256: component.sha256.clone(),
                        size: component.size,
                        install: install_destination(component.id, triple),
                    },
                );
        }
    }

    let mut components: Vec<ComponentEntry> = by_component
        .into_iter()
        .map(|(name, targets)| ComponentEntry { name, targets })
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

    fn emitted() -> Vec<(String, EmittedComponent)> {
        vec![
            (
                "x86_64-unknown-linux-gnu".to_string(),
                EmittedComponent {
                    id: ComponentId::Engine,
                    path: PathBuf::from("clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"),
                    sha256: "aa".repeat(32),
                    size: 24_000_000,
                },
            ),
            (
                "aarch64-apple-darwin".to_string(),
                EmittedComponent {
                    id: ComponentId::Engine,
                    path: PathBuf::from("clonk-rust-0.4.0-engine-aarch64-apple-darwin.zip"),
                    sha256: "cc".repeat(32),
                    size: 18_000_000,
                },
            ),
            (
                "x86_64-unknown-linux-gnu".to_string(),
                EmittedComponent {
                    id: ComponentId::Content,
                    path: PathBuf::from("content-bb.zip"),
                    sha256: "bb".repeat(32),
                    size: 250_000_000,
                },
            ),
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

    /// Path of the fixture that binds this producer to the client that reads it.
    const SHARED_FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/clonk-update/tests/fixtures/manifest.json"
    );

    #[test]
    fn the_producer_emits_the_shared_schema_fixture_byte_for_byte() {
        // `xtask` writes manifests and `clonk-update` reads them, but the two
        // types are hand-mirrored across crates. This fixture is the only thing
        // binding them: a field renamed on either side fails here or in
        // `clonk-update`'s counterpart test, instead of silently shipping a
        // manifest no client can parse.
        let expected = std::fs::read(SHARED_FIXTURE).expect("read shared fixture");
        let actual = manifest().to_bytes().expect("serialise");
        assert_eq!(
            String::from_utf8_lossy(&actual),
            String::from_utf8_lossy(&expected),
            "producer output drifted from the shared fixture; if this change is \
             intended, re-run the ignored regenerator and update the client test"
        );
    }

    #[test]
    #[ignore = "regenerates the shared schema fixture"]
    fn regenerate_shared_schema_fixture() {
        let bytes = manifest().to_bytes().expect("serialise");
        std::fs::write(SHARED_FIXTURE, bytes).expect("write fixture");
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
            assert!(!entry.targets.is_empty(), "{} needs a target", entry.name);
            for (triple, target) in &entry.targets {
                assert_eq!(
                    target.sha256.len(),
                    64,
                    "{}/{triple} needs a full SHA-256",
                    entry.name
                );
                assert!(target.size > 0, "{}/{triple} needs a size", entry.name);
                assert!(
                    !target.archive.is_empty(),
                    "{}/{triple} needs an archive",
                    entry.name
                );
            }
        }
    }

    #[test]
    fn each_triple_resolves_to_its_own_engine_archive() {
        // A single `archive` field could not express four per-triple engine
        // builds: a Windows client would fetch whichever was recorded last.
        let manifest = manifest();
        let engine = manifest
            .components
            .iter()
            .find(|entry| entry.name == "engine")
            .expect("engine entry");

        assert_eq!(
            engine.targets["x86_64-unknown-linux-gnu"].archive,
            "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"
        );
        assert_eq!(
            engine.targets["aarch64-apple-darwin"].archive,
            "clonk-rust-0.4.0-engine-aarch64-apple-darwin.zip"
        );
        assert_ne!(
            engine.targets["x86_64-unknown-linux-gnu"].sha256,
            engine.targets["aarch64-apple-darwin"].sha256,
            "different builds must not share a digest"
        );
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
            content.targets["aarch64-apple-darwin"].install,
            "Contents/Resources/content"
        );
        assert_eq!(
            content.targets["x86_64-unknown-linux-gnu"].install,
            "content"
        );
        // Same bytes on both platforms; only the destination differs.
        assert_eq!(
            content.targets["aarch64-apple-darwin"].sha256,
            content.targets["x86_64-unknown-linux-gnu"].sha256
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
        assert!(engine
            .targets
            .values()
            .all(|target| target.install.is_empty()));
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
