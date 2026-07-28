//! Binds this client to the manifest `xtask` actually publishes.
//!
//! The producer (`xtask/src/manifest.rs`) and this reader mirror the schema by
//! hand across crate boundaries, so nothing but this fixture stops them
//! drifting. A rename on either side fails here or in the producer's
//! counterpart test — rather than shipping a manifest no client can parse,
//! which no per-crate test would catch.

use clonk_update::manifest::Manifest;

const FIXTURE: &str = include_str!("fixtures/manifest.json");

#[test]
fn the_client_parses_the_manifest_the_producer_emits() {
    let manifest = Manifest::parse(FIXTURE.as_bytes()).expect("client parses producer output");

    assert_eq!(manifest.version, "0.4.0");
    assert_eq!(manifest.engine_version, [4, 9, 11, 0, 362]);
    // Data first, executables last: an interrupted apply must leave an
    // older-but-working binary that can retry.
    let names: Vec<_> = manifest
        .components
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    assert_eq!(names, ["content", "engine"]);
}

#[test]
fn every_triple_resolves_to_its_own_archive() {
    let manifest = Manifest::parse(FIXTURE.as_bytes()).expect("parse");
    let engine = manifest
        .components
        .iter()
        .find(|entry| entry.name == "engine")
        .expect("engine entry");

    let linux = engine
        .target_for("x86_64-unknown-linux-gnu")
        .expect("linux target");
    let macos = engine
        .target_for("aarch64-apple-darwin")
        .expect("macos target");

    // The flaw this fixture exists to catch: a single `archive` field could not
    // express four per-triple engine builds, so a client would fetch whichever
    // archive happened to be recorded last.
    assert_ne!(linux.archive, macos.archive);
    assert_ne!(linux.sha256, macos.sha256);
    // The engine archive already carries `bin/` or `Contents/`.
    assert!(linux.install.is_empty());
}

#[test]
fn a_shared_component_is_the_same_bytes_everywhere_it_lands() {
    let manifest = Manifest::parse(FIXTURE.as_bytes()).expect("parse");
    let content = manifest
        .components
        .iter()
        .find(|entry| entry.name == "content")
        .expect("content entry");

    let linux = content
        .target_for("x86_64-unknown-linux-gnu")
        .expect("linux");
    let macos = content.target_for("aarch64-apple-darwin").expect("macos");

    assert_eq!(linux.sha256, macos.sha256, "shared components must dedupe");
    assert_eq!(linux.archive, macos.archive);
    assert_eq!(linux.install, "content");
    assert_eq!(macos.install, "Contents/Resources/content");
}

#[test]
fn content_is_resolved_from_the_repository_that_publishes_it() {
    // `content.zip` is built and published where the game data lives, so the
    // producer records the release it came from. A client that dropped this
    // field would resolve the entry against a clonk-rs release that does not
    // carry the asset — a 404 behind a manifest that parses perfectly.
    let manifest = Manifest::parse(FIXTURE.as_bytes()).expect("parse");
    let content = manifest
        .components
        .iter()
        .find(|entry| entry.name == "content")
        .expect("content entry");

    for (triple, target) in &content.targets {
        let source = target
            .source
            .as_ref()
            .unwrap_or_else(|| panic!("content/{triple} must name its release"));
        assert_eq!(source.repo, "syb0rg/clonk-rs-content");
        assert!(source.tag.starts_with("content-"), "{}", source.tag);
        assert_eq!(target.archive, "content.zip");
    }

    // The mirror image: everything this repository builds omits the field, and
    // that absence is what routes it to the clonk-rs release.
    let engine = manifest
        .components
        .iter()
        .find(|entry| entry.name == "engine")
        .expect("engine entry");
    assert!(engine
        .targets
        .values()
        .all(|target| target.source.is_none()));
}
