//! Filesystem backend for stock C4 network resources.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use lc_engine::NetworkResourceCore;

const MAX_TEMP_SUFFIX: u32 = 999;
const STOCK_CHUNK_DATA_SIZE: u64 = 100 * 1024;

#[derive(Debug)]
pub enum ResourceFileStoreError {
    Io(io::Error),
    DuplicateResource(i32),
    UnknownResource(i32),
    ResourceNotLoadable(i32),
    ZeroChunkSize(i32),
    EmptyFilename(i32),
    NoTemporaryFilename,
    NotRegularFile(PathBuf),
    FileSizeMismatch {
        resource_id: i32,
        expected: u64,
        actual: u64,
    },
    FileCrcMismatch {
        resource_id: i32,
        expected: u32,
        actual: u32,
    },
    ChunkOutOfRange {
        resource_id: i32,
        chunk: u32,
        chunk_count: u32,
    },
    ResourceNotLoading(i32),
    ChunkExceedsFile {
        resource_id: i32,
        offset: u64,
        size: usize,
        file_size: u64,
    },
    ShortWrite {
        expected: usize,
        actual: usize,
    },
}

impl fmt::Display for ResourceFileStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "resource file I/O failed: {error}"),
            Self::DuplicateResource(resource_id) => {
                write!(formatter, "resource {resource_id} is already registered")
            }
            Self::UnknownResource(resource_id) => {
                write!(formatter, "resource {resource_id} is not registered")
            }
            Self::ResourceNotLoadable(resource_id) => {
                write!(formatter, "resource {resource_id} is not loadable")
            }
            Self::ZeroChunkSize(resource_id) => {
                write!(formatter, "resource {resource_id} has zero chunk size")
            }
            Self::EmptyFilename(resource_id) => {
                write!(formatter, "resource {resource_id} has an empty filename")
            }
            Self::NoTemporaryFilename => {
                formatter.write_str("no free stock resource filename from 2 through 999")
            }
            Self::NotRegularFile(path) => {
                write!(
                    formatter,
                    "resource path is not a regular file: {}",
                    path.display()
                )
            }
            Self::FileSizeMismatch {
                resource_id,
                expected,
                actual,
            } => write!(
                formatter,
                "resource {resource_id} has {actual} bytes; expected {expected}"
            ),
            Self::FileCrcMismatch {
                resource_id,
                expected,
                actual,
            } => write!(
                formatter,
                "resource {resource_id} CRC is {actual:08x}; expected {expected:08x}"
            ),
            Self::ChunkOutOfRange {
                resource_id,
                chunk,
                chunk_count,
            } => write!(
                formatter,
                "resource {resource_id} chunk {chunk} is outside 0..{chunk_count}"
            ),
            Self::ResourceNotLoading(resource_id) => {
                write!(formatter, "resource {resource_id} is not loading")
            }
            Self::ChunkExceedsFile {
                resource_id,
                offset,
                size,
                file_size,
            } => write!(
                formatter,
                "resource {resource_id} write of {size} bytes at {offset} exceeds {file_size}"
            ),
            Self::ShortWrite { expected, actual } => write!(
                formatter,
                "resource file wrote {actual} bytes; expected {expected}"
            ),
        }
    }
}

impl std::error::Error for ResourceFileStoreError {}

impl From<io::Error> for ResourceFileStoreError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceFileOwnership {
    Persistent,
    Temporary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkWriteOutcome {
    Stored {
        newly_received: bool,
        complete: bool,
    },
    WrittenOutsideChunkRange,
}

#[derive(Debug)]
enum ResourceFileState {
    Complete,
    Loading {
        received_chunks: BTreeSet<u32>,
        chunk_count: u32,
    },
}

#[derive(Debug)]
struct ResourceFile {
    core: NetworkResourceCore,
    path: PathBuf,
    ownership: ResourceFileOwnership,
    state: ResourceFileState,
}

#[derive(Debug)]
pub struct ResourceFileStore {
    root: PathBuf,
    resources: HashMap<i32, ResourceFile>,
}

impl ResourceFileStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ResourceFileStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            resources: HashMap::new(),
        })
    }

    pub fn create_remote(
        &mut self,
        core: &NetworkResourceCore,
    ) -> Result<PathBuf, ResourceFileStoreError> {
        self.validate_new_core(core)?;
        let filename =
            sanitized_basename(core).ok_or(ResourceFileStoreError::EmptyFilename(core.id))?;
        let (path, _file) = self.create_temporary_file(&filename)?;
        let chunk_count = chunk_count(core.file_size, core.chunk_size);
        self.resources.insert(
            core.id,
            ResourceFile {
                core: core.clone(),
                path: path.clone(),
                ownership: ResourceFileOwnership::Temporary,
                state: ResourceFileState::Loading {
                    received_chunks: BTreeSet::new(),
                    chunk_count,
                },
            },
        );
        Ok(path)
    }

    pub fn register_local_complete(
        &mut self,
        core: &NetworkResourceCore,
        path: impl AsRef<Path>,
        ownership: ResourceFileOwnership,
    ) -> Result<(), ResourceFileStoreError> {
        self.validate_new_core(core)?;
        let path = path.as_ref().to_path_buf();
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(ResourceFileStoreError::NotRegularFile(path));
        }
        if metadata.len() != u64::from(core.file_size) {
            return Err(ResourceFileStoreError::FileSizeMismatch {
                resource_id: core.id,
                expected: u64::from(core.file_size),
                actual: metadata.len(),
            });
        }
        let actual_crc = file_crc(&path)?;
        if actual_crc != core.file_crc {
            return Err(ResourceFileStoreError::FileCrcMismatch {
                resource_id: core.id,
                expected: core.file_crc,
                actual: actual_crc,
            });
        }
        self.resources.insert(
            core.id,
            ResourceFile {
                core: core.clone(),
                path,
                ownership,
                state: ResourceFileState::Complete,
            },
        );
        Ok(())
    }

    pub fn path(&self, resource_id: i32) -> Option<&Path> {
        self.resources
            .get(&resource_id)
            .map(|resource| resource.path.as_path())
    }

    pub fn is_complete(&self, resource_id: i32) -> bool {
        self.resources
            .get(&resource_id)
            .is_some_and(|resource| matches!(resource.state, ResourceFileState::Complete))
    }

    pub fn read_chunk(
        &self,
        resource_id: i32,
        chunk: u32,
    ) -> Result<Vec<u8>, ResourceFileStoreError> {
        let resource = self
            .resources
            .get(&resource_id)
            .ok_or(ResourceFileStoreError::UnknownResource(resource_id))?;
        let count = chunk_count(resource.core.file_size, resource.core.chunk_size);
        if chunk >= count {
            return Err(ResourceFileStoreError::ChunkOutOfRange {
                resource_id,
                chunk,
                chunk_count: count,
            });
        }
        let offset = u64::from(chunk) * u64::from(resource.core.chunk_size);
        let length = (u64::from(resource.core.file_size) - offset).min(STOCK_CHUNK_DATA_SIZE);
        let length =
            usize::try_from(length).map_err(|_| ResourceFileStoreError::ChunkOutOfRange {
                resource_id,
                chunk,
                chunk_count: count,
            })?;
        let mut file = File::open(&resource.path)?;
        file.seek(SeekFrom::Start(offset))?;
        let mut data = vec![0; length];
        file.read_exact(&mut data)?;
        Ok(data)
    }

    pub fn write_chunk(
        &mut self,
        resource_id: i32,
        chunk: u32,
        data: &[u8],
    ) -> Result<ChunkWriteOutcome, ResourceFileStoreError> {
        let resource = self
            .resources
            .get_mut(&resource_id)
            .ok_or(ResourceFileStoreError::UnknownResource(resource_id))?;
        if !matches!(resource.state, ResourceFileState::Loading { .. }) {
            return Err(ResourceFileStoreError::ResourceNotLoading(resource_id));
        }
        let offset = u64::from(chunk) * u64::from(resource.core.chunk_size);
        let file_size = u64::from(resource.core.file_size);
        let end = offset.checked_add(data.len() as u64).ok_or(
            ResourceFileStoreError::ChunkExceedsFile {
                resource_id,
                offset,
                size: data.len(),
                file_size,
            },
        )?;
        if end > file_size {
            return Err(ResourceFileStoreError::ChunkExceedsFile {
                resource_id,
                offset,
                size: data.len(),
                file_size,
            });
        }

        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&resource.path)?;
        file.seek(SeekFrom::Start(offset))?;
        let written = file.write(data)?;
        if written != data.len() {
            return Err(ResourceFileStoreError::ShortWrite {
                expected: data.len(),
                actual: written,
            });
        }

        let ResourceFileState::Loading {
            received_chunks,
            chunk_count,
        } = &mut resource.state
        else {
            return Err(ResourceFileStoreError::ResourceNotLoading(resource_id));
        };
        if chunk >= *chunk_count {
            return Ok(ChunkWriteOutcome::WrittenOutsideChunkRange);
        }
        let newly_received = received_chunks.insert(chunk);
        let complete = received_chunks.len() == *chunk_count as usize;
        if complete {
            resource.state = ResourceFileState::Complete;
        }
        Ok(ChunkWriteOutcome::Stored {
            newly_received,
            complete,
        })
    }

    pub fn remove(
        &mut self,
        resource_id: i32,
    ) -> Result<ResourceFileOwnership, ResourceFileStoreError> {
        let resource = self
            .resources
            .remove(&resource_id)
            .ok_or(ResourceFileStoreError::UnknownResource(resource_id))?;
        if resource.ownership == ResourceFileOwnership::Temporary {
            match fs::remove_file(&resource.path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(resource.ownership)
    }

    fn validate_new_core(&self, core: &NetworkResourceCore) -> Result<(), ResourceFileStoreError> {
        if self.resources.contains_key(&core.id) {
            return Err(ResourceFileStoreError::DuplicateResource(core.id));
        }
        if !core.loadable {
            return Err(ResourceFileStoreError::ResourceNotLoadable(core.id));
        }
        if core.chunk_size == 0 {
            return Err(ResourceFileStoreError::ZeroChunkSize(core.id));
        }
        Ok(())
    }

    fn create_temporary_file(
        &self,
        filename: &str,
    ) -> Result<(PathBuf, File), ResourceFileStoreError> {
        let first = self.root.join(filename);
        match create_exclusive(&first) {
            Ok(file) => return Ok((first, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }

        let (stem, extension) = split_extension(filename);
        for suffix in 2..=MAX_TEMP_SUFFIX {
            let candidate = self.root.join(format!("{stem}_{suffix}{extension}"));
            match create_exclusive(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        Err(ResourceFileStoreError::NoTemporaryFilename)
    }
}

impl Drop for ResourceFileStore {
    fn drop(&mut self) {
        self.resources.values().for_each(|resource| {
            if resource.ownership == ResourceFileOwnership::Temporary {
                let _ = fs::remove_file(&resource.path);
            }
        });
    }
}

fn create_exclusive(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

fn sanitized_basename(core: &NetworkResourceCore) -> Option<String> {
    let sanitized = core
        .filename
        .as_bytes()
        .iter()
        .map(|byte| match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'/' => *byte,
            _ => b'_',
        })
        .collect::<Vec<_>>();
    let basename = sanitized.rsplit(|byte| *byte == b'/').next()?;
    (!basename.is_empty()).then(|| String::from_utf8_lossy(basename).into_owned())
}

fn split_extension(filename: &str) -> (&str, &str) {
    filename
        .rfind('.')
        .map(|dot| (&filename[..dot], &filename[dot..]))
        .unwrap_or((filename, ""))
}

fn chunk_count(file_size: u32, chunk_size: u32) -> u32 {
    if file_size == 0 || chunk_size == 0 {
        0
    } else {
        (file_size - 1) / chunk_size + 1
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
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    crc ^ u32::MAX
}
