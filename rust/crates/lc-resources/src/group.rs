use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use byteorder::{LittleEndian, ReadBytesExt};
use thiserror::Error;
use walkdir::{Error as WalkDirError, WalkDir};

const GROUP_HEADER_SIZE: usize = 204;
const GROUP_ENTRY_SIZE: usize = 316;
const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";

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
    #[error(transparent)]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone)]
pub struct Group {
    kind: GroupKind,
}

#[derive(Debug, Clone)]
enum GroupKind {
    Directory(PathBuf),
    Packed(PackedGroup),
}

#[derive(Debug, Clone)]
enum PackedSource {
    File(PathBuf),
    Memory(Arc<Vec<u8>>),
}

#[derive(Debug, Clone)]
struct PackedGroup {
    path: PathBuf,
    source: PackedSource,
    header: PackedHeader,
    entries: Vec<PackedEntry>,
    index: HashMap<PathBuf, usize>,
    data_offset: u64,
}

#[derive(Debug, Clone)]
struct PackedHeader {
    maker: String,
}

#[derive(Debug, Clone)]
struct PackedEntry {
    relative_path: PathBuf,
    is_directory: bool,
    size: u64,
    offset: u64,
}

impl Group {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, GroupError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(GroupError::Missing(path.to_path_buf()));
        }
        if path.is_dir() {
            return Ok(Self {
                kind: GroupKind::Directory(path.to_path_buf()),
            });
        }

        match PackedGroup::open(path) {
            Ok(packed) => Ok(Self {
                kind: GroupKind::Packed(packed),
            }),
            Err(err) => Err(err),
        }
    }

    pub fn root(&self) -> &Path {
        match &self.kind {
            GroupKind::Directory(path) => path,
            GroupKind::Packed(packed) => &packed.path,
        }
    }

    pub fn entries(&self) -> Result<Vec<GroupEntry>, GroupError> {
        match &self.kind {
            GroupKind::Directory(root) => directory_entries(root),
            GroupKind::Packed(packed) => Ok(packed
                .entries
                .iter()
                .map(|entry| GroupEntry {
                    relative_path: entry.relative_path.clone(),
                    is_directory: entry.is_directory,
                    size: entry.size,
                })
                .collect()),
        }
    }

    pub fn read_file<P: AsRef<Path>>(&self, relative: P) -> Result<Vec<u8>, GroupError> {
        match &self.kind {
            GroupKind::Directory(root) => {
                let full_path = root.join(relative.as_ref());
                Ok(fs::read(full_path)?)
            }
            GroupKind::Packed(packed) => {
                let relative = normalize_path(relative.as_ref());
                packed.read_file(&relative)
            }
        }
    }

    pub fn exists<P: AsRef<Path>>(&self, relative: P) -> bool {
        match &self.kind {
            GroupKind::Directory(root) => root.join(relative.as_ref()).exists(),
            GroupKind::Packed(packed) => {
                let relative = normalize_path(relative.as_ref());
                packed.index.contains_key(&relative)
            }
        }
    }

    pub fn maker(&self) -> Option<&str> {
        match &self.kind {
            GroupKind::Packed(packed) => Some(packed.header.maker.as_str()),
            _ => None,
        }
    }

    pub fn is_directory(&self) -> bool {
        matches!(self.kind, GroupKind::Directory(_))
    }

    pub fn open_child<P: AsRef<Path>>(&self, relative: P) -> Result<Self, GroupError> {
        let relative = normalize_path(relative.as_ref());
        match &self.kind {
            GroupKind::Directory(root) => Self::open(root.join(&relative)),
            GroupKind::Packed(packed) => packed.open_child(&relative),
        }
    }

    /// Opens a packed group from in-memory bytes (gz-wrapped or raw) —
    /// e.g. the PlrData blob a CID_JoinPlr control packet carries
    /// (C4ControlJoinPlayer, C4Control.cpp:731-744 writes the .c4p file
    /// contents into the packet). `path` only labels error messages.
    pub fn from_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        Self::from_packed_bytes(path, data)
    }

    fn from_packed_bytes(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        let packed = PackedGroup::from_memory(path, data)?;
        Ok(Self {
            kind: GroupKind::Packed(packed),
        })
    }
}

impl PackedGroup {
    fn open(path: &Path) -> Result<Self, GroupError> {
        Self::from_source(path.to_path_buf(), PackedSource::File(path.to_path_buf()))
    }

    fn from_memory(path: PathBuf, data: Vec<u8>) -> Result<Self, GroupError> {
        Self::from_source(path, PackedSource::Memory(Arc::new(data)))
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
                    let data_arc = Arc::new(data);
                    let data_clone = Arc::clone(&data_arc);
                    let mut cursor = Cursor::new(data_clone.as_slice());
                    return Self::parse_from_reader(
                        path,
                        PackedSource::Memory(data_arc),
                        &mut cursor,
                    );
                }
                Self::parse_from_reader(path, PackedSource::File(file_path), &mut file)
            }
            PackedSource::Memory(data) => {
                let data = if data.len() >= 2
                    && (data[..2] == C4GROUP_GZ_MAGIC || data[..2] == GZ_MAGIC)
                {
                    Arc::new(decompress_group(data.as_ref().clone())?)
                } else {
                    data
                };
                let data_clone = Arc::clone(&data);
                let mut cursor = Cursor::new(data_clone.as_slice());
                Self::parse_from_reader(path, PackedSource::Memory(data), &mut cursor)
            }
        }
    }

    fn parse_from_reader<R: Read + Seek>(
        path: PathBuf,
        source: PackedSource,
        reader: &mut R,
    ) -> Result<Self, GroupError> {
        let mut header_bytes = [0u8; GROUP_HEADER_SIZE];
        reader.read_exact(&mut header_bytes)?;
        mem_unscramble(&mut header_bytes);
        let header = parse_header(&header_bytes)?;

        let mut entries = Vec::with_capacity(header.entry_count);
        for _ in 0..header.entry_count {
            let mut entry_bytes = [0u8; GROUP_ENTRY_SIZE];
            reader.read_exact(&mut entry_bytes)?;
            let entry = parse_entry(&entry_bytes)?;
            entries.push(entry);
        }

        let data_offset = reader.stream_position()?;
        let index = entries
            .iter()
            .enumerate()
            .map(|(idx, entry)| (entry.relative_path.clone(), idx))
            .collect();

        Ok(Self {
            path,
            source,
            header: header.header,
            entries,
            index,
            data_offset,
        })
    }

    fn read_file(&self, relative: &Path) -> Result<Vec<u8>, GroupError> {
        let entry_index = self
            .index
            .get(relative)
            .copied()
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        let entry = &self.entries[entry_index];
        if entry.is_directory {
            return Err(GroupError::InvalidGroup(format!(
                "entry '{}' is a child group",
                relative.display()
            )));
        }
        self.read_entry_bytes(entry)
    }

    fn read_entry_bytes(&self, entry: &PackedEntry) -> Result<Vec<u8>, GroupError> {
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
                Ok(buffer)
            }
            PackedSource::Memory(data) => {
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
                let data_len = data.len() as u64;
                if end > data_len {
                    return Err(GroupError::InvalidGroup(format!(
                        "entry '{}' exceeds group bounds",
                        entry.relative_path.display()
                    )));
                }
                let start = start as usize;
                let end = end as usize;
                Ok(data[start..end].to_vec())
            }
        }
    }

    fn open_child(&self, relative: &Path) -> Result<Group, GroupError> {
        let entry_index = self
            .index
            .get(relative)
            .copied()
            .ok_or_else(|| GroupError::EntryNotFound(relative.to_path_buf()))?;
        let entry = &self.entries[entry_index];
        let data = self.read_entry_bytes(entry)?;
        Group::from_packed_bytes(self.path.join(relative), data)
    }
}

fn directory_entries(root: &Path) -> Result<Vec<GroupEntry>, GroupError> {
    let mut entries = Vec::new();
    for entry in WalkDir::new(root).min_depth(1).max_depth(1) {
        let entry = entry.map_err(convert_walkdir_error)?;
        if entry.path() == root {
            continue;
        }
        let metadata = entry.metadata().map_err(convert_walkdir_error)?;
        let rel = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| GroupError::Io(io::Error::other("failed to strip prefix")))?;
        let rel = normalize_path(rel);
        entries.push(GroupEntry {
            relative_path: rel,
            is_directory: metadata.is_dir(),
            size: metadata.len(),
        });
    }
    Ok(entries)
}

fn parse_header(bytes: &[u8]) -> Result<ParsedHeader, GroupError> {
    let mut cursor = Cursor::new(bytes);
    let mut id = [0u8; 28];
    cursor.read_exact(&mut id)?;
    if !id.starts_with(GROUP_FILE_ID) {
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
    if entries < 0 {
        return Err(GroupError::InvalidGroup("negative entry count".into()));
    }
    let mut maker_bytes = [0u8; 32];
    cursor.read_exact(&mut maker_bytes)?;
    let maker = c_string(&maker_bytes);

    // Skip password and reserved fields
    let mut skip = [0u8; 32 + 4 + 4 + 92];
    cursor.read_exact(&mut skip)?;

    Ok(ParsedHeader {
        header: PackedHeader { maker },
        entry_count: entries as usize,
    })
}

fn parse_entry(bytes: &[u8]) -> Result<PackedEntry, GroupError> {
    let mut cursor = Cursor::new(bytes);
    let mut name_bytes = [0u8; 260];
    cursor.read_exact(&mut name_bytes)?;
    let name = c_string(&name_bytes);
    if name.is_empty() {
        return Err(GroupError::InvalidGroup("empty entry filename".into()));
    }
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
    let offset = cursor.read_i32::<LittleEndian>()?;
    if offset < 0 {
        return Err(GroupError::InvalidGroup(format!(
            "negative entry offset for {}",
            name
        )));
    }
    let _time = cursor.read_u32::<LittleEndian>()?;
    let _has_crc = cursor.read_u8()? != 0;
    let _crc = cursor.read_u32::<LittleEndian>()?;
    let _executable = cursor.read_u8()? != 0;
    let mut skip = [0u8; 26];
    cursor.read_exact(&mut skip)?;

    Ok(PackedEntry {
        relative_path: normalize_path(Path::new(&name)),
        is_directory: child,
        size: size as u64,
        offset: offset as u64,
    })
}

fn c_string(buf: &[u8]) -> String {
    let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..nul]).trim().to_string()
}

/// On-disk C4Group files are gzip streams whose two magic bytes are
/// replaced with `{0x1E, 0x8C}` so stock tools leave them alone
/// (StdGzCompressedFile.h:34, StdGzCompressedFile.cpp:62-95). Restore the
/// real magic and inflate; `MultiGzDecoder` accepts the concatenated
/// members C4Group appends on update.
const C4GROUP_GZ_MAGIC: [u8; 2] = [0x1E, 0x8C];
const GZ_MAGIC: [u8; 2] = [0x1F, 0x8B];

fn decompress_group(mut compressed: Vec<u8>) -> Result<Vec<u8>, GroupError> {
    compressed[0] = GZ_MAGIC[0];
    compressed[1] = GZ_MAGIC[1];
    let mut decoder = flate2::read::MultiGzDecoder::new(Cursor::new(compressed));
    let mut data = Vec::new();
    decoder
        .read_to_end(&mut data)
        .map_err(|error| GroupError::InvalidGroup(format!("gzip decompression failed: {error}")))?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupEntry {
    pub relative_path: PathBuf,
    pub is_directory: bool,
    pub size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::WriteBytesExt;
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use tempfile::tempdir;

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
    fn missing_group_errors() {
        assert!(matches!(
            Group::open("/path/does/not/exist"),
            Err(GroupError::Missing(_))
        ));
    }

    /// The raw (uncompressed) packed-group image used by the packed tests:
    /// scrambled header + one entry ("hello.txt" -> b"world").
    fn packed_group_image() -> Vec<u8> {
        let mut image = Vec::new();

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
        image.extend_from_slice(&header);

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
        image.extend_from_slice(&entry);
        image.extend_from_slice(b"world");
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
    fn open_packed_group() {
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

        let group = Group::open(&path).unwrap();
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
}
