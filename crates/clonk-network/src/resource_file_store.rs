//! Filesystem backend for stock C4 network resources.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use clonk_engine::NetworkResourceCore;

const MAX_TEMP_SUFFIX: u32 = 999;

#[derive(Debug)]
pub enum ResourceFileStoreError {
    Io(io::Error),
    DuplicateResource(i32),
    DuplicatePendingDerivation(i32),
    UnknownResource(i32),
    UnknownPendingDerivation(i32),
    ResourceNotComplete(i32),
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
            Self::DuplicatePendingDerivation(resource_id) => write!(
                formatter,
                "resource {resource_id} already has a pending derivation"
            ),
            Self::UnknownResource(resource_id) => {
                write!(formatter, "resource {resource_id} is not registered")
            }
            Self::UnknownPendingDerivation(resource_id) => write!(
                formatter,
                "resource {resource_id} has no pending derivation"
            ),
            Self::ResourceNotComplete(resource_id) => {
                write!(formatter, "resource {resource_id} is not complete")
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

#[derive(Debug, Clone)]
enum ResourceFileState {
    Complete,
    Loading {
        received_chunks: BTreeSet<u32>,
        chunk_count: u32,
    },
}

#[derive(Debug, Clone)]
struct ResourceFile {
    core: NetworkResourceCore,
    path: PathBuf,
    ownership: ResourceFileOwnership,
    local: bool,
    state: ResourceFileState,
}

#[derive(Debug)]
pub struct ResourceFileStore {
    root: PathBuf,
    resources: HashMap<i32, ResourceFile>,
    pending_derived: HashMap<i32, ResourceFile>,
    cleanup_temporary_on_drop: bool,
    non_owning_paths: BTreeSet<PathBuf>,
    candidate_owned_temporary_paths: BTreeSet<PathBuf>,
}

impl ResourceFileStore {
    pub fn new(root: impl AsRef<Path>) -> Result<Self, ResourceFileStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            resources: HashMap::new(),
            pending_derived: HashMap::new(),
            cleanup_temporary_on_drop: true,
            non_owning_paths: BTreeSet::new(),
            candidate_owned_temporary_paths: BTreeSet::new(),
        })
    }

    pub(crate) fn clone_for_round(
        &self,
        root: impl AsRef<Path>,
    ) -> Result<Self, ResourceFileStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let non_owning_paths = self
            .resources
            .values()
            .chain(self.pending_derived.values())
            .map(|resource| resource.path.clone())
            .collect();
        Ok(Self {
            root,
            resources: self.resources.clone(),
            // A pending derive belongs to the old round. Its temporary file is
            // still owned by the old store until the prepared replacement is
            // committed.
            pending_derived: HashMap::new(),
            cleanup_temporary_on_drop: false,
            non_owning_paths,
            candidate_owned_temporary_paths: BTreeSet::new(),
        })
    }

    pub(crate) fn disarm_temporary_cleanup(&mut self) {
        self.cleanup_temporary_on_drop = false;
    }

    pub(crate) fn arm_temporary_cleanup(&mut self) {
        self.cleanup_unreferenced_candidate_paths();
        self.non_owning_paths.clear();
        self.cleanup_temporary_on_drop = true;
    }

    pub(crate) fn arm_after_replacing(&mut self, previous: &mut Self) {
        let retained_paths = self
            .resources
            .values()
            .chain(self.pending_derived.values())
            .map(|resource| resource.path.clone())
            .collect::<BTreeSet<_>>();
        previous
            .resources
            .values_mut()
            .chain(previous.pending_derived.values_mut())
            .filter(|resource| retained_paths.contains(&resource.path))
            .for_each(|resource| resource.ownership = ResourceFileOwnership::Persistent);
        self.cleanup_unreferenced_candidate_paths();
        self.non_owning_paths.clear();
        self.cleanup_temporary_on_drop = true;
    }

    fn cleanup_unreferenced_candidate_paths(&mut self) {
        let referenced_paths = self
            .resources
            .values()
            .chain(self.pending_derived.values())
            .map(|resource| resource.path.clone())
            .collect::<BTreeSet<_>>();
        self.candidate_owned_temporary_paths
            .iter()
            .filter(|path| !referenced_paths.contains(*path))
            .for_each(|path| {
                let _ = fs::remove_file(path);
            });
        self.candidate_owned_temporary_paths.clear();
    }

    pub fn root(&self) -> &Path {
        &self.root
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
                local: false,
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
                path: path.clone(),
                ownership,
                local: true,
                state: ResourceFileState::Complete,
            },
        );
        if !self.cleanup_temporary_on_drop
            && ownership == ResourceFileOwnership::Temporary
            && !self.non_owning_paths.contains(&path)
        {
            self.candidate_owned_temporary_paths.insert(path);
        }
        Ok(())
    }

    /// Preserve a complete parent resource before its source is rewritten and
    /// remember that mutable source as an anonymous derived resource.
    pub fn begin_derive(
        &mut self,
        parent_resource_id: i32,
        mutable_source: impl AsRef<Path>,
        ownership: ResourceFileOwnership,
    ) -> Result<(), ResourceFileStoreError> {
        if self.pending_derived.contains_key(&parent_resource_id) {
            return Err(ResourceFileStoreError::DuplicatePendingDerivation(
                parent_resource_id,
            ));
        }

        let mutable_source = mutable_source.as_ref().to_path_buf();
        let metadata = fs::metadata(&mutable_source)?;
        // A persistent C4Group may be an unpacked directory. Its registered
        // parent already serves a packed standalone, while FinishDerive packs
        // the rewritten directory and rebinds the anonymous resource.
        if !metadata.is_file() && !metadata.is_dir() {
            return Err(ResourceFileStoreError::NotRegularFile(mutable_source));
        }

        let parent = self
            .resources
            .get(&parent_resource_id)
            .ok_or(ResourceFileStoreError::UnknownResource(parent_resource_id))?;
        if !matches!(parent.state, ResourceFileState::Complete) {
            return Err(ResourceFileStoreError::ResourceNotComplete(
                parent_resource_id,
            ));
        }
        let mut anonymous_core = parent.core.clone();
        let parent_is_local = parent.local;
        anonymous_core.id = -2;
        anonymous_core.derived_id = parent_resource_id;

        if parent.path == mutable_source {
            let filename = sanitized_basename(&parent.core)
                .ok_or(ResourceFileStoreError::EmptyFilename(parent_resource_id))?;
            let (rescued_path, rescued_file) = self.create_temporary_file(&filename)?;
            drop(rescued_file);
            if let Err(error) = fs::copy(&mutable_source, &rescued_path) {
                let _ = fs::remove_file(&rescued_path);
                return Err(error.into());
            }
            let parent = self
                .resources
                .get_mut(&parent_resource_id)
                .expect("parent resource checked above");
            parent.path = rescued_path;
            parent.ownership = ResourceFileOwnership::Temporary;
        }

        self.pending_derived.insert(
            parent_resource_id,
            ResourceFile {
                core: anonymous_core,
                path: mutable_source,
                ownership,
                local: parent_is_local,
                state: ResourceFileState::Complete,
            },
        );
        Ok(())
    }

    /// Point a pending derivation at the standalone file prepared for its
    /// final resource core.
    pub fn replace_pending_derived_file(
        &mut self,
        parent_resource_id: i32,
        path: impl AsRef<Path>,
        ownership: ResourceFileOwnership,
    ) -> Result<(), ResourceFileStoreError> {
        if !self.pending_derived.contains_key(&parent_resource_id) {
            return Err(ResourceFileStoreError::UnknownPendingDerivation(
                parent_resource_id,
            ));
        }
        let path = path.as_ref().to_path_buf();
        let metadata = fs::metadata(&path)?;
        if !metadata.is_file() {
            return Err(ResourceFileStoreError::NotRegularFile(path));
        }
        let pending = self
            .pending_derived
            .get_mut(&parent_resource_id)
            .expect("pending derivation checked above");
        if pending.ownership == ResourceFileOwnership::Temporary && pending.path != path {
            match fs::remove_file(&pending.path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        pending.path = path;
        pending.ownership = ownership;
        Ok(())
    }

    /// Bind a matching anonymous derivation to its allocated resource ID.
    /// C++ FinishDerive trusts the announced core and does not validate the
    /// standalone file's size or CRC here.
    pub fn finish_derived(
        &mut self,
        core: &NetworkResourceCore,
    ) -> Result<PathBuf, ResourceFileStoreError> {
        if !self.pending_derived.contains_key(&core.derived_id) {
            return Err(ResourceFileStoreError::UnknownPendingDerivation(
                core.derived_id,
            ));
        }
        if self.resources.contains_key(&core.id) {
            return Err(ResourceFileStoreError::DuplicateResource(core.id));
        }
        let mut resource = self
            .pending_derived
            .remove(&core.derived_id)
            .expect("pending derivation checked above");
        resource.core = core.clone();
        resource.state = ResourceFileState::Complete;
        let path = resource.path.clone();
        self.resources.insert(core.id, resource);
        Ok(path)
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

    /// Whether the complete bytes came from a local candidate rather than a
    /// network download. Ownership is deliberately separate: a locally packed
    /// standalone can be temporary, while a completed download remains remote
    /// even after its temporary file becomes complete.
    pub fn is_local(&self, resource_id: i32) -> bool {
        self.resources
            .get(&resource_id)
            .is_some_and(|resource| resource.local)
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
        // C4Network2ResChunk::Set caps this with the fixed `C4NetResChunkSize`
        // literal (`src/C4Network2Res.cpp:1269`) rather than the core's own
        // stride. The two are the same value for every core C++ publishes
        // (`src/C4Network2Res.cpp:81`, `:89`), but this port publishes a smaller
        // stride, and the literal would then serve each chunk overlapping the
        // following ones -- the whole file many times over per delivery.
        let length =
            (u64::from(resource.core.file_size) - offset).min(u64::from(resource.core.chunk_size));
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
        if self.cleanup_temporary_on_drop && resource.ownership == ResourceFileOwnership::Temporary
        {
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
        if !self.cleanup_temporary_on_drop {
            self.candidate_owned_temporary_paths
                .iter()
                .for_each(|path| {
                    let _ = fs::remove_file(path);
                });
            return;
        }
        self.resources
            .values()
            .chain(self.pending_derived.values())
            .for_each(|resource| {
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
    clonk_resources::c4group_crc32(initial, data)
}
