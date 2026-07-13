//! C++-faithful publication of complete host-side network resources.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(not(unix))]
use std::time::UNIX_EPOCH;

use lc_engine::{LegacyCString, NetworkResourceCore};
use lc_resources::{Group, GroupError, MutableGroup, MutableGroupError};
use thiserror::Error;

use crate::ResourceFileOwnership;

const STOCK_CHUNK_SIZE: u32 = 100 * 1024;
const DEFAULT_MAX_LOAD_FILE_SIZE: u32 = 100 * 1024 * 1024;

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
    group_maker: String,
    max_load_file_size: u32,
}

impl HostResourceCoreSpec {
    pub fn new(
        resource_type: HostResourceType,
        resource_id: i32,
        resource_name: LegacyCString,
        group_maker: impl Into<String>,
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
            group_maker: group_maker.into(),
            max_load_file_size: DEFAULT_MAX_LOAD_FILE_SIZE,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostResourcePublication {
    pub core: NetworkResourceCore,
    pub source_path: PathBuf,
    pub standalone_path: Option<PathBuf>,
    pub standalone_ownership: Option<ResourceFileOwnership>,
}

#[derive(Debug, Error)]
pub enum HostResourceCoreError {
    #[error("host resource I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("host C4Group could not be read: {0}")]
    Group(#[from] GroupError),
    #[error("host C4Group standalone could not be written: {0}")]
    GroupWriter(#[from] MutableGroupError),
    #[error("NRT_Player publication requires the stock portrait/BigIcon optimization")]
    PlayerOptimizationUnsupported,
    #[error("a C4Group maker unexpectedly contained an interior NUL")]
    InvalidGroupMaker,
    #[error("temporary directory resources require C++'s destructive in-place packing")]
    TemporaryDirectoryUnsupported,
    #[error("directory entry name is not valid UTF-8: {0}")]
    NonUtf8EntryName(PathBuf),
    #[error("packed child groups cannot yet be imported byte-for-byte: {0}")]
    PackedChildGroupUnsupported(PathBuf),
    #[error("directory entry has the C++-significant zero timestamp: {0}")]
    ZeroTimestampUnsupported(PathBuf),
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
    if spec.resource_type == HostResourceType::Player {
        // OptimizeStandalone always enters the player-specific copy/delete
        // path (src/C4Network2Res.cpp:1168-1206). lc-resources intentionally
        // has no mutable packed-group API yet, so claiming success would serve
        // different bytes.
        return Err(HostResourceCoreError::PlayerOptimizationUnsupported);
    }

    let group = Group::open(&source_path).ok();
    let (contents_crc, author) = match group.as_ref() {
        Some(group) => {
            let maker = if group.is_directory() {
                b"Open directory".as_slice()
            } else {
                group.maker_bytes().unwrap_or_default()
            };
            let author = LegacyCString::from_bytes(maker.to_vec())
                .ok_or(HostResourceCoreError::InvalidGroupMaker)?;
            (group.contents_crc()?, author)
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
        chunk_size: STOCK_CHUNK_SIZE,
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

    if spec.resource_type == HostResourceType::Definitions
        && metadata.is_dir()
        && directory_size_exceeds(&source_path, u64::from(spec.max_load_file_size))?
    {
        return Ok(unloadable_publication(core, source_path));
    }

    let (standalone_path, standalone_ownership, physical_size, physical_crc) = if metadata.is_dir()
    {
        if spec.source_ownership == ResourceFileOwnership::Temporary {
            return Err(HostResourceCoreError::TemporaryDirectoryUnsupported);
        }
        let filename = source_path
            .file_name()
            .and_then(|filename| filename.to_str())
            .ok_or_else(|| HostResourceCoreError::NonUtf8EntryName(source_path.clone()))?;
        let mutable = mutable_directory(&source_path, filename, &spec.group_maker)?;
        let packed = mutable.pack()?;
        let path = write_standalone(
            _standalone_directory.as_ref(),
            &source_path,
            packed.as_slice(),
        )?;
        let size = packed.len() as u64;
        let crc = crc32(0, &packed);
        (path, ResourceFileOwnership::Temporary, size, crc)
    } else {
        let size = metadata.len();
        let crc = file_crc(&source_path)?;
        (source_path.clone(), spec.source_ownership, size, crc)
    };

    if spec.resource_type == HostResourceType::Definitions
        && physical_size > u64::from(spec.max_load_file_size)
    {
        return Ok(unloadable_publication(core, source_path));
    }

    core.loadable = true;
    core.file_size = physical_size as u32;
    core.file_crc = physical_crc;
    Ok(HostResourcePublication {
        core,
        source_path,
        standalone_path: Some(standalone_path),
        standalone_ownership: Some(standalone_ownership),
    })
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

fn mutable_directory(
    path: &Path,
    filename: &str,
    group_maker: &str,
) -> Result<MutableGroup, HostResourceCoreError> {
    let mut group = MutableGroup::new(filename);
    group.set_maker(group_maker);
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| HostResourceCoreError::NonUtf8EntryName(entry_path.clone()))?;
        if ignored_group_entry(&name) {
            continue;
        }
        let metadata = fs::metadata(&entry_path)?;
        if metadata.is_dir() {
            let child = mutable_directory(&entry_path, &name, group_maker)?;
            // PackDirectoryTo first creates the child group as a temporary
            // file, so the parent entry gets that newly-created file's time.
            group.add_child(name, child)?;
        } else if metadata.is_file() {
            if Group::open(&entry_path).is_ok() {
                return Err(HostResourceCoreError::PackedChildGroupUnsupported(
                    entry_path,
                ));
            }
            let timestamp = entry_timestamp(&metadata);
            if timestamp == 0 {
                return Err(HostResourceCoreError::ZeroTimestampUnsupported(entry_path));
            }
            let executable = entry_is_executable(&entry_path);
            group.add_file_with_metadata(name, fs::read(entry_path)?, timestamp, executable)?;
        }
    }
    Ok(group)
}

fn ignored_group_entry(name: &str) -> bool {
    // C4Group_TestIgnore uses the stock "cvs;Thumbs.db" module list and keeps
    // the one special dotfile (src/C4Group.cpp:89,121-125).
    (name.starts_with('.') && name != ".legacyclonk")
        || name.eq_ignore_ascii_case("cvs")
        || name.eq_ignore_ascii_case("Thumbs.db")
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

fn write_standalone(
    directory: &Path,
    source: &Path,
    data: &[u8],
) -> Result<PathBuf, HostResourceCoreError> {
    fs::create_dir_all(directory)?;
    let filename = source
        .file_name()
        .map(|name| {
            name.as_encoded_bytes()
                .iter()
                .map(|byte| match byte {
                    b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'/' => char::from(*byte),
                    _ => '_',
                })
                .collect::<String>()
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "Resource".to_owned());
    let (stem, extension) = filename
        .rfind('.')
        .map(|dot| (&filename[..dot], &filename[dot..]))
        .unwrap_or((&filename, ""));
    for suffix in 1..=999 {
        let filename = if suffix == 1 {
            filename.clone()
        } else {
            format!("{stem}_{suffix}{extension}")
        };
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
