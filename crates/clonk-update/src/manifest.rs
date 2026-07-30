//! The published update manifest, as a client reads it.
//!
//! This mirrors `xtask/src/manifest.rs`, the producing side, and is
//! deserialise-only: a client never writes a manifest. Unknown fields are
//! tolerated so the publisher can add optional data within a schema, while a
//! bumped [`SUPPORTED_SCHEMA`] is the escape hatch for changes an older client
//! must not guess at.

use serde::Deserialize;
use std::collections::BTreeMap;
use thiserror::Error;

/// The one manifest schema this build understands.
///
/// Bumped by the publisher only when an older client would misread the
/// document; every client refuses anything it does not recognise rather than
/// interpreting a partially-understood manifest.
pub const SUPPORTED_SCHEMA: u32 = 1;

/// The release an archive is published in, when that is not the clonk-rs
/// release the manifest describes.
///
/// `content` is built and published by the repository the game data lives in,
/// so the engine repository stops re-uploading 225 MB of unchanged bytes on
/// every daily release. Neither field is ever pasted into a URL unchecked —
/// see `clonk_update_net::urls`, which rejects anything that could leave the
/// release it names.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ArchiveSource {
    /// `owner/name` of the GitHub repository holding the release.
    pub repo: String,
    /// The release tag inside `repo`, which names the exact content commit the
    /// engine repository's submodule pins.
    pub tag: String,
}

/// What one target triple downloads for a component, and where it lands.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TargetArchive {
    /// Release-asset file name; the fetching layer turns this into a URL.
    pub archive: String,
    /// Where `archive` was published, when that is not this release.
    ///
    /// Optional *within* [`SUPPORTED_SCHEMA`] rather than a schema bump: a
    /// client too old to read it would resolve `content` against a clonk-rs
    /// release, which is wrong — but bumping the schema makes that same client
    /// refuse the whole manifest, so it would stop seeing engine updates too.
    /// The narrower failure is the better one while no shipped build downloads
    /// components at all.
    #[serde(default)]
    pub source: Option<ArchiveSource>,
    /// Lowercase hex SHA-256 of the archive bytes. With no manifest signature
    /// this is the whole integrity story for the payload.
    pub sha256: String,
    pub size: u64,
    /// Where the archive's contents land, relative to the install root. Empty
    /// for `engine`, whose archive already carries `bin/` or `Contents/`.
    pub install: String,
}

/// One independently downloadable unit — `content`, `planet` or `engine` —
/// resolved per target triple.
///
/// Keyed by triple even for the shared components, which record the *same*
/// archive and digest under every triple and differ only in `install`. The
/// redundancy is what lets `engine`, which ships three genuinely different
/// builds, be expressed in the same shape.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ComponentEntry {
    pub name: String,
    pub targets: BTreeMap<String, TargetArchive>,
}

impl ComponentEntry {
    /// What `triple` downloads, or `None` when this release does not ship the
    /// component for that platform.
    pub fn target_for(&self, triple: &str) -> Option<&TargetArchive> {
        self.targets.get(triple)
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub schema: u32,
    /// The release this manifest describes — the port version, compared as
    /// semver against what is installed.
    pub version: String,
    /// `ENGINE_VERSION`, the C4XVer tuple, deliberately *not* the release
    /// version. See [`crate::decide`] for why a mismatch is fatal.
    pub engine_version: [i32; 5],
    pub released_at: String,
    pub components: Vec<ComponentEntry>,
}

/// Reads nothing but the schema, so an unreadable body cannot mask the reason
/// a manifest was refused.
#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("update manifest is not readable: {0}")]
    Malformed(#[source] serde_json::Error),
    #[error(
        "update manifest declares schema {found}; this build understands {SUPPORTED_SCHEMA} \
         and will not guess at a newer one"
    )]
    UnsupportedSchema { found: u32 },
}

impl Manifest {
    /// Parses manifest bytes exactly as fetched.
    ///
    /// The schema is checked first and separately: a newer schema may reshape
    /// every other field, and reporting that as a parse failure would look
    /// like a corrupt download rather than a client that is simply too old.
    pub fn parse(bytes: &[u8]) -> Result<Self, ManifestError> {
        let probe: SchemaProbe = serde_json::from_slice(bytes).map_err(ManifestError::Malformed)?;
        (probe.schema == SUPPORTED_SCHEMA).then_some(()).ok_or(
            ManifestError::UnsupportedSchema {
                found: probe.schema,
            },
        )?;
        serde_json::from_slice(bytes).map_err(ManifestError::Malformed)
    }

    pub fn component(&self, name: &str) -> Option<&ComponentEntry> {
        self.components.iter().find(|entry| entry.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{
      "schema": 1,
      "version": "0.4.0",
      "engine_version": [4, 9, 11, 0, 362],
      "released_at": "2026-07-28T10:00:00Z",
      "components": [
        {
          "name": "content",
          "targets": {
            "aarch64-apple-darwin": {
              "archive": "content-0123456789abcdef.zip",
              "sha256": "bb00112233445566778899aabbccddeeff00112233445566778899aabbccddee",
              "size": 250000000,
              "install": "Contents/Resources/content"
            },
            "x86_64-unknown-linux-gnu": {
              "archive": "content-0123456789abcdef.zip",
              "sha256": "bb00112233445566778899aabbccddeeff00112233445566778899aabbccddee",
              "size": 250000000,
              "install": "content"
            }
          }
        },
        {
          "name": "engine",
          "targets": {
            "aarch64-apple-darwin": {
              "archive": "clonk-rust-0.4.0-engine-aarch64-apple-darwin.zip",
              "sha256": "cc00112233445566778899aabbccddeeff00112233445566778899aabbccddee",
              "size": 18000000,
              "install": ""
            },
            "x86_64-unknown-linux-gnu": {
              "archive": "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip",
              "sha256": "aa00112233445566778899aabbccddeeff00112233445566778899aabbccddee",
              "size": 24000000,
              "install": ""
            }
          }
        }
      ]
    }"#;

    #[test]
    fn a_published_manifest_parses_into_its_components() {
        let manifest = Manifest::parse(VALID.as_bytes()).expect("valid manifest");
        assert_eq!(manifest.schema, SUPPORTED_SCHEMA);
        assert_eq!(manifest.version, "0.4.0");
        assert_eq!(manifest.engine_version, [4, 9, 11, 0, 362]);
        assert_eq!(manifest.components.len(), 2);
        let content = manifest.component("content").expect("content entry");
        let linux = content
            .target_for("x86_64-unknown-linux-gnu")
            .expect("linux target");
        assert_eq!(linux.size, 250_000_000);
        assert_eq!(linux.install, "content");
        assert_eq!(linux.archive, "content-0123456789abcdef.zip");
    }

    #[test]
    fn a_shared_component_offers_the_same_bytes_to_every_triple() {
        // `content` and `planet` are platform-independent: only the install
        // destination differs, never the archive or its digest.
        let manifest = Manifest::parse(VALID.as_bytes()).expect("valid manifest");
        let content = manifest.component("content").expect("content entry");
        let linux = content
            .target_for("x86_64-unknown-linux-gnu")
            .expect("linux target");
        let macos = content
            .target_for("aarch64-apple-darwin")
            .expect("macos target");
        assert_eq!(linux.sha256, macos.sha256);
        assert_eq!(linux.archive, macos.archive);
        assert_eq!(macos.install, "Contents/Resources/content");
    }

    #[test]
    fn each_triple_resolves_to_its_own_engine_archive() {
        // The engine ships three genuinely different builds, so a single
        // archive field per component could not say which one to fetch: a
        // client would take whichever was recorded last.
        let manifest = Manifest::parse(VALID.as_bytes()).expect("valid manifest");
        let engine = manifest.component("engine").expect("engine entry");
        let linux = engine
            .target_for("x86_64-unknown-linux-gnu")
            .expect("linux target");
        let macos = engine
            .target_for("aarch64-apple-darwin")
            .expect("macos target");
        assert_eq!(
            linux.archive,
            "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"
        );
        assert_ne!(linux.sha256, macos.sha256);
        // Its archive already carries `bin/…` or `Contents/…`.
        assert!(linux.install.is_empty());
    }

    #[test]
    fn a_component_published_elsewhere_names_the_release_it_came_from() {
        let published = VALID.replace(
            r#""archive": "content-0123456789abcdef.zip","#,
            r#""archive": "content.zip",
             "source": {"repo": "syb0rg/clonk-rs-content", "tag": "content-abc"},"#,
        );
        let manifest = Manifest::parse(published.as_bytes()).expect("valid manifest");
        let source = manifest
            .component("content")
            .and_then(|entry| entry.target_for("x86_64-unknown-linux-gnu"))
            .and_then(|target| target.source.as_ref())
            .expect("content names its release");
        assert_eq!(source.repo, "syb0rg/clonk-rs-content");
        assert_eq!(source.tag, "content-abc");
    }

    #[test]
    fn a_component_without_a_source_belongs_to_this_release() {
        // The field is absent for everything this repository builds, and its
        // absence is the instruction to resolve against the clonk-rs release.
        let manifest = Manifest::parse(VALID.as_bytes()).expect("valid manifest");
        assert!(manifest
            .components
            .iter()
            .flat_map(|entry| entry.targets.values())
            .all(|target| target.source.is_none()));
    }

    #[test]
    fn a_manifest_without_our_triple_simply_has_no_target() {
        let manifest = Manifest::parse(VALID.as_bytes()).expect("valid manifest");
        let content = manifest.component("content").expect("content entry");
        assert_eq!(content.target_for("riscv64gc-unknown-linux-gnu"), None);
    }

    #[test]
    fn an_unknown_schema_is_refused() {
        let newer = VALID.replace("\"schema\": 1", "\"schema\": 2");
        assert!(matches!(
            Manifest::parse(newer.as_bytes()),
            Err(ManifestError::UnsupportedSchema { found: 2 })
        ));
    }

    #[test]
    fn an_unknown_schema_is_refused_before_its_body_is_interpreted() {
        // A future schema is free to reshape every other field. Probing the
        // schema on its own keeps the refusal accurate instead of reporting a
        // deserialisation failure that would read like a corrupt download.
        let reshaped = r#"{"schema": 7, "release": {"name": "0.9.0"}}"#;
        assert!(matches!(
            Manifest::parse(reshaped.as_bytes()),
            Err(ManifestError::UnsupportedSchema { found: 7 })
        ));
    }

    #[test]
    fn a_truncated_manifest_is_reported_as_malformed() {
        let truncated = &VALID.as_bytes()[..40];
        assert!(matches!(
            Manifest::parse(truncated),
            Err(ManifestError::Malformed(_))
        ));
    }
}
