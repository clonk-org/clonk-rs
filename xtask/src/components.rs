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
//!    error, not a file that silently stops shipping. This is checked over
//!    [`ComponentId`], every component a *client* resolves — not over the ones
//!    built here, or `content/` would stop being covered by it.
//! 2. The `planet` archive is **prefix-free**, so its bytes are identical on
//!    every platform. Its name is its own digest, so a platform-dependent
//!    prefix would produce four different hashes for identical data and defeat
//!    deduplication entirely.
//!
//! `content` is a component a client fetches but this repository does **not**
//! build — see [`BuiltComponent`]. It is published by the repository the game
//! data lives in, so 225 MB of unchanged bytes stop being re-uploaded on every
//! daily release.

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
    /// Every component a client resolves, whoever produced it.
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

/// A component this repository builds, which is every one but `content`.
///
/// A type of its own rather than a filter over [`ComponentId`], because the
/// distinction is not advisory. `content.zip` is content-addressed, so two
/// "deterministic zip" implementations would have to agree byte for byte
/// forever — the day they drifted, the digest would move without the content
/// moving and every install would re-fetch 225 MB. There is therefore exactly
/// one builder, and it is `clonk-rs-content`, beside the files it is made of.
/// This repository *references* what that release published: see
/// `CONTENT_REPOSITORY` in `main.rs`.
///
/// [`component_sources`] and [`emit_component`] take this type, so the second
/// builder is not something a later change can add back by overlooking a
/// filter: there is no variant to pass them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuiltComponent {
    Engine,
    Planet,
}

impl BuiltComponent {
    /// Every component a packaging run can emit.
    pub const ALL: [BuiltComponent; 2] = [BuiltComponent::Engine, BuiltComponent::Planet];

    /// The client-facing component this builds.
    pub fn id(self) -> ComponentId {
        match self {
            BuiltComponent::Engine => ComponentId::Engine,
            BuiltComponent::Planet => ComponentId::Planet,
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
/// *is* the platform skeleton. `planet` drops its own directory name so the
/// archive is prefix-free and byte-identical across triples.
pub fn component_sources(
    component: BuiltComponent,
    package_dir: &Path,
) -> Result<ComponentSources> {
    let component = component.id();
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

pub fn hex_digest(digest: &[u8]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// SHA-256 over a file, streamed so a 299 MB component is not held in memory.
pub fn sha256_file(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open {} for hashing", path.display()))?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .with_context(|| format!("failed to read {}", path.display()))?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(context.finish().as_ref().to_vec())
}

/// A component archive on disk, with the identity a manifest records.
#[derive(Debug, Clone)]
pub struct EmittedComponent {
    pub id: ComponentId,
    pub path: PathBuf,
    /// Full SHA-256, hex. The manifest carries all 256 bits; only the filename
    /// is shortened.
    pub sha256: String,
    pub size: u64,
}

/// The archive filename for a component.
///
/// Shared components are named after their own digest so an unchanged `planet`
/// keeps the same name across releases and needs no re-upload. `engine` changes
/// every release and is versioned instead, which also keeps the per-triple
/// archives distinguishable.
pub fn component_archive_name(
    component: BuiltComponent,
    digest: &[u8],
    version: &str,
    target_triple: &str,
) -> String {
    let component = component.id();
    if component.is_platform_independent() {
        format!("{}-{}.zip", component.name(), short_digest(digest))
    } else {
        format!(
            "clonk-rust-{version}-{}-{target_triple}.zip",
            component.name()
        )
    }
}

/// Writes a component payload to `archive_path`, given the source root and a
/// predicate selecting which relative paths belong to the component.
pub type ArchiveWriter<'a> = &'a dyn Fn(&Path, &Path, &dyn Fn(&Path) -> bool) -> Result<()>;

/// Writes one component archive and names it from its own contents.
///
/// The archive is written to a scratch name first because its final name
/// depends on the digest of the bytes just written.
pub fn emit_component(
    component: BuiltComponent,
    package_dir: &Path,
    output_dir: &Path,
    version: &str,
    target_triple: &str,
    write_archive: ArchiveWriter<'_>,
) -> Result<EmittedComponent> {
    std::fs::create_dir_all(output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;

    let id = component.id();
    let sources = component_sources(component, package_dir)?;
    let owned = sources.entries.is_some();
    let include = move |relative: &Path| -> bool {
        if !owned {
            return true;
        }
        relative
            .components()
            .next()
            .map(|first| id.claims_top_level(&first.as_os_str().to_string_lossy()))
            .unwrap_or(false)
    };

    let scratch = output_dir.join(format!(".{}-staging.zip", id.name()));
    write_archive(&scratch, &sources.root, &include)?;

    let digest = sha256_file(&scratch)?;
    let size = std::fs::metadata(&scratch)
        .with_context(|| format!("failed to stat {}", scratch.display()))?
        .len();

    let final_path = output_dir.join(component_archive_name(
        component,
        &digest,
        version,
        target_triple,
    ));
    if final_path.exists() {
        std::fs::remove_file(&final_path)
            .with_context(|| format!("failed to replace {}", final_path.display()))?;
    }
    std::fs::rename(&scratch, &final_path)
        .with_context(|| format!("failed to name {} from its digest", final_path.display()))?;

    Ok(EmittedComponent {
        id,
        path: final_path,
        sha256: hex_digest(&digest),
        size,
    })
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
    fn content_is_referenced_rather_than_built_here() {
        // There must be exactly one builder of `content.zip`, and it is the
        // repository the game data lives in. Two deterministic-zip
        // implementations would have to agree byte for byte forever; the day
        // they drifted, the digest would move without the content moving and
        // every install would re-fetch 225 MB.
        //
        // Enforced by the type rather than by a filter over `ComponentId`:
        // `emit_component` takes a `BuiltComponent`, and there is no
        // `BuiltComponent::Content` to hand it. The second builder cannot be
        // written back by accident.
        assert_eq!(
            BuiltComponent::ALL.map(BuiltComponent::id),
            [ComponentId::Engine, ComponentId::Planet],
            "content is published by the content repository"
        );
        // Still a component a client downloads and applies — only the building
        // moved, so dropping it from `ALL` would stop shipping game data.
        assert!(ComponentId::ALL.contains(&ComponentId::Content));
        // And it still ships in the installer, so it must still be claimed.
        assert!(ComponentId::Content.claims_top_level("content"));
    }

    #[test]
    fn the_staged_layout_stays_covered_although_content_is_never_emitted() {
        // The cross-check is over every component a client resolves, not over
        // the ones this repository builds: `content/` is staged, shipped in the
        // installer and applied by an updating client, so it must still be
        // claimed by exactly one component even though nothing here zips it.
        //
        // Scoping the check to the built components instead would be the
        // tempting way to keep it passing, and would silently re-admit the very
        // thing it exists to catch — a staged directory that reaches no client.
        let staged = staged_layout();
        assert!(
            staged.path().join("content").is_dir(),
            "the fixture must still stage the directory nobody builds"
        );
        verify_components_cover_layout(staged.path())
            .expect("content is claimed although it is never emitted");

        // The same check still fails on an entry no component claims, so it has
        // not been weakened into a no-op.
        fs::write(staged.path().join("music"), b"packs").expect("write unmapped entry");
        assert!(verify_components_cover_layout(staged.path()).is_err());
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
        let sources =
            component_sources(BuiltComponent::Planet, staged.path()).expect("planet sources");
        assert_eq!(sources.root, staged.path().join("planet"));
        assert!(sources.entries.is_none(), "planet is a whole subtree");
    }

    #[test]
    fn engine_component_sources_name_the_entries_it_owns() {
        let staged = staged_layout();
        let sources =
            component_sources(BuiltComponent::Engine, staged.path()).expect("engine sources");
        assert_eq!(sources.root, staged.path());
        let entries = sources.entries.expect("engine is a subset of the layout");
        let mut names: Vec<_> = entries
            .keys()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["COPYING", "README.md", "bin", "credits.txt"]);
    }

    /// Stands in for `write_deterministic_zip`, which lives in the binary
    /// crate; the properties under test are naming and hashing, not zip
    /// mechanics (those are covered by the archive tests in `main.rs`).
    fn fake_archive_writer(
        archive_path: &Path,
        source_root: &Path,
        include: &dyn Fn(&Path) -> bool,
    ) -> Result<()> {
        let mut manifest = String::new();
        let mut paths: Vec<PathBuf> = Vec::new();
        fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
            for entry in fs::read_dir(dir).expect("read dir").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, into);
                } else {
                    into.push(path);
                }
            }
        }
        walk(source_root, &mut paths);
        paths.sort();
        for path in paths {
            let relative = path.strip_prefix(source_root).expect("relative");
            if !include(relative) {
                continue;
            }
            manifest.push_str(&relative.to_string_lossy());
            manifest.push('\n');
            manifest.push_str(&fs::read_to_string(&path).unwrap_or_default());
            manifest.push('\n');
        }
        fs::write(archive_path, manifest).expect("write fake archive");
        Ok(())
    }

    fn emit(
        component: BuiltComponent,
        staged: &Path,
        out: &Path,
        triple: &str,
    ) -> EmittedComponent {
        emit_component(
            component,
            staged,
            out,
            "0.4.0",
            triple,
            &fake_archive_writer,
        )
        .expect("emit component")
    }

    #[test]
    fn shared_components_are_byte_identical_across_target_triples() {
        // The property the entire component store rests on: if `planet` hashed
        // differently per platform there would be four copies of identical
        // data and no deduplication at all.
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        let arm = emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "aarch64-apple-darwin",
        );
        let win = emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "x86_64-pc-windows-gnu",
        );
        assert_eq!(
            arm.sha256, win.sha256,
            "planet must hash the same everywhere"
        );
        assert_eq!(arm.path, win.path, "and therefore carry the same name");
    }

    #[test]
    fn a_shared_component_archive_is_named_from_its_own_digest() {
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        let emitted = emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );

        let name = emitted
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(name, format!("planet-{}.zip", &emitted.sha256[..32]));
        assert!(emitted.path.exists(), "the digest-named archive exists");
    }

    #[test]
    fn changing_a_shared_component_changes_its_name() {
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        let before = emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );

        fs::write(staged.path().join("planet/System.c4g/C4.c"), b"edited").expect("edit planet");
        let after = emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );

        assert_ne!(before.sha256, after.sha256);
        assert_ne!(
            before.path, after.path,
            "a changed component needs a new name"
        );
    }

    #[test]
    fn the_engine_component_is_versioned_per_triple_not_content_addressed() {
        // Engine changes every release, so digest naming would only churn the
        // store; the four per-triple archives must also stay distinguishable.
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        let linux = emit(
            BuiltComponent::Engine,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );
        let windows = emit(
            BuiltComponent::Engine,
            staged.path(),
            out.path(),
            "x86_64-pc-windows-gnu",
        );

        assert_eq!(
            linux.path.file_name().unwrap().to_string_lossy(),
            "clonk-rust-0.4.0-engine-x86_64-unknown-linux-gnu.zip"
        );
        assert_ne!(
            linux.path, windows.path,
            "per-triple archives must not collide"
        );
    }

    #[test]
    fn the_engine_component_excludes_the_shared_subtrees() {
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        let emitted = emit(
            BuiltComponent::Engine,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );

        let body = fs::read_to_string(&emitted.path).expect("read emitted");
        assert!(body.contains("bin/clonk-app"), "engine ships the binaries");
        assert!(body.contains("COPYING"), "engine ships the documents");
        assert!(
            !body.contains("planet/"),
            "planet belongs to its own component"
        );
        assert!(
            !body.contains("content/"),
            "content belongs to its own component"
        );
    }

    #[test]
    fn emitting_leaves_no_scratch_archive_behind() {
        let staged = staged_layout();
        let out = TempDir::new().expect("output");
        emit(
            BuiltComponent::Planet,
            staged.path(),
            out.path(),
            "x86_64-unknown-linux-gnu",
        );

        let leftovers: Vec<_> = fs::read_dir(out.path())
            .expect("read output")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "scratch files remain: {leftovers:?}");
    }

    #[test]
    fn short_digest_is_the_first_sixteen_bytes_in_hex() {
        let digest: Vec<u8> = (0u8..32).collect();
        let short = short_digest(&digest);
        assert_eq!(short.len(), 32, "128 bits render as 32 hex characters");
        assert!(short.starts_with("000102030405060708090a0b0c0d0e0f"));
    }
}
