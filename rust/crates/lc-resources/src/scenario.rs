use crate::{Group, GroupError};
use image::{load_from_memory, ImageError};
use serde::Deserialize;
use std::cmp::Ordering;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

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
    #[error("legacy scenario core at {path} is not valid UTF-8")]
    LegacyCoreEncoding { path: PathBuf },
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
    /// The right-page Title.png/Title.bmp picture (C4ScenarioListLoader::
    /// Entry fctTitle, C4StartupScenSelDlg.cpp:532-534); shares pixel data
    /// with `preview` when both come from the same title image.
    pub title_picture: Option<ScenarioPreview>,
    pub children: Vec<ScenarioEntry>,
    pub folder_index: Option<i32>,
    pub icon_index: Option<i32>,
    pub difficulty: Option<i32>,
    /// `Author.txt`/group maker of packed groups (Entry::Load,
    /// C4StartupScenSelDlg.cpp:536-552); unpacked directories have none.
    pub author: Option<String>,
    /// `Version.txt` contents (C4CFN_Version, C4StartupScenSelDlg.cpp:554).
    pub version: Option<String>,
    /// Scenario.txt `[Definitions] LocalOnly` (C4Scenario.cpp:482).
    pub local_only: Option<bool>,
    /// Scenario.txt `[Definitions] AllowUserChange` (C4Scenario.cpp:483).
    pub allow_user_change: Option<bool>,
}

#[derive(Debug, Clone, Default)]
struct LegacyCoreInfo {
    title: Option<String>,
    description: Option<String>,
    icon: Option<i32>,
    difficulty: Option<i32>,
    save_game: Option<bool>,
    replay: Option<bool>,
    local_only: Option<bool>,
    allow_user_change: Option<bool>,
}

#[derive(Debug, Clone, Default)]
struct LegacyFolderInfo {
    title: Option<String>,
    index: Option<i32>,
}

/// The default language fallback sequence, mirroring the LanguageEx list the
/// C++ frontend composes for the default English config: primary code plus
/// the internal "US"/"DE" fallbacks (C4StartupOptionsDlg.cpp:1211-1231,
/// C4ConfigGeneral::DefaultLanguage, C4Config.cpp:1461-1474).
pub const DEFAULT_LANGUAGE_SEQUENCE: [&str; 2] = ["US", "DE"];

fn default_language_sequence() -> Vec<String> {
    DEFAULT_LANGUAGE_SEQUENCE
        .iter()
        .map(|code| code.to_string())
        .collect()
}

pub fn discover(root: impl AsRef<Path>) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    discover_with_languages(root, &default_language_sequence())
}

pub fn discover_with_languages(
    root: impl AsRef<Path>,
    languages: &[String],
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    discover_many_with_languages([root.as_ref()], languages)
}

pub fn discover_many<I, P>(roots: I) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    discover_many_with_languages(roots, &default_language_sequence())
}

pub fn discover_many_with_languages<I, P>(
    roots: I,
    languages: &[String],
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut entries = Vec::new();
    for root in roots {
        let root_path = root.as_ref();
        let mut discovered = collect_from_path(root_path, "", languages)?;
        entries.append(&mut discovered);
    }
    sort_entries(&mut entries);
    Ok(entries)
}

fn collect_from_directory(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
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
        // Entry types are decided by filename, mirroring
        // C4ScenarioListLoader::Entry::CreateEntryForFile
        // (C4StartupScenSelDlg.cpp:581-598): "*.c4s" -> Scenario, "*.c4f" ->
        // SubFolder, extension-less directories -> RegularFolder only when
        // they (recursively) contain scenarios. Anything else — including
        // .c4d/.c4g packs — is not listed.
        if is_scenario_filename(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_scenario_entry(&group, identifier, languages)?);
        } else if is_folder_filename(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_folder_entry(&group, identifier, languages)?);
        } else if file_type.is_dir()
            && Path::new(name).extension().is_none()
            && dir_contains_scenarios(&entry.path())
        {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_folder_entry(&group, identifier, languages)?);
        }
    }
    sort_entries(&mut result);
    Ok(result)
}

/// Recursive check whether a directory contains a `.c4s` or `.c4f` item,
/// mirroring `DirContainsScenarios` (C4StartupScenSelDlg.cpp:561-579).
fn dir_contains_scenarios(dir: &Path) -> bool {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return false;
    };
    read_dir.flatten().any(|entry| {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            return false;
        };
        if should_ignore_name(name) {
            return false;
        }
        if is_scenario_filename(name) || is_folder_filename(name) {
            return true;
        }
        entry
            .file_type()
            .map(|kind| kind.is_dir() && dir_contains_scenarios(&entry.path()))
            .unwrap_or(false)
    })
}

fn collect_from_path(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    if path.is_dir() {
        return collect_from_directory(path, parent_identifier, languages);
    }
    if path.is_file() {
        if !is_scenario_filename_os(path) && !is_folder_filename_os(path) {
            return Ok(Vec::new());
        }
        return collect_from_group_file(path, parent_identifier, languages);
    }
    Ok(Vec::new())
}

fn collect_children_from_group(
    group: &Group,
    parent_identifier: &str,
    languages: &[String],
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
        // Folder children are matched by the "*.c4s"/"*.c4f" search masks
        // only (C4ScenarioListLoader::SubFolder::DoLoadContents,
        // C4StartupScenSelDlg.cpp:973-1014); extension-less subdirectories
        // inside groups are not regarded (:588).
        if is_scenario_filename(name) {
            let child_group = group
                .open_child(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_scenario_entry(&child_group, identifier, languages)?);
        } else if is_folder_filename(name) {
            let child_group = group
                .open_child(&entry.relative_path)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_folder_entry(&child_group, identifier, languages)?);
        }
    }
    sort_entries(&mut result);
    Ok(result)
}

fn collect_from_group_file(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
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
    let entry = if is_scenario_filename(name) {
        build_scenario_entry(&group, identifier, languages)?
    } else {
        build_folder_entry(&group, identifier, languages)?
    };
    Ok(vec![entry])
}

fn legacy_core_info(group: &Group) -> Result<Option<LegacyCoreInfo>, ScenarioDiscoveryError> {
    let bytes = match group.read_file("Scenario.txt") {
        Ok(bytes) => bytes,
        Err(GroupError::EntryNotFound(_)) => return Ok(None),
        Err(GroupError::Io(error)) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            let path = group.root().join("Scenario.txt");
            return Err(group_error(&path, err));
        }
    };

    let text = decode_legacy_text(&bytes);

    Ok(Some(parse_legacy_core_info(&text)))
}

fn parse_legacy_core_info(text: &str) -> LegacyCoreInfo {
    let mut info = LegacyCoreInfo::default();
    let mut current_section = String::from("head");

    for raw_line in text.lines() {
        let without_bom = raw_line.trim_start_matches('\u{feff}');
        let mut line = without_bom.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if let Some(idx) = line.find("//") {
            line = line[..idx].trim_end();
            if line.is_empty() {
                continue;
            }
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if current_section.eq_ignore_ascii_case("definitions") {
            // C4SDefinitions::CompileFunc (C4Scenario.cpp:482-483).
            if info.local_only.is_none() && key.eq_ignore_ascii_case("localonly") {
                info.local_only = parse_bool_flag(value);
            } else if info.allow_user_change.is_none()
                && key.eq_ignore_ascii_case("allowuserchange")
            {
                info.allow_user_change = parse_bool_flag(value);
            }
            continue;
        }
        if !current_section.eq_ignore_ascii_case("head") {
            continue;
        }
        if info.title.is_none() && key.eq_ignore_ascii_case("title") {
            info.title = Some(value.to_string());
        } else if info.description.is_none()
            && (key.eq_ignore_ascii_case("description") || key.eq_ignore_ascii_case("desc"))
        {
            info.description = Some(value.to_string());
        } else if info.icon.is_none() && key.eq_ignore_ascii_case("icon") {
            if let Ok(parsed) = value.parse::<i32>() {
                info.icon = Some(parsed);
            }
        } else if info.difficulty.is_none() && key.eq_ignore_ascii_case("difficulty") {
            if let Ok(parsed) = value.parse::<i32>() {
                info.difficulty = Some(parsed);
            }
        } else if info.save_game.is_none() && key.eq_ignore_ascii_case("savegame") {
            if let Some(parsed) = parse_bool_flag(value) {
                info.save_game = Some(parsed);
            }
        } else if info.replay.is_none() && key.eq_ignore_ascii_case("replay") {
            if let Some(parsed) = parse_bool_flag(value) {
                info.replay = Some(parsed);
            }
        }
    }

    info
}

fn parse_bool_flag(value: &str) -> Option<bool> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    match normalized.as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

fn build_scenario_entry(
    group: &Group,
    identifier: String,
    languages: &[String],
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_path(group.root());
    let manifest = scenario_manifest_info(group)?;
    let legacy = legacy_core_info(group)?;
    // Name precedence mirrors C4ScenarioListLoader::Entry::Load
    // (C4StartupScenSelDlg.cpp:477-515): the language-resolved Title.txt wins,
    // then the Scenario.txt [Head] Title fallback (Scenario::LoadCustom,
    // :712-714), then the filename. The Scenario.json manifest is a Rust-port
    // extension slotted between Title.txt and the legacy core.
    let mut title = title_from_title_files(group, languages)?;

    if title.is_none() {
        title = manifest
            .as_ref()
            .and_then(|info| info.name.as_ref())
            .map(|name| name.trim())
            .filter(|name| !name.is_empty())
            .map(|name| name.to_string());
    }

    if title.is_none() {
        if let Some(core) = legacy
            .as_ref()
            .and_then(|info| info.title.as_ref())
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            title = Some(core.to_string());
        }
    }

    let title = title.unwrap_or(fallback);
    let (preview, title_picture) = load_preview_images(group)?;
    let description = manifest
        .as_ref()
        .and_then(|info| info.description.as_ref())
        .map(|desc| desc.trim())
        .filter(|desc| !desc.is_empty())
        .map(|desc| desc.to_string())
        .or_else(|| description_from_desc_files(group, languages))
        .or_else(|| {
            legacy
                .as_ref()
                .and_then(|info| info.description.as_ref())
                .map(|desc| desc.trim())
                .filter(|desc| !desc.is_empty())
                .map(|desc| desc.to_string())
        });
    let icon_index = legacy.as_ref().and_then(|info| info.icon);
    let difficulty = legacy.as_ref().and_then(|info| {
        if info.save_game.unwrap_or(false) || info.replay.unwrap_or(false) {
            None
        } else {
            info.difficulty
        }
    });

    Ok(ScenarioEntry {
        identifier,
        path: group.root().to_path_buf(),
        title,
        description,
        kind: ScenarioEntryKind::Scenario,
        is_editable: group.is_directory(),
        is_playable: true,
        preview,
        title_picture,
        children: Vec::new(),
        folder_index: None,
        icon_index,
        difficulty,
        author: load_author(group),
        version: load_version(group),
        local_only: legacy.as_ref().and_then(|info| info.local_only),
        allow_user_change: legacy.as_ref().and_then(|info| info.allow_user_change),
    })
}

fn build_folder_entry(
    group: &Group,
    identifier: String,
    languages: &[String],
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_path(group.root());
    let mut title = title_from_title_files(group, languages)?;
    let folder_info = folder_core_info(group)?;
    if title.is_none() {
        if let Some(info) = folder_info.as_ref().and_then(|info| info.title.clone()) {
            title = Some(info);
        }
    }

    let title = title.unwrap_or(fallback);
    let (preview, title_picture) = load_preview_images(group)?;
    // Extension-less directories are C4ScenarioListLoader::RegularFolder:
    // their contents come from a directory iteration that also accepts
    // nested plain directories (C4StartupScenSelDlg.cpp:1043-1085), while
    // .c4f folders (packed or unpacked) only search the "*.c4s"/"*.c4f"
    // masks (SubFolder::DoLoadContents, :973-1014).
    let children = if group.is_directory() && group.root().extension().is_none() {
        collect_from_directory(group.root(), &identifier, languages)?
    } else {
        collect_children_from_group(group, &identifier, languages)?
    };
    let folder_index = folder_info.and_then(|info| info.index);

    Ok(ScenarioEntry {
        identifier,
        path: group.root().to_path_buf(),
        title,
        description: description_from_desc_files(group, languages),
        kind: ScenarioEntryKind::Folder,
        is_editable: group.is_directory(),
        is_playable: false,
        preview,
        title_picture,
        children,
        folder_index,
        icon_index: None,
        difficulty: None,
        author: load_author(group),
        version: load_version(group),
        local_only: None,
        allow_user_change: None,
    })
}

/// The right-page description per `C4CFN_ScenarioDesc` = "Desc{}.rtf"
/// (C4Components.h:74): the first `Desc<code>.rtf` of the language sequence
/// that exists is converted from RTF to plain text
/// (C4StartupScenSelDlg.cpp:523-531).
fn description_from_desc_files(group: &Group, languages: &[String]) -> Option<String> {
    languages
        .iter()
        .map(|code| format!("Desc{code}.rtf"))
        .find(|candidate| group.exists(candidate))
        .and_then(|candidate| group.read_file(&candidate).ok())
        .map(|bytes| crate::rtf::rtf_to_plain_text(&bytes))
        .filter(|text| !text.is_empty())
}

/// `Version.txt` (C4CFN_Version, C4StartupScenSelDlg.cpp:554).
fn load_version(group: &Group) -> Option<String> {
    group
        .read_file("Version.txt")
        .ok()
        .map(|bytes| decode_legacy_text(&bytes).trim().to_string())
        .filter(|version| !version.is_empty())
}

/// The author of packed groups (Entry::Load, C4StartupScenSelDlg.cpp:536-552):
/// an `Author.txt` override is honoured for the hardcoded official makers,
/// otherwise the group maker itself; unpacked directories have no author.
fn load_author(group: &Group) -> Option<String> {
    if group.is_directory() {
        return None;
    }
    let maker = group
        .maker()
        .map(str::trim)
        .filter(|maker| !maker.is_empty())
        .map(str::to_string)?;
    const SECONDARY_AUTHOR_MAKERS: [&str; 3] =
        ["RedWolf Design", "Clonk History Project", "GWE-Team"];
    SECONDARY_AUTHOR_MAKERS
        .contains(&maker.as_str())
        .then(|| group.read_file("Author.txt").ok())
        .flatten()
        .map(|bytes| decode_legacy_text(&bytes).trim().to_string())
        .filter(|author| !author.is_empty())
        .or(Some(maker))
}

/// Loads the list preview image (title > loader > icon candidates) and, when
/// the winning candidate is a Title.* image, the right-page title picture
/// (fctTitle, C4StartupScenSelDlg.cpp:532-534) sharing the same pixel data.
fn load_preview_images(
    group: &Group,
) -> Result<(Option<ScenarioPreview>, Option<ScenarioPreview>), ScenarioDiscoveryError> {
    let entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?;

    let mut candidates: Vec<((u8, u8, String), PathBuf)> = Vec::new();
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
        candidates.push((key, entry.relative_path.clone()));
    }

    if candidates.is_empty() {
        return Ok((None, None));
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    for ((prefix_rank, _, _), relative_path) in candidates {
        let absolute_path = group.root().join(&relative_path);
        let bytes = group
            .read_file(&relative_path)
            .map_err(|err| group_error(&absolute_path, err))?;
        let image = match load_from_memory(&bytes) {
            Ok(image) => image,
            Err(source) => {
                debug!(
                    path = %absolute_path.display(),
                    error = %source,
                    "skipping unsupported scenario preview image"
                );
                continue;
            }
        };
        let rgba = image.to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        let data = rgba.into_raw();
        let preview = ScenarioPreview::new(width, height, data);
        let title_picture = (prefix_rank == 0).then(|| preview.clone());
        return Ok((Some(preview), title_picture));
    }

    Ok((None, None))
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

/// Resolves the entry name from its title component, mirroring
/// `C4ComponentHost::LoadEx` over `C4CFN_Title` = "Title{}.txt|Title.txt"
/// (C4ComponentHost.cpp:56-95, C4Components.h:67): the first existing
/// candidate file — "Title<code>.txt" per language code, then the plain
/// "Title.txt" — is loaded, and the language string is looked up in it.
fn title_from_title_files(
    group: &Group,
    languages: &[String],
) -> Result<Option<String>, ScenarioDiscoveryError> {
    let candidates = languages
        .iter()
        .map(|code| format!("Title{code}.txt"))
        .chain(std::iter::once("Title.txt".to_string()));
    for candidate in candidates {
        if !group.exists(&candidate) {
            continue;
        }
        let data = group
            .read_file(&candidate)
            .map_err(|err| group_error(&group.root().join(&candidate), err))?;
        let text = decode_legacy_text(&data);
        // Only the first found file is consulted (C4ComponentHost keeps a
        // single Data buffer); a failed language lookup falls back to the
        // caller's name chain, not to further title files
        // (C4StartupScenSelDlg.cpp:480-483).
        return Ok(resolve_language_string(&text, languages));
    }
    Ok(None)
}

/// `C4ComponentHost::GetLanguageString` (C4ComponentHost.cpp:238-260): for
/// each 2-letter code of the sequence, search the text body for "XX:" and
/// return the remainder of that line.
fn resolve_language_string(text: &str, languages: &[String]) -> Option<String> {
    languages.iter().find_map(|code| {
        let needle = format!("{code}:");
        text.find(&needle).map(|pos| {
            let rest = &text[pos + needle.len()..];
            let end = rest.find(['\r', '\n']).unwrap_or(rest.len());
            rest[..end].to_string()
        })
    })
}

/// Decodes legacy component text: UTF-8 when valid, otherwise the
/// Windows-1252 system charset of old Clonk content (the C++ engine converts
/// via `TextEncodingConverter.SystemToClonk`, C4StartupScenSelDlg.cpp:474).
pub(crate) fn decode_legacy_text(data: &[u8]) -> String {
    std::str::from_utf8(data)
        .map(str::to_string)
        .unwrap_or_else(|_| data.iter().map(|&byte| cp1252_char(byte)).collect())
}

/// Windows-1252 byte to Unicode; the 0x80..0x9F range holds the CP1252
/// specials, everything else maps like Latin-1.
fn cp1252_char(byte: u8) -> char {
    match byte {
        0x80 => '\u{20AC}',
        0x82 => '\u{201A}',
        0x83 => '\u{0192}',
        0x84 => '\u{201E}',
        0x85 => '\u{2026}',
        0x86 => '\u{2020}',
        0x87 => '\u{2021}',
        0x88 => '\u{02C6}',
        0x89 => '\u{2030}',
        0x8A => '\u{0160}',
        0x8B => '\u{2039}',
        0x8C => '\u{0152}',
        0x8E => '\u{017D}',
        0x91 => '\u{2018}',
        0x92 => '\u{2019}',
        0x93 => '\u{201C}',
        0x94 => '\u{201D}',
        0x95 => '\u{2022}',
        0x96 => '\u{2013}',
        0x97 => '\u{2014}',
        0x98 => '\u{02DC}',
        0x99 => '\u{2122}',
        0x9A => '\u{0161}',
        0x9B => '\u{203A}',
        0x9C => '\u{0153}',
        0x9E => '\u{017E}',
        0x9F => '\u{0178}',
        other => other as char,
    }
}

fn folder_core_info(group: &Group) -> Result<Option<LegacyFolderInfo>, ScenarioDiscoveryError> {
    if !group.exists("Folder.txt") {
        return Ok(None);
    }
    let data = group
        .read_file("Folder.txt")
        .map_err(|err| group_error(group.root(), err))?;
    let content = match std::str::from_utf8(&data) {
        Ok(content) => content,
        Err(_) => return Ok(None),
    };
    let info = parse_legacy_folder_core(content);
    if info.title.is_none() && info.index.is_none() {
        Ok(None)
    } else {
        Ok(Some(info))
    }
}

fn parse_legacy_folder_core(text: &str) -> LegacyFolderInfo {
    let mut info = LegacyFolderInfo::default();
    let mut current_section = String::from("head");

    for raw_line in text.lines() {
        let without_bom = raw_line.trim_start_matches('\u{feff}');
        let mut line = without_bom.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with(';') || line.starts_with('#') || line.starts_with("//") {
            continue;
        }
        if let Some(idx) = line.find("//") {
            line = line[..idx].trim_end();
            if line.is_empty() {
                continue;
            }
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if !current_section.eq_ignore_ascii_case("head") {
            continue;
        }
        let key = key.trim();
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if info.title.is_none() && key.eq_ignore_ascii_case("title") {
            info.title = Some(value.to_string());
        } else if info.index.is_none() && key.eq_ignore_ascii_case("index") {
            if let Ok(parsed) = value.parse::<i32>() {
                info.index = Some(parsed);
            }
        }
        if info.title.is_some() && info.index.is_some() {
            break;
        }
    }

    info
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

fn sort_entries(entries: &mut [ScenarioEntry]) {
    entries.sort_by(compare_entries);
    for entry in entries.iter_mut() {
        sort_entries(&mut entry.children);
    }
}

fn compare_entries(a: &ScenarioEntry, b: &ScenarioEntry) -> Ordering {
    let a_is_folder = matches!(a.kind, ScenarioEntryKind::Folder);
    let b_is_folder = matches!(b.kind, ScenarioEntryKind::Folder);
    if a_is_folder != b_is_folder {
        return if a_is_folder {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    let a_folder_index = a.folder_index.unwrap_or(0);
    let b_folder_index = b.folder_index.unwrap_or(0);
    if a_folder_index != 0 || b_folder_index != 0 {
        if a_folder_index == 0 {
            return Ordering::Greater;
        }
        if b_folder_index == 0 {
            return Ordering::Less;
        }
        match a_folder_index.cmp(&b_folder_index) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    if let Some(icon) = a.icon_index {
        if (2..=11).contains(&icon) {
            let other_icon = b.icon_index.unwrap_or(-1);
            let diff = icon - other_icon;
            if diff != 0 {
                return diff.cmp(&0);
            }
        }
    }

    let a_difficulty = a.difficulty.unwrap_or(0);
    let b_difficulty = b.difficulty.unwrap_or(0);
    if a_difficulty != 0 || b_difficulty != 0 {
        if a_difficulty == 0 {
            return Ordering::Greater;
        }
        if b_difficulty == 0 {
            return Ordering::Less;
        }
        match a_difficulty.cmp(&b_difficulty) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    let name_order = compare_case_insensitive(&a.title, &b.title);
    if name_order != Ordering::Equal {
        return name_order;
    }

    compare_case_insensitive(&a.identifier, &b.identifier)
}

fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
}

fn os_str_from_path(path: &Path) -> &std::ffi::OsStr {
    path.file_name().unwrap_or(path.as_os_str())
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
    fn orders_entries_like_legacy_loader() {
        let dir = tempdir().unwrap();

        let indexed_folder = dir.path().join("Indexed.c4f");
        fs::create_dir(&indexed_folder).unwrap();
        fs::write(
            indexed_folder.join("Folder.txt"),
            "[Head]\nTitle=Indexed\nIndex=2\n",
        )
        .unwrap();

        let unindexed_folder = dir.path().join("Unindexed.c4f");
        fs::create_dir(&unindexed_folder).unwrap();
        fs::write(
            unindexed_folder.join("Folder.txt"),
            "[Head]\nTitle=Unindexed\n",
        )
        .unwrap();

        let challenging = dir.path().join("Challenging.c4s");
        fs::create_dir(&challenging).unwrap();
        fs::write(
            challenging.join("Scenario.txt"),
            "[Head]\nTitle=Challenging\nDifficulty=4\n",
        )
        .unwrap();

        let free_play = dir.path().join("FreePlay.c4s");
        fs::create_dir(&free_play).unwrap();
        fs::write(
            free_play.join("Scenario.txt"),
            "[Head]\nTitle=FreePlay\nDifficulty=0\n",
        )
        .unwrap();

        let mission_a = dir.path().join("MissionA.c4s");
        fs::create_dir(&mission_a).unwrap();
        fs::write(
            mission_a.join("Scenario.txt"),
            "[Head]\nTitle=Mission A\nIcon=2\nDifficulty=3\n",
        )
        .unwrap();

        let mission_b = dir.path().join("MissionB.c4s");
        fs::create_dir(&mission_b).unwrap();
        fs::write(
            mission_b.join("Scenario.txt"),
            "[Head]\nTitle=Mission B\nIcon=5\nDifficulty=1\n",
        )
        .unwrap();

        let entries = discover(dir.path()).expect("discover");
        let titles: Vec<_> = entries.iter().map(|entry| entry.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Indexed",
                "Unindexed",
                "Challenging",
                "FreePlay",
                "Mission A",
                "Mission B"
            ]
        );
    }

    // C4ComponentHost::GetLanguageString (C4ComponentHost.cpp:238-260): each
    // 2-letter code of the language sequence is searched as "XX:" in the text
    // body; the match runs to the end of the line.
    #[test]
    fn resolves_language_prefixed_title_lines() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Tutorial.c4f");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Folder.txt"), "[Head]\nIndex=1\n").unwrap();
        fs::write(scenario_dir.join("Title.txt"), "DE:Lernrunden\r\nUS:Tutorial").unwrap();

        let us = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(us[0].title, "Tutorial");

        let de = discover_with_languages(dir.path(), &langs(&["DE", "US"])).expect("discover");
        assert_eq!(de[0].title, "Lernrunden");

        // Unknown language falls back through the sequence to the next code.
        let fr = discover_with_languages(dir.path(), &langs(&["FR", "DE"])).expect("discover");
        assert_eq!(fr[0].title, "Lernrunden");
    }

    // C4CFN_Title = "Title{}.txt|Title.txt" (C4Components.h:67): language-
    // suffixed title files are tried before the plain Title.txt.
    #[test]
    fn prefers_language_specific_title_file() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Scenario.txt"), "[Head]\n").unwrap();
        fs::write(scenario_dir.join("TitleUS.txt"), "US:From TitleUS").unwrap();
        fs::write(scenario_dir.join("Title.txt"), "US:From Title").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US"])).expect("discover");
        assert_eq!(entries[0].title, "From TitleUS");
    }

    // C4ScenarioListLoader::Entry::Load (C4StartupScenSelDlg.cpp:477-515):
    // Title.txt wins over the Scenario.txt [Head] Title fallback.
    #[test]
    fn title_txt_beats_scenario_core_title() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Goldmine.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Goldmine\n",
        )
        .unwrap();
        fs::write(scenario_dir.join("Title.txt"), "DE:Goldmine\nUS:Gold Mine\n").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(entries[0].title, "Gold Mine");
    }

    // Title bytes are Windows-1252 in legacy content; non-UTF-8 titles must
    // not be dropped (SystemToClonk conversion, C4StartupScenSelDlg.cpp:474).
    #[test]
    fn decodes_windows_1252_titles() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Umlaut.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Scenario.txt"), "[Head]\n").unwrap();
        fs::write(scenario_dir.join("Title.txt"), b"DE:R\xe4uber\n").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["DE"])).expect("discover");
        assert_eq!(entries[0].title, "Räuber");
    }

    // When no "XX:" line matches the sequence, the name falls back like C++
    // (fNameLoaded stays false -> C4S.Head.Title for scenarios).
    #[test]
    fn unmatched_language_falls_back_to_core_title() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Scenario.txt"), "[Head]\nTitle=CoreTitle\n").unwrap();
        fs::write(scenario_dir.join("Title.txt"), "DE:Nur Deutsch\n").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US"])).expect("discover");
        assert_eq!(entries[0].title, "CoreTitle");
    }

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| code.to_string()).collect()
    }

    // Entry::Load with fLoadEx (C4StartupScenSelDlg.cpp:520-531): the
    // description comes from Desc<code>.rtf per C4CFN_ScenarioDesc =
    // "Desc{}.rtf" (C4Components.h:74), converted from RTF to plain text.
    #[test]
    fn loads_description_from_language_desc_rtf() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Scenario.txt"), "[Head]\nTitle=Alpha\n").unwrap();
        fs::write(
            scenario_dir.join("DescDE.rtf"),
            br"{\rtf1 Deutsch\par}".as_slice(),
        )
        .unwrap();
        fs::write(
            scenario_dir.join("DescUS.rtf"),
            br"{\rtf1 English\par}".as_slice(),
        )
        .unwrap();

        let us = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(us[0].description.as_deref(), Some("English\n"));

        let de = discover_with_languages(dir.path(), &langs(&["DE", "US"])).expect("discover");
        assert_eq!(de[0].description.as_deref(), Some("Deutsch\n"));

        // A code without its own file falls through the sequence.
        let fr = discover_with_languages(dir.path(), &langs(&["FR", "DE"])).expect("discover");
        assert_eq!(fr[0].description.as_deref(), Some("Deutsch\n"));
    }

    // Folders load descriptions the same way (generic Entry::Load).
    #[test]
    fn folders_load_desc_rtf_too() {
        let dir = tempdir().unwrap();
        let folder = dir.path().join("Fantasy.c4f");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Folder.txt"), "[Head]\nIndex=9\n").unwrap();
        fs::write(
            folder.join("DescUS.rtf"),
            br"{\rtf1 Magic worlds.\par}".as_slice(),
        )
        .unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(entries[0].description.as_deref(), Some("Magic worlds.\n"));
    }

    // The right-page title picture is Title.png/Title.bmp only
    // (C4CFN_ScenarioTitlePNG/C4CFN_ScenarioTitle, C4StartupScenSelDlg.cpp:
    // 532-534); Loader*.jpg is a list-preview fallback, not a title picture.
    #[test]
    fn title_picture_requires_title_image() {
        let dir = tempdir().unwrap();

        let with_title = dir.path().join("Titled.c4s");
        fs::create_dir(&with_title).unwrap();
        fs::write(with_title.join("Scenario.txt"), "[Head]\nTitle=Titled\n").unwrap();
        fs::write(with_title.join("Title.png"), encode_test_png()).unwrap();

        let with_loader = dir.path().join("Loaded.c4s");
        fs::create_dir(&with_loader).unwrap();
        fs::write(with_loader.join("Scenario.txt"), "[Head]\nTitle=Loaded\n").unwrap();
        fs::write(with_loader.join("LoaderBG.png"), encode_test_png()).unwrap();

        let entries = discover(dir.path()).expect("discover");
        let titled = entries
            .iter()
            .find(|entry| entry.title == "Titled")
            .unwrap();
        let loaded = entries
            .iter()
            .find(|entry| entry.title == "Loaded")
            .unwrap();
        assert!(titled.title_picture.is_some());
        assert!(titled.preview.is_some());
        assert!(loaded.title_picture.is_none(), "loader is not a title pic");
        assert!(loaded.preview.is_some(), "loader still previews the list");
    }

    // [Definitions] LocalOnly/AllowUserChange feed the "Choose definitions"
    // checkbox (C4StartupScenSelDlg.cpp:1590-1599; defaults false,
    // C4Scenario.cpp:150,482-483). Version.txt feeds the version line.
    #[test]
    fn reads_definitions_flags_and_version() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Alpha\n\n[Definitions]\nLocalOnly=1\nAllowUserChange=1\n",
        )
        .unwrap();
        fs::write(scenario_dir.join("Version.txt"), "4.9.8.2\n").unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries[0].local_only, Some(true));
        assert_eq!(entries[0].allow_user_change, Some(true));
        assert_eq!(entries[0].version.as_deref(), Some("4.9.8.2"));
    }

    fn encode_test_png() -> Vec<u8> {
        let image = image::RgbaImage::from_raw(2, 2, vec![255u8; 16]).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageOutputFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    // C4ScenarioListLoader::Entry::CreateEntryForFile
    // (C4StartupScenSelDlg.cpp:581-598) only regards *.c4s, *.c4f and
    // extension-less directories; .c4d/.c4g packs must not be listed — a
    // Fantasy.c4d next to Fantasy.c4f previously produced a duplicate entry.
    #[test]
    fn skips_c4d_and_c4g_packs() {
        let dir = tempdir().unwrap();

        let folder = dir.path().join("Fantasy.c4f");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Title.txt"), "DE:Fantasy\nUS:Fantasy\n").unwrap();
        fs::write(folder.join("Folder.txt"), "[Head]\nIndex=9\n").unwrap();

        // A definition pack with the same title and inner directories, which
        // the old content-based classifier misread as a scenario folder.
        let pack = dir.path().join("Fantasy.c4d");
        fs::create_dir(&pack).unwrap();
        fs::write(pack.join("Title.txt"), "DE:Fantasy\nUS:Fantasy\n").unwrap();
        fs::create_dir(pack.join("Wizard.c4d")).unwrap();

        let gfx = dir.path().join("Material.c4g");
        fs::create_dir(&gfx).unwrap();
        fs::create_dir(gfx.join("SomeDir")).unwrap();

        let entries = discover(dir.path()).expect("discover");
        let titles: Vec<_> = entries.iter().map(|entry| entry.title.as_str()).collect();
        assert_eq!(titles, vec!["Fantasy"], "only the .c4f folder is listed");
    }

    // Extension-less directories are only listed when they (recursively)
    // contain scenarios or folders (DirContainsScenarios,
    // C4StartupScenSelDlg.cpp:561-579).
    #[test]
    fn lists_plain_directories_only_with_scenarios_inside() {
        let dir = tempdir().unwrap();

        let with = dir.path().join("Downloads");
        fs::create_dir(&with).unwrap();
        let nested = with.join("nested");
        fs::create_dir(&nested).unwrap();
        let child = nested.join("Custom.c4s");
        fs::create_dir(&child).unwrap();
        fs::write(child.join("Scenario.txt"), "[Head]\nTitle=Custom\n").unwrap();

        let without = dir.path().join("updates");
        fs::create_dir(&without).unwrap();
        fs::write(without.join("notes.txt"), "nothing").unwrap();

        let entries = discover(dir.path()).expect("discover");
        let titles: Vec<_> = entries.iter().map(|entry| entry.title.as_str()).collect();
        assert_eq!(titles, vec!["Downloads"]);
        // RegularFolder children come from directory iteration, so the
        // nested extension-less dir is listed as a folder in turn.
        assert_eq!(entries[0].children.len(), 1);
        assert_eq!(entries[0].children[0].title, "nested");
        assert_eq!(entries[0].children[0].children[0].title, "Custom");
    }

    #[test]
    fn falls_back_to_legacy_core_for_metadata() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Legacy.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Legacy Mission\nDescription=Restore Clonk parity\n",
        )
        .unwrap();

        let entries = discover(dir.path()).expect("discover legacy scenario");
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.title, "Legacy Mission");
        assert_eq!(entry.description.as_deref(), Some("Restore Clonk parity"));
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
