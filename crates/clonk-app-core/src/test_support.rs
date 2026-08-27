//! Fixture utilities shared by the extracted app crates' tests.
//!
//! Behind the `test-support` feature, which only dev-dependencies enable, so
//! none of this reaches a production build. It exists because tests that moved
//! out of the app binary carried a copy of the same repository-root walker
//! each, and three copies drift independently when the workspace layout moves
//! (clonk-org/clonk-rs#1260).

use std::path::{Path, PathBuf};

/// The workspace root, resolved from the calling crate's manifest.
///
/// Every extracted crate lives at `crates/<name>`, so the root is two levels
/// up from any of them — including this one, which is why the walk can live
/// here rather than being repeated per crate.
///
/// Panics with the resolved path when it does not look like the workspace
/// root. A silent wrong answer here surfaces much later as a missing
/// `content/` or `planet/` entry, which reads as broken test data rather than
/// a broken path.
pub fn repository_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate = manifest.join("../..");
    let root = candidate.canonicalize().unwrap_or_else(|error| {
        panic!(
            "could not resolve the workspace root from {}: {error}",
            manifest.display()
        )
    });
    assert!(
        is_workspace_root(&root),
        "resolved `{}` from {}, which is not the workspace root: expected a \
         Cargo.toml declaring [workspace] beside `crates/`",
        root.display(),
        manifest.display()
    );
    root
}

/// Whether this directory is the workspace root rather than some ancestor a
/// relocated crate happened to land on.
fn is_workspace_root(root: &Path) -> bool {
    if !root.join("crates").is_dir() {
        return false;
    }
    std::fs::read_to_string(root.join("Cargo.toml"))
        .is_ok_and(|manifest| manifest.contains("[workspace]"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_repository_root_is_the_workspace_manifest_directory() {
        let root = repository_root();
        assert!(is_workspace_root(&root));
        // The two directories the callers actually reach for.
        assert!(
            root.join("planet").is_dir(),
            "planet/ is missing from {}",
            root.display()
        );
        assert!(root.join("crates/clonk-app-core").is_dir());
    }

    #[test]
    fn an_ancestor_above_the_workspace_is_not_mistaken_for_it() {
        let root = repository_root();
        let above = root.parent().expect("the workspace root has a parent");
        assert!(
            !is_workspace_root(above),
            "{} was accepted as a workspace root",
            above.display()
        );
    }
}
