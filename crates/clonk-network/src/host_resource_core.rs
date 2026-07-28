//! C++-faithful publication of complete host-side network resources.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use clonk_engine::{LegacyCString, NetworkResourceCore};
use clonk_resources::{Group, GroupError, MutableGroup, MutableGroupError};
use sha1::{Digest, Sha1};
use thiserror::Error;

use crate::ResourceFileOwnership;

/// Chunk size advertised for resources this peer publishes.
///
/// OpenClonk's 10 KiB rather than LegacyClonk's 100 KiB
/// (`C4NetResChunkSize`, C4Network2Res.h:27). Chunk size is carried per
/// resource in the core and honoured by whoever downloads it, so this is a local
/// choice rather than a protocol change, and a stock C++ peer follows it.
///
/// Why it matters more than transfer throughput: resource chunks and control
/// share one *strictly-ordered* reliable-UDP sequence space whenever a peer has
/// no TCP route, which is the ordinary internet-play topology because NAT
/// punch-through is UDP-only (`GetDataConnection` falls back to the message
/// connection). A 100 KiB chunk is 206 datagrams at the 499-byte payload limit,
/// so it puts 206 sequence numbers ahead of every later control packet, and one
/// lost fragment withholds all of them from the game loop until the repair
/// lands -- at ten fragment asks per check packet. 10 KiB is 21 datagrams,
/// cutting that head-of-line window by an order of magnitude. LegacyClonk raised
/// this to 100 KiB in 2557ff3d to "better utilize available upload speed", which
/// is the right trade for a fast link and the wrong one for a narrow one.
const STOCK_CHUNK_SIZE: u32 = 10 * 1024;

/// What an unloadable core carries. Mirrors `clonk_engine`'s
/// `NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE`, which is the value C++ substitutes
/// when decoding one, so the core round-trips unchanged.
const UNLOADABLE_CHUNK_SIZE: u32 = 100 * 1024;
const DEFAULT_MAX_LOAD_FILE_SIZE: u32 = 100 * 1024 * 1024;
pub const MAX_PLAYER_BIG_ICON_SIZE: u64 = 20 * 1024;
static NEXT_STAGED_PATH: AtomicU64 = AtomicU64::new(0);

/// Values of `C4Network2ResType` used during pregame publication
/// (`src/C4Network2Res.h:38-48`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HostResourceType {
    Scenario = 1,
    Dynamic = 2,
    Player = 3,
    Definitions = 4,
    System = 5,
    Material = 6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResourceCoreSpec {
    resource_type: HostResourceType,
    resource_id: i32,
    resource_name: LegacyCString,
    source_ownership: ResourceFileOwnership,
    group_maker: LegacyCString,
    max_load_file_size: u32,
    standalone_name: Option<LegacyCString>,
}

impl HostResourceCoreSpec {
    pub fn new(
        resource_type: HostResourceType,
        resource_id: i32,
        resource_name: LegacyCString,
        group_maker: impl Into<String>,
    ) -> Self {
        let group_maker = group_maker.into();
        let maker_end = group_maker
            .as_bytes()
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(group_maker.len());
        let group_maker = LegacyCString::from_bytes(group_maker.as_bytes()[..maker_end].to_vec())
            .unwrap_or_default();
        Self::new_with_raw_group_maker(resource_type, resource_id, resource_name, group_maker)
    }

    pub fn new_with_raw_group_maker(
        resource_type: HostResourceType,
        resource_id: i32,
        resource_name: LegacyCString,
        group_maker: LegacyCString,
    ) -> Self {
        let source_ownership = if resource_type == HostResourceType::Dynamic {
            ResourceFileOwnership::Temporary
        } else {
            ResourceFileOwnership::Persistent
        };
        Self {
            resource_type,
            resource_id,
            resource_name,
            source_ownership,
            group_maker,
            max_load_file_size: DEFAULT_MAX_LOAD_FILE_SIZE,
            standalone_name: None,
        }
    }

    pub fn with_source_ownership(mut self, ownership: ResourceFileOwnership) -> Self {
        self.source_ownership = ownership;
        self
    }

    pub fn with_max_load_file_size(mut self, size: u32) -> Self {
        self.max_load_file_size = size;
        self
    }

    pub fn with_standalone_name(mut self, name: LegacyCString) -> Self {
        self.standalone_name = Some(name);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResourcePublication {
    pub core: NetworkResourceCore,
    pub source_path: PathBuf,
    pub standalone_path: Option<PathBuf>,
    pub standalone_ownership: Option<ResourceFileOwnership>,
}

impl HostResourcePublication {
    /// Adds C++'s optional league FileSHA to the retained resource core.
    pub fn calculate_file_sha(&mut self) -> Result<(), HostResourceCoreError> {
        if self.core.file_sha.is_some() {
            return Ok(());
        }

        let path = self.standalone_path.as_deref().unwrap_or(&self.source_path);
        let mut file = File::open(path)?;
        let mut hasher = Sha1::new();
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        self.core.file_sha = Some(hasher.finalize().into());
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum HostResourceCoreError {
    #[error("host resource I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("host C4Group could not be read: {0}")]
    Group(#[from] GroupError),
    #[error("host C4Group standalone could not be written: {0}")]
    GroupWriter(#[from] MutableGroupError),
    #[error("NRT_Player source is not a readable C4Group: {0}")]
    PlayerGroupRequired(PathBuf),
    #[error("opaque player child '{path}' has unsupported pre-rewrite CRC state {crc_state}")]
    OpaqueChildCrcStateUnsupported { path: PathBuf, crc_state: u8 },
    #[error("a C4Group maker unexpectedly contained an interior NUL")]
    InvalidGroupMaker,
    #[error("temporary directory resources require C++'s destructive in-place packing")]
    TemporaryDirectoryUnsupported,
    #[error("directory entry name is not valid UTF-8: {0}")]
    NonUtf8EntryName(PathBuf),
    #[error("packed child groups cannot yet be imported byte-for-byte: {0}")]
    PackedChildGroupUnsupported(PathBuf),
    #[error("no free local resource standalone filename from 1 through 999")]
    NoStandaloneFilename,
}

/// Builds the core and identifies the exact standalone bytes a host publishes.
pub fn build_host_resource_core(
    source_path: impl AsRef<Path>,
    _standalone_directory: impl AsRef<Path>,
    spec: HostResourceCoreSpec,
) -> Result<HostResourcePublication, HostResourceCoreError> {
    let source_path = source_path.as_ref().to_path_buf();
    let metadata = fs::metadata(&source_path)?;
    let standalone_name = spec
        .standalone_name
        .as_ref()
        .map(|name| name.as_bytes().to_vec())
        .unwrap_or_else(|| clonk_resources::path_to_legacy_bytes(&source_path));
    let group = Group::open(&source_path).ok();
    if spec.resource_type == HostResourceType::Player && group.is_none() {
        return Err(HostResourceCoreError::PlayerGroupRequired(source_path));
    }
    let (contents_crc, author) = match group.as_ref() {
        Some(group) => {
            let maker = if group.is_directory() {
                b"Open directory".as_slice()
            } else {
                group.maker_bytes().unwrap_or_default()
            };
            let author = LegacyCString::from_bytes(maker.to_vec())
                .ok_or(HostResourceCoreError::InvalidGroupMaker)?;
            let contents_crc = group.contents_crc_or_zero();
            (contents_crc, author)
        }
        None => (file_crc(&source_path)?, LegacyCString::default()),
    };

    let mut core = NetworkResourceCore {
        resource_type: spec.resource_type as u8,
        id: spec.resource_id,
        derived_id: -1,
        loadable: false,
        file_size: u32::MAX,
        file_crc: u32::MAX,
        // A non-loadable core carries no transferable payload, and C++ decodes
        // one by substituting the compiled-in defaults for size, CRC and chunk
        // size alike (`legacy.rs` mirrors that). A custom chunk size therefore
        // cannot round-trip here and would mean nothing if it did, so the stock
        // value is applied only once the resource becomes loadable below.
        chunk_size: UNLOADABLE_CHUNK_SIZE,
        contents_crc,
        file_sha: None,
        filename: spec.resource_name,
        author,
    };

    // AddByFile deliberately skips GetStandalone for NRT_System, so the core
    // retains Set's non-loadable sentinels (src/C4Network2Res.cpp:1443-1468).
    if spec.resource_type == HostResourceType::System {
        return Ok(HostResourcePublication {
            core,
            source_path,
            standalone_path: None,
            standalone_ownership: None,
        });
    }

    if spec.resource_type == HostResourceType::Definitions && metadata.is_dir() {
        match directory_size_exceeds(&source_path, u64::from(spec.max_load_file_size)) {
            Ok(true) | Err(_) => return Ok(unloadable_publication(core, source_path)),
            Ok(false) => {}
        }
    }

    let standalone = (|| -> Result<_, HostResourceCoreError> {
        if !metadata.is_dir() {
            if spec.resource_type == HostResourceType::Player
                && spec.source_ownership == ResourceFileOwnership::Persistent
            {
                // OptimizeStandalone never edits a persistent player in place.
                let path = write_standalone(
                    _standalone_directory.as_ref(),
                    &standalone_name,
                    &fs::read(&source_path)?,
                )?;
                return Ok((path, ResourceFileOwnership::Temporary, true));
            }
            return Ok((source_path.clone(), spec.source_ownership, false));
        }
        if spec.source_ownership == ResourceFileOwnership::Temporary
            && spec.resource_type != HostResourceType::Player
        {
            return Err(HostResourceCoreError::TemporaryDirectoryUnsupported);
        }
        let packed = pack_directory_standalone(&source_path, spec.group_maker.as_bytes())?;
        let path = if spec.source_ownership == ResourceFileOwnership::Temporary {
            // A temporary directory is destructively packed in place before
            // OptimizeStandalone, just like C4Group_PackDirectory.
            install_staged_file(&source_path, &packed, true)?;
            source_path.clone()
        } else {
            write_standalone(
                _standalone_directory.as_ref(),
                &standalone_name,
                packed.as_slice(),
            )?
        };
        let created = spec.source_ownership != ResourceFileOwnership::Temporary;
        Ok((path, ResourceFileOwnership::Temporary, created))
    })();
    let (standalone_path, standalone_ownership, standalone_created) = match standalone {
        Ok(standalone) => standalone,
        Err(_) if spec.resource_type == HostResourceType::Definitions => {
            return Ok(unloadable_publication(core, source_path));
        }
        Err(error) => return Err(error),
    };

    let finalized = (|| -> Result<(u64, u32), HostResourceCoreError> {
        if spec.resource_type == HostResourceType::Player {
            optimize_player_standalone(&standalone_path, spec.group_maker.as_bytes())?;
        }
        Ok((
            fs::metadata(&standalone_path)?.len(),
            file_crc(&standalone_path)?,
        ))
    })();
    let (physical_size, physical_crc) = match finalized {
        Ok(physical) => physical,
        Err(_) if spec.resource_type == HostResourceType::Definitions => {
            if standalone_created {
                return Ok(HostResourcePublication {
                    core,
                    source_path,
                    standalone_path: Some(standalone_path),
                    standalone_ownership: Some(standalone_ownership),
                });
            }
            return Ok(unloadable_publication(core, source_path));
        }
        Err(error) => {
            if standalone_created {
                let _ = fs::remove_file(&standalone_path);
            }
            return Err(error);
        }
    };

    if spec.resource_type == HostResourceType::Definitions
        && physical_size > u64::from(spec.max_load_file_size)
    {
        if metadata.is_dir() {
            // GetStandalone has already rewritten C4Network2Res::szFile to
            // this temporary packed path before the post-pack limit check
            // clears only szStandalone. Retain the file for AddByFile
            // identity and lifetime cleanup even though the synchronized
            // core stays unloadable.
            return Ok(HostResourcePublication {
                core,
                source_path,
                standalone_path: Some(standalone_path),
                standalone_ownership: Some(standalone_ownership),
            });
        }
        // For an ordinary physical file, szFile is never rewritten and the
        // failed standalone is simply cleared.
        return Ok(unloadable_publication(core, source_path));
    }

    core.loadable = true;
    core.chunk_size = STOCK_CHUNK_SIZE;
    core.file_size = physical_size as u32;
    core.file_crc = physical_crc;
    Ok(HostResourcePublication {
        core,
        source_path,
        standalone_path: Some(standalone_path),
        standalone_ownership: Some(standalone_ownership),
    })
}

struct OptimizedPlayerGroup {
    group: MutableGroup,
    changed: bool,
}

pub(crate) fn optimize_player_standalone(
    standalone_path: &Path,
    group_maker: &[u8],
) -> Result<(), HostResourceCoreError> {
    let group = Group::open(standalone_path)?;
    let filename = standalone_path
        .file_name()
        .map(|filename| clonk_resources::path_to_legacy_bytes(Path::new(filename)))
        .ok_or_else(|| HostResourceCoreError::NonUtf8EntryName(standalone_path.to_path_buf()))?;
    let optimized = optimize_player_group(&group, &filename, group_maker, true, Path::new(""))?;
    if optimized.changed {
        install_staged_file(standalone_path, &optimized.group.pack()?, false)?;
    }
    Ok(())
}

fn optimize_player_group(
    source: &Group,
    filename: &[u8],
    group_maker: &[u8],
    root: bool,
    prefix: &Path,
) -> Result<OptimizedPlayerGroup, HostResourceCoreError> {
    let mut target = MutableGroup::new_bytes(filename.to_vec());
    if let Some(header) = source.rewrite_header_template() {
        target.set_rewrite_header_template(header);
    }
    if !group_maker.is_empty() {
        target.set_maker_bytes(group_maker);
    }
    let mut changed = source.requires_rewrite();
    let mut changed_children = Vec::new();
    let mut unsupported_opaque_crc = None;

    for entry in source.entries()? {
        let name = entry.name_bytes.as_slice();
        if wildcard_match_ascii(b"Portrait*.*", name)
            || (root
                && name.eq_ignore_ascii_case(b"BigIcon.png")
                && entry.size > MAX_PLAYER_BIG_ICON_SIZE)
        {
            changed = true;
            continue;
        }

        if entry.is_directory {
            let entry_path = prefix.join(&entry.relative_path);
            let child_image = source.read_entry_bytes_exact(&entry)?;
            match Group::from_raw_memory(entry.relative_path.clone(), child_image.clone()) {
                Ok(child) => {
                    let optimized_child =
                        optimize_player_group(&child, name, group_maker, false, &entry_path)?;
                    if optimized_child.changed {
                        let contents_crc = optimized_child.group.contents_crc();
                        let packed = optimized_child.group.pack_raw()?;
                        changed_children.push((
                            entry.name_bytes,
                            packed,
                            contents_crc,
                            unix_time_now(),
                        ));
                        changed = true;
                    } else {
                        let contents_crc = if entry.crc_state == 2 {
                            entry.stored_crc
                        } else {
                            child.contents_crc_or_zero()
                        };
                        target.add_packed_child_bytes_with_metadata(
                            entry.name_bytes,
                            child_image,
                            contents_crc,
                            entry.time,
                            entry.executable,
                        )?;
                    }
                }
                Err(_) => {
                    // Recursive C4Group::Delete simply skips child-marked
                    // payloads that OpenAsChild cannot open.
                    if entry.crc_state != 2 && unsupported_opaque_crc.is_none() {
                        unsupported_opaque_crc = Some((entry_path, entry.crc_state));
                    }
                    target.add_packed_child_bytes_with_metadata(
                        entry.name_bytes,
                        child_image,
                        entry.stored_crc,
                        entry.time,
                        entry.executable,
                    )?;
                }
            }
        } else {
            let data = source.read_entry_bytes_exact(&entry)?;
            let contents_crc = rewritten_existing_file_crc(&entry, name, &data);
            target.add_existing_file_bytes_with_metadata(
                entry.name_bytes,
                data,
                contents_crc,
                entry.time,
                entry.executable,
            )?;
        }
    }

    // Closing a rewritten child moves it back into the mother through
    // AddEntry, which appends the replacement. Known group types sort later;
    // unknown types retain this tail position.
    for (name, data, contents_crc, time) in changed_children {
        target.add_packed_child_bytes_with_metadata(name, data, contents_crc, time, false)?;
    }

    if changed {
        if let Some((path, crc_state)) = unsupported_opaque_crc {
            return Err(HostResourceCoreError::OpaqueChildCrcStateUnsupported { path, crc_state });
        }
    }

    Ok(OptimizedPlayerGroup {
        group: target,
        changed,
    })
}

fn rewritten_existing_file_crc(
    entry: &clonk_resources::GroupEntry,
    filename: &[u8],
    data: &[u8],
) -> u32 {
    if entry.crc_state == 2 {
        entry.stored_crc
    } else if entry.size == 0 {
        0
    } else {
        let data_crc = if entry.crc_state == 1 {
            entry.stored_crc
        } else {
            crc32(0, data)
        };
        crc32(data_crc, filename)
    }
}

fn wildcard_match_ascii(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if value_index >= value.len() {
            break;
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .is_some_and(|byte| byte.eq_ignore_ascii_case(&value[value_index]))
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            let next_value = saved_value + 1;
            pattern_index = saved_pattern;
            value_index = next_value;
            backtrack_value = Some(next_value);
        } else {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

fn unix_time_now() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32
}

fn unloadable_publication(
    core: NetworkResourceCore,
    source_path: PathBuf,
) -> HostResourcePublication {
    HostResourcePublication {
        core,
        source_path,
        standalone_path: None,
        standalone_ownership: None,
    }
}

fn directory_size_exceeds(path: &Path, limit: u64) -> Result<bool, io::Error> {
    let mut total = 0_u64;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = fs::metadata(entry.path())?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
                if total > limit {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub(crate) fn pack_directory_standalone(
    path: &Path,
    group_maker: &[u8],
) -> Result<Vec<u8>, HostResourceCoreError> {
    let filename = path
        .file_name()
        .map(|filename| clonk_resources::path_to_legacy_bytes(Path::new(filename)))
        .ok_or_else(|| HostResourceCoreError::NonUtf8EntryName(path.to_path_buf()))?;
    mutable_directory(path, filename, group_maker)?
        .pack()
        .map_err(Into::into)
}

fn mutable_directory(
    path: &Path,
    filename: Vec<u8>,
    group_maker: &[u8],
) -> Result<MutableGroup, HostResourceCoreError> {
    let mut group = MutableGroup::new_bytes(filename);
    if !group_maker.is_empty() {
        group.set_maker_bytes(group_maker);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let name = clonk_resources::path_to_legacy_bytes(Path::new(&entry.file_name()));
        if ignored_group_entry(&name) {
            continue;
        }
        let metadata = fs::metadata(&entry_path)?;
        if metadata.is_dir() {
            let child = mutable_directory(&entry_path, name.clone(), group_maker)?;
            // PackDirectoryTo first creates the child group as a temporary
            // file, so the parent entry gets that newly-created file's time.
            group.add_child_bytes(name, child)?;
        } else if metadata.is_file() {
            let timestamp = entry_timestamp(&metadata);
            let executable = entry_is_executable(&entry_path);
            if let Ok(child) = Group::open(&entry_path) {
                group.add_packed_child_bytes_with_metadata(
                    name,
                    child.raw_image()?,
                    child.contents_crc_or_zero(),
                    timestamp,
                    executable,
                )?;
                continue;
            }
            group.add_disk_file_bytes_with_metadata(
                name,
                fs::read(entry_path)?,
                timestamp,
                executable,
            )?;
        }
    }
    Ok(group)
}

fn ignored_group_entry(name: &[u8]) -> bool {
    // C4Group_TestIgnore uses the stock "cvs;Thumbs.db" module list and keeps
    // the one special dotfile (src/C4Group.cpp:89,121-125).
    (name.first() == Some(&b'.') && name != b".legacyclonk")
        || name.eq_ignore_ascii_case(b"cvs")
        || name.eq_ignore_ascii_case(b"Thumbs.db")
}

#[cfg(unix)]
fn entry_timestamp(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    metadata.mtime() as u32
}

#[cfg(not(unix))]
fn entry_timestamp(metadata: &fs::Metadata) -> u32 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as u32)
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn entry_is_executable(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    CString::new(path.as_os_str().as_bytes()).is_ok_and(|path| {
        // C4Group::AddEntryOnDisk uses access(X_OK), rather than mode-bit
        // inspection, and only enables it on Linux (src/C4Group.cpp:1488-1493).
        unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 }
    })
}

#[cfg(not(target_os = "linux"))]
fn entry_is_executable(_path: &Path) -> bool {
    false
}

fn install_staged_file(
    target: &Path,
    data: &[u8],
    target_is_directory: bool,
) -> Result<(), HostResourceCoreError> {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let staged = create_staged_file(parent, data)?;
    let backup = unused_staged_path(parent, "backup")?;

    if let Err(error) = fs::rename(target, &backup) {
        let _ = fs::remove_file(&staged);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&staged, target) {
        let rollback = fs::rename(&backup, target);
        let _ = fs::remove_file(&staged);
        if let Err(rollback_error) = rollback {
            return Err(io::Error::other(format!(
                "staged resource install failed ({error}); rollback failed ({rollback_error}); original remains at '{}'",
                backup.display()
            ))
            .into());
        }
        return Err(error.into());
    }

    let cleanup = if target_is_directory {
        fs::remove_dir_all(&backup)
    } else {
        fs::remove_file(&backup)
    };
    cleanup?;
    Ok(())
}

fn create_staged_file(parent: &Path, data: &[u8]) -> Result<PathBuf, HostResourceCoreError> {
    for _ in 0..1_000 {
        let path = next_staged_path(parent, "new");
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                drop(file);
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(HostResourceCoreError::NoStandaloneFilename)
}

fn unused_staged_path(parent: &Path, purpose: &str) -> Result<PathBuf, HostResourceCoreError> {
    for _ in 0..1_000 {
        let path = next_staged_path(parent, purpose);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(HostResourceCoreError::NoStandaloneFilename)
}

fn next_staged_path(parent: &Path, purpose: &str) -> PathBuf {
    let unique = NEXT_STAGED_PATH.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".clonk-rust-{purpose}-{}-{unique}",
        std::process::id()
    ))
}

fn write_standalone(
    directory: &Path,
    source_name: &[u8],
    data: &[u8],
) -> Result<PathBuf, HostResourceCoreError> {
    fs::create_dir_all(directory)?;
    let basename = network_temp_basename(source_name);
    for suffix in 1..=999 {
        let filename = network_temp_candidate(&basename, suffix);
        let filename = String::from_utf8(filename).expect("FindTempResFileName produces ASCII");
        let path = directory.join(filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data) {
                    let _ = fs::remove_file(&path);
                    return Err(error.into());
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(HostResourceCoreError::NoStandaloneFilename)
}

/// C4Network2ResList::FindTempResFileName sanitizes the complete native
/// spelling before GetFilename. Win32 backslashes therefore flatten into the
/// basename, while forward slashes survive until the final split.
pub(crate) fn network_temp_basename(source_name: &[u8]) -> Vec<u8> {
    let safe = source_name
        .iter()
        .copied()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'/' => byte,
            _ => b'_',
        })
        .collect::<Vec<_>>();
    let basename = safe
        .rsplit(|byte| *byte == b'/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(b"Resource");
    if basename.is_empty() || basename == b"." || basename == b".." {
        b"Resource".to_vec()
    } else {
        basename.to_vec()
    }
}

pub(crate) fn network_temp_candidate(basename: &[u8], suffix: u32) -> Vec<u8> {
    if suffix <= 1 {
        return basename.to_vec();
    }
    let split = basename
        .iter()
        .rposition(|byte| *byte == b'.')
        // GetExtension returns the trailing NUL when there is no dot, then
        // FindTempResFileName subtracts one and retains that final byte after
        // the numeric suffix (for example `foo` -> `fo_2o`).
        .unwrap_or_else(|| basename.len().saturating_sub(1));
    let mut candidate = basename[..split].to_vec();
    candidate.push(b'_');
    candidate.extend(suffix.to_string().as_bytes());
    candidate.extend_from_slice(&basename[split..]);
    candidate
}

fn file_crc(path: &Path) -> Result<u32, io::Error> {
    let mut file = File::open(path)?;
    let mut crc = 0;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(crc);
        }
        crc = crc32(crc, &buffer[..read]);
    }
}

fn crc32(initial: u32, data: &[u8]) -> u32 {
    let mut crc = initial ^ u32::MAX;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0_u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    crc ^ u32::MAX
}

#[cfg(test)]
mod tests {
    use super::{network_temp_basename, network_temp_candidate};

    #[test]
    fn cpp_temp_names_sanitize_before_splitting_and_retain_extensionless_tail() {
        assert_eq!(
            network_temp_basename(b"C:\\LC\\Obj?cts.c4d"),
            b"C__LC_Obj_cts.c4d"
        );
        assert_eq!(network_temp_basename(b"Defs/Objects.c4d"), b"Objects.c4d");
        assert_eq!(network_temp_candidate(b"Objects.c4d", 2), b"Objects_2.c4d");
        assert_eq!(network_temp_candidate(b"plain", 2), b"plai_2n");
    }
}
