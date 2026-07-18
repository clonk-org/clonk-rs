//! C++-faithful local resolution for synchronized network-resource cores.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use lc_engine::NetworkResourceCore;
use lc_resources::{
    compress_c4group_image, Group, GroupError, MutableGroup, MutableGroupError,
};
use thiserror::Error;

use crate::{
    HostResourceType, ResourceFileOwnership, ResourceTransferBackend, ResourceTransferError,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalResourceMatch {
    core: NetworkResourceCore,
    source_path: PathBuf,
    standalone: LocalResourceStandalone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalResourceStandalone {
    BinaryCompatible {
        path: PathBuf,
        ownership: ResourceFileOwnership,
    },
    Unavailable,
}

impl LocalResourceMatch {
    pub fn core(&self) -> &NetworkResourceCore {
        &self.core
    }

    pub fn path(&self) -> &Path {
        self.standalone_path().unwrap_or(&self.source_path)
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn standalone_path(&self) -> Option<&Path> {
        match &self.standalone {
            LocalResourceStandalone::BinaryCompatible { path, .. } => Some(path),
            LocalResourceStandalone::Unavailable => None,
        }
    }

    pub fn binary_compatible(&self) -> bool {
        matches!(
            self.standalone,
            LocalResourceStandalone::BinaryCompatible { .. }
        )
    }

    pub fn standalone_ownership(&self) -> Option<ResourceFileOwnership> {
        match self.standalone {
            LocalResourceStandalone::BinaryCompatible { ownership, .. } => Some(ownership),
            LocalResourceStandalone::Unavailable => None,
        }
    }

    /// Registers either the verified standalone or the logical-only local source.
    pub fn register(
        self,
        backend: &mut ResourceTransferBackend,
    ) -> Result<(), ResourceTransferError> {
        match self {
            Self {
                core,
                source_path: _,
                standalone: LocalResourceStandalone::BinaryCompatible { path, ownership },
            } => backend.register_local_complete(core, path, ownership, true),
            Self {
                core,
                source_path,
                standalone: LocalResourceStandalone::Unavailable,
            } => backend.register_local_logical(core, source_path),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonLoadableResourceMismatch {
    pub resource_id: i32,
    pub filename: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalResourceResolution {
    Local(LocalResourceMatch),
    LoadRemote,
    FatalNonLoadable(NonLoadableResourceMismatch),
}

#[derive(Debug, Error)]
pub enum LocalResourceResolutionError {
    #[error("local resource I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("local C4Group could not be read: {0}")]
    Group(#[from] GroupError),
    #[error("local C4Group standalone could not be written: {0}")]
    GroupWriter(#[from] MutableGroupError),
    #[error("no free local resource standalone filename from 1 through 999")]
    NoStandaloneFilename,
}

/// Resolves the first contents-identical candidate in caller-supplied order.
pub fn resolve_local_resource<I, P>(
    core: &NetworkResourceCore,
    candidates: I,
    standalone_directory: impl AsRef<Path>,
) -> Result<LocalResourceResolution, LocalResourceResolutionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    resolve_local_resource_with_group_maker(core, candidates, standalone_directory, b"")
}

/// Resolves a local resource using the process-wide C4Group maker that a
/// player-file rewrite would stamp into its standalone.
pub fn resolve_local_resource_with_group_maker<I, P>(
    core: &NetworkResourceCore,
    candidates: I,
    standalone_directory: impl AsRef<Path>,
    group_maker: &[u8],
) -> Result<LocalResourceResolution, LocalResourceResolutionError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let standalone_directory = standalone_directory.as_ref();
    for candidate in candidates {
        let path = candidate.as_ref();
        let metadata = fs::metadata(path).ok();
        let opened_group = open_group_candidate(path);
        let contents_crc = if let Some(group) = opened_group.as_ref() {
            let Ok(contents_crc) = group.contents_crc() else {
                continue;
            };
            contents_crc
        } else if metadata.as_ref().is_some_and(fs::Metadata::is_file) {
            let Ok(contents_crc) = file_crc(path) else {
                continue;
            };
            contents_crc
        } else {
            continue;
        };
        if contents_crc != core.contents_crc {
            continue;
        }

        let standalone_result = if metadata.as_ref().is_some_and(fs::Metadata::is_dir) {
            pack_directory(path, group_maker)
                .and_then(|packed| {
                    write_standalone(standalone_directory, path, &packed)
                        .map(|path| (path, ResourceFileOwnership::Temporary))
                })
                .ok()
        } else if metadata.as_ref().is_some_and(fs::Metadata::is_file) {
            Some((path.to_path_buf(), ResourceFileOwnership::Persistent))
        } else if let Some(group) = opened_group.as_ref() {
            group
                .raw_image()
                .map_err(LocalResourceResolutionError::from)
                .and_then(|image| {
                    // ExtractEntry wraps child groups in the on-disk gzip
                    // envelope before OptimizeStandalone opens them.
                    if core.resource_type == HostResourceType::Player as u8 {
                        compress_c4group_image(&image).map_err(Into::into)
                    } else {
                        Ok(image)
                    }
                })
                .and_then(|packed| {
                    write_standalone(standalone_directory, path, &packed)
                        .map(|path| (path, ResourceFileOwnership::Temporary))
                })
                .ok()
        } else {
            None
        };
        let standalone_result = if core.resource_type == HostResourceType::Player as u8 {
            standalone_result.and_then(|(standalone, ownership)| {
                optimize_local_player_standalone(
                    standalone_directory,
                    path,
                    standalone,
                    ownership,
                    group_maker,
                )
            })
        } else {
            standalone_result
        };
        let standalone = standalone_result
            .and_then(|(standalone, ownership)| {
                let compatible = fs::metadata(&standalone)
                    .ok()
                    .zip(file_crc(&standalone).ok())
                    .is_some_and(|(metadata, physical_crc)| {
                        metadata.len() == u64::from(core.file_size) && physical_crc == core.file_crc
                    });
                if compatible {
                    Some(LocalResourceStandalone::BinaryCompatible {
                        path: standalone,
                        ownership,
                    })
                } else {
                    if ownership == ResourceFileOwnership::Temporary {
                        let _ = fs::remove_file(standalone);
                    }
                    None
                }
            })
            .unwrap_or(LocalResourceStandalone::Unavailable);
        return Ok(LocalResourceResolution::Local(LocalResourceMatch {
            core: core.clone(),
            source_path: path.to_path_buf(),
            standalone,
        }));
    }
    Ok(fallback(core))
}

fn optimize_local_player_standalone(
    standalone_directory: &Path,
    source_path: &Path,
    standalone_path: PathBuf,
    ownership: ResourceFileOwnership,
    group_maker: &[u8],
) -> Option<(PathBuf, ResourceFileOwnership)> {
    let (standalone_path, ownership) = if ownership == ResourceFileOwnership::Persistent {
        let source = fs::read(&standalone_path).ok()?;
        let standalone_path = write_standalone(standalone_directory, source_path, &source).ok()?;
        (standalone_path, ResourceFileOwnership::Temporary)
    } else {
        (standalone_path, ownership)
    };

    if crate::host_resource_core::optimize_player_standalone(&standalone_path, group_maker).is_err()
    {
        if ownership == ResourceFileOwnership::Temporary {
            let _ = fs::remove_file(&standalone_path);
        }
        return None;
    }
    Some((standalone_path, ownership))
}

fn open_group_candidate(path: &Path) -> Option<Group> {
    Group::open(path).ok().or_else(|| {
        let mother = path
            .ancestors()
            .skip(1)
            .find(|ancestor| fs::metadata(ancestor).is_ok())?;
        let relative = path.strip_prefix(mother).ok()?;
        Group::open(mother).ok()?.open_child(relative).ok()
    })
}

fn pack_directory(
    path: &Path,
    group_maker: &[u8],
) -> Result<Vec<u8>, LocalResourceResolutionError> {
    let filename = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let group = mutable_directory(path, &filename, group_maker)?;
    group.pack().map_err(Into::into)
}

fn mutable_directory(
    path: &Path,
    filename: &str,
    group_maker: &[u8],
) -> Result<MutableGroup, LocalResourceResolutionError> {
    let mut group = MutableGroup::new(filename);
    if !group_maker.is_empty() {
        group.set_maker_bytes(group_maker);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if ignored_group_entry(&name) {
            continue;
        }
        let entry_path = entry.path();
        let metadata = fs::metadata(&entry_path)?;
        let modified = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs() as u32)
            .unwrap_or(0);
        if metadata.is_dir() {
            let child = mutable_directory(&entry_path, &name, group_maker)?;
            group.add_child_with_metadata(name, child, modified, executable(&metadata))?;
        } else if metadata.is_file() {
            group.add_file_with_metadata(
                name,
                fs::read(entry_path)?,
                modified,
                executable(&metadata),
            )?;
        }
    }
    Ok(group)
}

fn ignored_group_entry(name: &str) -> bool {
    (name.starts_with('.') && name != ".legacyclonk")
        || name.eq_ignore_ascii_case("cvs")
        || name.eq_ignore_ascii_case("Thumbs.db")
}

#[cfg(unix)]
fn executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn write_standalone(
    directory: &Path,
    candidate: &Path,
    data: &[u8],
) -> Result<PathBuf, LocalResourceResolutionError> {
    fs::create_dir_all(directory)?;
    let filename = candidate
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
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
    Err(LocalResourceResolutionError::NoStandaloneFilename)
}

fn fallback(core: &NetworkResourceCore) -> LocalResourceResolution {
    if core.loadable {
        LocalResourceResolution::LoadRemote
    } else {
        LocalResourceResolution::FatalNonLoadable(NonLoadableResourceMismatch {
            resource_id: core.id,
            filename: core.filename.as_bytes().to_vec(),
        })
    }
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
