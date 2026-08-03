//! Unpacking a component archive that is assumed hostile.
//!
//! A matching SHA-256 proves the archive is the one the manifest named; it
//! proves nothing about what is inside. Whoever can publish a manifest can
//! publish its digest too, so extraction is written as if the archive were
//! attacker-supplied.
//!
//! Every guard rejects the **whole archive**, not the offending entry. A
//! component is only meaningful complete: silently dropping one entry produces
//! an install that looks successful and is subtly broken, which is worse than a
//! refusal the user can retry. To make that real, extraction stages into a
//! directory this call owns and removes it entirely on any failure, so a
//! partially-written component can never be mistaken for a finished one.
//!
//! Guards, in the order they run:
//!
//! 1. Declared unpacked size against the caller's budget, before any write.
//! 2. Entry paths whose canonical-caseless components would alias a file or
//!    directory on a folding filesystem — also before any write, since a
//!    collision is a property of the archive rather than of the entry that
//!    happens to arrive second.
//! 3. Per entry: our own path rules (no `..`, nothing absolute, no `\` or `:`
//!    smuggling, no empty or `.` segments, and no names Windows rewrites or
//!    reserves).
//! 4. Per entry: `ZipFile::enclosed_name`, the reader's independent
//!    containment check, as a backstop for names our rules did not anticipate.
//! 5. Per entry: no symlinks — the classic way an archive turns a later
//!    innocuous entry into a write outside the destination.
//! 6. Every archive-controlled filesystem node is created exclusively. Only
//!    an exact directory path this extraction already created may be reused.
//! 7. Bytes actually written against the same budget, because a declared size
//!    is only a bound if it is honest.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use unicode_normalization::UnicodeNormalization;

/// How an entry failed containment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryFault {
    /// Contains a `..` segment, or a `\` that would become one on Windows.
    Traversal,
    /// Rooted, or drive-qualified, on this platform or another.
    Absolute,
    /// Rejected by `ZipFile::enclosed_name` — the reader's own check.
    Unenclosed,
    /// Empty, ambiguous, or platform-reserved components a publisher never
    /// emits.
    Malformed,
    /// A symbolic link.
    Symlink,
    /// Names the same destination path as another entry.
    Collision,
}

impl EntryFault {
    fn describe(self) -> &'static str {
        match self {
            Self::Traversal => "climbs out of the destination",
            Self::Absolute => "is an absolute path",
            Self::Unenclosed => "is not enclosed by the destination",
            Self::Malformed => "is not a usable relative path",
            Self::Symlink => "is a symbolic link",
            Self::Collision => "collides with another entry's destination path",
        }
    }
}

#[derive(Debug, Error)]
pub enum ExtractError {
    #[error("failed to open component archive {archive}: {source}")]
    Open {
        archive: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("component archive {archive} is not readable: {source}")]
    Malformed {
        archive: PathBuf,
        #[source]
        source: zip::result::ZipError,
    },
    #[error("failed to write {path} while unpacking {archive}: {source}")]
    Write {
        archive: PathBuf,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("component archive {archive} rejected: entry {entry:?} {}", fault.describe())]
    Unsafe {
        archive: PathBuf,
        entry: String,
        fault: EntryFault,
    },
    #[error(
        "component archive {archive} unpacks to more than the {allowed} bytes it declared \
         (reached {reached})"
    )]
    TooLarge {
        archive: PathBuf,
        allowed: u64,
        reached: u64,
    },
    #[error("refusing to unpack {archive} into {destination}, which already holds files")]
    DestinationNotEmpty {
        archive: PathBuf,
        destination: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractSummary {
    pub files: usize,
    pub bytes: u64,
}

/// `S_IFMT` / `S_IFLNK`: the file-type bits of a stored unix mode.
const UNIX_FILE_TYPE_MASK: u32 = 0o170000;
const UNIX_SYMLINK: u32 = 0o120000;

/// How a filesystem that ignores case would see an entry's path.
///
/// Canonical caseless matching follows Unicode's required order: normalize,
/// fully case-fold, then normalize again. This catches the decomposition and
/// multi-character aliases used by default macOS filesystems without applying
/// compatibility normalization to circled or full-width characters that
/// remain distinct there.
fn fold_case(component: &str) -> String {
    let normalized: String = component.chars().nfd().collect();
    icu_casemap::CaseMapper::new()
        .fold_string(&normalized)
        .chars()
        .nfd()
        .collect()
}

/// Device names Win32 resolves instead of creating as ordinary files.
///
/// Windows ignores an extension and spaces before it for this purpose, and
/// recognizes the superscript forms of COM/LPT 1-3 as digits too.
fn is_windows_reserved_device_name(component: &str) -> bool {
    let stem = component
        .split(['.', ':'])
        .next()
        .unwrap_or(component)
        .trim_end_matches(' ');
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$"]
        .into_iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
    {
        return true;
    }

    stem.get(..3)
        .zip(stem.get(3..))
        .is_some_and(|(prefix, suffix)| {
            (prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT"))
                && matches!(
                    suffix,
                    "1" | "2"
                        | "3"
                        | "4"
                        | "5"
                        | "6"
                        | "7"
                        | "8"
                        | "9"
                        | "\u{b9}"
                        | "\u{b2}"
                        | "\u{b3}"
                )
        })
}

fn has_windows_illegal_character(component: &str) -> bool {
    component.chars().any(|character| {
        ('\u{1}'..='\u{1f}').contains(&character)
            || matches!(character, '<' | '>' | '"' | '|' | '?' | '*')
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimedNodeKind {
    ImplicitDirectory,
    ExplicitDirectory,
    File,
}

#[derive(Debug)]
struct ClaimedNode {
    spelling: String,
    kind: ClaimedNodeKind,
}

/// A component trie for paths the archive would create.
///
/// Keying each node by its parent and one canonical-caseless component keeps
/// the preflight linear in total name bytes. Storing the original spelling is
/// what rejects two directories that a case-folding filesystem merges even
/// when their different children leave the complete paths distinct.
#[derive(Debug, Default)]
struct ClaimedPaths {
    children: HashMap<usize, HashMap<String, usize>>,
    nodes: Vec<ClaimedNode>,
}

impl ClaimedPaths {
    const ROOT: usize = usize::MAX;

    fn claim(&mut self, relative: &Path, is_directory: bool) -> bool {
        let mut components = relative.components().peekable();
        let mut parent = Self::ROOT;

        while let Some(component) = components.next() {
            let Component::Normal(component) = component else {
                return false;
            };
            let spelling = component.to_string_lossy().into_owned();
            let key = fold_case(&spelling);
            let is_leaf = components.peek().is_none();
            let kind = if is_leaf {
                if is_directory {
                    ClaimedNodeKind::ExplicitDirectory
                } else {
                    ClaimedNodeKind::File
                }
            } else {
                ClaimedNodeKind::ImplicitDirectory
            };

            let existing = self
                .children
                .get(&parent)
                .and_then(|children| children.get(&key))
                .copied();
            let node = match existing {
                Some(node) => {
                    let claimed = &mut self.nodes[node];
                    if claimed.spelling != spelling {
                        return false;
                    }
                    match (claimed.kind, kind) {
                        (
                            ClaimedNodeKind::ImplicitDirectory,
                            ClaimedNodeKind::ImplicitDirectory,
                        )
                        | (
                            ClaimedNodeKind::ExplicitDirectory,
                            ClaimedNodeKind::ImplicitDirectory,
                        ) => {}
                        (
                            ClaimedNodeKind::ImplicitDirectory,
                            ClaimedNodeKind::ExplicitDirectory,
                        ) => {
                            claimed.kind = ClaimedNodeKind::ExplicitDirectory;
                        }
                        _ => return false,
                    }
                    node
                }
                None => {
                    let node = self.nodes.len();
                    self.nodes.push(ClaimedNode { spelling, kind });
                    self.children.entry(parent).or_default().insert(key, node);
                    node
                }
            };
            parent = node;
        }
        parent != Self::ROOT
    }
}

/// Applies our own containment rules to a stored entry name.
///
/// Deliberately platform-independent: a name that is relative on Linux but
/// absolute on Windows is refused on both, so an archive cannot be safe on the
/// machine that tests it and dangerous on the machine that installs it.
fn safe_entry_path(name: &str) -> Result<PathBuf, EntryFault> {
    if name.starts_with('/') || name.starts_with('\\') {
        return Err(EntryFault::Absolute);
    }
    let without_directory_marker = name.strip_suffix('/').unwrap_or(name);
    if without_directory_marker.is_empty() {
        return Err(EntryFault::Malformed);
    }
    without_directory_marker
        .split('/')
        .try_fold(PathBuf::new(), |mut path, segment| {
            match segment {
                ".." => return Err(EntryFault::Traversal),
                // `C:foo` is drive-relative on Windows and would escape.
                _ if segment.contains(':') => return Err(EntryFault::Absolute),
                // A backslash is a separator on Windows, so it can smuggle
                // both traversal and rooting past a `/`-only reading.
                _ if segment.contains('\\') => return Err(EntryFault::Traversal),
                "" | "." => return Err(EntryFault::Malformed),
                _ if is_windows_reserved_device_name(segment) => return Err(EntryFault::Malformed),
                _ if has_windows_illegal_character(segment) => return Err(EntryFault::Malformed),
                _ if segment.starts_with(' ')
                    || segment.ends_with(' ')
                    || segment.ends_with('.') =>
                {
                    return Err(EntryFault::Malformed);
                }
                _ => path.push(segment),
            }
            Ok(path)
        })
        .and_then(|path| {
            // Belt and braces: whatever the segment rules concluded, the
            // assembled path must still be plainly relative.
            path.components()
                .all(|component| matches!(component, Component::Normal(_)))
                .then_some(path)
                .ok_or(EntryFault::Traversal)
        })
}

/// Unpacks `archive` into `destination`, refusing anything that does not stay
/// inside it or that exceeds `unpacked_size` bytes in total.
///
/// `unpacked_size` is the caller's budget, and deliberately not read from the
/// manifest: the schema records only the *archive* size, so the applier — which
/// knows what expansion is plausible for the component and how much free space
/// it just checked for — is the layer that has to supply the bound.
///
/// `destination` must not already contain files: staging is always into a
/// directory this call owns, and it is removed in full if anything goes wrong.
pub fn extract_archive(
    archive: &Path,
    destination: &Path,
    unpacked_size: u64,
) -> Result<ExtractSummary, ExtractError> {
    let file = std::fs::File::open(archive).map_err(|source| ExtractError::Open {
        archive: archive.to_path_buf(),
        source,
    })?;
    let zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|source| {
        ExtractError::Malformed {
            archive: archive.to_path_buf(),
            source,
        }
    })?;

    let occupied = std::fs::read_dir(destination)
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false);
    if occupied {
        return Err(ExtractError::DestinationNotEmpty {
            archive: archive.to_path_buf(),
            destination: destination.to_path_buf(),
        });
    }

    // zip 8 indexes entries by their effective name and retains only the last
    // duplicate. Scan the central directory independently so that upgrading
    // the reader cannot turn our reject-whole-archive collision policy into a
    // silent last-entry-wins policy. Comparing record counts also catches
    // names rewritten by extra fields before they enter the index.
    let central_directory_start = zip.central_directory_start();
    let indexed_entry_count = zip.len();
    let mut reader = zip.into_inner();
    let result = match duplicate_central_entry_name(
        &mut reader,
        central_directory_start,
        indexed_entry_count,
    ) {
        Ok(Some(_)) => Err(ExtractError::Unsafe {
            archive: archive.to_path_buf(),
            entry: "<duplicate central-directory name>".to_owned(),
            fault: EntryFault::Collision,
        }),
        Ok(None) => zip::ZipArchive::new(reader)
            .map_err(|source| ExtractError::Malformed {
                archive: archive.to_path_buf(),
                source,
            })
            .and_then(|mut zip| extract_entries(&mut zip, archive, destination, unpacked_size)),
        Err(source) => Err(ExtractError::Malformed {
            archive: archive.to_path_buf(),
            source,
        }),
    };
    if result.is_err() {
        // A half-unpacked component must never be left where an applier could
        // mistake it for a complete one.
        let _ = std::fs::remove_dir_all(destination);
    }
    result
}

/// Returns the first stored name beyond the reader's indexed cardinality.
///
/// `ZipArchive` intentionally exposes a name-indexed view, so compare its
/// length with the number of central-directory records. An excess record means
/// the reader collapsed some effective name, though it does not reveal which
/// earlier record was replaced. This catches exact stored-name duplicates and
/// names made identical by a recognized extra field, without duplicating the
/// reader's name-decoding rules. The bounded scanner reads only fixed-size
/// central headers and their at-most-`u16::MAX` names; payload bytes are never
/// loaded.
fn duplicate_central_entry_name<R: Read + Seek>(
    reader: &mut R,
    central_directory_start: u64,
    indexed_entry_count: usize,
) -> zip::result::ZipResult<Option<String>> {
    const CENTRAL_DIRECTORY_HEADER: [u8; 4] = [b'P', b'K', 1, 2];
    const HEADER_AFTER_SIGNATURE: usize = 42;

    reader.seek(SeekFrom::Start(central_directory_start))?;
    let mut record_count = 0usize;

    loop {
        let mut signature = [0u8; 4];
        reader.read_exact(&mut signature)?;
        if signature != CENTRAL_DIRECTORY_HEADER {
            return (record_count == indexed_entry_count)
                .then_some(None)
                .ok_or_else(|| {
                    zip::result::ZipError::InvalidArchive(
                        "central-directory record count does not match the archive index".into(),
                    )
                });
        }

        let mut header = [0u8; HEADER_AFTER_SIGNATURE];
        reader.read_exact(&mut header)?;
        let filename_length = u16::from_le_bytes([header[24], header[25]]) as usize;
        let extra_length = u16::from_le_bytes([header[26], header[27]]) as i64;
        let comment_length = u16::from_le_bytes([header[28], header[29]]) as i64;
        let mut name = vec![0u8; filename_length];
        reader.read_exact(&mut name)?;

        if record_count == indexed_entry_count {
            return Ok(Some(String::from_utf8_lossy(&name).into_owned()));
        }
        record_count += 1;
        reader.seek(SeekFrom::Current(extra_length + comment_length))?;
    }
}

fn extract_entries<R: std::io::Read + std::io::Seek>(
    zip: &mut zip::ZipArchive<R>,
    archive: &Path,
    destination: &Path,
    unpacked_size: u64,
) -> Result<ExtractSummary, ExtractError> {
    let too_large = |reached| ExtractError::TooLarge {
        archive: archive.to_path_buf(),
        allowed: unpacked_size,
        reached,
    };

    // Cheap pre-pass over the central directory: a bomb that admits its own
    // size, and an archive whose names cannot coexist, are both rejected
    // before a single byte lands on disk.
    let mut claimed = ClaimedPaths::default();
    let declared = (0..zip.len()).try_fold(0u64, |total, index| {
        let entry = zip.by_index_raw(index).map_err(|source| {
            tracing::debug!(%index, "component archive entry unreadable");
            ExtractError::Malformed {
                archive: archive.to_path_buf(),
                source,
            }
        })?;
        let name = entry.name().to_string();
        // A name our own rules reject is refused by the loop below, with the
        // fault that actually describes it; it never reaches a write, so it
        // has nothing to collide with here.
        if let Ok(relative) = safe_entry_path(&name) {
            claimed
                .claim(&relative, entry.is_dir())
                .then_some(())
                .ok_or_else(|| ExtractError::Unsafe {
                    archive: archive.to_path_buf(),
                    entry: name,
                    fault: EntryFault::Collision,
                })?;
        }
        Ok::<_, ExtractError>(total.saturating_add(entry.size()))
    })?;
    if declared > unpacked_size {
        return Err(too_large(declared));
    }

    let created_destination_directories = missing_directories(destination);
    std::fs::create_dir_all(destination).map_err(|source| ExtractError::Write {
        archive: archive.to_path_buf(),
        path: destination.to_path_buf(),
        source,
    })?;

    let mut directories = HashSet::from([destination.to_path_buf()]);
    let mut summary = ExtractSummary { files: 0, bytes: 0 };
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|source| ExtractError::Malformed {
                archive: archive.to_path_buf(),
                source,
            })?;
        let name = entry.name().to_string();
        let unsafe_entry = |fault| ExtractError::Unsafe {
            archive: archive.to_path_buf(),
            entry: name.clone(),
            fault,
        };

        let relative = safe_entry_path(&name).map_err(unsafe_entry)?;
        // The reader's independent containment check, run even though ours
        // already passed: it knows about encodings ours does not.
        entry
            .enclosed_name()
            .ok_or(EntryFault::Unenclosed)
            .map_err(unsafe_entry)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & UNIX_FILE_TYPE_MASK == UNIX_SYMLINK)
        {
            return Err(unsafe_entry(EntryFault::Symlink));
        }

        let path = destination.join(&relative);
        let write_error = |source| ExtractError::Write {
            archive: archive.to_path_buf(),
            path: path.clone(),
            source,
        };
        let creation_error = |source: std::io::Error| {
            if source.kind() == std::io::ErrorKind::AlreadyExists {
                unsafe_entry(EntryFault::Collision)
            } else {
                write_error(source)
            }
        };
        if entry.is_dir() {
            create_archive_directories(&mut directories, destination, &path)
                .map_err(creation_error)?;
            continue;
        }
        if let Some(parent) = path.parent() {
            create_archive_directories(&mut directories, destination, parent)
                .map_err(creation_error)?;
        }

        // The declared sizes were only a promise; count what is actually
        // written against the same budget. Reading one byte past the remainder
        // is what makes the overrun observable without unpacking it.
        // Saturating throughout: an archive is untrusted input and must not be
        // able to reach an arithmetic panic.
        let remaining = unpacked_size.saturating_sub(summary.bytes);
        let mut file = create_archive_file(&path).map_err(creation_error)?;
        let written = std::io::copy(
            &mut entry.by_ref().take(remaining.saturating_add(1)),
            &mut file,
        )
        .map_err(write_error)?;
        if written > remaining {
            return Err(too_large(summary.bytes.saturating_add(written)));
        }
        finalize_extracted_file(&file, &path, entry.unix_mode()).map_err(write_error)?;

        summary.files += 1;
        summary.bytes = summary.bytes.saturating_add(written);
    }
    sync_extracted_directories_with(&directories, &created_destination_directories, |path| {
        sync_extracted_directory(path).map_err(|source| ExtractError::Write {
            archive: archive.to_path_buf(),
            path: path.to_path_buf(),
            source,
        })
    })?;
    Ok(summary)
}

fn create_archive_directories(
    directories: &mut HashSet<PathBuf>,
    destination: &Path,
    target: &Path,
) -> Result<(), std::io::Error> {
    // Recursive creation accepts AlreadyExists at every level, which would
    // silently merge names through filesystem rules we cannot model (such as
    // an NTFS 8.3 alias). Reuse only exact paths this extraction recorded.
    let relative = target
        .strip_prefix(destination)
        .map_err(|source| std::io::Error::new(std::io::ErrorKind::InvalidInput, source))?;
    let mut path = destination.to_path_buf();
    for component in relative.components() {
        path.push(component.as_os_str());
        if directories.contains(&path) {
            continue;
        }
        std::fs::create_dir(&path)?;
        directories.insert(path.clone());
    }
    Ok(())
}

fn create_archive_file(path: &Path) -> Result<std::fs::File, std::io::Error> {
    // `File::create` truncates an existing target. Exclusive creation turns an
    // unknown filesystem alias or race into a reject-whole-archive failure.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

fn missing_directories(path: &Path) -> Vec<PathBuf> {
    path.ancestors()
        .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !ancestor.exists())
        .map(Path::to_path_buf)
        .collect()
}

fn durability_parent(path: &Path) -> Option<PathBuf> {
    path.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            parent.to_path_buf()
        }
    })
}

fn sync_extracted_directories_with<E, F>(
    directories: &HashSet<PathBuf>,
    created_destination_directories: &[PathBuf],
    mut sync: F,
) -> Result<(), E>
where
    F: FnMut(&Path) -> Result<(), E>,
{
    let mut paths = directories.clone();
    paths.extend(
        created_destination_directories
            .iter()
            .filter_map(|directory| durability_parent(directory)),
    );
    let mut paths: Vec<_> = paths.into_iter().collect();
    paths.sort_by(|left, right| {
        durability_depth(right)
            .cmp(&durability_depth(left))
            .then_with(|| left.cmp(right))
    });
    paths.iter().try_for_each(|path| sync(path))
}

fn durability_depth(path: &Path) -> usize {
    path.components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count()
}

/// Makes directory entries created by extraction durable.
///
/// Windows does not expose a portable directory flush through `std`; file
/// contents and metadata are still synced individually, while its directory
/// entry durability is left to the filesystem's rename semantics.
#[cfg(unix)]
fn sync_extracted_directory(path: &Path) -> Result<(), std::io::Error> {
    std::fs::File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_extracted_directory(_path: &Path) -> Result<(), std::io::Error> {
    Ok(())
}

fn finalize_extracted_file(
    file: &std::fs::File,
    path: &Path,
    mode: Option<u32>,
) -> Result<(), std::io::Error> {
    finalize_extracted_file_with(file, path, mode, std::fs::File::sync_all)
}

fn finalize_extracted_file_with<F>(
    file: &std::fs::File,
    path: &Path,
    mode: Option<u32>,
    sync: F,
) -> Result<(), std::io::Error>
where
    F: FnOnce(&std::fs::File) -> Result<(), std::io::Error>,
{
    // Permissions are metadata too. Apply them before the durability boundary
    // so a power cut cannot leave an executable present but unlaunchable.
    apply_entry_mode(path, mode)?;
    sync(file)
}

#[cfg(unix)]
fn apply_entry_mode(path: &Path, mode: Option<u32>) -> Result<(), std::io::Error> {
    use std::os::unix::fs::PermissionsExt;

    // The engine component ships executables; dropping the mode would produce
    // an install that unpacks cleanly and then cannot start.
    mode.map_or(Ok(()), |mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o777))
    })
}

#[cfg(not(unix))]
fn apply_entry_mode(_path: &Path, _mode: Option<u32>) -> Result<(), std::io::Error> {
    // Windows has no mode bits to restore; executability follows the extension.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use tempfile::TempDir;
    use zip::write::{FullFileOptions, SimpleFileOptions};
    use zip::ZipWriter;

    fn archive_of(build: impl FnOnce(&mut ZipWriter<Cursor<Vec<u8>>>)) -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        build(&mut writer);
        writer.finish().expect("finish archive").into_inner()
    }

    fn plain() -> Vec<u8> {
        archive_of(|writer| {
            writer
                .start_file("Graphics.c4g/Info.txt", SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(b"hello").expect("write file");
            writer
                .start_file("System.c4g/Nested/Deep.txt", SimpleFileOptions::default())
                .expect("start nested file");
            writer.write_all(b"deep").expect("write nested file");
        })
    }

    fn with_entry(name: &str) -> Vec<u8> {
        archive_of(|writer| {
            writer
                .start_file(name, SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(b"payload").expect("write file");
        })
    }

    /// Rewrites the *last* central-directory record's declared uncompressed
    /// size, producing an archive whose header understates what it unpacks to.
    /// A publisher never emits one; an attacker trivially does.
    fn understate_last_entry(bytes: &mut [u8], declared: u32) {
        let header = bytes
            .windows(4)
            .rposition(|window| window == [b'P', b'K', 1, 2])
            .expect("a central directory header");
        // Central header layout: signature(4) … compressed size(20)
        // uncompressed size(24).
        bytes[header + 24..header + 28].copy_from_slice(&declared.to_le_bytes());
    }

    /// Rewrites the last entry's local and central names without changing
    /// their lengths. Current writers reject duplicate names, while an
    /// attacker can still put them in an archive directly.
    fn rename_last_entry(bytes: &mut [u8], replacement: &str) {
        let local = bytes
            .windows(4)
            .rposition(|window| window == [b'P', b'K', 3, 4])
            .expect("a local file header");
        let central = bytes
            .windows(4)
            .rposition(|window| window == [b'P', b'K', 1, 2])
            .expect("a central directory header");
        let local_name_length = u16::from_le_bytes([bytes[local + 26], bytes[local + 27]]) as usize;
        let central_name_length =
            u16::from_le_bytes([bytes[central + 28], bytes[central + 29]]) as usize;
        assert_eq!(local_name_length, replacement.len());
        assert_eq!(central_name_length, replacement.len());
        bytes[local + 30..local + 30 + local_name_length].copy_from_slice(replacement.as_bytes());
        bytes[central + 46..central + 46 + central_name_length]
            .copy_from_slice(replacement.as_bytes());
    }

    fn crc32(bytes: &[u8]) -> u32 {
        !bytes.iter().fold(!0u32, |crc, byte| {
            (0..8).fold(crc ^ u32::from(*byte), |crc, _| {
                (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1))
            })
        })
    }

    fn unicode_path_options(unicode_name: &str) -> FullFileOptions<'static> {
        let mut data = Vec::with_capacity(5 + unicode_name.len());
        data.push(1);
        // The writer validates custom fields before it knows the entry name,
        // so start with the CRC of its empty placeholder and repair it once
        // the central directory has been written.
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(unicode_name.as_bytes());
        let mut options = FullFileOptions::default();
        options
            .add_extra_data(0x7075, data, true)
            .expect("add Unicode Path extra field");
        options
    }

    fn repair_unicode_path_crcs(bytes: &mut [u8]) {
        let central_headers: Vec<_> = bytes
            .windows(4)
            .enumerate()
            .filter_map(|(offset, signature)| (signature == [b'P', b'K', 1, 2]).then_some(offset))
            .collect();
        for header in central_headers {
            let name_length = u16::from_le_bytes([bytes[header + 28], bytes[header + 29]]) as usize;
            let extra = header + 46 + name_length;
            assert_eq!(&bytes[extra..extra + 2], &0x7075u16.to_le_bytes());
            let name = bytes[header + 46..header + 46 + name_length].to_vec();
            bytes[extra + 5..extra + 9].copy_from_slice(&crc32(&name).to_le_bytes());
        }
    }

    fn write_archive(directory: &Path, bytes: &[u8]) -> PathBuf {
        let path = directory.join("component.zip");
        std::fs::write(&path, bytes).expect("write archive");
        path
    }

    fn extract(
        bytes: &[u8],
        unpacked_size: u64,
    ) -> (TempDir, Result<ExtractSummary, ExtractError>) {
        let directory = TempDir::new().expect("directory");
        let archive = write_archive(directory.path(), bytes);
        let destination = directory.path().join("staged");
        let result = extract_archive(&archive, &destination, unpacked_size);
        (directory, result)
    }

    #[test]
    fn a_well_formed_archive_extracts_files_and_nested_directories() {
        let (directory, result) = extract(&plain(), 1024);
        let summary = result.expect("extract");
        assert_eq!(summary.files, 2);
        assert_eq!(summary.bytes, 9);

        let staged = directory.path().join("staged");
        assert_eq!(
            std::fs::read_to_string(staged.join("Graphics.c4g/Info.txt")).expect("read"),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(staged.join("System.c4g/Nested/Deep.txt")).expect("read"),
            "deep"
        );
    }

    #[test]
    fn an_entry_climbing_out_of_the_destination_rejects_the_archive() {
        let (_directory, result) = extract(&with_entry("../escaped.txt"), 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Traversal,
                ..
            })
        ));
    }

    #[test]
    fn an_absolute_entry_path_rejects_the_archive() {
        for name in [
            "/etc/passwd",
            "\\Windows\\System32\\evil.dll",
            "C:/evil.txt",
        ] {
            let (_directory, result) = extract(&with_entry(name), 1024);
            assert!(
                matches!(
                    result,
                    Err(ExtractError::Unsafe {
                        fault: EntryFault::Absolute,
                        ..
                    })
                ),
                "{name:?} should be rejected as absolute, got {result:?}"
            );
        }
    }

    #[test]
    fn a_windows_reserved_device_name_rejects_before_writing() {
        for name in [
            "CON",
            "con.txt",
            "CON .txt",
            "NUL  .txt",
            "conin$",
            "CONOUT$.log",
            "clock$.txt",
            "safe/PRN.log",
            "AUX",
            "NUL.txt",
            "COM1",
            "com9.dat",
            "LPT1",
            "lpt9.log",
            "COM\u{b9}.txt",
            "COM\u{b2}",
            "COM\u{b3}",
            "LPT\u{b9}.txt",
            "LPT\u{b2}",
            "LPT\u{b3}",
        ] {
            let (directory, result) = extract(&with_entry(name), 1024);
            assert!(
                matches!(
                    result,
                    Err(ExtractError::Unsafe {
                        fault: EntryFault::Malformed,
                        ..
                    })
                ),
                "{name:?} should be rejected, got {result:?}"
            );
            assert!(!directory.path().join("staged").exists());
        }

        let directory_entry = archive_of(|writer| {
            writer
                .add_directory("NUL/", SimpleFileOptions::default())
                .expect("add reserved directory");
        });
        let (directory, result) = extract(&directory_entry, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Malformed,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn non_device_windows_lookalikes_remain_usable() {
        let allowed = [
            "COM0.txt",
            "COM10.txt",
            "LPT0",
            "LPT10",
            "NULish",
            "console",
            ".temp",
            "COM\u{2074}",
            "\u{a0}nonbreaking-space.txt",
        ];
        let bytes = archive_of(|writer| {
            for name in allowed {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start allowed lookalike");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (_directory, result) = extract(&bytes, 1024);
        assert_eq!(
            result.expect("extract allowed lookalikes").files,
            allowed.len()
        );
    }

    #[test]
    fn a_windows_illegal_character_rejects_before_writing() {
        for name in [
            "evil<name.txt",
            "evil>name.txt",
            "evil\"name.txt",
            "evil|name.txt",
            "evil?name.txt",
            "evil*name.txt",
            "safe/evil\u{1}name.txt",
            "safe/evil\u{1f}name.txt",
        ] {
            let (directory, result) = extract(&with_entry(name), 1024);
            assert!(
                matches!(
                    result,
                    Err(ExtractError::Unsafe {
                        fault: EntryFault::Malformed,
                        ..
                    })
                ),
                "{name:?} should be rejected, got {result:?}"
            );
            assert!(!directory.path().join("staged").exists());
        }

        let directory_entry = archive_of(|writer| {
            writer
                .add_directory("evil*/", SimpleFileOptions::default())
                .expect("add invalid directory");
        });
        let (directory, result) = extract(&directory_entry, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Malformed,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn an_entry_name_the_zip_reader_cannot_enclose_rejects_the_archive() {
        // `enclosed_name` is the reader's independent containment check. NUL
        // remains its responsibility so this defense-in-depth guard stays
        // exercised even as our platform-independent rules grow stricter.
        let (directory, result) = extract(&with_entry("evil\0.txt"), 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Unenclosed,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn a_windows_trimmed_component_rejects_before_writing() {
        for name in [
            " leading.txt",
            "trailing.txt ",
            "trailing.",
            "safe/ nested.txt",
            "safe/nested.txt ",
            "safe/nested.",
        ] {
            let (directory, result) = extract(&with_entry(name), 1024);
            assert!(
                matches!(
                    result,
                    Err(ExtractError::Unsafe {
                        fault: EntryFault::Malformed,
                        ..
                    })
                ),
                "{name:?} should be rejected, got {result:?}"
            );
            assert!(!directory.path().join("staged").exists());
        }

        let directory_entry = archive_of(|writer| {
            writer
                .add_directory("trailing. /", SimpleFileOptions::default())
                .expect("add trimmed directory");
        });
        let (directory, result) = extract(&directory_entry, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Malformed,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn repeated_directory_separators_reject_before_writing() {
        let bytes = archive_of(|writer| {
            writer
                .add_directory("dir//", SimpleFileOptions::default())
                .expect("add malformed directory");
        });
        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Malformed,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn a_symlink_entry_rejects_the_archive() {
        // A symlink is how an archive turns a later, innocent-looking entry
        // into a write outside the destination, so no component ever contains
        // one and the whole archive is refused if one appears.
        let bytes = archive_of(|writer| {
            writer
                .add_symlink("planet/System.c4g", "/etc", SimpleFileOptions::default())
                .expect("add symlink");
        });
        let (_directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Symlink,
                ..
            })
        ));
    }

    #[test]
    fn entries_differing_only_in_case_reject_the_archive() {
        // macOS and Windows fold case, so two such entries become one file:
        // the later one silently overwrites the earlier, and the component
        // installs looking complete while holding contents no case-sensitive
        // machine ever produced. Building the archive by hand is the point —
        // a publisher's tree cannot contain the pair, an attacker's can.
        let bytes = archive_of(|writer| {
            writer
                .start_file("Graphics.c4g/Info.txt", SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(b"real").expect("write file");
            writer
                .start_file("graphics.c4g/INFO.TXT", SimpleFileOptions::default())
                .expect("start colliding file");
            writer.write_all(b"shadow").expect("write colliding file");
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(
            matches!(
                result,
                Err(ExtractError::Unsafe {
                    fault: EntryFault::Collision,
                    ..
                })
            ),
            "a case-folding collision should reject the archive, got {result:?}"
        );
        // Rejected in the pre-pass, so not one byte of it reached the disk.
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn canonically_equivalent_entry_names_reject_before_writing() {
        let bytes = archive_of(|writer| {
            for name in ["Caf\u{e9}.txt", "Cafe\u{301}.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start canonically equivalent file");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(
            matches!(
                result,
                Err(ExtractError::Unsafe {
                    fault: EntryFault::Collision,
                    ..
                })
            ),
            "canonical aliases should reject the archive, got {result:?}"
        );
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn full_case_folded_directory_aliases_reject_before_writing() {
        let bytes = archive_of(|writer| {
            for name in ["Stra\u{df}e/a.txt", "STRASSE/b.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start full-fold alias");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Collision,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn three_character_full_fold_aliases_reject_before_writing() {
        let bytes = archive_of(|writer| {
            for name in ["\u{fb03}le.txt", "ffile.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start ligature alias");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Collision,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn aliased_directory_prefixes_reject_before_writing() {
        let bytes = archive_of(|writer| {
            for name in ["Dir/a.txt", "dir/b.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start child of aliased directory");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Collision,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn file_and_directory_nodes_cannot_share_a_path() {
        for child_first in [false, true] {
            let bytes = archive_of(|writer| {
                let mut file = |name: &str| {
                    writer
                        .start_file(name, SimpleFileOptions::default())
                        .expect("start file");
                    writer.write_all(b"payload").expect("write file");
                };
                if child_first {
                    file("node/child.txt");
                    file("node");
                } else {
                    file("node");
                    file("node/child.txt");
                }
            });

            let (directory, result) = extract(&bytes, 1024);
            assert!(matches!(
                result,
                Err(ExtractError::Unsafe {
                    fault: EntryFault::Collision,
                    ..
                })
            ));
            assert!(!directory.path().join("staged").exists());
        }
    }

    #[test]
    fn an_explicit_directory_can_share_its_exact_path_with_children() {
        for child_first in [false, true] {
            let bytes = archive_of(|writer| {
                if child_first {
                    writer
                        .start_file("node/child.txt", SimpleFileOptions::default())
                        .expect("start child");
                    writer.write_all(b"payload").expect("write child");
                    writer
                        .add_directory("node/", SimpleFileOptions::default())
                        .expect("add directory");
                } else {
                    writer
                        .add_directory("node/", SimpleFileOptions::default())
                        .expect("add directory");
                    writer
                        .start_file("node/child.txt", SimpleFileOptions::default())
                        .expect("start child");
                    writer.write_all(b"payload").expect("write child");
                }
            });

            let (_directory, result) = extract(&bytes, 1024);
            assert_eq!(result.expect("extract directory and child").files, 1);
        }
    }

    #[test]
    fn canonical_caseless_key_normalizes_before_case_folding() {
        let bytes = archive_of(|writer| {
            for name in ["\u{3b1}\u{345}\u{300}.txt", "\u{3b1}\u{300}\u{345}.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start reordered-mark alias");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Collision,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn unicode_seventeen_case_pair_rejects_before_writing() {
        let bytes = archive_of(|writer| {
            for name in ["\u{a7ce}.txt", "\u{a7cf}.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start Unicode 17 case alias");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(
            result,
            Err(ExtractError::Unsafe {
                fault: EntryFault::Collision,
                ..
            })
        ));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn compatibility_equivalent_names_remain_distinct() {
        let bytes = archive_of(|writer| {
            for name in ["\u{2460}.txt", "1.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start compatibility-distinct file");
                writer.write_all(b"payload").expect("write file");
            }
        });

        let (_directory, result) = extract(&bytes, 1024);
        assert_eq!(result.expect("extract distinct names").files, 2);
    }

    #[test]
    fn a_repeated_entry_name_rejects_the_archive() {
        // The same collision without the case fold: the second entry would
        // overwrite the first on every filesystem. The writer refuses to
        // produce such an archive, so emulate the hostile central directory.
        let first = "planet/System.c4g/Rank-one.txt";
        let mut bytes = archive_of(|writer| {
            for name in [first, "planet/System.c4g/Rank-two.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start file");
                writer.write_all(b"payload").expect("write file");
            }
        });
        rename_last_entry(&mut bytes, first);
        let (_directory, result) = extract(&bytes, 1024);
        assert!(
            matches!(
                &result,
                Err(ExtractError::Unsafe {
                    fault: EntryFault::Collision,
                    ..
                })
            ),
            "a repeated name should reject the archive, got {result:?}"
        );
    }

    #[test]
    fn unicode_path_aliases_reject_the_archive_before_writing() {
        // Info-ZIP Unicode Path fields can replace different stored names with
        // the same effective name. zip 8 indexes that effective name and keeps
        // only the last entry, so extraction must detect the collapsed record.
        let mut bytes = archive_of(|writer| {
            for raw_name in ["first.txt", "other.txt"] {
                writer
                    .start_file(raw_name, unicode_path_options("shared.txt"))
                    .expect("start aliased file");
                writer.write_all(b"payload").expect("write file");
            }
        });
        repair_unicode_path_crcs(&mut bytes);

        let (directory, result) = extract(&bytes, 1024);
        assert!(
            matches!(
                result,
                Err(ExtractError::Unsafe {
                    fault: EntryFault::Collision,
                    ..
                })
            ),
            "Unicode Path aliases should reject the archive, got {result:?}"
        );
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn unicode_path_field_with_wrong_crc_rejects_before_writing() {
        let bytes = archive_of(|writer| {
            writer
                .start_file("plain.txt", unicode_path_options("alias.txt"))
                .expect("start file with invalid Unicode Path CRC");
            writer.write_all(b"payload").expect("write file");
        });

        let (directory, result) = extract(&bytes, 1024);
        assert!(matches!(result, Err(ExtractError::Malformed { .. })));
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn names_that_only_share_a_folded_prefix_still_extract() {
        // The guard folds complete components, not substrings: `Info.txt` beside
        // `info-2.txt` is an ordinary archive and must not be refused.
        let bytes = archive_of(|writer| {
            for name in ["Graphics.c4g/Info.txt", "Graphics.c4g/info-2.txt"] {
                writer
                    .start_file(name, SimpleFileOptions::default())
                    .expect("start file");
                writer.write_all(b"payload").expect("write file");
            }
        });
        let (_directory, result) = extract(&bytes, 1024);
        assert_eq!(result.expect("extract").files, 2);
    }

    #[test]
    fn an_archive_declaring_more_than_it_may_unpack_is_rejected() {
        let (_directory, result) = extract(&plain(), 4);
        assert!(matches!(result, Err(ExtractError::TooLarge { .. })));
    }

    #[test]
    fn an_entry_that_lies_about_its_size_is_caught_while_unpacking() {
        // The declared sizes only bound the archive if they are honest, so the
        // budget is also counted against the bytes actually written.
        let mut bytes = archive_of(|writer| {
            writer
                .start_file("keep.txt", SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(b"kept").expect("write file");
            writer
                .start_file("bomb.bin", SimpleFileOptions::default())
                .expect("start bomb");
            writer.write_all(&vec![0u8; 8192]).expect("write bomb");
        });
        understate_last_entry(&mut bytes, 1);

        let (_directory, result) = extract(&bytes, 512);
        assert!(matches!(result, Err(ExtractError::TooLarge { .. })));
    }

    #[test]
    fn a_rejected_archive_leaves_nothing_behind() {
        // "Reject the archive" has to mean the whole archive: a component
        // half-unpacked into a staging directory would be applied as if it
        // were complete.
        let mut bytes = archive_of(|writer| {
            writer
                .start_file("keep.txt", SimpleFileOptions::default())
                .expect("start file");
            writer.write_all(b"kept").expect("write file");
            writer
                .start_file("bomb.bin", SimpleFileOptions::default())
                .expect("start bomb");
            writer.write_all(&vec![0u8; 8192]).expect("write bomb");
        });
        understate_last_entry(&mut bytes, 1);

        let (directory, result) = extract(&bytes, 512);
        assert!(result.is_err());
        assert!(!directory.path().join("staged").exists());
    }

    #[test]
    fn a_destination_that_already_holds_files_is_refused() {
        // Unpacking over an existing tree would mix two releases; staging is
        // always into a directory this call owns.
        let directory = TempDir::new().expect("directory");
        let archive = write_archive(directory.path(), &plain());
        let destination = directory.path().join("staged");
        std::fs::create_dir_all(&destination).expect("create destination");
        std::fs::write(destination.join("stale.txt"), b"old").expect("stale file");

        assert!(matches!(
            extract_archive(&archive, &destination, 1024),
            Err(ExtractError::DestinationNotEmpty { .. })
        ));
        assert!(destination.join("stale.txt").exists());
    }

    #[test]
    fn archive_directory_creation_rejects_an_unclaimed_existing_path() {
        let directory = TempDir::new().expect("directory");
        let destination = directory.path().join("staged");
        let existing = destination.join("SHORT~1");
        std::fs::create_dir_all(&existing).expect("create filesystem alias stand-in");
        let mut claimed = HashSet::from([destination.clone()]);

        let result = create_archive_directories(&mut claimed, &destination, &existing);

        assert_eq!(
            result.expect_err("reject unclaimed existing path").kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(claimed, HashSet::from([destination]));
    }

    #[test]
    fn archive_file_creation_never_truncates_an_existing_path() {
        let directory = TempDir::new().expect("directory");
        let existing = directory.path().join("alias.txt");
        std::fs::write(&existing, b"sentinel").expect("write sentinel");

        let result = create_archive_file(&existing);

        assert_eq!(
            result.expect_err("reject existing file").kind(),
            std::io::ErrorKind::AlreadyExists
        );
        assert_eq!(
            std::fs::read(&existing).expect("read sentinel"),
            b"sentinel"
        );
    }

    #[test]
    fn an_extracted_file_is_synced_after_its_final_mode_is_applied() {
        let directory = TempDir::new().expect("directory");
        let path = directory.path().join("clonk-game");
        let file = std::fs::File::create(&path).expect("create file");
        let mut synced = false;

        finalize_extracted_file_with(&file, &path, Some(0o755), |_| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let mode = std::fs::metadata(&path)?.permissions().mode();
                assert_eq!(mode & 0o777, 0o755);
            }
            synced = true;
            Ok(())
        })
        .expect("finish file");

        assert!(synced);
    }

    #[test]
    fn extracted_directories_are_synced_bottom_up() {
        let destination = PathBuf::from("install/staged");
        let directories = HashSet::from([
            destination.clone(),
            destination.join("System.c4g"),
            destination.join("System.c4g/Nested"),
        ]);
        let created = vec![destination];
        let mut synced = Vec::new();

        sync_extracted_directories_with(&directories, &created, |path| {
            synced.push(path.to_path_buf());
            Ok::<_, std::io::Error>(())
        })
        .expect("sync directories");

        assert_eq!(
            synced,
            [
                PathBuf::from("install/staged/System.c4g/Nested"),
                PathBuf::from("install/staged/System.c4g"),
                PathBuf::from("install/staged"),
                PathBuf::from("install"),
            ]
        );
    }

    #[test]
    fn a_relative_destination_syncs_the_current_directory() {
        let destination = PathBuf::from("staged");
        let directories = HashSet::from([destination.clone()]);
        let mut synced = Vec::new();

        sync_extracted_directories_with(&directories, &[destination], |path| {
            synced.push(path.to_path_buf());
            Ok::<_, std::io::Error>(())
        })
        .expect("sync directories");

        assert_eq!(synced, [PathBuf::from("staged"), PathBuf::from(".")]);
    }

    #[cfg(unix)]
    #[test]
    fn the_executable_bit_survives_extraction() {
        // The engine component ships executables; losing the mode would
        // produce an install that unpacks cleanly and cannot start.
        use std::os::unix::fs::PermissionsExt;

        let bytes = archive_of(|writer| {
            writer
                .start_file(
                    "bin/clonk-game",
                    SimpleFileOptions::default().unix_permissions(0o755),
                )
                .expect("start file");
            writer.write_all(b"#!/bin/sh\n").expect("write file");
        });
        let (directory, result) = extract(&bytes, 1024);
        result.expect("extract");

        let mode = std::fs::metadata(directory.path().join("staged/bin/clonk-game"))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
