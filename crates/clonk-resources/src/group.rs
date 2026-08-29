use std::borrow::Cow;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use byteorder::{LittleEndian, ReadBytesExt};
use thiserror::Error;
use walkdir::{Error as WalkDirError, WalkDir};

use crate::group_writer::{
    ImportedPackedChildCoreMetadata, MutableGroup, MutableGroupChildMut, MutableGroupEntryData,
    MutableGroupError,
};

const GROUP_HEADER_SIZE: usize = 204;
const GROUP_ENTRY_SIZE: usize = 316;
const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";
/// A child view may keep a modest parent allocation alive to avoid copying,
/// but a small child must not pin an arbitrarily large decompressed archive.
const MAX_SHARED_PACKED_PARENT_EXCESS_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum GroupError {
    #[error("path does not exist: {0}")]
    Missing(PathBuf),
    #[error("path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("invalid group: {0}")]
    InvalidGroup(String),
    #[error("entry not found: {0}")]
    EntryNotFound(PathBuf),
    #[error("entry is empty: {0}")]
    EmptyEntry(PathBuf),
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct Group {
    kind: GroupKind,
}

// `PackedGroup` is much larger than a `PathBuf`, and boxing it would put an
// allocation in front of every packed-group read for a type that is cloned
// rarely and read constantly. Whether the difference crosses clippy's
// threshold depends on `PathBuf`'s width, so this fires only on some targets.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
enum GroupKind {
    Directory(DirectoryGroup),
    Packed(PackedGroup),
}

#[derive(Debug, Clone)]
struct DirectoryGroup {
    root: PathBuf,
    index: Option<Arc<DirectoryIndex>>,
}

#[derive(Debug)]
struct DirectoryIndex {
    entries: Vec<GroupEntry>,
    first_by_name: HashMap<Vec<u8>, usize>,
}

#[derive(Debug, Clone)]
enum PackedSource {
    File(PathBuf),
    Memory {
        data: Arc<Vec<u8>>,
        range: Range<usize>,
    },
}

impl PackedSource {
    fn from_memory(data: Vec<u8>) -> Self {
        let len = data.len();
        Self::Memory {
            data: Arc::new(data),
            range: 0..len,
        }
    }

    fn memory_slice(&self) -> Option<&[u8]> {
        match self {
            Self::File(_) => None,
            Self::Memory { data, range } => data.get(range.clone()),
        }
    }

    fn from_memory_range(data: &Arc<Vec<u8>>, range: Range<usize>) -> Option<Self> {
        let bytes = data.get(range.clone())?;
        if data.capacity().saturating_sub(bytes.len()) > MAX_SHARED_PACKED_PARENT_EXCESS_BYTES {
            Some(Self::from_memory(bytes.to_vec()))
        } else {
            Some(Self::Memory {
                data: Arc::clone(data),
                range,
            })
        }
    }
}

#[derive(Debug, Clone)]
struct PackedGroup {
    path: PathBuf,
    source: PackedSource,
    header: PackedHeader,
    entries: Vec<PackedEntry>,
    index: HashMap<Vec<u8>, usize>,
    data_offset: u64,
    requires_rewrite: bool,
}

#[derive(Debug, Clone)]
struct PackedHeader {
    maker: String,
    maker_bytes: Vec<u8>,
    maker_field: [u8; 32],
    raw: Box<[u8; GROUP_HEADER_SIZE]>,
}

#[derive(Debug, Clone)]
struct PackedEntry {
    relative_path: PathBuf,
    name_bytes: Vec<u8>,
    is_directory: bool,
    size: u64,
    offset: u64,
    time: u32,
    crc_state: u8,
    stored_crc: u32,
    executable: bool,
}

#[derive(Debug, Clone, Copy)]
enum PackedEntryNamePolicy {
    RootValidated,
    ChildBasename,
}

impl Group {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, GroupError> {
        Self::open_with_directory_index(path.as_ref(), false)
    }

    /// Opens a resource group whose unpacked directory listings remain fixed
    /// for this handle and its opened children. Scenario activation treats its
    /// input tree as immutable, so retaining each native-order listing avoids
    /// re-reading the same directory for every resource probe.
    ///
    /// Ordinary [`Self::open`] groups stay live and continue to observe later
    /// filesystem changes. Each child handle snapshots its own direct listing
    /// when opened; file bytes and recursive content CRCs remain live. Opening
    /// a new indexed group is the refresh boundary.
    pub fn open_indexed<P: AsRef<Path>>(path: P) -> Result<Self, GroupError> {
        Self::open_with_directory_index(path.as_ref(), true)
    }

    fn open_with_directory_index(path: &Path, indexed: bool) -> Result<Self, GroupError> {
        // ENOENT and ENOTDIR mean the entity is absent, exactly as the
        // previous exists() probe classified them. Any other stat failure
        // (EMFILE, EACCES, EIO, ...) keeps its concrete io::Error.
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
                ) =>
            {
                // Not a real reference. C4Group::Open truncates components
                // until one exists, opens that as the mother and reopens the
                // remainder as a child (C4Group.cpp:695-716) — which is how a
                // packed scenario folder addresses its contents, as in
                // `Pack.c4f/Scenario.c4s`.
                return Self::open_below_mother(path, indexed)
                    .ok_or_else(|| GroupError::Missing(path.to_path_buf()));
            }
            Err(error) => return Err(GroupError::Io(error)),
        };
        if metadata.is_dir() {
            if path.file_name().is_some_and(|name| {
                ignored_group_entry_bytes(&crate::path_to_legacy_bytes(Path::new(name)))
            }) {
                return Err(GroupError::InvalidGroup(format!(
                    "ignored directory name: {}",
                    path.display()
                )));
            }
            return Ok(Self {
                kind: GroupKind::Directory(DirectoryGroup::new(path.to_path_buf(), indexed)?),
            });
        }

        match PackedGroup::open(path) {
            Ok(packed) => Ok(Self {
                kind: GroupKind::Packed(packed),
            }),
            Err(err) => Err(err),
        }
    }

    /// `C4Group::Open`'s mother trace-back (C4Group.cpp:697-716): walk up to
    /// the nearest real file or folder, open it, and reopen everything below
    /// it as a child chain. C++ reports one undifferentiated failure for every
    /// way this can go wrong, so the caller keeps its own `Missing` error.
    fn open_below_mother(path: &Path, indexed: bool) -> Option<Self> {
        let mut mother = path.parent()?;
        let mut child = PathBuf::from(path.file_name()?);
        while !mother.as_os_str().is_empty() {
            if fs::metadata(mother).is_ok() {
                return Self::open_with_directory_index(mother, indexed)
                    .and_then(|group| group.open_child(&child))
                    .ok();
            }
            child = Path::new(mother.file_name()?).join(&child);
            mother = mother.parent()?;
        }
        None
    }

    pub fn root(&self) -> &Path {
        match &self.kind {
            GroupKind::Directory(directory) => &directory.root,
            GroupKind::Packed(packed) => &packed.path,
        }
    }

    /// Returns an indexed view of this group. Packed groups are already
    /// indexed; callers on a hot path can retain their original packed handle.
    pub fn indexed(&self) -> Result<Self, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => Ok(Self {
                kind: GroupKind::Directory(directory.indexed()?),
            }),
            GroupKind::Packed(_) => Ok(self.clone()),
        }
    }

    pub fn entries(&self) -> Result<Vec<GroupEntry>, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => directory.entries(),
            GroupKind::Packed(packed) => Ok(packed
                .entries
                .iter()
                .map(|entry| GroupEntry {
                    relative_path: entry.relative_path.clone(),
                    name_bytes: entry.name_bytes.clone(),
                    is_directory: entry.is_directory,
                    size: entry.size,
                    time: entry.time,
                    executable: entry.executable,
                    crc_state: entry.crc_state,
                    stored_crc: entry.stored_crc,
                })
                .collect()),
        }
    }

    pub fn read_file<P: AsRef<Path>>(&self, relative: P) -> Result<Vec<u8>, GroupError> {
        self.read_file_cow(relative).map(Cow::into_owned)
    }

    /// Reads an entry while borrowing directly from an in-memory packed group
    /// whenever possible. Directory and raw-file groups retain the owned-read
    /// behavior required by their backing storage.
    pub fn read_file_cow<P: AsRef<Path>>(&self, relative: P) -> Result<Cow<'_, [u8]>, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => {
                let full_path = directory.resolve_entry(relative.as_ref())?;
                Ok(Cow::Owned(fs::read(full_path)?))
            }
            GroupKind::Packed(packed) => {
                let relative = normalize_path(relative.as_ref());
                packed.read_file_cow(&relative)
            }
        }
    }

    /// Reads a text component with `C4Group::LoadEntryString` semantics.
    ///
    /// Unlike `LoadEntry`, the C++ string loader rejects zero-sized entries.
    /// Keep that distinction separate from `read_file`, whose callers may
    /// legitimately need an empty binary payload.
    pub fn load_entry_string<P: AsRef<Path>>(&self, relative: P) -> Result<Vec<u8>, GroupError> {
        let relative = relative.as_ref();
        let bytes = self.read_file(relative)?;
        if bytes.is_empty() {
            return Err(GroupError::EmptyEntry(relative.to_path_buf()));
        }
        Ok(bytes)
    }

    /// Reads an entry's physical payload, including the raw nested-group image
    /// for child entries. C4Group rewrites copy unchanged child payloads
    /// byte-for-byte when another entry is deleted from the parent.
    pub fn read_entry_bytes<P: AsRef<Path>>(&self, relative: P) -> Result<Vec<u8>, GroupError> {
        self.read_entry_bytes_cow(relative).map(Cow::into_owned)
    }

    /// Reads a physical entry payload without copying memory-backed packed
    /// data. Child-group payloads remain in their raw, uncompressed form.
    pub fn read_entry_bytes_cow<P: AsRef<Path>>(
        &self,
        relative: P,
    ) -> Result<Cow<'_, [u8]>, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => {
                let full_path = directory.resolve_entry(relative.as_ref())?;
                if full_path.is_dir() {
                    return Err(GroupError::InvalidGroup(format!(
                        "entry '{}' is an unpacked directory",
                        relative.as_ref().display()
                    )));
                }
                Ok(Cow::Owned(fs::read(full_path)?))
            }
            GroupKind::Packed(packed) => {
                let relative = normalize_path(relative.as_ref());
                packed.read_entry_bytes_by_path_cow(&relative)
            }
        }
    }

    /// Reads the payload belonging to a concrete entry without round-tripping
    /// its legacy byte-string name through UTF-8. This is required when a
    /// C4Group contains names written in a legacy single-byte charset.
    pub fn read_entry_bytes_exact(&self, entry: &GroupEntry) -> Result<Vec<u8>, GroupError> {
        self.read_entry_bytes_exact_cow(entry).map(Cow::into_owned)
    }

    /// Exact-name counterpart to [`Self::read_entry_bytes_cow`].
    pub fn read_entry_bytes_exact_cow<'a>(
        &'a self,
        entry: &GroupEntry,
    ) -> Result<Cow<'a, [u8]>, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => Ok(Cow::Owned(fs::read(
                directory.root.join(&entry.relative_path),
            )?)),
            GroupKind::Packed(packed) => packed.read_entry_bytes_by_name_cow(&entry.name_bytes),
        }
    }

    /// Returns the complete uncompressed group image. Nested C4Groups are
    /// stored in this raw form even when the outer file is gzip wrapped.
    pub fn raw_image(&self) -> Result<Vec<u8>, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => Err(GroupError::InvalidGroup(format!(
                "'{}' is an unpacked directory",
                directory.root.display()
            ))),
            GroupKind::Packed(packed) => packed.raw_image(),
        }
    }

    /// Reports whether opening the group already introduced a deleted entry.
    /// C4Group::Close rewrites such groups even when no explicit mutation was
    /// requested (for example, after duplicate-name replacement).
    pub fn requires_rewrite(&self) -> bool {
        match &self.kind {
            GroupKind::Directory(_) => false,
            GroupKind::Packed(packed) => packed.requires_rewrite,
        }
    }

    pub fn exists<P: AsRef<Path>>(&self, relative: P) -> bool {
        match &self.kind {
            GroupKind::Directory(directory) => directory.resolve_entry(relative.as_ref()).is_ok(),
            GroupKind::Packed(packed) => {
                let relative = normalize_path(relative.as_ref());
                packed.index.contains_key(&case_fold_group_path(&relative))
            }
        }
    }

    pub fn maker(&self) -> Option<&str> {
        match &self.kind {
            GroupKind::Packed(packed) => Some(packed.header.maker.as_str()),
            _ => None,
        }
    }

    /// Returns the exact NUL-terminated-byte-string body stored in the group
    /// header. Network resource cores serialize this value without a text
    /// transcoding step (`C4Network2Res::SetByGroup`).
    pub fn maker_bytes(&self) -> Option<&[u8]> {
        match &self.kind {
            GroupKind::Packed(packed) => Some(packed.header.maker_bytes.as_slice()),
            _ => None,
        }
    }

    /// Returns all 32 physical maker bytes, including bytes after the first
    /// NUL. C++ `SCopy` overwrites only through the new terminator when a group
    /// is rewritten, so the tail remains significant to the file CRC.
    pub fn maker_field(&self) -> Option<&[u8; 32]> {
        match &self.kind {
            GroupKind::Packed(packed) => Some(&packed.header.maker_field),
            _ => None,
        }
    }

    /// Returns the complete unscrambled header used as C4Group's rewrite
    /// template. Close updates only selected fields and retains password and
    /// reserved bytes from the opened group.
    pub fn rewrite_header_template(&self) -> Option<&[u8; GROUP_HEADER_SIZE]> {
        match &self.kind {
            GroupKind::Packed(packed) => Some(packed.header.raw.as_ref()),
            _ => None,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.kind, GroupKind::Directory(_))
    }

    /// Reports C4Group's exact `GetOriginal()` marker. Directory groups do
    /// not have a packed header and are never marked original.
    pub fn is_original(&self) -> bool {
        match &self.kind {
            GroupKind::Directory(_) => false,
            GroupKind::Packed(packed) => {
                i32::from_le_bytes(
                    packed.header.raw[108..112]
                        .try_into()
                        .expect("C4Group original header field has a fixed width"),
                ) == 1_234_567
            }
        }
    }

    pub fn open_child<P: AsRef<Path>>(&self, relative: P) -> Result<Self, GroupError> {
        let relative = normalize_path(relative.as_ref());
        if crate::path_to_legacy_bytes(&relative).contains(&b'*') {
            return Err(GroupError::InvalidGroup(
                "OpenAsChild: No wildcards allowed".to_string(),
            ));
        }

        let missing = || GroupError::EntryNotFound(relative.clone());
        let mut components = relative.components();
        let Some(Component::Normal(first)) = components.next() else {
            return Err(missing());
        };
        let mut child = self.open_direct_child(Path::new(first))?;
        for component in components {
            let Component::Normal(name) = component else {
                return Err(missing());
            };
            child = child.open_direct_child(Path::new(name))?;
        }
        Ok(child)
    }

    /// Opens the concrete child returned by [`Self::entries`] without
    /// round-tripping its legacy byte-string name through UTF-8.
    pub fn open_child_entry_exact(&self, entry: &GroupEntry) -> Result<Self, GroupError> {
        if entry.name_bytes.contains(&b'*') {
            return Err(GroupError::InvalidGroup(
                "OpenAsChild: No wildcards allowed".to_string(),
            ));
        }
        match &self.kind {
            GroupKind::Directory(directory) => {
                let path = directory.root.join(&entry.relative_path);
                if path.is_dir() {
                    Self::open_with_directory_index(&path, directory.is_indexed())
                } else {
                    Self::from_child_bytes(path.clone(), fs::read(path)?)
                }
            }
            GroupKind::Packed(packed) => packed.open_child_by_name(&entry.name_bytes),
        }
    }

    fn open_direct_child(&self, relative: &Path) -> Result<Self, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => {
                let path = directory.resolve_child_entry(relative)?;
                if path.is_dir() {
                    Self::open_with_directory_index(&path, directory.is_indexed())
                } else {
                    Self::from_child_bytes(path.clone(), fs::read(path)?)
                }
            }
            GroupKind::Packed(packed) => packed.open_child(relative),
        }
    }

    /// Computes `C4Group::EntryCRC32`, including its stored-CRC compatibility
    /// rules for old and new packed entry cores.
    pub fn contents_crc(&self) -> Result<u32, GroupError> {
        match &self.kind {
            GroupKind::Directory(directory) => directory_contents_crc(&directory.root),
            GroupKind::Packed(packed) => packed.contents_crc(),
        }
    }

    fn direct_child_contents_crc(
        &self,
        entry: &GroupEntry,
        physical_data: &[u8],
    ) -> Result<u32, GroupError> {
        let child = match &self.kind {
            GroupKind::Directory(_) => self.open_child(&entry.relative_path)?,
            GroupKind::Packed(packed) => Group::from_raw_memory(
                packed
                    .path
                    .join(path_component_from_name_bytes(&entry.name_bytes)),
                physical_data.to_vec(),
            )?,
        };
        // Child.EntryCRC32 exposes a recursive failure as the numeric value
        // zero; only failure to open this direct child aborts the containing
        // group's CRC pass.
        Ok(child.contents_crc_or_zero())
    }

    /// Computes C4Group::EntryCRC32's observable return value when a nested
    /// CRC calculation fails. A directly unopenable child makes this group
    /// return zero, while a successfully opened parent treats a nested
    /// group's zero result as that child's CRC and continues its own XOR.
    pub fn contents_crc_or_zero(&self) -> u32 {
        match &self.kind {
            GroupKind::Directory(directory) => {
                directory_contents_crc_or_zero(&directory.root).unwrap_or(0)
            }
            GroupKind::Packed(packed) => packed.contents_crc_or_zero().unwrap_or(0),
        }
    }

    /// Opens a packed group from in-memory bytes (gz-wrapped or raw) —
    /// e.g. the PlrData blob a CID_JoinPlr control packet carries
    /// (C4ControlJoinPlayer, C4Control.cpp:731-744 writes the .c4p file
    /// contents into the packet). `path` only labels error messages.
    pub fn from_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        Self::from_packed_bytes(path, data)
    }

    /// Opens bytes with the same envelope requirement as a physical,
    /// top-level C4Group file. This is used when a caller has extracted an
    /// ordinary entry before applying `C4Group_IsGroup` classification.
    pub fn from_top_level_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        if !data.starts_with(&C4GROUP_GZ_MAGIC) && !data.starts_with(&GZ_MAGIC) {
            return Err(GroupError::InvalidGroup(
                "invalid compressed group magic".into(),
            ));
        }
        Self::from_packed_bytes(path, data)
    }

    /// Opens an uncompressed nested-group image without accepting a gzip
    /// wrapper. C4Group::OpenAsChild reads the header in place and therefore
    /// skips child-marked payloads that are standalone compressed groups.
    pub fn from_raw_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        let packed = PackedGroup::from_raw_memory(path, data)?;
        Ok(Self {
            kind: GroupKind::Packed(packed),
        })
    }

    fn from_child_bytes(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        let packed = PackedGroup::from_child_memory(path, data)?;
        Ok(Self {
            kind: GroupKind::Packed(packed),
        })
    }

    fn from_packed_bytes(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        let packed = PackedGroup::from_memory(path, data)?;
        Ok(Self {
            kind: GroupKind::Packed(packed),
        })
    }
}

impl MutableGroup {
    /// Creates a writable C4Group rewrite from a read-only group while
    /// retaining its header template, legacy entry names, timestamps,
    /// executable bits, calculated CRCs, and file/child distinctions.
    ///
    /// Packed children remain opaque raw group images, matching
    /// `C4Group::AppendEntry2StdFile`. Directory children are cloned
    /// recursively so the resulting group has the same child hierarchy.
    pub fn from_group(group: &Group) -> Result<Self, MutableGroupError> {
        let mut mutable = Self::new_bytes(crate::path_to_legacy_bytes(group.root()));
        if let Some(header) = group.rewrite_header_template() {
            mutable.set_rewrite_header_template(header);
        }

        let entries = group
            .entries()
            .map_err(|error| MutableGroupError::SourceGroup(error.to_string()))?;
        for entry in entries {
            if group.is_directory() && entry.is_directory {
                let child = group
                    .open_child(&entry.relative_path)
                    .map_err(|error| MutableGroupError::SourceGroup(error.to_string()))?;
                let child = Self::from_group(&child)?;
                mutable.add_existing_child_bytes_with_metadata(
                    entry.name_bytes,
                    child,
                    entry.time,
                    entry.executable,
                )?;
                continue;
            }

            // A top-level-openable C4Group file inside a folder group is a
            // child too (C4Group_IsGroup/AddEntryOnDisk), but a raw unwrapped
            // nested-group image remains an ordinary file. A recognized
            // child's already-uncompressed image is copied opaquely; opening
            // it eagerly would rewrite its own header.
            if group.is_directory() {
                let path = group.root().join(&entry.relative_path);
                if let Ok(child) = Group::open(path) {
                    let data = child
                        .raw_image()
                        .map_err(|error| MutableGroupError::SourceGroup(error.to_string()))?;
                    let contents_crc = child.contents_crc_or_zero();
                    mutable.add_packed_child_bytes_with_metadata(
                        entry.name_bytes,
                        data,
                        contents_crc,
                        entry.time,
                        entry.executable,
                    )?;
                    continue;
                }
            }

            let data = group
                .read_entry_bytes_exact(&entry)
                .map_err(|error| MutableGroupError::SourceGroup(error.to_string()))?;
            if entry.is_directory {
                // Preserve the original core even when this complete payload
                // cannot be opened. Close calculates CRCs in final entry
                // order, so the writer also retains the successful result for
                // use only if traversal actually reaches this entry.
                let child_contents_crc = group.direct_child_contents_crc(&entry, &data).ok();
                mutable.add_imported_packed_child_core_bytes_with_metadata(
                    entry.name_bytes,
                    data,
                    ImportedPackedChildCoreMetadata {
                        crc_state: entry.crc_state,
                        stored_crc: entry.stored_crc,
                        child_contents_crc,
                        time: entry.time,
                        executable: entry.executable,
                    },
                )?;
            } else {
                mutable.add_imported_file_core_bytes_with_metadata(
                    entry.name_bytes,
                    data,
                    entry.crc_state,
                    entry.stored_crc,
                    entry.time,
                    entry.executable,
                )?;
            }
        }
        Ok(mutable)
    }

    /// Finds a child using C4Group's ASCII-case-insensitive entry matching.
    /// Packed children imported by [`Self::from_group`] are opened lazily, so
    /// untouched sibling children retain their opaque payloads.
    pub fn child_mut(&mut self, name: &str) -> Result<MutableGroupChildMut<'_>, MutableGroupError> {
        let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.name_bytes.eq_ignore_ascii_case(name.as_bytes()))
        else {
            return Ok(MutableGroupChildMut::Missing);
        };

        if let MutableGroupEntryData::PackedChild { data, .. } = &self.entries[index].data {
            let label = PathBuf::from(
                String::from_utf8_lossy(&self.entries[index].name_bytes).into_owned(),
            );
            let source = Group::from_raw_memory(label, data.clone())
                .map_err(|error| MutableGroupError::SourceGroup(error.to_string()))?;
            let child = Self::from_group(&source)?;
            self.entries[index].data = MutableGroupEntryData::Child(Box::new(child));
        }

        if matches!(&self.entries[index].data, MutableGroupEntryData::Child(_)) {
            self.entries[index].mark_child_rewritten();
        }

        Ok(match &mut self.entries[index].data {
            MutableGroupEntryData::Child(child) => MutableGroupChildMut::Child(child),
            MutableGroupEntryData::File(_) | MutableGroupEntryData::ExistingFile { .. } => {
                MutableGroupChildMut::File
            }
            MutableGroupEntryData::PackedChild { .. } => unreachable!("packed child was opened"),
        })
    }
}

impl PackedGroup {
    fn open(path: &Path) -> Result<Self, GroupError> {
        let mut file = File::open(path)?;
        let mut magic = [0u8; 2];
        file.read_exact(&mut magic)?;
        if magic != C4GROUP_GZ_MAGIC && magic != GZ_MAGIC {
            return Err(GroupError::InvalidGroup(
                "invalid compressed group magic".into(),
            ));
        }
        Self::from_source(path.to_path_buf(), PackedSource::File(path.to_path_buf()))
    }

    fn from_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        Self::from_source(path, PackedSource::from_memory(data))
    }

    fn from_raw_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        Self::from_raw_source(path, PackedSource::from_memory(data))
    }

    fn from_raw_source(path: PathBuf, source: PackedSource) -> Result<Self, GroupError> {
        let reader_source = source.clone();
        let data = reader_source.memory_slice().ok_or_else(|| {
            GroupError::InvalidGroup("raw memory group has no memory source".into())
        })?;
        let mut cursor = Cursor::new(data);
        Self::parse_from_reader(
            path,
            source,
            &mut cursor,
            PackedEntryNamePolicy::ChildBasename,
        )
    }

    fn from_child_memory(path: PathBuf, mut data: Vec<u8>) -> Result<Self, GroupError> {
        if data.len() >= 2 && (data[..2] == C4GROUP_GZ_MAGIC || data[..2] == GZ_MAGIC) {
            data = decompress_group(data)?;
        }
        Self::from_raw_memory(path, data)
    }

    fn from_source(path: PathBuf, source: PackedSource) -> Result<Self, GroupError> {
        match source {
            PackedSource::File(file_path) => {
                let mut file = File::open(&file_path)?;
                let mut magic = [0u8; 2];
                let is_compressed = file.read_exact(&mut magic).is_ok()
                    && (magic == C4GROUP_GZ_MAGIC || magic == GZ_MAGIC);
                file.seek(SeekFrom::Start(0))?;
                if is_compressed {
                    let mut compressed = Vec::new();
                    file.read_to_end(&mut compressed)?;
                    let data = decompress_group(compressed)?;
                    let source = PackedSource::from_memory(data);
                    let reader_source = source.clone();
                    let mut cursor =
                        Cursor::new(reader_source.memory_slice().ok_or_else(|| {
                            GroupError::InvalidGroup(
                                "decompressed group has no memory source".into(),
                            )
                        })?);
                    return Self::parse_from_reader(
                        path,
                        source,
                        &mut cursor,
                        PackedEntryNamePolicy::RootValidated,
                    );
                }
                Self::parse_from_reader(
                    path,
                    PackedSource::File(file_path),
                    &mut file,
                    PackedEntryNamePolicy::RootValidated,
                )
            }
            memory @ PackedSource::Memory { .. } => {
                let bytes = memory.memory_slice().ok_or_else(|| {
                    GroupError::InvalidGroup("memory group has invalid bounds".into())
                })?;
                let source = if bytes.len() >= 2
                    && (bytes[..2] == C4GROUP_GZ_MAGIC || bytes[..2] == GZ_MAGIC)
                {
                    PackedSource::from_memory(decompress_group(bytes.to_vec())?)
                } else {
                    memory
                };
                let reader_source = source.clone();
                let mut cursor = Cursor::new(reader_source.memory_slice().ok_or_else(|| {
                    GroupError::InvalidGroup("memory group has invalid bounds".into())
                })?);
                Self::parse_from_reader(
                    path,
                    source,
                    &mut cursor,
                    PackedEntryNamePolicy::RootValidated,
                )
            }
        }
    }

    fn parse_from_reader<R: Read + Seek>(
        path: PathBuf,
        source: PackedSource,
        reader: &mut R,
        entry_name_policy: PackedEntryNamePolicy,
    ) -> Result<Self, GroupError> {
        let mut header_bytes = [0u8; GROUP_HEADER_SIZE];
        reader.read_exact(&mut header_bytes)?;
        mem_unscramble(&mut header_bytes);
        let header = parse_header(&header_bytes)?;

        // The entry table is read sequentially, so the header's count is a
        // claim about bytes that must already be present: each entry costs
        // GROUP_ENTRY_SIZE. Reserve for what the image can actually hold
        // rather than for what it asks for — the count is a raw i32 from
        // attacker-shaped input, and a 204-byte header naming i32::MAX
        // entries otherwise reserves hundreds of gigabytes before the first
        // read_exact gets to reject it.
        let table_start = reader.stream_position()?;
        let image_end = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(table_start))?;
        let readable_entries =
            usize::try_from(image_end.saturating_sub(table_start) / GROUP_ENTRY_SIZE as u64)
                .unwrap_or(usize::MAX);
        let mut entries: Vec<PackedEntry> =
            Vec::with_capacity(header.entry_count.min(readable_entries));
        let mut requires_rewrite = false;
        let mut next_entry_offset = 0;
        for _ in 0..header.entry_count {
            let mut entry_bytes = [0u8; GROUP_ENTRY_SIZE];
            reader.read_exact(&mut entry_bytes)?;
            let mut entry = parse_entry(&entry_bytes, entry_name_policy)?;
            entry.offset = next_entry_offset;
            next_entry_offset += entry.size;
            if let Some(existing) = entries
                .iter()
                .position(|candidate| group_name_eq(&candidate.name_bytes, &entry.name_bytes))
            {
                entries.remove(existing);
                requires_rewrite = true;
            }
            entries.push(entry);
        }

        let data_offset = reader.stream_position()?;
        let index = entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| (case_fold_group_name(&entry.name_bytes), idx))
            .collect();

        Ok(Self {
            path,
            source,
            header: header.header,
            entries,
            index,
            data_offset,
            requires_rewrite,
        })
    }

    fn raw_image(&self) -> Result<Vec<u8>, GroupError> {
        match &self.source {
            PackedSource::File(path) => Ok(fs::read(path)?),
            PackedSource::Memory { .. } => self
                .source
                .memory_slice()
                .map(<[u8]>::to_vec)
                .ok_or_else(|| GroupError::InvalidGroup("memory group has invalid bounds".into())),
        }
    }

    fn read_file_cow(&self, relative: &Path) -> Result<Cow<'_, [u8]>, GroupError> {
        let entry_index = self
            .index
            .get(&case_fold_group_path(relative))
            .copied()
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        self.read_entry_bytes_cow(&self.entries[entry_index])
    }

    fn read_entry_bytes_cow<'a>(
        &'a self,
        entry: &PackedEntry,
    ) -> Result<Cow<'a, [u8]>, GroupError> {
        if entry.size > usize::MAX as u64 {
            return Err(GroupError::InvalidGroup(format!(
                "entry '{}' exceeds platform limits",
                entry.relative_path.display()
            )));
        }
        match &self.source {
            PackedSource::File(path) => {
                let mut file = File::open(path)?;
                file.seek(SeekFrom::Start(
                    self.data_offset.checked_add(entry.offset).ok_or_else(|| {
                        GroupError::InvalidGroup(format!(
                            "entry '{}' has invalid offset",
                            entry.relative_path.display()
                        ))
                    })?,
                ))?;
                let mut buffer = vec![0u8; entry.size as usize];
                file.read_exact(&mut buffer)?;
                Ok(Cow::Owned(buffer))
            }
            PackedSource::Memory { data, range } => {
                let start = self.data_offset.checked_add(entry.offset).ok_or_else(|| {
                    GroupError::InvalidGroup(format!(
                        "entry '{}' has invalid offset",
                        entry.relative_path.display()
                    ))
                })?;
                let end = start.checked_add(entry.size).ok_or_else(|| {
                    GroupError::InvalidGroup(format!(
                        "entry '{}' has invalid size",
                        entry.relative_path.display()
                    ))
                })?;
                let source_len = range.len() as u64;
                if end > source_len {
                    return Err(GroupError::InvalidGroup(format!(
                        "entry '{}' exceeds group bounds",
                        entry.relative_path.display()
                    )));
                }
                let start = range.start + start as usize;
                let end = range.start + end as usize;
                Ok(Cow::Borrowed(&data[start..end]))
            }
        }
    }

    fn read_entry_bytes_by_path_cow(&self, relative: &Path) -> Result<Cow<'_, [u8]>, GroupError> {
        let entry_index = self
            .index
            .get(&case_fold_group_path(relative))
            .copied()
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        self.read_entry_bytes_cow(&self.entries[entry_index])
    }

    fn read_entry_bytes_by_name_cow(&self, name: &[u8]) -> Result<Cow<'_, [u8]>, GroupError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name_bytes == name)
            .ok_or_else(|| GroupError::EntryNotFound(crate::path_from_legacy_bytes(name)))?;
        self.read_entry_bytes_cow(entry)
    }

    fn contents_crc(&self) -> Result<u32, GroupError> {
        self.entries.iter().try_fold(0, |crc, entry| {
            let entry_crc = if entry.crc_state == 2 {
                entry.stored_crc
            } else if entry.is_directory {
                self.open_child_for_crc(entry)?.contents_crc()?
            } else if entry.size == 0 {
                0
            } else {
                let data_crc = if entry.crc_state == 1 {
                    entry.stored_crc
                } else {
                    crc32(0, self.read_entry_bytes_cow(entry)?.as_ref())
                };
                crc32(data_crc, &entry.name_bytes)
            };
            Ok(crc ^ entry_crc)
        })
    }

    fn contents_crc_or_zero(&self) -> Result<u32, GroupError> {
        self.entries.iter().try_fold(0, |crc, entry| {
            let entry_crc = if entry.crc_state == 2 {
                entry.stored_crc
            } else if entry.is_directory {
                self.open_child_for_crc(entry)?.contents_crc_or_zero()
            } else if entry.size == 0 {
                0
            } else {
                let data_crc = if entry.crc_state == 1 {
                    entry.stored_crc
                } else {
                    crc32(0, self.read_entry_bytes_cow(entry)?.as_ref())
                };
                crc32(data_crc, &entry.name_bytes)
            };
            Ok(crc ^ entry_crc)
        })
    }

    /// Mirrors the `CalcCRC32 -> OpenAsChild` path for one child-marked core.
    /// Only `*` is rejected; `?` selects the first stored-order match and that
    /// selection is terminal even when it is an ordinary file.
    fn open_child_for_crc(&self, entry: &PackedEntry) -> Result<Group, GroupError> {
        if entry.name_bytes.contains(&b'*') {
            return Err(GroupError::InvalidGroup(
                "OpenAsChild: No wildcards allowed".to_string(),
            ));
        }
        self.open_child(&path_component_from_name_bytes(&entry.name_bytes))
    }

    fn open_child(&self, relative: &Path) -> Result<Group, GroupError> {
        let pattern = crate::path_to_legacy_bytes(relative);
        // C4Group::GetEntry applies WildcardMatch while walking stored order,
        // then validates only that selected entry as a child.
        let entry = self
            .entries
            .iter()
            .find(|entry| group_name_wildcard_match(&pattern, &entry.name_bytes))
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        self.open_child_entry(entry)
    }

    fn open_child_by_name(&self, name: &[u8]) -> Result<Group, GroupError> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.name_bytes == name)
            .ok_or_else(|| GroupError::EntryNotFound(path_component_from_name_bytes(name)))?;
        self.open_child_entry(entry)
    }

    fn open_child_entry(&self, entry: &PackedEntry) -> Result<Group, GroupError> {
        if !entry.is_directory {
            return Err(GroupError::InvalidGroup(format!(
                "entry '{}' is not a child group",
                entry.relative_path.display()
            )));
        }
        let path = self
            .path
            .join(path_component_from_name_bytes(&entry.name_bytes));
        let packed = match &self.source {
            PackedSource::File(_) => {
                PackedGroup::from_raw_memory(path, self.read_entry_bytes_cow(entry)?.into_owned())?
            }
            PackedSource::Memory { data, range } => {
                let relative_start = self
                    .data_offset
                    .checked_add(entry.offset)
                    .and_then(|offset| usize::try_from(offset).ok())
                    .ok_or_else(|| {
                        GroupError::InvalidGroup(format!(
                            "entry '{}' has invalid offset",
                            entry.relative_path.display()
                        ))
                    })?;
                let size = usize::try_from(entry.size).map_err(|_| {
                    GroupError::InvalidGroup(format!(
                        "entry '{}' exceeds platform limits",
                        entry.relative_path.display()
                    ))
                })?;
                let start = range.start.checked_add(relative_start).ok_or_else(|| {
                    GroupError::InvalidGroup(format!(
                        "entry '{}' has invalid offset",
                        entry.relative_path.display()
                    ))
                })?;
                let end = start.checked_add(size).ok_or_else(|| {
                    GroupError::InvalidGroup(format!(
                        "entry '{}' has invalid size",
                        entry.relative_path.display()
                    ))
                })?;
                if end > range.end {
                    return Err(GroupError::InvalidGroup(format!(
                        "entry '{}' exceeds group bounds",
                        entry.relative_path.display()
                    )));
                }
                let source =
                    PackedSource::from_memory_range(data, start..end).ok_or_else(|| {
                        GroupError::InvalidGroup(format!(
                            "entry '{}' exceeds group bounds",
                            entry.relative_path.display()
                        ))
                    })?;
                PackedGroup::from_raw_source(path, source)?
            }
        };
        Ok(Group {
            kind: GroupKind::Packed(packed),
        })
    }
}

impl DirectoryGroup {
    fn new(root: PathBuf, indexed: bool) -> Result<Self, GroupError> {
        let index = indexed
            .then(|| DirectoryIndex::read(&root))
            .transpose()?
            .map(Arc::new);
        Ok(Self { root, index })
    }

    fn indexed(&self) -> Result<Self, GroupError> {
        match &self.index {
            Some(_) => Ok(self.clone()),
            None => Self::new(self.root.clone(), true),
        }
    }

    fn is_indexed(&self) -> bool {
        self.index.is_some()
    }

    fn entries(&self) -> Result<Vec<GroupEntry>, GroupError> {
        self.index
            .as_ref()
            .map(|index| index.entries.clone())
            .map_or_else(|| directory_entries(&self.root), Ok)
    }

    fn resolve_entry(&self, relative: &Path) -> Result<PathBuf, GroupError> {
        let Some(index) = &self.index else {
            return resolve_directory_entry(&self.root, relative);
        };
        let missing = || GroupError::EntryNotFound(relative.to_path_buf());
        let mut components = relative.components().peekable();
        let mut current_root = self.root.clone();
        let mut current_index = Arc::clone(index);
        let mut resolved_any = false;

        while let Some(component) = components.next() {
            let Component::Normal(requested) = component else {
                return Err(missing());
            };
            let requested = crate::path_to_legacy_bytes(Path::new(requested));
            let entry = current_index
                .first_by_name
                .get(&case_fold_group_name(&requested))
                .and_then(|index| current_index.entries.get(*index))
                .ok_or_else(&missing)?;
            current_root = current_root.join(&entry.relative_path);
            resolved_any = true;

            if components.peek().is_some() {
                if !current_root.is_dir() {
                    return Err(missing());
                }
                current_index = Arc::new(DirectoryIndex::read(&current_root)?);
            }
        }

        resolved_any.then_some(current_root).ok_or_else(missing)
    }

    fn resolve_child_entry(&self, relative: &Path) -> Result<PathBuf, GroupError> {
        let Some(index) = &self.index else {
            return resolve_directory_child_entry(&self.root, relative);
        };
        let pattern = crate::path_to_legacy_bytes(relative);
        index
            .entries
            .iter()
            .find(|entry| group_name_wildcard_match(&pattern, &entry.name_bytes))
            .map(|entry| self.root.join(&entry.relative_path))
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))
    }
}

impl DirectoryIndex {
    fn read(root: &Path) -> Result<Self, GroupError> {
        let entries = directory_entries(root)?;
        let mut first_by_name = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            first_by_name
                .entry(case_fold_group_name(&entry.name_bytes))
                .or_insert(index);
        }
        Ok(Self {
            entries,
            first_by_name,
        })
    }
}

fn directory_entries(root: &Path) -> Result<Vec<GroupEntry>, GroupError> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).min_depth(1).max_depth(1) {
        let entry = entry.map_err(convert_walkdir_error)?;
        if entry.path() == root {
            continue;
        }
        let name_bytes = crate::path_to_legacy_bytes(Path::new(&entry.file_name()));
        if ignored_group_entry_bytes(&name_bytes) {
            continue;
        }
        // Unix C4GroupEntry::Set uses stat(2), so symbolic links project the
        // target's metadata. If stat fails (including a dangling link), C++
        // retains the directory entry with its zero-initialized metadata.
        #[cfg(not(windows))]
        let metadata = fs::metadata(entry.path()).ok();
        // Windows copies the `_finddata_t` entry metadata instead.
        #[cfg(windows)]
        let metadata = Some(entry.metadata().map_err(convert_walkdir_error)?);
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| GroupError::Io(io::Error::other("failed to strip prefix")))?;
        let rel = normalize_path(rel);
        entries.push(GroupEntry {
            relative_path: rel,
            name_bytes,
            is_directory: metadata.as_ref().is_some_and(fs::Metadata::is_dir),
            size: metadata.as_ref().map_or(0, fs::Metadata::len),
            time: metadata.as_ref().map_or(0, directory_entry_time),
            executable: metadata
                .as_ref()
                .is_some_and(|_| directory_entry_is_executable(entry.path())),
            crc_state: 0,
            stored_crc: 0,
        });
    }
    Ok(entries)
}

/// Resolves a folder-backed group entry using the same observable name scan
/// as C4Group's `GRPF_Folder` search: native directory order, ignored entries
/// removed, and ASCII-case-insensitive matching against the actual basename.
///
/// Rust callers may pass a nested convenience path, so resolve each group
/// level separately. Reject non-relative components instead of allowing a
/// path to escape the group root.
fn resolve_directory_entry(root: &Path, relative: &Path) -> Result<PathBuf, GroupError> {
    let missing = || GroupError::EntryNotFound(relative.to_path_buf());
    let mut current = root.to_path_buf();
    let mut resolved_any = false;

    for component in relative.components() {
        let Component::Normal(requested) = component else {
            return Err(missing());
        };
        let requested = crate::path_to_legacy_bytes(Path::new(requested));
        let entries = fs::read_dir(&current).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) {
                missing()
            } else {
                GroupError::Io(error)
            }
        })?;
        let mut matched = None;
        for entry in entries {
            let entry = entry?;
            let name = crate::path_to_legacy_bytes(Path::new(&entry.file_name()));
            if name.eq_ignore_ascii_case(&requested) && !ignored_group_entry_bytes(&name) {
                matched = Some(entry);
                break;
            }
        }
        let entry = matched.ok_or_else(&missing)?;
        current = entry.path();
        resolved_any = true;
    }

    resolved_any.then_some(current).ok_or_else(missing)
}

/// Resolves one `OpenAsChild` component. Unlike the generic concrete-path
/// resolver, classic child opening admits `?` as exactly one native byte and
/// selects the first matching directory entry without sorting.
fn resolve_directory_child_entry(root: &Path, relative: &Path) -> Result<PathBuf, GroupError> {
    let missing = || GroupError::EntryNotFound(relative.to_path_buf());
    let pattern = crate::path_to_legacy_bytes(relative);
    let entries = fs::read_dir(root).map_err(|error| {
        if matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        ) {
            missing()
        } else {
            GroupError::Io(error)
        }
    })?;
    for entry in entries {
        let entry = entry?;
        let name = crate::path_to_legacy_bytes(Path::new(&entry.file_name()));
        if group_name_wildcard_match(&pattern, &name) && !ignored_group_entry_bytes(&name) {
            return Ok(entry.path());
        }
    }
    Err(missing())
}

fn ignored_group_entry_bytes(name: &[u8]) -> bool {
    (name.first() == Some(&b'.') && name != b".legacyclonk")
        || name.eq_ignore_ascii_case(b"cvs")
        || name.eq_ignore_ascii_case(b"Thumbs.db")
}

#[cfg(unix)]
fn directory_entry_time(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::MetadataExt;
    metadata.mtime() as u32
}

#[cfg(not(unix))]
fn directory_entry_time(metadata: &fs::Metadata) -> u32 {
    use std::time::UNIX_EPOCH;
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as u32)
        .unwrap_or(0)
}

#[cfg(target_os = "linux")]
fn directory_entry_is_executable(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    CString::new(path.as_os_str().as_bytes())
        .is_ok_and(|path| unsafe { libc::access(path.as_ptr(), libc::X_OK) == 0 })
}

#[cfg(not(target_os = "linux"))]
fn directory_entry_is_executable(_path: &Path) -> bool {
    false
}

fn directory_contents_crc(root: &Path) -> Result<u32, GroupError> {
    directory_entries(root)?
        .into_iter()
        .try_fold(0, |crc, entry| {
            let path = root.join(&entry.relative_path);
            let child = Group::open(&path).ok();
            let entry_crc = if let Some(child) = child {
                child.contents_crc()?
            } else if entry.size == 0 {
                0
            } else {
                let data_crc = crc32(0, &fs::read(path)?);
                crc32(data_crc, &entry.name_bytes)
            };
            Ok(crc ^ entry_crc)
        })
}

fn directory_contents_crc_or_zero(root: &Path) -> Result<u32, GroupError> {
    directory_entries(root)?
        .into_iter()
        .try_fold(0, |crc, entry| {
            let path = root.join(&entry.relative_path);
            let child = Group::open(&path).ok();
            let entry_crc = if let Some(child) = child {
                child.contents_crc_or_zero()
            } else if entry.size == 0 {
                0
            } else {
                let data_crc = crc32(0, &fs::read(path)?);
                crc32(data_crc, &entry.name_bytes)
            };
            Ok(crc ^ entry_crc)
        })
}

fn parse_header(bytes: &[u8]) -> Result<ParsedHeader, GroupError> {
    let raw: [u8; GROUP_HEADER_SIZE] = bytes
        .try_into()
        .map_err(|_| GroupError::InvalidGroup("invalid header size".into()))?;
    let mut cursor = Cursor::new(bytes);
    let mut id = [0u8; 28];
    cursor.read_exact(&mut id)?;
    if !id.starts_with(GROUP_FILE_ID) || id.get(GROUP_FILE_ID.len()) != Some(&0) {
        return Err(GroupError::InvalidGroup("invalid signature".into()));
    }
    let ver1 = cursor.read_i32::<LittleEndian>()?;
    let ver2 = cursor.read_i32::<LittleEndian>()?;
    if ver1 != 1 || ver2 > 2 {
        return Err(GroupError::InvalidGroup(format!(
            "unsupported version {}.{}",
            ver1, ver2
        )));
    }
    let entries = cursor.read_i32::<LittleEndian>()?;
    let mut maker_field = [0u8; 32];
    cursor.read_exact(&mut maker_field)?;
    let maker_bytes = c_bytes(&maker_field).to_vec();
    let maker = String::from_utf8_lossy(&maker_bytes).into_owned();

    // Skip password and reserved fields
    let mut skip = [0u8; 32 + 4 + 4 + 92];
    cursor.read_exact(&mut skip)?;

    Ok(ParsedHeader {
        header: PackedHeader {
            maker,
            maker_bytes,
            maker_field,
            raw: Box::new(raw),
        },
        entry_count: entries.max(0) as usize,
    })
}

fn parse_entry(
    bytes: &[u8],
    name_policy: PackedEntryNamePolicy,
) -> Result<PackedEntry, GroupError> {
    let mut cursor = Cursor::new(bytes);
    let mut name_bytes = [0u8; 260];
    cursor.read_exact(&mut name_bytes)?;
    let name_bytes = match name_policy {
        PackedEntryNamePolicy::RootValidated => {
            sanitize_group_entry_filename_bytes(c_bytes(&name_bytes))
        }
        PackedEntryNamePolicy::ChildBasename => {
            child_group_entry_filename_bytes(c_bytes(&name_bytes)).to_vec()
        }
    };
    let name = clonk_script::c4_string_from_bytes(&name_bytes);
    let _packed = cursor.read_i32::<LittleEndian>()?;
    let child = cursor.read_i32::<LittleEndian>()? != 0;
    let size = cursor.read_i32::<LittleEndian>()?;
    if size < 0 {
        return Err(GroupError::InvalidGroup(format!(
            "negative entry size for {}",
            name
        )));
    }
    let _unused = cursor.read_i32::<LittleEndian>()?;
    let _offset = cursor.read_i32::<LittleEndian>()?;
    let _time = cursor.read_u32::<LittleEndian>()?;
    let crc_state = cursor.read_u8()?;
    let stored_crc = cursor.read_u32::<LittleEndian>()?;
    let executable = cursor.read_u8()? != 0;
    let mut skip = [0u8; 26];
    cursor.read_exact(&mut skip)?;

    Ok(PackedEntry {
        relative_path: normalize_path(&crate::path_from_legacy_bytes(&name_bytes)),
        name_bytes,
        is_directory: child,
        size: size as u64,
        offset: 0,
        time: _time,
        crc_state,
        stored_crc,
        executable,
    })
}

fn child_group_entry_filename_bytes(name: &[u8]) -> &[u8] {
    let separator = name
        .iter()
        .rposition(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\'));
    separator.map_or(name, |index| &name[index + 1..])
}

fn path_component_from_name_bytes(name: &[u8]) -> PathBuf {
    crate::path_from_legacy_bytes(name)
}

fn crc32(initial: u32, data: &[u8]) -> u32 {
    crate::group_writer::c4group_crc32(initial, data)
}

fn c_bytes(buf: &[u8]) -> &[u8] {
    let nul = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    &buf[..nul]
}

fn sanitize_group_entry_filename_bytes(name: &[u8]) -> Vec<u8> {
    let mut name = if name.is_empty() {
        b"empty".to_vec()
    } else {
        name.to_vec()
    };
    for byte in &mut name {
        if matches!(
            *byte,
            b'/' | b'\\' | b'*' | b'?' | b'<' | b'>' | b';' | b'|' | b':'
        ) {
            *byte = b'_';
        }
    }
    let mut index = 0;
    while index + 1 < name.len() {
        if name[index] == b'.' && name[index + 1] == b'.' {
            name[index] = b'_';
            name[index + 1] = b'_';
            index += 2;
        } else {
            index += 1;
        }
    }
    name
}

/// On-disk C4Group files are gzip streams whose two magic bytes are
/// replaced with `{0x1E, 0x8C}` so stock tools leave them alone
/// (StdGzCompressedFile.h:34, StdGzCompressedFile.cpp:62-95). C++ checks
/// and restores those bytes at the start of every concatenated member.
const C4GROUP_GZ_MAGIC: [u8; 2] = [0x1E, 0x8C];
const GZ_MAGIC: [u8; 2] = [0x1F, 0x8B];

fn decompress_group(mut compressed: Vec<u8>) -> Result<Vec<u8>, GroupError> {
    let mut data = Vec::new();
    let mut offset = 0;

    while offset < compressed.len() {
        match compressed.get(offset..offset + GZ_MAGIC.len()) {
            Some(magic) if magic == C4GROUP_GZ_MAGIC => {
                compressed[offset..offset + GZ_MAGIC.len()].copy_from_slice(&GZ_MAGIC);
            }
            Some(magic) if magic == GZ_MAGIC => {}
            _ => {
                return Err(GroupError::InvalidGroup(
                    "gzip decompression failed: invalid gzip header".to_string(),
                ));
            }
        }

        let input_len = compressed.len() - offset;
        let mut decoder = flate2::bufread::GzDecoder::new(&compressed[offset..]);
        decoder.read_to_end(&mut data).map_err(|error| {
            GroupError::InvalidGroup(format!("gzip decompression failed: {error}"))
        })?;
        let remaining = decoder.into_inner().len();
        let consumed = input_len - remaining;
        if consumed == 0 {
            return Err(GroupError::InvalidGroup(
                "gzip decompression failed: decoder made no progress".to_string(),
            ));
        }
        offset += consumed;
    }

    Ok(data)
}

fn mem_unscramble(buffer: &mut [u8]) {
    for byte in buffer.iter_mut() {
        *byte ^= 237;
    }
    let mut i = 0;
    while i + 2 < buffer.len() {
        buffer.swap(i, i + 2);
        i += 3;
    }
}

fn convert_walkdir_error(err: WalkDirError) -> GroupError {
    if let Some(io_err) = err.io_error() {
        GroupError::Io(io::Error::new(io_err.kind(), io_err.to_string()))
    } else {
        GroupError::Io(io::Error::other(err.to_string()))
    }
}

#[derive(Debug)]
struct ParsedHeader {
    header: PackedHeader,
    entry_count: usize,
}

fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

fn case_fold_group_path(path: &Path) -> Vec<u8> {
    case_fold_group_name(&crate::path_to_legacy_bytes(path))
}

fn case_fold_group_name(name: &[u8]) -> Vec<u8> {
    let mut folded = name.to_vec();
    folded.make_ascii_lowercase();
    folded
}

fn group_name_eq(left: &[u8], right: &[u8]) -> bool {
    left.eq_ignore_ascii_case(right)
}

/// `WildcardMatch` (StdFile.cpp:337-366), the matcher `C4Group::GetEntry`
/// applies while walking stored entry order (C4Group.cpp:1221,:1230).
///
/// The single backtracking point is the last `*` seen: when the tail after it
/// fails to match, the match restarts from that `*` one character further
/// along the name, which is why `a*b*c` can retry a star several times before
/// it succeeds. `?` consumes exactly one character and never the end of the
/// name -- C++ tests the exhausted-name arm before the `?` arm -- while a
/// trailing `*` matches the empty remainder. Case folding is ASCII-only, which
/// is what `tolower` does under the C locale the engine runs in.
pub fn group_name_wildcard_match(pattern: &[u8], name: &[u8]) -> bool {
    let (mut wild, mut pos) = (0usize, 0usize);
    // `pLWild`/`pLPos`: both are set together, so one Option gates both.
    let (mut last_wild, mut last_pos) = (None, 0usize);

    while wild < pattern.len() || last_wild.is_some() {
        if pattern.get(wild) == Some(&b'*') {
            wild += 1;
            last_wild = Some(wild);
            last_pos = pos;
        } else if pos >= name.len() {
            break;
        } else if pattern
            .get(wild)
            .is_some_and(|byte| *byte == b'?' || byte.eq_ignore_ascii_case(&name[pos]))
        {
            wild += 1;
            pos += 1;
        } else if let Some(resume) = last_wild {
            wild = resume;
            last_pos += 1;
            pos = last_pos;
        } else {
            return false;
        }
    }

    wild >= pattern.len() && pos >= name.len()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub relative_path: PathBuf,
    /// The exact post-validation legacy filename bytes used by C4Group.
    pub name_bytes: Vec<u8>,
    pub is_directory: bool,
    pub size: u64,
    pub time: u32,
    pub executable: bool,
    pub crc_state: u8,
    pub stored_crc: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

    #[test]
    fn open_reports_missing_only_for_absent_paths() {
        let dir = tempdir().unwrap();
        assert!(matches!(
            Group::open(dir.path().join("absent.c4g")),
            Err(GroupError::Missing(_))
        ));

        // Traversal through a plain file (ENOTDIR) is "absent" in the
        // c4group model, exactly as the previous exists() probe reported it.
        fs::write(dir.path().join("plain.txt"), b"not a dir").unwrap();
        assert!(matches!(
            Group::open(dir.path().join("plain.txt/child.c4g")),
            Err(GroupError::Missing(_))
        ));

        // Any other stat failure (here EACCES through an unsearchable
        // directory) must keep its concrete io::Error instead of being
        // collapsed into Missing.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sealed = dir.path().join("sealed");
            fs::create_dir(&sealed).unwrap();
            fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();
            let result = Group::open(sealed.join("child.c4g"));
            fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).unwrap();
            match result {
                Err(GroupError::Io(error)) => {
                    assert_eq!(error.raw_os_error(), Some(13), "expected EACCES: {error}");
                }
                other => panic!("expected GroupError::Io(EACCES), got {other:?}"),
            }
        }
    }

    #[test]
    fn open_traces_back_to_the_mother_group_for_a_child_path() {
        // `C4Group::Open` only stats the path when it is a real reference;
        // otherwise it truncates components until one exists, opens that as
        // the mother and reopens the remainder as a child
        // (C4Group.cpp:670-716). A packed scenario folder therefore names its
        // scenarios as `Pack.c4f/Scenario.c4s`, a path no stat can resolve.
        let dir = tempdir().unwrap();
        let inner = packed_group_image_with_entry("Scenario.txt", false, b"[Head]");
        let pack = dir.path().join("Pack.c4f");
        fs::write(
            &pack,
            gz_wrapped(&packed_group_image_with_entry("Scenario.c4s", true, &inner)),
        )
        .unwrap();

        let group = Group::open(pack.join("Scenario.c4s")).unwrap();
        assert_eq!(group.read_file("Scenario.txt").unwrap(), b"[Head]");
    }

    #[test]
    fn open_traces_back_past_several_packed_levels() {
        // C++ truncates in a loop (`do { TruncatePath } while (!FileExists)`,
        // C4Group.cpp:699-702), so an arbitrarily deep chain of packed groups
        // resolves against the one real file at its head. UCC folder groups
        // nest a pack inside a category folder inside the compilation.
        let dir = tempdir().unwrap();
        let scenario = packed_group_image_with_entry("Scenario.txt", false, b"[Head]");
        let inner = packed_group_image_with_entry("Scenario.c4s", true, &scenario);
        let pack = dir.path().join("Pack.c4f");
        fs::write(
            &pack,
            gz_wrapped(&packed_group_image_with_entry("Inner.c4f", true, &inner)),
        )
        .unwrap();

        let group = Group::open(pack.join("Inner.c4f").join("Scenario.c4s")).unwrap();
        assert_eq!(group.read_file("Scenario.txt").unwrap(), b"[Head]");
    }

    #[test]
    fn open_directory_group() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/file.txt"), b"hello").unwrap();
        fs::write(dir.path().join("root.txt"), b"world").unwrap();

        let group = Group::open(dir.path()).unwrap();
        assert_eq!(group.root(), dir.path());
        let entries = group.entries().unwrap();
        assert_eq!(entries.len(), 2);
        let file_entry = entries
            .iter()
            .find(|entry| entry.relative_path == Path::new("root.txt"))
            .unwrap();
        assert!(!file_entry.is_directory);
        assert_eq!(file_entry.size, 5);
        let dir_entry = entries
            .iter()
            .find(|entry| entry.relative_path == Path::new("sub"))
            .unwrap();
        assert!(dir_entry.is_directory);

        let data = group.read_file("sub/file.txt").unwrap();
        assert_eq!(data, b"hello");
    }

    #[test]
    fn indexed_directory_group_holds_a_stable_listing_while_live_group_stays_fresh() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Early.txt"), b"early").unwrap();

        let live = Group::open(dir.path()).unwrap();
        let indexed = Group::open_indexed(dir.path()).unwrap();
        fs::write(dir.path().join("Late.txt"), b"late").unwrap();

        assert!(!indexed.exists("Late.txt"));
        assert!(!indexed
            .entries()
            .unwrap()
            .iter()
            .any(|entry| entry.relative_path == Path::new("Late.txt")));
        assert!(live.exists("Late.txt"));
        assert_eq!(live.read_file("Late.txt").unwrap(), b"late");

        let refreshed = Group::open_indexed(dir.path()).unwrap();
        assert!(refreshed.exists("Late.txt"));
        assert_eq!(refreshed.read_file("Late.txt").unwrap(), b"late");
    }

    #[test]
    fn indexed_directory_children_inherit_the_listing_mode() {
        let dir = tempdir().unwrap();
        let child_path = dir.path().join("Child.c4d");
        fs::create_dir(&child_path).unwrap();
        fs::write(child_path.join("Early.txt"), b"early").unwrap();

        let indexed_root = Group::open_indexed(dir.path()).unwrap();
        let indexed_child = indexed_root.open_child("Child.c4d").unwrap();
        fs::write(child_path.join("Late.txt"), b"late").unwrap();

        assert!(!indexed_child.exists("Late.txt"));
        assert!(Group::open(dir.path())
            .unwrap()
            .open_child("Child.c4d")
            .unwrap()
            .exists("Late.txt"));
        assert!(Group::open_indexed(dir.path())
            .unwrap()
            .open_child("Child.c4d")
            .unwrap()
            .exists("Late.txt"));
    }

    #[test]
    fn indexed_directory_preserves_casefold_and_question_wildcard_resolution() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("teams.txt"), b"teams").unwrap();
        let first = packed_group_image_with_entry("marker.txt", false, b"first");
        let second = packed_group_image_with_entry("marker.txt", false, b"second");
        fs::write(dir.path().join("ChoiceA.c4g"), &first).unwrap();
        fs::write(dir.path().join("ChoiceB.c4g"), &second).unwrap();
        let expected = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| {
                matches!(
                    entry.file_name().as_encoded_bytes(),
                    b"ChoiceA.c4g" | b"ChoiceB.c4g"
                )
            })
            .unwrap();
        let expected_marker = if expected.file_name().as_encoded_bytes() == b"ChoiceA.c4g" {
            &b"first"[..]
        } else {
            &b"second"[..]
        };

        let indexed = Group::open_indexed(dir.path()).unwrap();
        assert_eq!(indexed.read_file("TeAmS.TxT").unwrap(), b"teams");
        let selected = indexed.open_child("cHOICE?.C4G").unwrap();
        assert_eq!(selected.read_file("marker.txt").unwrap(), expected_marker);
    }

    #[test]
    fn directory_group_lookup_is_ascii_case_insensitive() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("teams.txt"), b"teams").unwrap();
        fs::create_dir(dir.path().join("material.c4g")).unwrap();
        fs::write(
            dir.path().join("material.c4g/texmap.txt"),
            b"1=Earth-Rough\n",
        )
        .unwrap();

        let resolved = resolve_directory_entry(dir.path(), Path::new("TEAMS.TXT")).unwrap();
        assert_eq!(
            resolved.strip_prefix(dir.path()).unwrap(),
            Path::new("teams.txt")
        );

        let group = Group::open(dir.path()).unwrap();
        assert!(group.exists("TeAmS.TxT"));
        assert_eq!(group.read_file("Teams.Txt").unwrap(), b"teams");
        let material = group.open_child("MaTeRiAl.C4G").unwrap();
        assert!(material.exists("TEXMAP.TXT"));
        assert_eq!(
            material.read_file("TexMap.Txt").unwrap(),
            b"1=Earth-Rough\n"
        );
    }

    #[test]
    fn directory_group_lookups_hide_cpp_ignored_entries() {
        let dir = tempdir().unwrap();
        for name in ["cVs", "thumbs.DB", ".secret"] {
            fs::write(dir.path().join(name), name.as_bytes()).unwrap();
        }
        fs::create_dir(dir.path().join(".hidden.c4g")).unwrap();
        fs::write(dir.path().join(".hidden.c4g/inside.txt"), b"hidden").unwrap();
        fs::write(dir.path().join(".legacyclonk"), b"visible").unwrap();

        let group = Group::open(dir.path()).unwrap();
        assert_eq!(
            group
                .entries()
                .unwrap()
                .into_iter()
                .map(|entry| entry.relative_path)
                .collect::<Vec<_>>(),
            vec![PathBuf::from(".legacyclonk")]
        );
        for query in ["CVS", "Thumbs.db", ".secret"] {
            assert!(!group.exists(query));
            assert!(matches!(
                group.read_file(query),
                Err(GroupError::EntryNotFound(path)) if path == Path::new(query)
            ));
        }
        assert!(!group.exists(".HIDDEN.C4G"));
        assert!(matches!(
            group.open_child(".hidden.c4g"),
            Err(GroupError::EntryNotFound(path)) if path == Path::new(".hidden.c4g")
        ));

        // The exemption is case-sensitive on the physical basename. Lookup
        // remains case-insensitive, so an uppercase query still finds the
        // actual lowercase `.legacyclonk` entry.
        assert!(group.exists(".LEGACYCLONK"));
        assert_eq!(group.read_file(".LegacyClonk").unwrap(), b"visible");
    }

    #[test]
    fn top_level_ignored_directories_do_not_open_as_groups() {
        let parent = tempdir().unwrap();
        for name in [".hidden", "cvs", ".legacyclonk"] {
            fs::create_dir(parent.path().join(name)).unwrap();
        }

        assert!(Group::open(parent.path().join(".hidden")).is_err());
        assert!(Group::open(parent.path().join("cvs")).is_err());
        assert!(Group::open(parent.path().join(".legacyclonk")).is_ok());
    }

    #[test]
    fn directory_and_packed_groups_have_identical_lookup_results() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("teams.txt"), b"teams").unwrap();
        fs::create_dir(dir.path().join("material.c4g")).unwrap();
        fs::write(dir.path().join("material.c4g/texmap.txt"), b"texmap").unwrap();

        let directory = Group::open(dir.path()).unwrap();
        let packed_bytes = MutableGroup::from_group(&directory)
            .unwrap()
            .pack()
            .unwrap();
        let packed = Group::from_memory(PathBuf::from("same.c4s"), packed_bytes).unwrap();

        for group in [&directory, &packed] {
            assert!(group.exists("TEAMS.TXT"));
            assert_eq!(group.read_file("Teams.Txt").unwrap(), b"teams");
            assert!(!group.exists("missing.txt"));
            assert!(matches!(
                group.read_file("missing.txt"),
                Err(GroupError::EntryNotFound(path)) if path == Path::new("missing.txt")
            ));

            let material = group.open_child("MATERIAL.C4G").unwrap();
            assert!(material.exists("TEXMAP.TXT"));
            assert_eq!(material.read_file("TexMap.Txt").unwrap(), b"texmap");
        }
    }

    #[test]
    fn load_entry_string_rejects_empty_directory_and_packed_entries() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("empty.txt"), []).unwrap();
        fs::write(dir.path().join("nonempty.txt"), b"text").unwrap();

        let directory = Group::open(dir.path()).unwrap();
        let packed_bytes = MutableGroup::from_group(&directory)
            .unwrap()
            .pack()
            .unwrap();
        let packed = Group::from_memory(PathBuf::from("same.c4g"), packed_bytes).unwrap();

        for group in [&directory, &packed] {
            assert_eq!(group.read_file("empty.txt").unwrap(), b"");
            assert!(matches!(
                group.load_entry_string("empty.txt"),
                Err(GroupError::EmptyEntry(path)) if path == Path::new("empty.txt")
            ));
            assert_eq!(group.load_entry_string("nonempty.txt").unwrap(), b"text");
        }
    }

    #[test]
    fn missing_group_errors() {
        assert!(matches!(
            Group::open("/path/does/not/exist"),
            Err(GroupError::Missing(_))
        ));
    }

    #[test]
    fn rejects_group_signature_with_non_nul_suffix() {
        // C4Group::OpenRealGrpFile compares Head.id with C4GroupFileID using
        // SEqual/strcmp, not a prefix match (src/C4Group.cpp:776-779;
        // src/C4Strings.cpp:104-108).
        let mut header = [0u8; GROUP_HEADER_SIZE];
        let mut cursor = Cursor::new(&mut header[..]);
        let mut id = [0u8; 28];
        id[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
        id[GROUP_FILE_ID.len()] = b'X';
        cursor.write_all(&id).unwrap();
        cursor.write_i32::<LittleEndian>(1).unwrap();
        cursor.write_i32::<LittleEndian>(2).unwrap();
        cursor.write_i32::<LittleEndian>(0).unwrap();

        assert!(matches!(
            parse_header(&header),
            Err(GroupError::InvalidGroup(message)) if message == "invalid signature"
        ));
    }

    /// The raw (uncompressed) packed-group image used by the packed tests:
    /// scrambled header + one entry ("hello.txt" -> b"world").
    fn packed_group_image() -> Vec<u8> {
        packed_group_image_with_entry("hello.txt", false, b"world")
    }

    /// Wraps a raw packed image in the on-disk gz envelope, whose magic bytes
    /// are replaced by {0x1E, 0x8C} (StdGzCompressedFile.cpp:62-95).
    fn gz_wrapped(image: &[u8]) -> Vec<u8> {
        let mut compressed = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(image).unwrap();
            encoder.finish().unwrap();
        }
        compressed[0] = 0x1E;
        compressed[1] = 0x8C;
        compressed
    }

    fn packed_group_image_with_entry(name: &str, child: bool, data: &[u8]) -> Vec<u8> {
        packed_group_image_with_entries(&[(name, child, data)])
    }

    fn packed_group_image_with_entries(entries: &[(&str, bool, &[u8])]) -> Vec<u8> {
        let entries = entries
            .iter()
            .map(|(name, child, data)| (name.as_bytes(), *child, *data))
            .collect::<Vec<_>>();
        packed_group_image_with_byte_entries(&entries)
    }

    fn packed_group_image_with_byte_entries(entries: &[(&[u8], bool, &[u8])]) -> Vec<u8> {
        let mut image = Vec::new();

        let mut header = [0u8; GROUP_HEADER_SIZE];
        {
            let mut cursor = Cursor::new(&mut header[..]);
            let mut id_bytes = [0u8; 28];
            id_bytes[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
            cursor.write_all(&id_bytes).unwrap();
            cursor.write_i32::<LittleEndian>(1).unwrap();
            cursor.write_i32::<LittleEndian>(2).unwrap();
            cursor
                .write_i32::<LittleEndian>(i32::try_from(entries.len()).unwrap())
                .unwrap();
            cursor.write_all(&[0u8; 32]).unwrap(); // maker
            cursor.write_all(&[0u8; 32 + 4 + 4 + 92]).unwrap();
        }
        mem_unscramble(&mut header);
        image.extend_from_slice(&header);

        let mut offset = 0usize;
        for (name, child, data) in entries {
            let mut entry = [0u8; GROUP_ENTRY_SIZE];
            {
                let mut cursor = Cursor::new(&mut entry[..]);
                let mut name_bytes = [0u8; 260];
                name_bytes[..name.len()].copy_from_slice(name);
                cursor.write_all(&name_bytes).unwrap();
                cursor.write_i32::<LittleEndian>(0).unwrap();
                cursor.write_i32::<LittleEndian>(i32::from(*child)).unwrap();
                cursor
                    .write_i32::<LittleEndian>(i32::try_from(data.len()).unwrap())
                    .unwrap();
                cursor.write_i32::<LittleEndian>(0).unwrap();
                cursor
                    .write_i32::<LittleEndian>(i32::try_from(offset).unwrap())
                    .unwrap();
                cursor.write_u32::<LittleEndian>(0).unwrap();
                cursor.write_u8(0).unwrap();
                cursor.write_u32::<LittleEndian>(0).unwrap();
                cursor.write_u8(0).unwrap();
                cursor.write_all(&[0u8; 26]).unwrap();
            }
            image.extend_from_slice(&entry);
            offset += data.len();
        }
        for (_, _, data) in entries {
            image.extend_from_slice(data);
        }
        image
    }

    fn gzip_member(data: &[u8], magic: [u8; 2]) -> Vec<u8> {
        let mut compressed = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(data).unwrap();
            encoder.finish().unwrap();
        }
        compressed[..2].copy_from_slice(&magic);
        compressed
    }

    #[test]
    fn decompress_group_accepts_scrambled_magic_on_every_gzip_member() {
        let mut compressed = gzip_member(b"first", C4GROUP_GZ_MAGIC);
        compressed.extend(gzip_member(b"second", C4GROUP_GZ_MAGIC));

        assert_eq!(decompress_group(compressed).unwrap(), b"firstsecond");
    }

    #[test]
    fn decompress_group_accepts_standard_multi_member_gzip() {
        let mut compressed = gzip_member(b"first", GZ_MAGIC);
        compressed.extend(gzip_member(b"second", GZ_MAGIC));

        assert_eq!(decompress_group(compressed).unwrap(), b"firstsecond");
    }

    fn gzip_group_image(image: &[u8]) -> Vec<u8> {
        gzip_member(image, GZ_MAGIC)
    }

    fn packed_group_image_with_entry_count(entry_count: i32) -> Vec<u8> {
        let mut image = packed_group_image_with_entries(&[]);
        let mut header: [u8; GROUP_HEADER_SIZE] = image[..GROUP_HEADER_SIZE].try_into().unwrap();
        mem_unscramble(&mut header);
        header[36..40].copy_from_slice(&entry_count.to_le_bytes());
        mem_unscramble(&mut header);
        image[..GROUP_HEADER_SIZE].copy_from_slice(&header);
        image
    }

    #[test]
    fn open_gz_wrapped_packed_group() {
        // On-disk C4Group files are gzip streams with the magic bytes
        // replaced by {0x1E, 0x8C} (StdGzCompressedFile.h:34,
        // StdGzCompressedFile.cpp:62-95); the packed image lives inside.
        // Real player/scenario files (e.g. Tyler.c4p) use this wrapping.
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.c4p");

        let mut compressed = Vec::new();
        {
            use std::io::Write as _;
            let mut encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(&packed_group_image()).unwrap();
            encoder.finish().unwrap();
        }
        compressed[0] = 0x1E;
        compressed[1] = 0x8C;
        fs::write(&path, &compressed).unwrap();

        let group = Group::open(&path).unwrap();
        let data = group.read_file("hello.txt").unwrap();
        assert_eq!(data, b"world");
    }

    #[test]
    fn top_level_raw_group_file_is_rejected_but_raw_memory_opens() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.c4group");
        let mut file = File::create(&path).unwrap();

        let mut header = [0u8; GROUP_HEADER_SIZE];
        {
            let mut cursor = Cursor::new(&mut header[..]);
            let mut id_bytes = [0u8; 28];
            id_bytes[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
            cursor.write_all(&id_bytes).unwrap();
            cursor.write_i32::<LittleEndian>(1).unwrap();
            cursor.write_i32::<LittleEndian>(2).unwrap();
            cursor.write_i32::<LittleEndian>(1).unwrap();
            cursor.write_all(&[0u8; 32]).unwrap(); // maker
            cursor.write_all(&[0u8; 32 + 4 + 4 + 92]).unwrap();
        }
        mem_unscramble(&mut header);
        file.write_all(&header).unwrap();

        let mut entry = [0u8; GROUP_ENTRY_SIZE];
        {
            let mut cursor = Cursor::new(&mut entry[..]);
            let mut name = [0u8; 260];
            name[..b"hello.txt".len()].copy_from_slice(b"hello.txt");
            cursor.write_all(&name).unwrap();
            cursor.write_i32::<LittleEndian>(0).unwrap();
            cursor.write_i32::<LittleEndian>(0).unwrap();
            cursor.write_i32::<LittleEndian>(5).unwrap();
            cursor.write_i32::<LittleEndian>(0).unwrap();
            cursor.write_i32::<LittleEndian>(0).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();
            cursor.write_u8(0).unwrap();
            cursor.write_u32::<LittleEndian>(0).unwrap();
            cursor.write_u8(0).unwrap();
            cursor.write_all(&[0u8; 26]).unwrap();
        }
        file.write_all(&entry).unwrap();
        file.write_all(b"world").unwrap();
        drop(file);

        Group::open(&path).expect_err("top-level packed files require a gzip envelope");
        let group = Group::from_raw_memory(path.clone(), fs::read(&path).unwrap())
            .expect("raw nested-group parser remains available");
        let entries = group.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, Path::new("hello.txt"));
        assert!(!entries[0].is_directory);
        assert_eq!(entries[0].size, 5);

        let data = group.read_file("hello.txt").unwrap();
        assert_eq!(data, b"world");
        assert!(group.exists("hello.txt"));
        assert_eq!(group.maker(), Some(""));
    }

    #[test]
    fn negative_entry_count_opens_as_an_empty_group() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("negative.c4g");
        fs::write(
            &path,
            gzip_group_image(&packed_group_image_with_entry_count(-1)),
        )
        .unwrap();

        let group = Group::open(path).expect("negative count is the C++ empty-loop case");
        assert!(group.entries().unwrap().is_empty());
    }

    #[test]
    fn packed_group_preserves_raw_maker_bytes() {
        // C4Network2Res::SetByGroup copies C4Group::GetMaker into the network
        // core as a byte string; it does not decode or replace non-UTF-8 bytes
        // (src/C4Network2Res.cpp:409-425; src/C4Group.cpp:2278-2281).
        let mut image = packed_group_image();
        let mut header: [u8; GROUP_HEADER_SIZE] = image[..GROUP_HEADER_SIZE].try_into().unwrap();
        mem_unscramble(&mut header);
        header[40..44].copy_from_slice(&[0xff, b'A', b'B', 0]);
        mem_unscramble(&mut header);
        image[..GROUP_HEADER_SIZE].copy_from_slice(&header);

        let group = Group::from_memory(PathBuf::from("raw-maker.c4g"), image).unwrap();

        assert_eq!(group.maker_bytes(), Some(&[0xff, b'A', b'B'][..]));
    }

    #[test]
    fn packed_group_reconstructs_entry_offsets_from_sizes() {
        // C++ does not pass the serialized entry offset to AddEntry while
        // opening a group; AddEntry rebuilds offsets in directory order from
        // entry sizes (src/C4Group.cpp:784-792, 861-877).
        let mut image = packed_group_image_with_entries(&[
            ("first.txt", false, b"abc"),
            ("second.txt", false, b"world"),
        ]);
        for (index, stored_offset) in [1234_i32, -567].into_iter().enumerate() {
            let offset_field =
                GROUP_HEADER_SIZE + index * GROUP_ENTRY_SIZE + 260 + 4 * std::mem::size_of::<i32>();
            image[offset_field..offset_field + std::mem::size_of::<i32>()]
                .copy_from_slice(&stored_offset.to_le_bytes());
        }

        let group =
            Group::from_memory(PathBuf::from("test.c4group"), image).expect("valid packed group");

        assert_eq!(group.read_file("first.txt").unwrap(), b"abc");
        assert_eq!(group.read_file("second.txt").unwrap(), b"world");
    }

    #[test]
    fn packed_group_entry_lookup_is_ascii_case_insensitive() {
        // C4Group::GetEntry searches entry names with WildcardMatch
        // (src/C4Group.cpp:896-904), whose character comparison ignores case
        // (src/StdFile.cpp:337-367).
        let group = Group::from_memory(PathBuf::from("test.c4group"), packed_group_image())
            .expect("valid packed group");

        assert!(group.exists("HELLO.TXT"));
        assert_eq!(group.read_file("Hello.Txt").unwrap(), b"world");
    }

    #[cfg(unix)]
    #[test]
    fn packed_group_lookup_preserves_distinct_legacy_name_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        fn legacy_path(bytes: &[u8]) -> &Path {
            Path::new(OsStr::from_bytes(bytes))
        }

        // C4Group::GetEntry compares raw name bytes and folds only their ASCII
        // case (src/C4Group.cpp:896-904; src/StdFile.cpp:337-367). Lossy UTF-8
        // would map each of these legacy high bytes to the same U+FFFD key.
        let u_child = packed_group_image_with_entry("marker.txt", false, b"u-child");
        let o_child = packed_group_image_with_entry("marker.txt", false, b"o-child");
        let image = packed_group_image_with_byte_entries(&[
            (b"Gr\xfcn.txt", false, b"u-file"),
            (b"Gr\xf6n.txt", false, b"o-file"),
            (b"Gr\xfcn.c4g", true, &u_child),
            (b"Gr\xf6n.c4g", true, &o_child),
        ]);
        let group = Group::from_memory(PathBuf::from("legacy-names.c4g"), image)
            .expect("valid packed group");

        let u_file = legacy_path(b"gR\xfcN.TXT");
        let o_file = legacy_path(b"GR\xf6n.txt");
        assert!(group.exists(u_file));
        assert!(group.exists(o_file));
        assert!(!group.exists(legacy_path(b"Gr\xe4n.txt")));
        assert_eq!(group.read_file(u_file).unwrap(), b"u-file");
        assert_eq!(group.read_file(o_file).unwrap(), b"o-file");
        assert_eq!(group.read_entry_bytes(u_file).unwrap(), b"u-file");
        assert_eq!(group.read_entry_bytes(o_file).unwrap(), b"o-file");

        let u_child = group.open_child(legacy_path(b"gR\xfcN.C4G")).unwrap();
        let o_child = group.open_child(legacy_path(b"GR\xf6n.c4g")).unwrap();
        assert_eq!(u_child.read_file("marker.txt").unwrap(), b"u-child");
        assert_eq!(o_child.read_file("marker.txt").unwrap(), b"o-child");
    }

    #[test]
    fn packed_exact_child_open_preserves_distinct_legacy_name_bytes() {
        let u_child = packed_group_image_with_entry("marker.txt", false, b"u-child");
        let o_child = packed_group_image_with_entry("marker.txt", false, b"o-child");
        let image = packed_group_image_with_byte_entries(&[
            (b"Gr\xfcn.c4d", true, &u_child),
            (b"Gr\xf6n.C4D", true, &o_child),
            (b"Gr\xe4n.c4d", false, &u_child),
        ]);
        let root_path = PathBuf::from("legacy-children.c4g");
        let group = Group::from_memory(root_path.clone(), image).expect("valid packed group");
        let entries = group.entries().expect("enumerate exact child entries");

        let u_child = group
            .open_child_entry_exact(&entries[0])
            .expect("first exact legacy child opens");
        let o_child = group
            .open_child_entry_exact(&entries[1])
            .expect("second exact legacy child opens");
        assert_eq!(u_child.read_file("marker.txt").unwrap(), b"u-child");
        assert_eq!(o_child.read_file("marker.txt").unwrap(), b"o-child");
        assert_eq!(
            u_child.root(),
            root_path.join(path_component_from_name_bytes(b"Gr\xfcn.c4d"))
        );
        assert_eq!(
            o_child.root(),
            root_path.join(path_component_from_name_bytes(b"Gr\xf6n.C4D"))
        );

        let error = group
            .open_child_entry_exact(&entries[2])
            .expect_err("a concrete plain entry must not open as a child");
        assert!(matches!(
            error,
            GroupError::InvalidGroup(message) if message.contains("not a child group")
        ));
    }

    #[test]
    fn packed_duplicate_entry_replaces_earlier_case_insensitive_name() {
        // C4Group::AddEntry marks an existing same-name entry deleted before
        // appending the replacement (src/C4Group.cpp:849-891). GetEntry uses
        // case-insensitive WildcardMatch (src/C4Group.cpp:896-904;
        // src/StdFile.cpp:337-367), so the later casing and payload win.
        let image = packed_group_image_with_entries(&[
            ("Same.txt", false, b"first"),
            ("same.TXT", false, b"second"),
        ]);
        let group =
            Group::from_memory(PathBuf::from("test.c4group"), image).expect("valid packed group");

        let entries = group.entries().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].relative_path, Path::new("same.TXT"));
        assert_eq!(group.read_file("SAME.txt").unwrap(), b"second");
    }

    #[test]
    fn packed_group_sanitizes_entry_filenames_like_cpp() {
        // OpenRealGrpFile validates every directory entry with VAL_Filename
        // (src/C4Group.cpp:784-793): separators become '_', ".." becomes
        // "__", and '?' is prohibited (src/C4InputValidation.cpp:39-95).
        let image = packed_group_image_with_entry("../bad?.txt", false, b"safe");
        let group =
            Group::from_memory(PathBuf::from("test.c4group"), image).expect("valid packed group");

        assert_eq!(
            group.entries().unwrap()[0].relative_path,
            Path::new("___bad_.txt")
        );
        assert_eq!(group.read_file("___bad_.txt").unwrap(), b"safe");
    }

    #[test]
    fn packed_child_preserves_raw_entry_names_and_hashes_them_into_contents_crc() {
        let child_payload = b"nested";
        let child_image = packed_group_image_with_entry("a*b.txt", false, child_payload);
        let outer = packed_group_image_with_entries(&[
            ("a*b.txt", false, b"root"),
            ("Child.c4g", true, &child_image),
        ]);
        let root = Group::from_memory(PathBuf::from("Outer.c4g"), outer).expect("root group opens");

        let root_entries = root.entries().unwrap();
        assert_eq!(root_entries[0].name_bytes, b"a_b.txt");
        assert!(
            root.exists("a_b.txt"),
            "OpenRealGrpFile validates root names"
        );

        let child = root.open_child("Child.c4g").expect("nested child opens");
        assert_eq!(child.entries().unwrap()[0].name_bytes, b"a*b.txt");
        assert_eq!(child.read_file("a*b.txt").unwrap(), child_payload);
        assert!(!child.exists("a_b.txt"));

        let expected_crc = crc32(crc32(0, child_payload), b"a*b.txt");
        assert_eq!(expected_crc, 0x5a38_ebb8, "C++/zlib EntryCRC32 oracle");
        assert_eq!(child.contents_crc().unwrap(), expected_crc);
        let root_file_crc = crc32(crc32(0, b"root"), b"a_b.txt");
        assert_eq!(root.contents_crc().unwrap(), root_file_crc ^ expected_crc);
    }

    #[test]
    fn memory_backed_packed_child_shares_its_parent_image() {
        let child_image = packed_group_image_with_entry("marker.txt", false, b"nested");
        let outer = packed_group_image_with_entry("Child.c4g", true, &child_image);
        let root = Group::from_memory(PathBuf::from("Outer.c4g"), outer).unwrap();
        let child = root.open_child("Child.c4g").unwrap();
        assert_eq!(child.read_file("marker.txt").unwrap(), b"nested");

        let GroupKind::Packed(root) = &root.kind else {
            panic!("memory image opens as packed group");
        };
        let GroupKind::Packed(child) = &child.kind else {
            panic!("child image opens as packed group");
        };
        let PackedSource::Memory {
            data: root_image, ..
        } = &root.source
        else {
            panic!("root retains its memory image");
        };
        let PackedSource::Memory {
            data: child_image, ..
        } = &child.source
        else {
            panic!("child retains a memory image");
        };

        assert!(Arc::ptr_eq(root_image, child_image));
    }

    #[test]
    fn small_memory_backed_child_detaches_from_an_oversized_parent_image() {
        let child_image = packed_group_image_with_entry("marker.txt", false, b"nested");
        let padding = vec![0_u8; 8 * 1024 * 1024 + 1];
        let outer = packed_group_image_with_entries(&[
            ("Child.c4g", true, &child_image),
            ("Padding.bin", false, &padding),
        ]);
        drop(padding);
        let root = Group::from_memory(PathBuf::from("Outer.c4g"), outer).unwrap();
        let child = root.open_child("Child.c4g").unwrap();
        assert_eq!(child.read_file("marker.txt").unwrap(), b"nested");

        let GroupKind::Packed(root) = &root.kind else {
            panic!("memory image opens as packed group");
        };
        let GroupKind::Packed(child) = &child.kind else {
            panic!("child image opens as packed group");
        };
        let PackedSource::Memory {
            data: root_image, ..
        } = &root.source
        else {
            panic!("root retains its memory image");
        };
        let PackedSource::Memory {
            data: child_image,
            range: child_range,
        } = &child.source
        else {
            panic!("child retains a memory image");
        };

        assert!(
            !Arc::ptr_eq(root_image, child_image),
            "a small escaping child must not pin an arbitrarily large parent archive"
        );
        assert_eq!(
            child_image.len(),
            child_range.len(),
            "the detached backing contains only the child image"
        );
    }

    #[test]
    fn memory_backed_packed_entry_exposes_borrowed_bytes() {
        let image = packed_group_image_with_entry("marker.txt", false, b"nested");
        let group = Group::from_memory(PathBuf::from("Memory.c4g"), image).unwrap();

        assert!(matches!(
            group.read_file_cow("marker.txt").unwrap(),
            std::borrow::Cow::Borrowed(b"nested")
        ));
    }

    #[test]
    fn directory_packed_child_uses_child_entry_name_policy() {
        let dir = tempdir().unwrap();
        let child_image = packed_group_image_with_entry("a*b.txt", false, b"nested");
        let mut compressed = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
            encoder.write_all(&child_image).unwrap();
            encoder.finish().unwrap();
        }
        compressed[..2].copy_from_slice(&C4GROUP_GZ_MAGIC);
        fs::write(dir.path().join("Child.c4g"), compressed).unwrap();

        let root = Group::open(dir.path()).expect("directory mother opens");
        let child = root.open_child("Child.c4g").expect("packed child opens");
        assert_eq!(child.entries().unwrap()[0].name_bytes, b"a*b.txt");
        assert_eq!(child.read_file("a*b.txt").unwrap(), b"nested");
    }

    #[test]
    fn packed_group_renames_empty_entry_like_cpp() {
        // VAL_Filename rewrites an empty string to "empty" and retains the
        // entry (src/C4InputValidation.cpp:39-56).
        let image = packed_group_image_with_entry("", false, b"data");
        let group = Group::from_memory(PathBuf::from("test.c4group"), image)
            .expect("empty source name is sanitized");

        assert_eq!(
            group.entries().unwrap()[0].relative_path,
            Path::new("empty")
        );
        assert_eq!(group.read_file("empty").unwrap(), b"data");
    }

    #[test]
    fn packed_group_preserves_entry_filename_whitespace() {
        // OpenRealGrpFile copies the validated C4GroupEntryCore::FileName
        // unchanged into AddEntry (src/C4Group.cpp:784-793). VAL_Filename
        // does not trim spaces; trimming is exclusive to VAL_Name* modes
        // (src/C4InputValidation.cpp:59-95, 97-127).
        let image = packed_group_image_with_entry(" report .txt ", false, b"data");
        let group =
            Group::from_memory(PathBuf::from("test.c4group"), image).expect("valid packed group");

        assert_eq!(
            group.entries().unwrap()[0].relative_path,
            Path::new(" report .txt ")
        );
        assert_eq!(group.read_file(" report .txt ").unwrap(), b"data");
        assert!(!group.exists("report .txt"));
    }

    #[test]
    fn packed_open_child_walks_successive_packed_components() {
        let leaf = packed_group_image_with_entry("marker.txt", false, b"nested");
        let intermediate = packed_group_image_with_entry("B.c4d", true, &leaf);
        let outer = packed_group_image_with_entry("A.c4d", true, &intermediate);
        let group =
            Group::from_memory(PathBuf::from("outer.c4g"), outer).expect("valid outer group");

        let child = group
            .open_child("a.C4D/B.C4d")
            .expect("multi-component packed child path");
        assert_eq!(child.read_file("marker.txt").unwrap(), b"nested");
    }

    #[test]
    fn directory_open_child_walks_through_a_packed_file() {
        let dir = tempdir().unwrap();
        let leaf = packed_group_image_with_entry("marker.txt", false, b"nested");
        let packed = packed_group_image_with_entry("B.c4d", true, &leaf);
        fs::write(dir.path().join("A.c4d"), packed).unwrap();
        let group = Group::open(dir.path()).unwrap();

        let child = group
            .open_child("a.C4D/B.C4d")
            .expect("directory-to-packed child path");
        assert_eq!(child.read_file("marker.txt").unwrap(), b"nested");
    }

    #[test]
    fn open_child_question_mark_selects_first_child_like_cpp() {
        let first = packed_group_image_with_entry("marker.txt", false, b"packed-first");
        let second = packed_group_image_with_entry("marker.txt", false, b"packed-second");
        let short = packed_group_image_with_entry("marker.txt", false, b"short");
        let long = packed_group_image_with_entry("marker.txt", false, b"long");
        let packed_root = PathBuf::from("question-mother.c4g");
        let packed = Group::from_memory(
            packed_root.clone(),
            packed_group_image_with_entries(&[
                ("Choice.c4g", true, &short),
                ("ChoiceAA.c4g", true, &long),
                ("ChoiceB.c4g", true, &first),
                ("ChoiceA.c4g", true, &second),
            ]),
        )
        .expect("valid packed mother");

        let selected = packed
            .open_child("cHOICE?.C4G")
            .expect("question wildcard opens first stored match");
        assert_eq!(selected.read_file("marker.txt").unwrap(), b"packed-first");
        assert_eq!(selected.root(), packed_root.join("ChoiceB.c4g"));

        // Nested entry tables bypass root filename validation. The original
        // request contains no `*`, so C++ does not reject a selected actual
        // name that contains one before its internal exact open.
        let wildcard_named = packed_group_image_with_entry("Wild*.c4d", true, &first);
        let nested_root = PathBuf::from("question-nested-name.c4f");
        let nested = Group::from_memory(
            nested_root.clone(),
            packed_group_image_with_entry("Nested.c4f", true, &wildcard_named),
        )
        .expect("valid nested packed mother")
        .open_child("Nested.c4f")
        .expect("nested mother opens");
        let selected = nested
            .open_child("Wild?.c4d")
            .expect("matched actual asterisk is not re-rejected");
        assert_eq!(selected.read_file("marker.txt").unwrap(), b"packed-first");
        assert_eq!(
            selected.root(),
            nested_root.join("Nested.c4f").join("Wild*.c4d")
        );

        let directory_root = tempdir().unwrap();
        fs::write(directory_root.path().join("Choice.c4g"), &short).unwrap();
        fs::write(directory_root.path().join("ChoiceAA.c4g"), &long).unwrap();
        fs::write(directory_root.path().join("ChoiceA.c4g"), &first).unwrap();
        fs::write(directory_root.path().join("ChoiceB.c4g"), &second).unwrap();
        let expected = fs::read_dir(directory_root.path())
            .unwrap()
            .map(|entry| entry.unwrap())
            .find(|entry| {
                let name = entry.file_name();
                name.as_encoded_bytes() == b"ChoiceA.c4g"
                    || name.as_encoded_bytes() == b"ChoiceB.c4g"
            })
            .expect("one matching physical child");
        let expected_name = expected.file_name();
        let expected_marker = if expected_name.as_encoded_bytes() == b"ChoiceA.c4g" {
            &b"packed-first"[..]
        } else {
            &b"packed-second"[..]
        };
        let directory = Group::open(directory_root.path()).expect("open directory mother");

        let selected = directory
            .open_child("cHOICE?.C4G")
            .expect("question wildcard opens first native directory match");
        assert_eq!(selected.read_file("marker.txt").unwrap(), expected_marker);
        assert_eq!(selected.root(), directory_root.path().join(expected_name));
    }

    #[test]
    fn open_child_question_mark_does_not_skip_first_plain_match() {
        let child = packed_group_image_with_entry("marker.txt", false, b"child");
        let packed = Group::from_memory(
            PathBuf::from("question-plain-first.c4g"),
            packed_group_image_with_entries(&[
                ("ChoiceA.c4g", false, b"plain"),
                ("ChoiceB.c4g", true, &child),
            ]),
        )
        .expect("valid packed mother");

        let error = packed
            .open_child("Choice?.c4g")
            .expect_err("the first name match is terminal");
        assert!(matches!(
            error,
            GroupError::InvalidGroup(message)
                if message.contains("ChoiceA.c4g") && message.contains("not a child group")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn open_child_question_mark_matches_one_native_byte_not_one_unicode_scalar() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let utf8 = packed_group_image_with_entry("marker.txt", false, b"utf8");
        let legacy = packed_group_image_with_entry("marker.txt", false, b"legacy");
        let root = PathBuf::from("question-native-byte.c4g");
        let packed = Group::from_memory(
            root.clone(),
            packed_group_image_with_byte_entries(&[
                (b"Gr\xc3\xbcn.c4g", true, &utf8),
                (b"Gr\xfcn.c4g", true, &legacy),
            ]),
        )
        .expect("valid packed mother");

        let selected = packed
            .open_child(Path::new(OsStr::from_bytes(b"Gr?n.c4g")))
            .expect("one native byte matches");
        assert_eq!(selected.read_file("marker.txt").unwrap(), b"legacy");
        assert_eq!(
            selected.root(),
            root.join(path_component_from_name_bytes(b"Gr\xfcn.c4g"))
        );
    }

    #[test]
    fn open_child_rejects_asterisk_before_lookup() {
        let dir = tempdir().unwrap();
        let directory = Group::open(dir.path()).unwrap();
        let packed = Group::from_memory(PathBuf::from("empty.c4g"), packed_group_image())
            .expect("valid empty packed group");

        for group in [&directory, &packed] {
            let error = group
                .open_child("missing/child*.c4d")
                .expect_err("C4Group forbids wildcard child names");
            assert!(matches!(
                error,
                GroupError::InvalidGroup(message)
                    if message == "OpenAsChild: No wildcards allowed"
            ));
        }
    }

    #[test]
    fn exact_child_open_rejects_asterisk_in_enumerated_name() {
        let child = packed_group_image();
        let nested = packed_group_image_with_entry("Wild*.c4d", true, &child);
        let outer = packed_group_image_with_entry("Nested.c4f", true, &nested);
        let root = Group::from_memory(PathBuf::from("outer.c4f"), outer)
            .expect("valid packed outer group");
        let nested = root.open_child("Nested.c4f").expect("nested mother opens");
        let entry = nested
            .entries()
            .expect("enumerate nested mother")
            .into_iter()
            .next()
            .expect("wildcard child entry");
        assert_eq!(entry.name_bytes, b"Wild*.c4d");

        let error = nested
            .open_child_entry_exact(&entry)
            .expect_err("OpenAsChild rejects an asterisk even after enumeration");
        assert!(matches!(
            error,
            GroupError::InvalidGroup(message) if message == "OpenAsChild: No wildcards allowed"
        ));
    }

    #[test]
    fn packed_open_child_rejects_entry_without_child_group_flag() {
        // C4Group::OpenAsChild resolves the entry, then rejects it when the
        // directory's ChildGroup flag is clear, before parsing its payload
        // (src/C4Group.cpp:1846-1862).
        let inner = packed_group_image();
        let outer = packed_group_image_with_entry("payload.bin", false, &inner);
        let group =
            Group::from_memory(PathBuf::from("outer.c4group"), outer).expect("valid outer group");

        let error = group
            .open_child("payload.bin")
            .expect_err("plain file must not open as a child group");
        assert!(matches!(
            error,
            GroupError::InvalidGroup(message) if message.contains("not a child group")
        ));
    }

    #[test]
    fn child_marked_file_loads_physical_payload_like_cpp() {
        // C4Group::LoadEntry and LoadEntryString read an entry's declared
        // bytes without consulting ChildGroup. OpenAsChild remains the API
        // that validates the flag and interprets those bytes as a group
        // (src/C4Group.cpp:1917-1937,2214-2270).
        let payload = b"child-marked\0physical payload";
        let outer = packed_group_image_with_entry("payload.bin", true, payload);
        let group =
            Group::from_memory(PathBuf::from("outer.c4group"), outer).expect("valid outer group");
        let entry = group
            .entries()
            .expect("enumerate outer group")
            .into_iter()
            .next()
            .expect("child-marked entry");

        assert!(entry.is_directory);
        assert_eq!(group.read_file("PAYLOAD.BIN").unwrap(), payload);
        assert_eq!(group.load_entry_string("payload.bin").unwrap(), payload);
        assert_eq!(group.read_entry_bytes("payload.bin").unwrap(), payload);
        assert_eq!(group.read_entry_bytes_exact(&entry).unwrap(), payload);

        for error in [
            group
                .open_child("payload.bin")
                .expect_err("malformed child payload must still fail path-based group parsing"),
            group
                .open_child_entry_exact(&entry)
                .expect_err("malformed child payload must still fail exact group parsing"),
        ] {
            assert!(matches!(
                error,
                GroupError::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof
            ));
        }
    }

    #[test]
    fn packed_open_child_rejects_gzip_wrapped_child_payload() {
        let compressed_child = gzip_group_image(&packed_group_image());
        let outer = packed_group_image_with_entry("child.c4g", true, &compressed_child);
        let group =
            Group::from_memory(PathBuf::from("outer.c4g"), outer).expect("valid outer group");

        group
            .open_child("child.c4g")
            .expect_err("OpenAsChild reads an envelope-free raw image in place");
        Group::from_raw_memory(PathBuf::from("child.c4g"), compressed_child)
            .expect_err("raw nested parser must reject gzip magic");
    }

    #[test]
    fn directory_open_child_accepts_raw_physical_group_image() {
        let directory = tempdir().unwrap();
        fs::write(directory.path().join("child.c4g"), packed_group_image()).unwrap();
        let group = Group::open(directory.path()).expect("open unpacked mother");

        let child = group
            .open_child("child.c4g")
            .expect("unpacked mothers expose raw physical child images");
        assert_eq!(child.read_file("hello.txt").unwrap(), b"world");
    }
}
