//! A path-independent index from an install's identity to its in-flight
//! transaction.
//!
//! A macOS bundle cannot keep its journal or its backups inside the `.app` —
//! `codesign --verify --strict` refuses a bundle holding anything its seal does
//! not cover — so they wait in a sidecar beside the `.app`, under the bundle's
//! *name* ([`crate::apply::InstallLayout::work_dir`]). That sidecar does not
//! follow the bundle. A rename is still reachable, because the sidecar is a
//! sibling of the one the renamed bundle now derives; a bundle **moved to a
//! different directory** derives a namespace that never held anything, and
//! nothing it can compute from its own path leads back.
//!
//! This registry is the one place recovery can look that no install path is
//! derived from. Entries are keyed by [`InstallIdentity`] and never by
//! pathname, which is what keeps the two failure modes apart:
//!
//! * a *different* install placed at an interrupted one's old path reports a
//!   different identity, so it finds no entry — and the journal's own identity
//!   check refuses it a second time; and
//! * the *same* install found at a new path reports the identity it always did,
//!   so its entry still names the sidecar it left behind.
//!
//! Every operation here is best effort. The registry only widens what recovery
//! can find: losing it costs the moved-bundle case and nothing else, so a
//! failure to write one is never a reason to fail an update.

use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::journal::{sync_directory, InstallIdentity};

/// Below the per-user data directory, so it survives every install path.
const REGISTRY_DIR_NAME: &str = "update-recovery";

const ENTRY_SCHEMA: u32 = 1;

/// Where one install's interrupted transaction is waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecoveryPointer {
    schema: u32,
    /// The directory holding the journal, the lock and every backup.
    transaction_dir: PathBuf,
    /// The install the transaction belongs to.
    ///
    /// Recorded for diagnosis only: a lookup matches on the identity in the
    /// entry's *name*, and the journal it finds is checked against the identity
    /// again. Matching on this would reintroduce the pathname test the registry
    /// exists to replace.
    install_root: PathBuf,
}

/// The entry name for an identity, which is the whole key.
///
/// Both variants are plain integers, so the name needs no escaping and stays
/// legible to anyone looking at the directory.
fn entry_name(identity: InstallIdentity) -> String {
    match identity {
        InstallIdentity::Inode { volume, file } => format!("inode-{volume}-{file}.json"),
        InstallIdentity::Created { at } => format!("created-{at}.json"),
    }
}

/// Records where this install's transaction is waiting.
///
/// Durable before it returns, because the crash it exists for is the one that
/// takes the page cache with it.
pub(crate) fn record(
    registry: &Path,
    identity: InstallIdentity,
    transaction_dir: &Path,
    install_root: &Path,
) -> std::io::Result<()> {
    std::fs::create_dir_all(registry)?;
    let pointer = RecoveryPointer {
        schema: ENTRY_SCHEMA,
        transaction_dir: transaction_dir.to_path_buf(),
        install_root: install_root.to_path_buf(),
    };
    let bytes = serde_json::to_vec_pretty(&pointer)
        .map_err(|source| std::io::Error::new(std::io::ErrorKind::InvalidData, source))?;
    let path = registry.join(entry_name(identity));
    let temporary = registry.join(format!(
        ".{}.tmp-{}",
        entry_name(identity),
        std::process::id()
    ));

    let mut file = std::fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    std::fs::rename(&temporary, &path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })?;
    sync_directory(registry)
}

/// The transaction directory this install last recorded, if any.
///
/// A malformed or newer entry reads as absent: the registry is a hint, and
/// declining to use one leaves recovery exactly where it was without it.
pub(crate) fn locate(registry: &Path, identity: InstallIdentity) -> Option<PathBuf> {
    let bytes = std::fs::read(registry.join(entry_name(identity))).ok()?;
    let pointer: RecoveryPointer = serde_json::from_slice(&bytes).ok()?;
    (pointer.schema == ENTRY_SCHEMA).then_some(pointer.transaction_dir)
}

/// Drops this install's entry, which is what finishing or adopting a
/// transaction means. Absence is success.
pub(crate) fn forget(registry: &Path, identity: InstallIdentity) {
    let path = registry.join(entry_name(identity));
    match std::fs::remove_file(&path) {
        Ok(()) => {
            let _ = sync_directory(registry);
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            tracing::warn!(?source, path = %path.display(), "could not drop the update recovery entry")
        }
    }
}

/// The per-user registry directory this install records into.
///
/// Deliberately **not** `AppPaths::user_data_dir`: startup recovery runs before
/// path discovery, and that directory can itself be relocated by the config's
/// `UserPath` — which would make the one location that must not move a
/// configurable one. `None` where the environment names no home directory,
/// which leaves recovery on the sidecar searches alone.
///
/// A **test build resolves nothing**. A unit test reaching this would write
/// into the developer's real per-user directory, and on a runner into whatever
/// `HOME` happens to be — outside the tree it created either way. Tests name a
/// registry explicitly through [`crate::apply::InstallLayout::with_recovery_registry`],
/// and the resolution itself is pinned below through [`dir_from`].
pub(crate) fn default_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    dir_from(|name| std::env::var_os(name))
}

/// [`default_dir`] over an explicit environment, which is what makes the
/// platform mapping testable without touching the host's own.
fn dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    product_data_dir_from(var).map(|product| product.join(REGISTRY_DIR_NAME))
}

#[cfg(target_os = "windows")]
fn product_data_dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    var("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .or_else(|| var("APPDATA").filter(|value| !value.is_empty()))
        .map(|value| PathBuf::from(value).join(clonk_platform::PRODUCT_NAME))
}

#[cfg(target_os = "macos")]
fn product_data_dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    var("HOME").filter(|value| !value.is_empty()).map(|value| {
        PathBuf::from(value)
            .join("Library/Application Support")
            .join(clonk_platform::PRODUCT_NAME)
    })
}

#[cfg(all(unix, not(target_os = "macos")))]
fn product_data_dir_from(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    var("XDG_DATA_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            var("HOME")
                .filter(|value| !value.is_empty())
                .map(|value| PathBuf::from(value).join(".local/share"))
        })
        .map(|data| data.join(clonk_platform::PRODUCT_SLUG))
}

#[cfg(not(any(unix, windows)))]
fn product_data_dir_from(_var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const INSTALL: InstallIdentity = InstallIdentity::Inode {
        volume: 7,
        file: 42,
    };

    #[test]
    fn a_recorded_pointer_is_found_by_identity() {
        let directory = TempDir::new().expect("registry parent");
        let registry = directory.path().join("update-recovery");
        record(
            &registry,
            INSTALL,
            Path::new("/Volumes/Data/.clonk-update/Clonk.app"),
            Path::new("/Volumes/Data/Clonk.app"),
        )
        .expect("record the pointer");

        assert_eq!(
            locate(&registry, INSTALL),
            Some(PathBuf::from("/Volumes/Data/.clonk-update/Clonk.app"))
        );
    }

    #[test]
    fn another_install_finds_nothing_of_its_own() {
        // The whole point of keying by identity: a second install never reads
        // the first one's entry, however its path relates to it.
        let directory = TempDir::new().expect("registry parent");
        let registry = directory.path().join("update-recovery");
        record(
            &registry,
            INSTALL,
            Path::new("/Volumes/Data/.clonk-update/Clonk.app"),
            Path::new("/Volumes/Data/Clonk.app"),
        )
        .expect("record the pointer");

        assert_eq!(
            locate(
                &registry,
                InstallIdentity::Inode {
                    volume: 7,
                    file: 43
                }
            ),
            None
        );
    }

    #[test]
    fn forgetting_is_idempotent() {
        let directory = TempDir::new().expect("registry parent");
        let registry = directory.path().join("update-recovery");
        record(
            &registry,
            INSTALL,
            Path::new("/Volumes/Data/.clonk-update/Clonk.app"),
            Path::new("/Volumes/Data/Clonk.app"),
        )
        .expect("record the pointer");

        forget(&registry, INSTALL);
        assert_eq!(locate(&registry, INSTALL), None);
        forget(&registry, INSTALL);
    }

    #[test]
    fn an_entry_from_a_newer_client_reads_as_absent() {
        // Recovery without the registry is correct, just narrower, so an entry
        // this build cannot interpret must not become a guess.
        let directory = TempDir::new().expect("registry parent");
        let registry = directory.path().join("update-recovery");
        std::fs::create_dir_all(&registry).expect("registry");
        std::fs::write(
            registry.join(entry_name(INSTALL)),
            br#"{"schema":2,"transaction_dir":"/elsewhere","install_root":"/elsewhere"}"#,
        )
        .expect("write a newer entry");

        assert_eq!(locate(&registry, INSTALL), None);
    }

    #[test]
    fn a_test_build_resolves_no_per_user_directory() {
        // The guard that keeps a unit test from writing into the developer's
        // own home. Everything here names a registry of its own.
        assert_eq!(default_dir(), None);
    }

    #[test]
    fn the_registry_sits_under_this_product_s_per_user_directory() {
        let home = PathBuf::from("/home/player");
        let resolved = dir_from(|name| match name {
            "HOME" => Some(OsString::from(&home)),
            "LOCALAPPDATA" | "APPDATA" => Some(OsString::from("C:\\Users\\player\\AppData\\Local")),
            _ => None,
        })
        .expect("a named home resolves a registry");

        assert!(
            resolved.ends_with(REGISTRY_DIR_NAME),
            "{} does not end with {REGISTRY_DIR_NAME}",
            resolved.display()
        );
        assert!(
            resolved
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|product| product
                    == std::ffi::OsStr::new(clonk_platform::PRODUCT_NAME)
                    || product == std::ffi::OsStr::new(clonk_platform::PRODUCT_SLUG)),
            "{} is not below this product's directory",
            resolved.display()
        );
    }

    #[test]
    fn an_environment_naming_no_home_resolves_nothing() {
        // Recovery without a registry is the behaviour every build had before
        // it, so an unnameable one declines rather than inventing a path.
        assert_eq!(dir_from(|_| None), None);
        assert_eq!(dir_from(|_| Some(OsString::new())), None);
    }

    #[test]
    fn recording_twice_replaces_the_pointer_and_leaves_no_temporary() {
        let directory = TempDir::new().expect("registry parent");
        let registry = directory.path().join("update-recovery");
        record(
            &registry,
            INSTALL,
            Path::new("/first/.clonk-update/Clonk.app"),
            Path::new("/first/Clonk.app"),
        )
        .expect("record the first pointer");
        record(
            &registry,
            INSTALL,
            Path::new("/second/.clonk-update/Clonk.app"),
            Path::new("/second/Clonk.app"),
        )
        .expect("record the second pointer");

        assert_eq!(
            locate(&registry, INSTALL),
            Some(PathBuf::from("/second/.clonk-update/Clonk.app"))
        );
        assert_eq!(
            std::fs::read_dir(&registry)
                .expect("read the registry")
                .count(),
            1,
            "the durable write leaves no temporary behind"
        );
    }
}
