//! Per-component release archives for in-app updating.
//!
//! A full package is ~336 MB, but `content/` (299 MB) and `planet/` (49 MB)
//! change roughly four times per ninety days while the code changes ~144 times.
//! Publishing the payload as separate components lets a client fetch only what
//! actually differs, which is the same distinction C++ drew between an
//! engine-only update and a full objects update (`C4UpdateDlg.cpp:134-140`).
//!
//! Two invariants make that safe, and both are enforced here rather than in a
//! test, because a synthetic fixture cannot vouch for the real 348 MB tree:
//!
//! 1. Every entry of the staged layout belongs to exactly one component. A
//!    seventh top-level entry appearing without a component mapping is a hard
//!    error, not a file that silently stops shipping.
//! 2. `content` and `planet` archives are **prefix-free**, so their bytes are
//!    identical on every platform. Their names are their own digests, so a
//!    platform-dependent prefix would produce four different hashes for
//!    identical data and defeat deduplication entirely.

use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The three units a client can update independently.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ComponentId {
    /// Executables plus the small top-level documents that track them.
    Engine,
    /// `planet/` — engine-side game data.
    Planet,
    /// `content/` — the game packs, and the largest component by far.
    Content,
}

impl ComponentId {
    pub const ALL: [ComponentId; 3] = [
        ComponentId::Engine,
        ComponentId::Planet,
        ComponentId::Content,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ComponentId::Engine => "engine",
            ComponentId::Planet => "planet",
            ComponentId::Content => "content",
        }
    }

    /// Whether the archive is identical on every target triple.
    ///
    /// `engine` carries the platform's own binaries; the other two are data and
    /// must hash the same everywhere for the component store to dedupe them.
    pub fn is_platform_independent(self) -> bool {
        !matches!(self, ComponentId::Engine)
    }

    /// The staged top-level entry this component owns, if it is a whole subtree.
    fn owned_directory(self) -> Option<&'static str> {
        match self {
            ComponentId::Engine => None,
            ComponentId::Planet => Some("planet"),
            ComponentId::Content => Some("content"),
        }
    }

    /// Claims a staged top-level entry by name.
    pub fn claims_top_level(self, entry: &str) -> bool {
        match self {
            // `bin` holds the executables; the documents track the code, are
            // tiny, and would otherwise belong to no component at all.
            ComponentId::Engine => matches!(entry, "bin" | "COPYING" | "README.md" | "credits.txt"),
            other => other.owned_directory() == Some(entry),
        }
    }
}

/// Assigns every staged top-level entry to exactly one component.
///
/// Returns the offending entry names rather than panicking so the caller can
/// report all of them at once.
pub fn unassigned_top_level_entries(package_dir: &Path) -> Result<Vec<String>> {
    let mut unassigned = Vec::new();
    let listing = std::fs::read_dir(package_dir)
        .with_context(|| format!("failed to read staged layout {}", package_dir.display()))?;
    for entry in listing {
        let entry = entry.context("failed to read a staged layout entry")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let owners = ComponentId::ALL
            .iter()
            .filter(|component| component.claims_top_level(&name))
            .count();
        if owners != 1 {
            unassigned.push(name);
        }
    }
    unassigned.sort();
    Ok(unassigned)
}

/// Fails unless the components together account for the staged layout exactly.
///
/// This runs on the real tree during packaging, which is what makes it worth
/// more than the equivalent assertion over a fixture.
pub fn verify_components_cover_layout(package_dir: &Path) -> Result<()> {
    let unassigned = unassigned_top_level_entries(package_dir)?;
    if !unassigned.is_empty() {
        bail!(
            "staged layout entries belong to no component (or to several): {}; \
             add them to `ComponentId::claims_top_level` or they will never reach \
             an updating client",
            unassigned.join(", ")
        );
    }
    Ok(())
}

/// The files a component contributes, keyed by their path inside the archive.
///
/// `engine` keeps the platform layout it was staged with (`bin/…`), because it
/// *is* the platform skeleton. `planet` and `content` drop their own directory
/// name so the archive is prefix-free and byte-identical across triples.
pub fn component_sources(component: ComponentId, package_dir: &Path) -> Result<ComponentSources> {
    match component.owned_directory() {
        Some(directory) => {
            let root = package_dir.join(directory);
            if !root.is_dir() {
                bail!(
                    "component {} expects {} to be a directory",
                    component.name(),
                    root.display()
                );
            }
            Ok(ComponentSources {
                root,
                entries: None,
            })
        }
        None => {
            let mut entries = BTreeMap::new();
            let listing = std::fs::read_dir(package_dir).with_context(|| {
                format!("failed to read staged layout {}", package_dir.display())
            })?;
            for entry in listing {
                let entry = entry.context("failed to read a staged layout entry")?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if component.claims_top_level(&name) {
                    entries.insert(PathBuf::from(&name), entry.path());
                }
            }
            Ok(ComponentSources {
                root: package_dir.to_path_buf(),
                entries: Some(entries),
            })
        }
    }
}

/// Where a component's payload lives inside the staged layout.
#[derive(Debug)]
pub struct ComponentSources {
    /// The directory the archive paths are relative to.
    pub root: PathBuf,
    /// Explicit membership, when the component is a subset of `root` rather
    /// than the whole subtree.
    pub entries: Option<BTreeMap<PathBuf, PathBuf>>,
}

/// The first 128 bits of a digest, which is what names a component archive.
///
/// Content-addressing the filename turns "has this component changed?" into an
/// existence check against the store, with no sidecar metadata to keep in sync.
pub fn short_digest(digest: &[u8]) -> String {
    digest
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn staged_layout() -> TempDir {
        let temp = TempDir::new().expect("temporary staged layout");
        let root = temp.path();
        for name in ["COPYING", "README.md", "credits.txt"] {
            fs::write(root.join(name), name.as_bytes()).expect("write document");
        }
        fs::create_dir_all(root.join("bin")).expect("create bin");
        fs::write(root.join("bin").join("clonk-app"), b"runtime").expect("write runtime");
        fs::create_dir_all(root.join("planet/System.c4g")).expect("create planet");
        fs::write(root.join("planet/System.c4g/C4.c"), b"system").expect("write system");
        fs::create_dir_all(root.join("content/Objects.c4d")).expect("create content");
        fs::write(root.join("content/Objects.c4d/DefCore.txt"), b"objects").expect("write objects");
        temp
    }

    #[test]
    fn every_staged_top_level_entry_belongs_to_exactly_one_component() {
        // Enumerates the layout rather than restating it, so a seventh entry
        // added later fails here instead of silently never shipping.
        let staged = staged_layout();
        assert_eq!(
            unassigned_top_level_entries(staged.path()).expect("scan layout"),
            Vec::<String>::new()
        );
        verify_components_cover_layout(staged.path()).expect("layout is covered");
    }

    #[test]
    fn an_unassigned_layout_entry_is_rejected_by_name() {
        let staged = staged_layout();
        fs::write(staged.path().join("EXTRA.md"), b"new").expect("write extra");

        let error = verify_components_cover_layout(staged.path())
            .expect_err("an unmapped entry must fail packaging");
        assert!(
            error.to_string().contains("EXTRA.md"),
            "error names the offending entry: {error}"
        );
    }

    #[test]
    fn shared_components_own_their_subtree_and_engine_owns_the_rest() {
        assert!(ComponentId::Planet.claims_top_level("planet"));
        assert!(ComponentId::Content.claims_top_level("content"));
        for entry in ["bin", "COPYING", "README.md", "credits.txt"] {
            assert!(
                ComponentId::Engine.claims_top_level(entry),
                "engine should own {entry}"
            );
        }
        assert!(!ComponentId::Engine.claims_top_level("planet"));
        assert!(!ComponentId::Planet.claims_top_level("content"));
    }

    #[test]
    fn only_the_engine_component_varies_by_target_triple() {
        assert!(!ComponentId::Engine.is_platform_independent());
        assert!(ComponentId::Planet.is_platform_independent());
        assert!(ComponentId::Content.is_platform_independent());
    }

    #[test]
    fn shared_component_sources_are_rooted_at_their_own_subtree() {
        // Rooting `planet` at `<staged>/planet` is what makes its archive
        // prefix-free, and therefore identical on every platform.
        let staged = staged_layout();
        let sources = component_sources(ComponentId::Planet, staged.path()).expect("planet sources");
        assert_eq!(sources.root, staged.path().join("planet"));
        assert!(sources.entries.is_none(), "planet is a whole subtree");
    }

    #[test]
    fn engine_component_sources_name_the_entries_it_owns() {
        let staged = staged_layout();
        let sources = component_sources(ComponentId::Engine, staged.path()).expect("engine sources");
        assert_eq!(sources.root, staged.path());
        let entries = sources.entries.expect("engine is a subset of the layout");
        let mut names: Vec<_> = entries.keys().map(|p| p.to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["COPYING", "README.md", "bin", "credits.txt"]);
    }

    #[test]
    fn short_digest_is_the_first_sixteen_bytes_in_hex() {
        let digest: Vec<u8> = (0u8..32).collect();
        let short = short_digest(&digest);
        assert_eq!(short.len(), 32, "128 bits render as 32 hex characters");
        assert!(short.starts_with("000102030405060708090a0b0c0d0e0f"));
    }
}
