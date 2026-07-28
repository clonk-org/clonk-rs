//! The crash-safety record for an apply in progress.
//!
//! Applying an update is a sequence of renames, and a machine can lose power
//! between any two of them. The journal is what turns that from "the install is
//! now whatever it happens to be" into a recoverable state: it names every step,
//! records how far each got, and is durable *before* the rename it describes as
//! well as after it.
//!
//! The invariant the whole design rests on is that `rename` is atomic, so for
//! every step either the destination or its backup exists at every instant.
//! [`crate::apply::resume_interrupted_update`] reads a journal, decides whether
//! to roll forward or back, and drives each step to a definite end.
//!
//! Paths are **derived, never stored**. A step records only which component it
//! belongs to and where that component lands, relative to the install root and
//! always `/`-separated; the staging and backup locations follow from the
//! layout plus the journal's one nonce. Storing them would let a hand-edited or
//! corrupted journal name a directory to delete.

use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

/// The journal's own schema, independent of the manifest's and the installed
/// state's.
pub const JOURNAL_SCHEMA: u32 = 1;

pub const JOURNAL_FILE_NAME: &str = "clonk-update-journal.json";

/// How far one component's swap got.
///
/// Three states, because there are exactly two renames: `dest -> backup` and
/// `staged -> dest`. `BackupMoved` is both "after the first" and "before the
/// second", which is precisely the window where the destination may be absent
/// and the backup is the only copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Pending,
    BackupMoved,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalStep {
    /// `content`, `planet` or `engine`.
    pub component: String,
    /// Where the component lands, relative to the install root, `/`-separated.
    pub destination: String,
    /// Top-level names moved out of the old tree into the staged one because
    /// the new archive does not contain them — a user's own packs, typically.
    ///
    /// Recorded before the moves happen, so a rollback can put every one of
    /// them back without having to guess which entries were ours.
    pub carried: Vec<String>,
    pub state: StepState,
}

impl JournalStep {
    pub fn new(component: &str, destination: &str) -> Self {
        Self {
            component: component.to_string(),
            destination: destination.to_string(),
            carried: Vec::new(),
            state: StepState::Pending,
        }
    }

    /// Resolves this step's destination under `install_root`.
    ///
    /// Fallible because the journal is a file on disk: an edited or corrupted
    /// one must not be able to point a rename at `/`.
    pub fn destination_in(&self, install_root: &Path) -> Result<PathBuf, JournalError> {
        relative_path(&self.destination)
            .map(|relative| install_root.join(relative))
            .ok_or_else(|| JournalError::UnsafeDestination {
                destination: self.destination.clone(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub schema: u32,
    /// The release being applied, recorded so a resume can report what it
    /// finished rather than only that it finished something.
    pub version: String,
    /// Names every staging and backup path of this attempt, so a second
    /// attempt can never collide with the leftovers of the first.
    pub nonce: String,
    pub steps: Vec<JournalStep>,
}

#[derive(Debug, Error)]
pub enum JournalError {
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
    #[error("{path} is not a readable update journal: {source}")]
    Malformed {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("{path} records schema {found}; this build understands {JOURNAL_SCHEMA}")]
    UnsupportedSchema { path: PathBuf, found: u32 },
    #[error("update journal names destination {destination:?}, which is not inside the install")]
    UnsafeDestination { destination: String },
}

/// Reads nothing but the schema, so a newer journal is refused on its version
/// rather than on whichever field it happened to reshape.
#[derive(Deserialize)]
struct SchemaProbe {
    schema: u32,
}

/// Turns a journalled `/`-separated destination into a relative path, or
/// refuses it.
///
/// Platform-independent on purpose: a name that is relative on Linux but
/// absolute on Windows must be refused on both, and every segment has to be an
/// ordinary one so joining it can never escape the install root.
fn relative_path(text: &str) -> Option<PathBuf> {
    let rejected = |segment: &str| {
        segment.is_empty()
            || matches!(segment, "." | "..")
            || segment.contains('\\')
            || segment.contains(':')
    };
    if text.is_empty() || text.split('/').any(rejected) {
        return None;
    }
    let path: PathBuf = text.split('/').collect();
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(path)
}

/// The staging partner for an atomic save: a sibling of the journal, because
/// `rename` is atomic only within one filesystem.
fn temporary_path_in(directory: &Path) -> PathBuf {
    directory.join(format!(".{JOURNAL_FILE_NAME}.tmp-{}", std::process::id()))
}

/// Makes a directory's own contents durable.
///
/// Renaming the temporary over the journal is atomic, but the *directory entry*
/// that publishes it is only guaranteed to survive a power cut once the
/// directory itself is synced. Windows has no equivalent handle to sync — a
/// directory cannot be opened as a file without backup semantics — and its
/// rename already carries the metadata write, so the sync is unix-only.
#[cfg(unix)]
fn sync_directory(directory: &Path) -> std::io::Result<()> {
    std::fs::File::open(directory)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_directory: &Path) -> std::io::Result<()> {
    Ok(())
}

impl Journal {
    pub fn new(version: &str, nonce: &str, steps: Vec<JournalStep>) -> Self {
        Self {
            schema: JOURNAL_SCHEMA,
            version: version.to_string(),
            nonce: nonce.to_string(),
            steps,
        }
    }

    pub fn path_in(directory: &Path) -> PathBuf {
        directory.join(JOURNAL_FILE_NAME)
    }

    /// Reads the journal, or `None` when no apply was interrupted — which is
    /// the case on every ordinary launch. An unreadable one is an error rather
    /// than a silent `None`: recovering from a journal we could not read is
    /// exactly the situation where guessing destroys an install.
    pub fn load(directory: &Path) -> Result<Option<Self>, JournalError> {
        let path = Self::path_in(directory);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => return Err(JournalError::Read { path, source }),
        };
        let malformed = |source| JournalError::Malformed {
            path: path.clone(),
            source,
        };
        let probe: SchemaProbe = serde_json::from_slice(&bytes).map_err(malformed)?;
        (probe.schema == JOURNAL_SCHEMA)
            .then_some(())
            .ok_or_else(|| JournalError::UnsupportedSchema {
                path: path.clone(),
                found: probe.schema,
            })?;
        let journal: Self = serde_json::from_slice(&bytes).map_err(malformed)?;
        // Validated on the way in, so no later code has to remember to.
        journal
            .steps
            .iter()
            .try_for_each(|step| step.destination_in(Path::new("")).map(|_| ()))?;
        Ok(Some(journal))
    }

    /// Writes the journal durably: a full temporary beside the target, synced,
    /// renamed over it, and the directory synced in turn.
    ///
    /// Truncating in place would leave a half-written journal exactly when the
    /// machine died mid-apply, which is the one moment its contents matter.
    pub fn save(&self, directory: &Path) -> Result<(), JournalError> {
        let temporary = temporary_path_in(directory);
        let path = Self::path_in(directory);
        let bytes = serde_json::to_vec_pretty(self).map_err(|source| JournalError::Malformed {
            path: path.clone(),
            source,
        })?;

        let write = |target: &Path| -> Result<(), std::io::Error> {
            let mut file = std::fs::File::create(target)?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            // Durable before the rename, so the rename can never publish a
            // name pointing at unflushed content.
            file.sync_all()
        };
        write(&temporary).map_err(|source| JournalError::Write {
            path: temporary.clone(),
            source,
        })?;

        std::fs::rename(&temporary, &path).map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            JournalError::Write {
                path: path.clone(),
                source,
            }
        })?;
        sync_directory(directory).map_err(|source| JournalError::Write { path, source })
    }

    /// Removes the journal, which is what declares the apply finished.
    ///
    /// Absence is success: a resume that got as far as deleting it and then
    /// died has nothing left to do.
    pub fn remove(directory: &Path) -> Result<(), JournalError> {
        let path = Self::path_in(directory);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                sync_directory(directory).map_err(|source| JournalError::Write { path, source })
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(JournalError::Write { path, source }),
        }
    }

    /// Whether any step got far enough that rolling back would undo work the
    /// user's install already depends on.
    pub fn any_step_completed(&self) -> bool {
        self.steps
            .iter()
            .any(|step| step.state == StepState::Completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn journal() -> Journal {
        Journal::new(
            "0.4.0",
            "beef1234",
            vec![
                JournalStep {
                    component: "content".to_string(),
                    destination: "content".to_string(),
                    carried: vec!["MyPack.c4f".to_string()],
                    state: StepState::Completed,
                },
                JournalStep::new("engine", "bin"),
            ],
        )
    }

    #[test]
    fn a_saved_journal_round_trips() {
        let directory = TempDir::new().expect("directory");
        journal().save(directory.path()).expect("save");
        assert_eq!(
            Journal::load(directory.path()).expect("load"),
            Some(journal())
        );
    }

    #[test]
    fn a_missing_journal_means_no_apply_was_interrupted() {
        // The overwhelmingly common case: every launch that did not crash
        // mid-update reads no journal, and that is not a failure.
        let directory = TempDir::new().expect("directory");
        assert_eq!(Journal::load(directory.path()).expect("load"), None);
    }

    #[test]
    fn the_three_states_serialize_under_their_documented_names() {
        // The journal is read by a *different* build after a crash — the one
        // the update installed — so the wire names are part of the contract.
        let directory = TempDir::new().expect("directory");
        let mut written = journal();
        written.steps[1].state = StepState::BackupMoved;
        written.save(directory.path()).expect("save");

        let text = std::fs::read_to_string(Journal::path_in(directory.path())).expect("read");
        assert!(text.contains("\"completed\""), "{text}");
        assert!(text.contains("\"backup_moved\""), "{text}");
        assert!(!text.contains("Completed"), "{text}");
    }

    #[test]
    fn saving_leaves_no_temporary_behind() {
        let directory = TempDir::new().expect("directory");
        journal().save(directory.path()).expect("first save");
        journal().save(directory.path()).expect("second save");

        let names: Vec<_> = std::fs::read_dir(directory.path())
            .expect("read directory")
            .map(|entry| entry.expect("entry").file_name())
            .collect();
        assert_eq!(names, [JOURNAL_FILE_NAME]);
    }

    #[test]
    fn the_temporary_is_a_sibling_of_the_journal() {
        // `rename` is atomic only within a filesystem, and an install root can
        // be a separate mount from the system temp directory.
        let directory = TempDir::new().expect("directory");
        let temporary = temporary_path_in(directory.path());
        assert_eq!(temporary.parent(), Some(directory.path()));
        assert_ne!(temporary.file_name(), Some(JOURNAL_FILE_NAME.as_ref()));
    }

    #[test]
    fn a_destination_that_climbs_out_of_the_install_is_refused() {
        // The journal is a file on disk that drives renames and deletions, so
        // it is treated as untrusted input like any other.
        for destination in [
            "..",
            "../evil",
            "/etc",
            "content/../../evil",
            "content\\..\\evil",
            "C:\\Windows",
            "./content",
            "",
        ] {
            let step = JournalStep::new("content", destination);
            assert!(
                matches!(
                    step.destination_in(Path::new("/install")),
                    Err(JournalError::UnsafeDestination { .. })
                ),
                "{destination:?} should not resolve to a destination"
            );
        }
    }

    #[test]
    fn a_journal_naming_an_unsafe_destination_is_refused_on_load() {
        let directory = TempDir::new().expect("directory");
        let mut written = journal();
        written.steps[0].destination = "../elsewhere".to_string();
        written.save(directory.path()).expect("save");
        assert!(matches!(
            Journal::load(directory.path()),
            Err(JournalError::UnsafeDestination { .. })
        ));
    }

    #[test]
    fn a_safe_destination_resolves_under_the_install_root() {
        let step = JournalStep::new("content", "Contents/Resources/content");
        assert_eq!(
            step.destination_in(Path::new("/install")).expect("resolve"),
            Path::new("/install/Contents/Resources/content")
        );
    }

    #[test]
    fn a_corrupt_journal_is_reported_rather_than_treated_as_absent() {
        // Reading it as "nothing was interrupted" would leave a half-applied
        // install permanently half-applied, with no message saying why.
        let directory = TempDir::new().expect("directory");
        std::fs::write(Journal::path_in(directory.path()), b"{ not json").expect("write");
        assert!(matches!(
            Journal::load(directory.path()),
            Err(JournalError::Malformed { .. })
        ));
    }

    #[test]
    fn a_journal_from_a_newer_client_is_refused() {
        let directory = TempDir::new().expect("directory");
        std::fs::write(
            Journal::path_in(directory.path()),
            br#"{"schema": 9, "version": "9.0.0", "nonce": "x", "steps": []}"#,
        )
        .expect("write");
        assert!(matches!(
            Journal::load(directory.path()),
            Err(JournalError::UnsupportedSchema { found: 9, .. })
        ));
    }

    #[test]
    fn removing_a_journal_that_is_not_there_is_not_a_failure() {
        // Resume has to be safe to run twice, and the second run finds the
        // journal already gone.
        let directory = TempDir::new().expect("directory");
        Journal::remove(directory.path()).expect("first remove");
        journal().save(directory.path()).expect("save");
        Journal::remove(directory.path()).expect("second remove");
        Journal::remove(directory.path()).expect("third remove");
        assert_eq!(Journal::load(directory.path()).expect("load"), None);
    }

    #[test]
    fn a_completed_step_is_what_makes_a_resume_roll_forward() {
        let mut journal = journal();
        assert!(journal.any_step_completed());
        journal.steps[0].state = StepState::BackupMoved;
        assert!(!journal.any_step_completed());
    }
}
