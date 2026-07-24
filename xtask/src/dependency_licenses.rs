use std::fs;
use std::path::Path;

use anyhow::{bail, Context, Result};

const LOCK_FINGERPRINT_LABEL: &str = "Cargo.lock FNV-1a 64: ";
const INPUT_FINGERPRINT_LABEL: &str = "Release dependency inputs FNV-1a 64: ";
const SOURCE_SCOPE: &str = "Project source: ISC";
const CONTENT_SCOPE: &str = "Game content: separate CC BY-NC terms";
const DEPENDENCY_SCOPE: &str = "Third-party Rust crates: their respective licenses below";

/// Validates the committed dependency notice before it is copied into a binary
/// package.
///
/// Notice generation is intentionally kept out of the workspace dependency
/// graph. `scripts/generate-rust-dependency-notices.sh` uses a pinned external
/// cargo-about release and records stable fingerprints of Cargo.lock, the
/// workspace manifests, and the generator configuration. This inexpensive
/// check prevents a dependency or feature-graph update from shipping with a
/// stale notice without making the release build depend on cargo-about itself.
pub(crate) fn validate_runtime_dependency_notices(
    workspace_dir: &Path,
    notice_path: &Path,
) -> Result<()> {
    let lock_path = workspace_dir.join("Cargo.lock");
    let lock = fs::read(&lock_path)
        .with_context(|| format!("failed to read dependency lockfile {}", lock_path.display()))?;
    let notice = fs::read_to_string(notice_path)
        .with_context(|| format!("failed to read dependency notice {}", notice_path.display()))?;

    for required_scope in [SOURCE_SCOPE, CONTENT_SCOPE, DEPENDENCY_SCOPE] {
        if !notice.contains(required_scope) {
            bail!(
                "dependency notice {} is missing required license-scope marker `{required_scope}`",
                notice_path.display()
            );
        }
    }
    for escaped in ["&quot;", "&#x27;", "&lt;", "&gt;", "&amp;"] {
        if notice.contains(escaped) {
            bail!(
                "dependency notice {} contains HTML-escaped legal text `{escaped}`; regenerate it with the plain-text template",
                notice_path.display()
            );
        }
    }

    let recorded_lock = notice
        .lines()
        .find_map(|line| line.strip_prefix(LOCK_FINGERPRINT_LABEL))
        .context("dependency notice does not record a Cargo.lock fingerprint")?;
    let expected_lock = fingerprint_bytes(&lock);
    if recorded_lock != expected_lock {
        bail!(
            "dependency notice {} is stale for Cargo.lock (recorded {recorded_lock}, expected {expected_lock}); regenerate it with scripts/generate-rust-dependency-notices.sh",
            notice_path.display()
        );
    }

    let recorded_inputs = notice
        .lines()
        .find_map(|line| line.strip_prefix(INPUT_FINGERPRINT_LABEL))
        .context("dependency notice does not record a release-input fingerprint")?;
    let expected_inputs = release_input_fingerprint(workspace_dir)?;
    if recorded_inputs != expected_inputs {
        bail!(
            "dependency notice {} is stale for the release dependency inputs (recorded {recorded_inputs}, expected {expected_inputs}); regenerate it with scripts/generate-rust-dependency-notices.sh",
            notice_path.display()
        );
    }

    Ok(())
}

fn release_input_fingerprint(workspace_dir: &Path) -> Result<String> {
    let mut relative_paths = [
        "Cargo.lock",
        "Cargo.toml",
        "about.toml",
        "scripts/generate-rust-dependency-notices.sh",
        "scripts/rust-dependency-notices.hbs",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .collect::<Vec<_>>();
    for directory in ["crates", "third_party"] {
        collect_cargo_manifests(
            workspace_dir,
            &workspace_dir.join(directory),
            &mut relative_paths,
        )?;
    }
    relative_paths.sort_by_cached_key(|path| path.to_string_lossy().replace('\\', "/"));
    relative_paths.dedup();

    let hash = relative_paths.into_iter().try_fold(
        0xcbf29ce484222325_u64,
        |mut hash, relative| -> Result<u64> {
            let path_text = relative.to_string_lossy().replace('\\', "/");
            hash = update_fnv(hash, path_text.as_bytes());
            hash = update_fnv(hash, &[0]);
            let path = workspace_dir.join(&relative);
            let contents = fs::read(&path)
                .with_context(|| format!("failed to read release input {}", path.display()))?;
            hash = update_fnv(hash, &contents);
            Ok(update_fnv(hash, &[0]))
        },
    )?;
    Ok(format!("{hash:016x}"))
}

fn collect_cargo_manifests(
    workspace_dir: &Path,
    directory: &Path,
    relative_paths: &mut Vec<std::path::PathBuf>,
) -> Result<()> {
    if !directory.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(directory)
        .with_context(|| format!("failed to enumerate {}", directory.display()))?
    {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            collect_cargo_manifests(workspace_dir, &entry.path(), relative_paths)?;
        } else if file_type.is_file() && entry.file_name() == "Cargo.toml" {
            relative_paths.push(
                entry
                    .path()
                    .strip_prefix(workspace_dir)
                    .context("Cargo manifest was outside the workspace")?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

fn fingerprint_bytes(bytes: &[u8]) -> String {
    // FNV-1a is not a security boundary. It is a small, dependency-free drift
    // detector whose result is stable across platforms and Rust releases.
    let hash = update_fnv(0xcbf29ce484222325_u64, bytes);
    format!("{hash:016x}")
}

fn update_fnv(hash: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(hash, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_workspace_inputs(root: &Path, lock: &[u8]) {
        for (relative, contents) in [
            ("Cargo.lock", lock),
            ("Cargo.toml", b"[workspace]\n"),
            ("about.toml", b"accepted = [\"MIT\"]\n"),
            (
                "scripts/generate-rust-dependency-notices.sh",
                b"#!/usr/bin/env bash\n",
            ),
            ("scripts/rust-dependency-notices.hbs", b"{{{text}}}\n"),
            (
                "crates/runtime/Cargo.toml",
                b"[package]\nname = \"runtime\"\n",
            ),
            (
                "third_party/native/Cargo.toml",
                b"[package]\nname = \"native\"\n",
            ),
        ] {
            let path = root.join(relative);
            fs::create_dir_all(path.parent().expect("input parent"))
                .expect("create input directory");
            fs::write(path, contents).expect("write release input");
        }
    }

    fn write_notice(root: &Path) -> std::path::PathBuf {
        let notice_path = root.join("licenses/RUST_THIRD_PARTY_LICENSES.txt");
        fs::create_dir_all(notice_path.parent().expect("notice parent"))
            .expect("create notice directory");
        let lock = fs::read(root.join("Cargo.lock")).expect("read lockfile");
        fs::write(
            &notice_path,
            format!(
                "Clonk Rust dependency notices\n\
                 {LOCK_FINGERPRINT_LABEL}{}\n\
                 {INPUT_FINGERPRINT_LABEL}{}\n\
                 {SOURCE_SCOPE}\n\
                 {CONTENT_SCOPE}\n\
                 {DEPENDENCY_SCOPE}\n",
                fingerprint_bytes(&lock),
                release_input_fingerprint(root).expect("fingerprint release inputs"),
            ),
        )
        .expect("write notice");
        notice_path
    }

    #[test]
    fn accepts_notice_for_current_lockfile_and_separate_license_scopes() {
        let temp = TempDir::new().expect("temporary workspace");
        let lock = b"version = 4\n";
        write_workspace_inputs(temp.path(), lock);
        let notice_path = write_notice(temp.path());

        validate_runtime_dependency_notices(temp.path(), &notice_path)
            .expect("current notice must validate");
    }

    #[test]
    fn rejects_notice_after_dependency_lockfile_changes() {
        let temp = TempDir::new().expect("temporary workspace");
        write_workspace_inputs(temp.path(), b"version = 3\n");
        let notice_path = write_notice(temp.path());
        fs::write(temp.path().join("Cargo.lock"), b"version = 4\n").expect("update lockfile");

        let error = validate_runtime_dependency_notices(temp.path(), &notice_path)
            .expect_err("stale notice must fail");

        assert!(error.to_string().contains("is stale for Cargo.lock"));
        assert!(error
            .to_string()
            .contains("generate-rust-dependency-notices.sh"));
    }

    #[test]
    fn rejects_notice_that_blurs_project_and_content_licenses() {
        let temp = TempDir::new().expect("temporary workspace");
        let lock = b"version = 4\n";
        write_workspace_inputs(temp.path(), lock);
        let notice_path = write_notice(temp.path());
        let notice = fs::read_to_string(&notice_path).expect("read notice");
        fs::write(
            &notice_path,
            notice.replace(CONTENT_SCOPE, "All content: ISC"),
        )
        .expect("rewrite notice");

        let error = validate_runtime_dependency_notices(temp.path(), &notice_path)
            .expect_err("ambiguous notice must fail");

        assert!(error.to_string().contains("license-scope marker"));
        assert!(error.to_string().contains(CONTENT_SCOPE));
    }

    #[test]
    fn rejects_html_escaped_legal_text() {
        let temp = TempDir::new().expect("temporary workspace");
        let lock = b"version = 4\n";
        write_workspace_inputs(temp.path(), lock);
        let notice_path = write_notice(temp.path());
        let mut notice = fs::read_to_string(&notice_path).expect("read notice");
        notice.push_str("the &quot;License&quot;\n");
        fs::write(&notice_path, notice).expect("rewrite notice");

        let error = validate_runtime_dependency_notices(temp.path(), &notice_path)
            .expect_err("escaped notice must fail");

        assert!(error.to_string().contains("HTML-escaped legal text"));
    }

    #[test]
    fn rejects_notice_after_runtime_feature_graph_changes_without_lockfile_change() {
        let temp = TempDir::new().expect("temporary workspace");
        write_workspace_inputs(temp.path(), b"version = 4\n");
        let notice_path = write_notice(temp.path());
        fs::write(
            temp.path().join("crates/runtime/Cargo.toml"),
            b"[package]\nname = \"runtime\"\n[features]\nbundled = []\n",
        )
        .expect("update runtime manifest");

        let error = validate_runtime_dependency_notices(temp.path(), &notice_path)
            .expect_err("manifest drift must fail");

        assert!(error
            .to_string()
            .contains("stale for the release dependency inputs"));
    }

    #[test]
    fn committed_notice_matches_workspace_release_inputs() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask is inside the workspace");
        let notice = workspace.join("licenses/RUST_THIRD_PARTY_LICENSES.txt");

        validate_runtime_dependency_notices(workspace, &notice)
            .expect("committed dependency notice must be current");
    }
}
