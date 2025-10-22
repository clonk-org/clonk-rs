use crate::{Group, GroupError};
use image::{load_from_memory, ImageError};
use serde::Deserialize;
use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioDiscoveryError {
    #[error("failed to read directory {path}: {source}")]
    ReadDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to inspect entry in {path}: {source}")]
    ReadEntry {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("resource error at {path}: {source}")]
    Group {
        path: PathBuf,
        #[source]
        source: GroupError,
    },
    #[error("failed to parse scenario manifest at {path}: {source}")]
    Manifest {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to decode preview image at {path}: {source}")]
    Preview {
        path: PathBuf,
        #[source]
        source: ImageError,
    },
    #[error("path is not valid UTF-8: {path}")]
    NonUtf8Path { path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioEntryKind {
    Scenario,
    Folder,
    Editor,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioPreview {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl ScenarioPreview {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels: Arc::from(pixels.into_boxed_slice()),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn into_arc(self) -> (u32, u32, Arc<[u8]>) {
        (self.width, self.height, self.pixels)
    }

    pub fn clone_data(&self) -> Arc<[u8]> {
        Arc::clone(&self.pixels)
    }
}

#[derive(Debug, Clone)]
pub struct ScenarioEntry {
    pub identifier: String,
    pub path: PathBuf,
    pub title: String,
    pub description: Option<String>,
    pub kind: ScenarioEntryKind,
    pub is_editable: bool,
    pub is_playable: bool,
    pub preview: Option<ScenarioPreview>,
    pub children: Vec<ScenarioEntry>,
}

pub fn discover(root: impl AsRef<Path>) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    discover_many([root.as_ref()])
}

pub fn discover_many<I, P>(roots: I) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut entries = Vec::new();
    for root in roots {
        let root_path = root.as_ref();
        let mut discovered = collect_from_path(root_path, "")?;
        entries.append(&mut discovered);
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn collect_from_directory(
    path: &Path,
    parent_identifier: &str,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    let mut entries: Vec<fs::DirEntry> = fs::read_dir(path)
        .map_err(|source| ScenarioDiscoveryError::ReadDirectory {
            path: path.to_path_buf(),
            source,
        })?
        .collect::<Result<_, _>>()
        .map_err(|source| ScenarioDiscoveryError::ReadEntry {
            path: path.to_path_buf(),
            source,
        })?;

    entries.sort_by(|a, b| {
        compare_case_insensitive(
            a.file_name().to_string_lossy().as_ref(),
            b.file_name().to_string_lossy().as_ref(),
        )
    });

    let mut result = Vec::new();
    for entry in entries {
        let file_type = entry
            .file_type()
            .map_err(|source| ScenarioDiscoveryError::ReadEntry {
                path: path.to_path_buf(),
                source,
            })?;

        let name_os = entry.file_name();
        let name = match name_os.to_str() {
            Some(name) => name,
            None => return Err(ScenarioDiscoveryError::NonUtf8Path { path: entry.path() }),
        };

        if should_ignore_name(name) {
            continue;
        }

        let identifier = join_identifier(parent_identifier, name);
        if file_type.is_dir() {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            match classify_group(&group)? {
                GroupContentKind::Scenario => {
                    result.push(build_scenario_entry(&group, identifier)?);
                }
                GroupContentKind::Folder => {
                    result.push(build_folder_entry(&group, identifier)?);
                }
                GroupContentKind::Other => {
                    continue;
                }
            }
        } else if is_scenario_filename(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_scenario_entry(&group, identifier)?);
        } else if is_folder_filename(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_folder_entry(&group, identifier)?);
        }
    }
    sort_entries(&mut result);
    Ok(result)
}

fn collect_from_path(
    path: &Path,
    parent_identifier: &str,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    if path.is_dir() {
        return collect_from_directory(path, parent_identifier);
    }
    if path.is_file() {
        if !is_scenario_filename_os(path) && !is_folder_filename_os(path) {
            return Ok(Vec::new());
        }
        return collect_from_group_file(path, parent_identifier);
    }
    Ok(Vec::new())
}

fn collect_children_from_group(
    group: &Group,
    parent_identifier: &str,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    let mut entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?
        .into_iter()
        .filter(|entry| entry.relative_path.components().count() == 1)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        compare_case_insensitive(
            os_str_from_path(&a.relative_path)
                .to_string_lossy()
                .as_ref(),
            os_str_from_path(&b.relative_path)
                .to_string_lossy()
                .as_ref(),
        )
    });

    let mut result = Vec::new();
    for entry in entries {
        let name_os = os_str_from_path(&entry.relative_path);
        let name = match name_os.to_str() {
            Some(name) => name,
            None => {
                return Err(ScenarioDiscoveryError::NonUtf8Path {
                    path: group.root().join(&entry.relative_path),
                })
            }
        };
        if should_ignore_name(name) {
            continue;
        }
        let identifier = join_identifier(parent_identifier, name);
        if entry.is_directory {
            let child_group = group
                .open_child(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            match classify_group(&child_group)? {
                GroupContentKind::Scenario => {
                    result.push(build_scenario_entry(&child_group, identifier)?);
                }
                GroupContentKind::Folder => {
                    result.push(build_folder_entry(&child_group, identifier)?);
                }
                GroupContentKind::Other => continue,
            }
        } else if is_scenario_filename(name) {
            let child_group = group
                .open_child(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_scenario_entry(&child_group, identifier)?);
        } else if is_folder_filename(name) {
            let child_group = group
                .open_child(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_folder_entry(&child_group, identifier)?);
        }
    }
    sort_entries(&mut result);
    Ok(result)
}

fn collect_from_group_file(
    path: &Path,
    parent_identifier: &str,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    let name_os = match path.file_name() {
        Some(name) => name,
        None => return Ok(Vec::new()),
    };
    let name = match name_os.to_str() {
        Some(name) => name,
        None => {
            return Err(ScenarioDiscoveryError::NonUtf8Path {
                path: path.to_path_buf(),
            })
        }
    };
    let group = Group::open(path).map_err(|source| ScenarioDiscoveryError::Group {
        path: path.to_path_buf(),
        source,
    })?;
    let identifier = join_identifier(parent_identifier, name);
    let entry = match classify_group(&group)? {
        GroupContentKind::Scenario => build_scenario_entry(&group, identifier)?,
        GroupContentKind::Folder => build_folder_entry(&group, identifier)?,
        GroupContentKind::Other => return Ok(Vec::new()),
    };
    Ok(vec![entry])
}

fn build_scenario_entry(
    group: &Group,
    identifier: String,
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_path(group.root());
    let manifest = scenario_manifest_info(group)?;
    let mut title = manifest
        .as_ref()
        .and_then(|info| info.name.as_ref())
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string());
    let description = manifest.and_then(|info| info.description).and_then(|desc| {
        let trimmed = desc.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    });

    if title.is_none() {
        title = title_from_title_files(group)?;
    }

    let title = title.unwrap_or(fallback);
    let preview = load_preview_image(group)?;

    Ok(ScenarioEntry {
        identifier,
        path: group.root().to_path_buf(),
        title,
        description,
        kind: ScenarioEntryKind::Scenario,
        is_editable: group.is_directory(),
        is_playable: true,
        preview,
        children: Vec::new(),
    })
}

fn build_folder_entry(
    group: &Group,
    identifier: String,
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_path(group.root());
    let mut title = title_from_title_files(group)?;
    if title.is_none() {
        title = title_from_folder_core(group)?;
    }

    let title = title.unwrap_or(fallback);
    let preview = load_preview_image(group)?;
    let children = collect_children_from_group(group, &identifier)?;

    Ok(ScenarioEntry {
        identifier,
        path: group.root().to_path_buf(),
        title,
        description: None,
        kind: ScenarioEntryKind::Folder,
        is_editable: group.is_directory(),
        is_playable: false,
        preview,
        children,
    })
}

fn load_preview_image(group: &Group) -> Result<Option<ScenarioPreview>, ScenarioDiscoveryError> {
    let entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?;

    let mut best: Option<((u8, u8, String), PathBuf)> = None;
    for entry in entries.iter() {
        if entry.is_directory {
            continue;
        }
        if entry.relative_path.components().count() != 1 {
            continue;
        }
        let Some(name) = entry
            .relative_path
            .file_name()
            .and_then(|name| name.to_str())
        else {
            continue;
        };
        let Some(key) = preview_candidate_key(name) else {
            continue;
        };
        if best
            .as_ref()
            .map(|(current, _)| key < *current)
            .unwrap_or(true)
        {
            best = Some((key, entry.relative_path.clone()));
        }
    }

    let Some((_, relative_path)) = best else {
        return Ok(None);
    };

    let absolute_path = group.root().join(&relative_path);
    let bytes = group
        .read_file(&relative_path)
        .map_err(|err| group_error(&absolute_path, err))?;
    let image = load_from_memory(&bytes).map_err(|source| ScenarioDiscoveryError::Preview {
        path: absolute_path,
        source,
    })?;
    let rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    let data = rgba.into_raw();
    Ok(Some(ScenarioPreview::new(width, height, data)))
}

fn preview_candidate_key(name: &str) -> Option<(u8, u8, String)> {
    let lower = name.to_ascii_lowercase();
    let prefix_rank = if lower.starts_with("title") {
        0
    } else if lower.starts_with("loader") {
        1
    } else if lower.starts_with("icon") {
        2
    } else {
        return None;
    };

    let extension = Path::new(name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())?;
    let ext_rank = match extension.as_str() {
        "png" => 0,
        "jpg" | "jpeg" => 1,
        "bmp" => 2,
        _ => return None,
    };

    Some((prefix_rank, ext_rank, lower))
}

fn classify_group(group: &Group) -> Result<GroupContentKind, ScenarioDiscoveryError> {
    if group.exists("Scenario.json") || group.exists("Scenario.txt") {
        return Ok(GroupContentKind::Scenario);
    }
    let entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?;
    if entries.iter().any(|entry| {
        entry.relative_path.components().count() == 1
            && (entry.is_directory
                || is_scenario_filename_os(&entry.relative_path)
                || is_folder_filename_os(&entry.relative_path))
    }) {
        return Ok(GroupContentKind::Folder);
    }
    if group.exists("Folder.txt") {
        return Ok(GroupContentKind::Folder);
    }
    Ok(GroupContentKind::Other)
}

fn scenario_manifest_info(
    group: &Group,
) -> Result<Option<ScenarioManifestPreview>, ScenarioDiscoveryError> {
    if !group.exists("Scenario.json") {
        return Ok(None);
    }
    let bytes = group
        .read_file("Scenario.json")
        .map_err(|err| group_error(group.root(), err))?;
    let manifest =
        serde_json::from_slice(&bytes).map_err(|source| ScenarioDiscoveryError::Manifest {
            path: group.root().join("Scenario.json"),
            source,
        })?;
    Ok(Some(manifest))
}

fn title_from_title_files(group: &Group) -> Result<Option<String>, ScenarioDiscoveryError> {
    let entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?;
    for entry in entries {
        if entry.is_directory {
            continue;
        }
        if entry.relative_path.components().count() != 1 {
            continue;
        }
        let name = match entry
            .relative_path
            .file_name()
            .and_then(|name| name.to_str())
        {
            Some(name) => name,
            None => {
                return Err(ScenarioDiscoveryError::NonUtf8Path {
                    path: group.root().join(&entry.relative_path),
                })
            }
        };
        if is_title_filename(name) {
            let data = group
                .read_file(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            if let Ok(text) = std::str::from_utf8(&data) {
                if let Some(line) = text.lines().map(str::trim).find(|line| !line.is_empty()) {
                    return Ok(Some(line.to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn title_from_folder_core(group: &Group) -> Result<Option<String>, ScenarioDiscoveryError> {
    if !group.exists("Folder.txt") {
        return Ok(None);
    }
    let data = group
        .read_file("Folder.txt")
        .map_err(|err| group_error(group.root(), err))?;
    if let Ok(content) = std::str::from_utf8(&data) {
        for line in content.lines() {
            if let Some(value) = line.split('=').nth(1) {
                let trimmed = value.trim();
                if !trimmed.is_empty() {
                    return Ok(Some(trimmed.to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn fallback_title_for_path(path: &Path) -> String {
    if let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) {
        if !stem.is_empty() {
            return stem.replace('_', " ");
        }
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn join_identifier(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_string()
    } else {
        format!("{parent}/{child}")
    }
}

fn should_ignore_name(name: &str) -> bool {
    name.starts_with('.')
        || name.eq_ignore_ascii_case("CVS")
        || name.eq_ignore_ascii_case("Thumbs.db")
}

fn is_scenario_filename(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.eq_ignore_ascii_case("c4s"))
        .unwrap_or(false)
}

fn is_folder_filename(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| ext.eq_ignore_ascii_case("c4f"))
        .unwrap_or(false)
}

fn is_scenario_filename_os(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("c4s"))
        .unwrap_or(false)
}

fn is_folder_filename_os(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("c4f"))
        .unwrap_or(false)
}

fn is_title_filename(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "title.txt" || (lower.starts_with("title") && lower.ends_with(".txt"))
}

fn sort_entries(entries: &mut [ScenarioEntry]) {
    entries.sort_by(|a, b| compare_case_insensitive(&a.title, &b.title));
    for entry in entries.iter_mut() {
        sort_entries(&mut entry.children);
    }
}

fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
}

fn os_str_from_path(path: &Path) -> &std::ffi::OsStr {
    path.file_name().unwrap_or_else(|| path.as_os_str())
}

fn group_error(path: &Path, err: GroupError) -> ScenarioDiscoveryError {
    ScenarioDiscoveryError::Group {
        path: path.to_path_buf(),
        source: err,
    }
}

#[derive(Debug, Deserialize)]
struct ScenarioManifestPreview {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

enum GroupContentKind {
    Scenario,
    Folder,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use byteorder::{LittleEndian, WriteBytesExt};
    use std::io::Cursor;
    use std::io::Write;
    use tempfile::tempdir;

    const GROUP_HEADER_SIZE: usize = 204;
    const GROUP_ENTRY_SIZE: usize = 316;
    const GROUP_FILE_ID: &[u8] = b"RedWolf Design GrpFolder";

    #[test]
    fn discovers_directory_scenario() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.json"),
            br#"{"name":"Alpha Mission","description":"Test"}"#,
        )
        .unwrap();

        let entries = discover(dir.path()).expect("discover scenarios");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.kind, ScenarioEntryKind::Scenario);
        assert_eq!(entry.title, "Alpha Mission");
        assert_eq!(entry.identifier, "Alpha.c4s");
        assert_eq!(
            entry.description.as_deref(),
            Some("Test"),
            "description propagated"
        );
    }

    #[test]
    fn discovers_directory_folder_with_child() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Missions.c4f");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Folder.txt"), "Title=Missions Pack\n").unwrap();

        let child = folder.join("Bravo.c4s");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("Scenario.json"), br#"{"name":"Bravo"}"#).unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries.len(), 1);
        let folder_entry = &entries[0];
        assert_eq!(folder_entry.kind, ScenarioEntryKind::Folder);
        assert_eq!(folder_entry.title, "Missions Pack");
        assert_eq!(folder_entry.children.len(), 1);
        assert_eq!(folder_entry.children[0].title, "Bravo");
    }

    #[test]
    fn discovers_packed_scenario_file() {
        let dir = tempdir().unwrap();
        let scenario_path = dir.path().join("Packed.c4s");
        let scenario_bytes =
            build_group(&[("Scenario.json", br#"{"name":"Packed Scenario"}"#.to_vec())]);
        fs::write(&scenario_path, scenario_bytes).unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.title, "Packed Scenario");
        assert_eq!(entry.kind, ScenarioEntryKind::Scenario);
    }

    #[test]
    fn discovers_packed_folder_with_child_scenario() {
        let dir = tempdir().unwrap();
        let folder_path = dir.path().join("Campaign.c4f");

        let child_scenario =
            build_group(&[("Scenario.json", br#"{"name":"Packed Child"}"#.to_vec())]);
        let folder_bytes = build_group(&[
            ("Folder.txt", b"Title=Campaign".to_vec()),
            ("Child.c4s", child_scenario),
        ]);
        fs::write(&folder_path, folder_bytes).unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries.len(), 1);
        let folder_entry = &entries[0];
        assert_eq!(folder_entry.title, "Campaign");
        assert_eq!(folder_entry.kind, ScenarioEntryKind::Folder);
        assert_eq!(folder_entry.children.len(), 1);
        assert_eq!(folder_entry.children[0].title, "Packed Child");
    }

    #[test]
    fn discover_many_handles_directory_and_file_roots() {
        let dir = tempdir().unwrap();

        let dir_root = dir.path().join("Root");
        fs::create_dir(&dir_root).unwrap();
        let alpha = dir_root.join("Alpha.c4s");
        fs::create_dir(&alpha).unwrap();
        fs::write(alpha.join("Scenario.json"), br#"{"name":"Alpha"}"#).unwrap();

        let packed_path = dir.path().join("Packed.c4s");
        let packed_bytes = build_group(&[("Scenario.json", br#"{"name":"Packed"}"#.to_vec())]);
        fs::write(&packed_path, packed_bytes).unwrap();

        let entries =
            discover_many([dir_root.as_path(), packed_path.as_path()]).expect("discover many");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].title, "Alpha");
        assert_eq!(entries[1].title, "Packed");
    }

    fn build_group(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut buffer = Vec::new();
        let mut header = [0u8; GROUP_HEADER_SIZE];
        {
            let mut cursor = Cursor::new(&mut header[..]);
            let mut id_bytes = [0u8; 28];
            id_bytes[..GROUP_FILE_ID.len()].copy_from_slice(GROUP_FILE_ID);
            cursor.write_all(&id_bytes).unwrap();
            cursor.write_i32::<LittleEndian>(1).unwrap();
            cursor.write_i32::<LittleEndian>(2).unwrap();
            cursor
                .write_i32::<LittleEndian>(entries.len() as i32)
                .unwrap();
            cursor.write_all(&[0u8; 32]).unwrap(); // maker
            cursor.write_all(&[0u8; 32 + 4 + 4 + 92]).unwrap();
        }
        scramble(&mut header);
        buffer.extend_from_slice(&header);

        let mut data_offset = 0u32;
        let mut entry_records = Vec::new();
        for (name, data) in entries {
            let mut record = [0u8; GROUP_ENTRY_SIZE];
            {
                let mut cursor = Cursor::new(&mut record[..]);
                let mut name_buf = [0u8; 260];
                name_buf[..name.len()].copy_from_slice(name.as_bytes());
                cursor.write_all(&name_buf).unwrap();
                cursor.write_i32::<LittleEndian>(0).unwrap();
                cursor.write_i32::<LittleEndian>(0).unwrap();
                cursor.write_i32::<LittleEndian>(data.len() as i32).unwrap();
                cursor.write_i32::<LittleEndian>(0).unwrap();
                cursor
                    .write_i32::<LittleEndian>(data_offset as i32)
                    .unwrap();
                cursor.write_u32::<LittleEndian>(0).unwrap();
                cursor.write_u8(0).unwrap();
                cursor.write_u32::<LittleEndian>(0).unwrap();
                cursor.write_u8(0).unwrap();
                cursor.write_all(&[0u8; 26]).unwrap();
            }
            entry_records.push(record);
            data_offset += data.len() as u32;
        }

        for record in &entry_records {
            buffer.extend_from_slice(record);
        }
        for (_, data) in entries {
            buffer.extend_from_slice(data);
        }
        buffer
    }

    fn scramble(buffer: &mut [u8]) {
        for byte in buffer.iter_mut() {
            *byte ^= 237;
        }
        let mut i = 0;
        while i + 2 < buffer.len() {
            buffer.swap(i, i + 2);
            i += 3;
        }
    }
}
