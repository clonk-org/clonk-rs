//! What is actually installed, recorded beside the install root.
//!
//! The updater compares the manifest's per-component digests against this file
//! to decide what actually has to be downloaded, so a release that changes only
//! the engine never re-fetches 299 MB of content.
//!
//! It lives in the install root rather than the user data directory because it
//! describes the *installation*: two installs sharing one profile must not
//! share it, and reinstalling by replacing the tree must not leave a stale
//! record behind.

use clonk_platform::AppPaths;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// The state file's own schema, independent of the manifest's.
pub const STATE_SCHEMA: u32 = 1;

pub const STATE_FILE_NAME: &str = "clonk-rust-installed.json";

/// What a single component is at, right now, on disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledComponent {
    /// The release this component's bytes came from.
    pub version: String,
    /// SHA-256 of the archive that produced them, so the next manifest can be
    /// compared without re-hashing the unpacked tree.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledState {
    pub schema: u32,
    /// `BTreeMap` so the file's bytes do not depend on hash iteration order —
    /// a state file that churns on every save is noise in backups and diffs.
    pub components: BTreeMap<String, InstalledComponent>,
}

impl Default for InstalledState {
    fn default() -> Self {
        Self {
            schema: STATE_SCHEMA,
            components: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("failed to read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not readable installed-component state: {source}")]
    Malformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} records schema {found}; this build understands {STATE_SCHEMA}")]
    UnsupportedSchema { path: PathBuf, found: u32 },
    #[error("{path} does not contain an installed-state object")]
    InvalidRoot { path: PathBuf },
}

/// Reads nothing but the schema, so a newer state file is refused on its
/// version rather than on whichever field it happened to reshape.
#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

/// Parsed state plus the exact bytes that represented it on disk.
///
/// Apply rollback needs both: the parsed value produces the next state, while
/// restoring the original bytes preserves a macOS bundle's existing seal even
/// when the JSON used different whitespace or carried fields this build does
/// not know yet.
#[derive(Debug, Clone)]
pub(crate) struct InstalledStateSnapshot {
    pub state: Option<InstalledState>,
    raw: Option<Vec<u8>>,
}

/// The staging partner for an atomic save.
///
/// A sibling of the state file, because `rename` is atomic only within one
/// filesystem; the process id keeps two concurrent writers off each other's
/// temporary, and the leading dot keeps it out of casual directory listings.
fn temporary_path_in(install_root: &Path) -> PathBuf {
    install_root.join(format!(".{STATE_FILE_NAME}.tmp-{}", std::process::id()))
}

fn temporary_prefix() -> String {
    format!(".{STATE_FILE_NAME}.tmp-")
}

#[cfg(unix)]
fn sync_directory(directory: &Path) -> std::io::Result<()> {
    std::fs::File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> std::io::Result<()> {
    Ok(())
}

fn write_atomic(install_root: &Path, bytes: &[u8]) -> Result<(), StateError> {
    discard_temporaries(install_root)?;
    let temporary = temporary_path_in(install_root);
    let write = |path: &Path| -> Result<(), std::io::Error> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    if let Err(source) = write(&temporary) {
        let _ = std::fs::remove_file(&temporary);
        return Err(StateError::Write {
            path: temporary,
            source,
        });
    }

    let path = InstalledState::path_in(install_root);
    std::fs::rename(&temporary, &path).map_err(|source| {
        let _ = std::fs::remove_file(&temporary);
        StateError::Write {
            path: path.clone(),
            source,
        }
    })?;
    sync_directory(install_root).map_err(|source| StateError::Write { path, source })
}

fn discard_temporaries(install_root: &Path) -> Result<(), StateError> {
    let prefix = temporary_prefix();
    let listing = std::fs::read_dir(install_root).map_err(|source| StateError::Read {
        path: install_root.to_path_buf(),
        source,
    })?;
    let mut removed = false;
    for entry in listing {
        let entry = entry.map_err(|source| StateError::Read {
            path: install_root.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(pid) = name.strip_prefix(&prefix) else {
            continue;
        };
        if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
            continue;
        }
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path).map_err(|source| StateError::Read {
            path: path.clone(),
            source,
        })?;
        if metadata.is_dir() {
            continue;
        }
        std::fs::remove_file(&path).map_err(|source| StateError::Write {
            path: path.clone(),
            source,
        })?;
        removed = true;
    }
    if removed {
        sync_directory(install_root).map_err(|source| StateError::Write {
            path: install_root.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

impl InstalledStateSnapshot {
    pub(crate) fn raw(&self) -> Option<&[u8]> {
        self.raw.as_deref()
    }

    pub fn restore(&self, install_root: &Path) -> Result<(), StateError> {
        InstalledState::restore_bytes(install_root, self.raw())
    }
}

impl InstalledState {
    pub fn path_in(install_root: &Path) -> PathBuf {
        install_root.join(STATE_FILE_NAME)
    }

    /// The state file for a discovered installation.
    pub fn path_for(paths: &AppPaths) -> PathBuf {
        Self::path_in(paths.install_root())
    }

    /// Reads the recorded state, or `None` when the installation predates the
    /// updater. An unreadable file is an error, never a silent `None`.
    pub fn load(install_root: &Path) -> Result<Option<Self>, StateError> {
        Self::load_snapshot(install_root).map(|snapshot| snapshot.state)
    }

    pub(crate) fn load_snapshot(install_root: &Path) -> Result<InstalledStateSnapshot, StateError> {
        let path = Self::path_in(install_root);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Ok(InstalledStateSnapshot {
                    state: None,
                    raw: None,
                });
            }
            Err(source) => return Err(StateError::Read { path, source }),
        };
        let malformed = |source| StateError::Malformed {
            path: path.clone(),
            source,
        };
        let probe: SchemaProbe = serde_json::from_slice(&bytes).map_err(malformed)?;
        (probe.schema == STATE_SCHEMA)
            .then_some(())
            .ok_or_else(|| StateError::UnsupportedSchema {
                path: path.clone(),
                found: probe.schema,
            })?;
        let state = serde_json::from_slice(&bytes).map_err(malformed)?;
        Ok(InstalledStateSnapshot {
            state: Some(state),
            raw: Some(bytes),
        })
    }

    /// Restores exact journalled bytes, or exact absence, without normalizing
    /// whitespace or discarding fields this build does not know.
    pub(crate) fn restore_bytes(
        install_root: &Path,
        expected: Option<&[u8]>,
    ) -> Result<(), StateError> {
        discard_temporaries(install_root)?;
        let path = Self::path_in(install_root);
        match (expected, std::fs::read(&path)) {
            (Some(expected), Ok(current)) if expected == current => return Ok(()),
            (None, Err(source)) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            _ => {}
        }
        expected.map_or_else(
            || Self::remove(install_root),
            |bytes| {
                let malformed = |source| StateError::Malformed {
                    path: path.clone(),
                    source,
                };
                let probe: SchemaProbe = serde_json::from_slice(bytes).map_err(malformed)?;
                (probe.schema == STATE_SCHEMA)
                    .then_some(())
                    .ok_or_else(|| StateError::UnsupportedSchema {
                        path: path.clone(),
                        found: probe.schema,
                    })?;
                serde_json::from_slice::<Self>(bytes).map_err(malformed)?;
                write_atomic(install_root, bytes)
            },
        )
    }

    /// Writes the state atomically: a full temporary beside the target, then a
    /// rename over it.
    ///
    /// Truncating in place would leave a half-written file if the machine lost
    /// power mid-apply, and the next launch would refuse to read it — turning a
    /// recoverable interruption into a permanently broken updater.
    pub fn save(&self, install_root: &Path) -> Result<(), StateError> {
        let mut bytes =
            serde_json::to_vec_pretty(self).map_err(|source| StateError::Malformed {
                path: Self::path_in(install_root),
                source,
            })?;
        bytes.push(b'\n');
        write_atomic(install_root, &bytes)
    }

    /// Saves known fields while retaining same-schema extension fields from a
    /// previously validated representation.
    pub(crate) fn save_preserving_unknown(
        &self,
        install_root: &Path,
        previous: Option<&[u8]>,
    ) -> Result<(), StateError> {
        let path = Self::path_in(install_root);
        let malformed = |source| StateError::Malformed {
            path: path.clone(),
            source,
        };
        let mut document = previous
            .map(serde_json::from_slice::<serde_json::Value>)
            .transpose()
            .map_err(malformed)?
            .unwrap_or_else(|| serde_json::Value::Object(serde_json::Map::new()));
        let root = document
            .as_object_mut()
            .ok_or_else(|| StateError::InvalidRoot { path: path.clone() })?;
        root.insert("schema".to_string(), serde_json::Value::from(self.schema));
        let mut previous_components = root
            .remove("components")
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        let components = self
            .components
            .iter()
            .map(|(name, component)| {
                let mut fields = previous_components
                    .remove(name)
                    .and_then(|value| value.as_object().cloned())
                    .unwrap_or_default();
                fields.insert(
                    "version".to_string(),
                    serde_json::Value::String(component.version.clone()),
                );
                fields.insert(
                    "sha256".to_string(),
                    serde_json::Value::String(component.sha256.clone()),
                );
                (name.clone(), serde_json::Value::Object(fields))
            })
            .collect();
        root.insert(
            "components".to_string(),
            serde_json::Value::Object(components),
        );
        let mut bytes = serde_json::to_vec_pretty(&document).map_err(malformed)?;
        bytes.push(b'\n');
        write_atomic(install_root, &bytes)
    }

    /// Removes recorded state durably, treating absence as success.
    pub fn remove(install_root: &Path) -> Result<(), StateError> {
        let path = Self::path_in(install_root);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                sync_directory(install_root).map_err(|source| StateError::Write { path, source })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(StateError::Write { path, source }),
        }
    }

    pub fn component(&self, name: &str) -> Option<&InstalledComponent> {
        self.components.get(name)
    }

    pub fn record(&mut self, name: &str, version: &str, sha256: &str) {
        self.components.insert(
            name.to_string(),
            InstalledComponent {
                version: version.to_string(),
                sha256: sha256.to_string(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn state() -> InstalledState {
        let mut state = InstalledState::default();
        state.record("content", "0.3.0", &"bb".repeat(32));
        state.record("engine", "0.3.0", &"aa".repeat(32));
        state
    }

    #[test]
    fn a_missing_state_file_is_absence_not_failure() {
        // Every build before the updater shipped has no state file, and that
        // is the normal first-run case rather than an error.
        let root = TempDir::new().expect("install root");
        assert_eq!(InstalledState::load(root.path()).expect("load"), None);
    }

    #[test]
    fn a_saved_state_round_trips() {
        let root = TempDir::new().expect("install root");
        state().save(root.path()).expect("save");

        let loaded = InstalledState::load(root.path()).expect("load");
        assert_eq!(loaded, Some(state()));
    }

    #[test]
    fn a_component_records_the_digest_and_the_release_it_came_from() {
        let state = state();
        let content = state.component("content").expect("content");
        assert_eq!(content.version, "0.3.0");
        assert_eq!(content.sha256, "bb".repeat(32));
        assert_eq!(state.component("planet"), None);
    }

    #[test]
    fn saving_leaves_no_temporary_file_behind() {
        let root = TempDir::new().expect("install root");
        state().save(root.path()).expect("first save");
        state().save(root.path()).expect("second save");

        let names: Vec<_> = std::fs::read_dir(root.path())
            .expect("read install root")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [STATE_FILE_NAME]);
    }

    #[test]
    fn removing_state_is_idempotent() {
        let root = TempDir::new().expect("install root");
        state().save(root.path()).expect("save");

        InstalledState::remove(root.path()).expect("first remove");
        InstalledState::remove(root.path()).expect("second remove");

        assert_eq!(InstalledState::load(root.path()).expect("load"), None);
    }

    #[test]
    fn restoring_a_snapshot_removes_a_stranded_atomic_write_temporary() {
        let root = TempDir::new().expect("install root");
        state().save(root.path()).expect("save");
        let snapshot = InstalledState::load_snapshot(root.path()).expect("snapshot");
        let temporary = temporary_path_in(root.path());
        std::fs::write(&temporary, b"partial state").expect("strand temporary");

        snapshot.restore(root.path()).expect("restore");

        assert!(!temporary.exists());
    }

    #[test]
    fn the_temporary_file_is_a_sibling_of_the_state_file() {
        // `rename` is only atomic within a filesystem. Staging in the system
        // temp directory would break on any install root that is a separate
        // mount, so the swap partner has to live next to its target.
        let root = TempDir::new().expect("install root");
        let temporary = temporary_path_in(root.path());
        assert_eq!(temporary.parent(), Some(root.path()));
        assert_ne!(temporary.file_name(), Some(STATE_FILE_NAME.as_ref()));
    }

    #[test]
    fn a_corrupt_state_file_is_reported_rather_than_treated_as_absent() {
        // Silently reading it as "nothing installed" would make the updater
        // re-download every component forever without ever saying why.
        let root = TempDir::new().expect("install root");
        std::fs::write(InstalledState::path_in(root.path()), b"{ not json").expect("write");
        assert!(matches!(
            InstalledState::load(root.path()),
            Err(StateError::Malformed { .. })
        ));
    }

    #[test]
    fn a_state_file_from_a_newer_client_is_refused() {
        let root = TempDir::new().expect("install root");
        std::fs::write(
            InstalledState::path_in(root.path()),
            br#"{"schema": 9, "components": {}}"#,
        )
        .expect("write");
        assert!(matches!(
            InstalledState::load(root.path()),
            Err(StateError::UnsupportedSchema { found: 9, .. })
        ));
    }
}
