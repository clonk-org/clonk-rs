//! C++-faithful local resolution for synchronized network-resource cores.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use clonk_engine::NetworkResourceCore;
use clonk_resources::{compress_c4group_image, Group, GroupError, MutableGroupError};
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
pub(crate) struct LocalResourceCandidate {
    path: PathBuf,
    lookup_name: Vec<u8>,
}

impl LocalResourceCandidate {
    pub(crate) fn exact(path: PathBuf) -> Self {
        let lookup_name = clonk_resources::path_to_legacy_bytes(&path);
        Self { path, lookup_name }
    }

    pub(crate) fn with_lookup_name(path: PathBuf, lookup_name: Vec<u8>) -> Self {
        Self { path, lookup_name }
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn lookup_name(&self) -> &[u8] {
        &self.lookup_name
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LocalResourceStandalone {
    BinaryCompatible {
        path: PathBuf,
        ownership: ResourceFileOwnership,
    },
    /// Packed image of a directory candidate whose bytes differ from the
    /// served core. It can never be served, but it is still what has to be
    /// *loaded*: an unpacked directory enumerates in host `readdir` order
    /// while every peer that took the resource as a file enumerates it in
    /// C4CFN_FLS order, and entry order decides material slots
    /// (C4Material.cpp:263-299). C++ never has to make this distinction
    /// because its candidates are always packed files.
    LoadableOnly {
        path: PathBuf,
        ownership: ResourceFileOwnership,
    },
    Unavailable,
}

impl LocalResourceMatch {
    pub fn core(&self) -> &NetworkResourceCore {
        &self.core
    }

    /// The path the engine loads this resource from. Every peer must agree on
    /// its contents entry for entry, so a packed image always wins over the
    /// directory it was packed from.
    pub fn path(&self) -> &Path {
        match &self.standalone {
            LocalResourceStandalone::BinaryCompatible { path, .. }
            | LocalResourceStandalone::LoadableOnly { path, .. } => path,
            LocalResourceStandalone::Unavailable => &self.source_path,
        }
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    /// The servable standalone. A `LoadableOnly` image is deliberately absent:
    /// only a byte-identical one may answer a chunk request.
    pub fn standalone_path(&self) -> Option<&Path> {
        match &self.standalone {
            LocalResourceStandalone::BinaryCompatible { path, .. } => Some(path),
            LocalResourceStandalone::LoadableOnly { .. } | LocalResourceStandalone::Unavailable => {
                None
            }
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
            LocalResourceStandalone::BinaryCompatible { ownership, .. }
            | LocalResourceStandalone::LoadableOnly { ownership, .. } => Some(ownership),
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
            // Logical like the Unavailable arm — the catalog clears its chunk
            // set, so it is never advertised or served — but loaded from the
            // packed image rather than the directory it was packed from.
            Self {
                core,
                source_path: _,
                standalone: LocalResourceStandalone::LoadableOnly { path, .. },
            } => backend.register_local_logical(core, path),
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
    let candidates = candidates
        .into_iter()
        .map(|candidate| LocalResourceCandidate::exact(candidate.as_ref().to_path_buf()))
        .collect::<Vec<_>>();
    resolve_local_resource_candidates_with_group_maker(
        core,
        &candidates,
        standalone_directory,
        group_maker,
    )
}

pub(crate) fn resolve_local_resource_candidates_with_group_maker(
    core: &NetworkResourceCore,
    candidates: &[LocalResourceCandidate],
    standalone_directory: impl AsRef<Path>,
    group_maker: &[u8],
) -> Result<LocalResourceResolution, LocalResourceResolutionError> {
    let standalone_directory = standalone_directory.as_ref();
    for candidate in candidates {
        let path = candidate.path.as_path();
        let metadata = fs::metadata(path).ok();
        let opened_group = open_group_candidate(path);
        let standalone_name = opened_group
            .as_ref()
            .map(|group| opened_local_group_name(candidate, group))
            .unwrap_or_else(|| candidate.lookup_name().to_vec());
        let contents_crc = if let Some(group) = opened_group.as_ref() {
            group.contents_crc_or_zero()
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

        let from_directory = metadata.as_ref().is_some_and(fs::Metadata::is_dir);
        let standalone_result = if from_directory {
            crate::host_resource_core::pack_directory_standalone(path, group_maker)
                .ok()
                .and_then(|packed| {
                    write_standalone(standalone_directory, &standalone_name, &packed)
                        .map(|path| (path, ResourceFileOwnership::Temporary))
                        .ok()
                })
        } else if metadata.as_ref().is_some_and(fs::Metadata::is_file) {
            Some((path.to_path_buf(), ResourceFileOwnership::Persistent))
        } else if let Some(group) = opened_group.as_ref() {
            group
                .raw_image()
                .map_err(LocalResourceResolutionError::from)
                // ExtractEntry wraps every child group in the on-disk gzip
                // envelope. Player resources are optimized afterward, but
                // the extraction itself is type-independent.
                .and_then(|image| compress_c4group_image(&image).map_err(Into::into))
                .and_then(|packed| {
                    write_standalone(standalone_directory, &standalone_name, &packed)
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
                    &standalone_name,
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
                } else if from_directory {
                    // The image is not servable, but discarding it would leave
                    // the directory as the load source and its readdir order
                    // as this peer's entry order.
                    Some(LocalResourceStandalone::LoadableOnly {
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
    standalone_name: &[u8],
    standalone_path: PathBuf,
    ownership: ResourceFileOwnership,
    group_maker: &[u8],
) -> Option<(PathBuf, ResourceFileOwnership)> {
    let (standalone_path, ownership) = if ownership == ResourceFileOwnership::Persistent {
        let source = fs::read(&standalone_path).ok()?;
        let standalone_path =
            write_standalone(standalone_directory, standalone_name, &source).ok()?;
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

fn opened_local_group_name(candidate: &LocalResourceCandidate, group: &Group) -> Vec<u8> {
    let separator = std::path::MAIN_SEPARATOR as u8;
    let mut opened_name = candidate
        .lookup_name()
        .iter()
        .map(|byte| if *byte == b'\\' { separator } else { *byte })
        .collect::<Vec<_>>();

    let mut lookup_physical = candidate.path.clone();
    let mut selected_child_count = 0;
    while !lookup_physical.exists() {
        if !lookup_physical.pop() {
            break;
        }
        selected_child_count += 1;
    }
    if selected_child_count != 0 {
        for _ in 0..selected_child_count {
            truncate_last_legacy_component(&mut opened_name);
        }
        let mut selected_components = group
            .root()
            .components()
            .rev()
            .filter_map(|component| match component {
                std::path::Component::Normal(name) => {
                    Some(clonk_resources::path_to_legacy_bytes(Path::new(name)))
                }
                _ => None,
            })
            .take(selected_child_count)
            .collect::<Vec<_>>();
        selected_components.reverse();
        for component in selected_components {
            if !opened_name.is_empty()
                && !opened_name
                    .last()
                    .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
            {
                opened_name.push(separator);
            }
            opened_name.extend(component);
        }
    }

    #[cfg(windows)]
    if let Some(actual_basename) = original_local_basename_bytes(group.root()) {
        let basename_start = opened_name
            .iter()
            .rposition(|byte| matches!(byte, b'/' | b'\\'))
            .map_or(0, |index| index + 1);
        opened_name.truncate(basename_start);
        opened_name.extend(actual_basename);
    }

    opened_name
}

fn truncate_last_legacy_component(path: &mut Vec<u8>) {
    while path.last().is_some_and(|byte| matches!(byte, b'/' | b'\\')) && path.len() > 1 {
        path.pop();
    }
    match path.iter().rposition(|byte| matches!(byte, b'/' | b'\\')) {
        Some(0) => path.truncate(1),
        Some(separator) => path.truncate(separator),
        None => path.clear(),
    }
}

#[cfg(windows)]
fn original_local_basename_bytes(path: &Path) -> Option<Vec<u8>> {
    let requested = clonk_resources::path_to_legacy_bytes(Path::new(path.file_name()?));
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::read_dir(parent).ok()?.find_map(|entry| {
        let entry = entry.ok()?;
        let name = clonk_resources::path_to_legacy_bytes(Path::new(&entry.file_name()));
        wildcard_match_legacy(&requested, &name).then_some(name)
    })
}

#[cfg(windows)]
fn wildcard_match_legacy(pattern: &[u8], value: &[u8]) -> bool {
    let (mut pattern_index, mut value_index) = (0usize, 0usize);
    let (mut backtrack_pattern, mut backtrack_value) = (None, None);
    while pattern_index < pattern.len() || backtrack_pattern.is_some() {
        if pattern.get(pattern_index) == Some(&b'*') {
            pattern_index += 1;
            backtrack_pattern = Some(pattern_index);
            backtrack_value = Some(value_index);
        } else if pattern.get(pattern_index) == Some(&b'?')
            || pattern
                .get(pattern_index)
                .zip(value.get(value_index))
                .is_some_and(|(left, right)| {
                    legacy_byte_capital(*left) == legacy_byte_capital(*right)
                })
        {
            pattern_index += 1;
            value_index += 1;
        } else if let (Some(saved_pattern), Some(saved_value)) =
            (backtrack_pattern, backtrack_value)
        {
            pattern_index = saved_pattern;
            value_index = saved_value + 1;
            backtrack_value = Some(value_index);
        } else {
            return false;
        }
        if value_index > value.len() {
            return false;
        }
    }
    pattern_index == pattern.len() && value_index == value.len()
}

#[cfg(windows)]
fn legacy_byte_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - 32,
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

fn write_standalone(
    directory: &Path,
    candidate_name: &[u8],
    data: &[u8],
) -> Result<PathBuf, LocalResourceResolutionError> {
    fs::create_dir_all(directory)?;
    let filename = crate::host_resource_core::network_temp_basename(candidate_name);
    for suffix in 1..=999 {
        let filename = crate::host_resource_core::network_temp_candidate(&filename, suffix);
        let path = directory.join(clonk_resources::path_from_legacy_bytes(&filename));
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
