use crate::{
    language::component_language_string, load_image_from_memory, ComponentGroups, Group,
    GroupError, LanguagePacks,
};
use image::ImageError;
use serde::Deserialize;
use std::cmp::Ordering;
use std::fs;
use std::io;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::debug;

#[derive(Debug, thiserror::Error)]
pub enum ScenarioDiscoveryError {
    #[error("scenario discovery cancelled")]
    Cancelled,
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

/// Progress through a scenario discovery walk. The total grows as nested
/// folders are opened, matching the loader's incremental work estimation
/// without performing a second filesystem/archive traversal up front. The
/// percentage is held at its previous high-water mark when that growth would
/// otherwise make it move backwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScenarioDiscoveryProgress {
    pub current: usize,
    pub total: usize,
    percent_floor: u8,
}

impl ScenarioDiscoveryProgress {
    pub fn percent(self) -> u8 {
        let percent = (self.current as u128) * 100 / (self.total.max(1) as u128);
        u8::try_from(percent.min(100))
            .unwrap_or(100)
            .max(self.percent_floor)
    }
}

struct DiscoveryContext<'a> {
    callback: &'a mut dyn FnMut(ScenarioDiscoveryProgress) -> ControlFlow<()>,
    current: usize,
    total: usize,
    emitted_percent: u8,
}

impl<'a> DiscoveryContext<'a> {
    fn new(callback: &'a mut dyn FnMut(ScenarioDiscoveryProgress) -> ControlFlow<()>) -> Self {
        Self {
            callback,
            current: 0,
            total: 0,
            emitted_percent: 0,
        }
    }

    fn report(&mut self) -> Result<(), ScenarioDiscoveryError> {
        let progress = ScenarioDiscoveryProgress {
            current: self.current,
            total: self.total,
            percent_floor: self.emitted_percent,
        };
        self.emitted_percent = progress.percent();
        match (self.callback)(progress) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(()) => Err(ScenarioDiscoveryError::Cancelled),
        }
    }

    fn add_work(&mut self, count: usize) -> Result<(), ScenarioDiscoveryError> {
        self.total = self.total.saturating_add(count);
        self.report()
    }

    fn complete_work(&mut self) -> Result<(), ScenarioDiscoveryError> {
        self.current = self.current.saturating_add(1).min(self.total);
        self.report()
    }

    fn finish(&mut self) -> Result<(), ScenarioDiscoveryError> {
        self.total = self.total.max(1);
        self.current = self.total;
        self.report()
    }
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
    /// Scenario.txt `[Head] MissionAccess`; kept separate from `is_playable`
    /// because grants can change while the catalog remains loaded.
    pub mission_access: Option<String>,
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
    /// Ordered external definition modules from `Definitions` or the legacy
    /// `Definition1`...`Definition10` fallback (C4Scenario.cpp:484-493).
    pub definition_modules: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct LegacyCoreInfo {
    title: Option<String>,
    description: Option<String>,
    icon: Option<i32>,
    difficulty: Option<i32>,
    save_game: Option<bool>,
    replay: Option<bool>,
    mission_access: Option<String>,
    local_only: Option<bool>,
    allow_user_change: Option<bool>,
    definition_modules: Vec<String>,
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
    discover_with_languages_and_packs(root, languages, &LanguagePacks::default())
}

pub fn discover_with_languages_and_packs(
    root: impl AsRef<Path>,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    discover_with_languages_and_packs_with_progress(root, languages, language_packs, |_| {
        ControlFlow::Continue(())
    })
}

/// Discovers one named `.c4s`/`.c4f` entry without flattening a directory
/// container. This mirrors loading that entry from its parent folder in
/// `C4ScenarioListLoader::Entry::Load` (C4StartupScenSelDlg.cpp:440-558).
pub fn discover_entry_with_languages_and_packs(
    path: impl AsRef<Path>,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<Option<ScenarioEntry>, ScenarioDiscoveryError> {
    discover_entry_with_languages_and_packs_with_progress(path, languages, language_packs, |_| {
        ControlFlow::Continue(())
    })
}

/// Progress-reporting form of [`discover_entry_with_languages_and_packs`].
pub fn discover_entry_with_languages_and_packs_with_progress<F>(
    path: impl AsRef<Path>,
    languages: &[String],
    language_packs: &LanguagePacks,
    mut progress: F,
) -> Result<Option<ScenarioEntry>, ScenarioDiscoveryError>
where
    F: FnMut(ScenarioDiscoveryProgress) -> ControlFlow<()>,
{
    let mut context = DiscoveryContext::new(&mut progress);
    context.add_work(1)?;
    let entry = collect_group_entry(path.as_ref(), "", languages, language_packs, &mut context)?;
    context.complete_work()?;
    context.finish()?;
    Ok(entry)
}

/// Discovers one scenario root while reporting incremental recursive work.
/// Returning [`ControlFlow::Break`] cancels at the next filesystem/group
/// checkpoint and yields [`ScenarioDiscoveryError::Cancelled`].
pub fn discover_with_languages_and_packs_with_progress<F>(
    root: impl AsRef<Path>,
    languages: &[String],
    language_packs: &LanguagePacks,
    progress: F,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    F: FnMut(ScenarioDiscoveryProgress) -> ControlFlow<()>,
{
    discover_many_with_languages_and_packs_with_progress(
        [root.as_ref()],
        languages,
        language_packs,
        progress,
    )
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
    discover_many_with_languages_and_packs(roots, languages, &LanguagePacks::default())
}

pub fn discover_many_with_languages_and_packs<I, P>(
    roots: I,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    discover_many_with_languages_and_packs_with_progress(roots, languages, language_packs, |_| {
        ControlFlow::Continue(())
    })
}

pub fn discover_many_with_languages_and_packs_with_progress<I, P, F>(
    roots: I,
    languages: &[String],
    language_packs: &LanguagePacks,
    mut progress: F,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
    F: FnMut(ScenarioDiscoveryProgress) -> ControlFlow<()>,
{
    let roots = roots
        .into_iter()
        .map(|root| root.as_ref().to_path_buf())
        .collect::<Vec<_>>();
    let mut context = DiscoveryContext::new(&mut progress);
    context.add_work(roots.len())?;
    let mut entries = Vec::new();
    for root in roots {
        let mut discovered = collect_from_path(&root, "", languages, language_packs, &mut context)?;
        entries.append(&mut discovered);
        context.complete_work()?;
    }
    sort_entries(&mut entries);
    context.finish()?;
    Ok(entries)
}

fn collect_from_directory(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
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
        compare_case_insensitive_bytes(
            a.file_name().as_encoded_bytes(),
            b.file_name().as_encoded_bytes(),
        )
    });
    context.add_work(entries.len())?;

    let mut result = Vec::new();
    for entry in entries {
        context.report()?;
        let file_type = entry
            .file_type()
            .map_err(|source| ScenarioDiscoveryError::ReadEntry {
                path: path.to_path_buf(),
                source,
            })?;

        let name_os = entry.file_name();
        let name = name_os.as_encoded_bytes();

        if should_ignore_name_bytes(name) {
            context.complete_work()?;
            continue;
        }

        let identifier = join_identifier_bytes(parent_identifier, name);
        // Entry types are decided by filename, mirroring
        // C4ScenarioListLoader::Entry::CreateEntryForFile
        // (C4StartupScenSelDlg.cpp:581-598): "*.c4s" -> Scenario, "*.c4f" ->
        // SubFolder, extension-less directories -> RegularFolder only when
        // they (recursively) contain scenarios. Anything else — including
        // .c4d/.c4g packs — is not listed.
        if is_scenario_filename_bytes(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_scenario_entry(
                &group,
                identifier,
                name,
                languages,
                language_packs,
            )?);
        } else if is_folder_filename_bytes(name) {
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_folder_entry(
                &group,
                identifier,
                name,
                languages,
                language_packs,
                context,
            )?);
        } else if file_type.is_dir() && Path::new(name_os.as_os_str()).extension().is_none() {
            if !dir_contains_scenarios(&entry.path(), context)? {
                context.complete_work()?;
                continue;
            }
            let group = Group::open(entry.path()).map_err(|err| ScenarioDiscoveryError::Group {
                path: entry.path(),
                source: err,
            })?;
            result.push(build_folder_entry(
                &group,
                identifier,
                name,
                languages,
                language_packs,
                context,
            )?);
        }
        context.complete_work()?;
    }
    sort_entries(&mut result);
    Ok(result)
}

/// Recursive check whether a directory contains a `.c4s` or `.c4f` item,
/// mirroring `DirContainsScenarios` (C4StartupScenSelDlg.cpp:561-579).
fn dir_contains_scenarios(
    dir: &Path,
    context: &mut DiscoveryContext<'_>,
) -> Result<bool, ScenarioDiscoveryError> {
    context.report()?;
    let Ok(read_dir) = fs::read_dir(dir) else {
        return Ok(false);
    };
    for entry in read_dir.flatten() {
        context.report()?;
        let name = entry.file_name();
        let name = name.as_encoded_bytes();
        if should_ignore_name_bytes(name) {
            continue;
        }
        if is_scenario_filename_bytes(name) || is_folder_filename_bytes(name) {
            return Ok(true);
        }
        let contains_scenarios = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
            && dir_contains_scenarios(&entry.path(), context)?;
        if contains_scenarios {
            return Ok(true);
        }
    }
    Ok(false)
}

fn collect_from_path(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    context.report()?;
    if path.is_dir() {
        return collect_from_directory(path, parent_identifier, languages, language_packs, context);
    }
    if path.is_file() {
        let Some(name) = path.file_name() else {
            return Ok(Vec::new());
        };
        if !is_scenario_filename_bytes(name.as_encoded_bytes())
            && !is_folder_filename_bytes(name.as_encoded_bytes())
        {
            return Ok(Vec::new());
        }
        return collect_from_group_file(
            path,
            parent_identifier,
            languages,
            language_packs,
            context,
        );
    }
    Ok(Vec::new())
}

fn collect_children_from_group(
    group: &Group,
    parent_identifier: &str,
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    let mut entries = group
        .entries()
        .map_err(|err| group_error(group.root(), err))?
        .into_iter()
        .filter(|entry| entry.relative_path.components().count() == 1)
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| compare_case_insensitive_bytes(&a.name_bytes, &b.name_bytes));
    context.add_work(entries.len())?;

    let mut result = Vec::new();
    for entry in entries {
        context.report()?;
        let name = entry.name_bytes.as_slice();
        if should_ignore_name_bytes(name) {
            context.complete_work()?;
            continue;
        }
        let identifier = join_identifier_bytes(parent_identifier, name);
        // Folder children are matched by the "*.c4s"/"*.c4f" search masks
        // only (C4ScenarioListLoader::SubFolder::DoLoadContents,
        // C4StartupScenSelDlg.cpp:973-1014); extension-less subdirectories
        // inside groups are not regarded (:588).
        if is_scenario_filename_bytes(name) {
            let child_group = group
                .open_child_entry_exact(&entry)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_scenario_entry(
                &child_group,
                identifier,
                name,
                languages,
                language_packs,
            )?);
        } else if is_folder_filename_bytes(name) {
            let child_group = group
                .open_child_entry_exact(&entry)
                .map_err(|err| group_error(&group.root().join(&entry.relative_path), err))?;
            result.push(build_folder_entry(
                &child_group,
                identifier,
                name,
                languages,
                language_packs,
                context,
            )?);
        }
        context.complete_work()?;
    }
    sort_entries(&mut result);
    Ok(result)
}

fn collect_from_group_file(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
) -> Result<Vec<ScenarioEntry>, ScenarioDiscoveryError> {
    Ok(
        collect_group_entry(path, parent_identifier, languages, language_packs, context)?
            .into_iter()
            .collect(),
    )
}

fn collect_group_entry(
    path: &Path,
    parent_identifier: &str,
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
) -> Result<Option<ScenarioEntry>, ScenarioDiscoveryError> {
    context.report()?;
    let name_os = match path.file_name() {
        Some(name) => name,
        None => return Ok(None),
    };
    let name = name_os.as_encoded_bytes();
    if !is_scenario_filename_bytes(name) && !is_folder_filename_bytes(name) {
        return Ok(None);
    }
    let group = Group::open(path).map_err(|source| ScenarioDiscoveryError::Group {
        path: path.to_path_buf(),
        source,
    })?;
    let identifier = join_identifier_bytes(parent_identifier, name);
    let entry = if is_scenario_filename_bytes(name) {
        build_scenario_entry(&group, identifier, name, languages, language_packs)?
    } else {
        build_folder_entry(&group, identifier, name, languages, language_packs, context)?
    };
    Ok(Some(entry))
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
    let mut info = parse_legacy_core_info(&text);
    // Mission passwords are opaque native bytes, unlike presentation text.
    // Reparse just this field through the C4 string byte projection so it
    // compares losslessly with Config.General.MissionAccess.
    let visible_len = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    let native_text = clonk_script::c4_string_from_bytes(&bytes[..visible_len]);
    info.mission_access = parse_legacy_mission_access(&native_text);

    Ok(Some(info))
}

/// Reads the first exact `[Head] MissionAccess` value through RCT_All. Unlike
/// the older presentation-metadata parser below, this preserves native bytes,
/// inline `//`, and trailing spaces; section/key names are case-sensitive.
fn parse_legacy_mission_access(text: &str) -> Option<String> {
    struct Node {
        name: String,
        value: Option<String>,
        section: bool,
        indent: isize,
        parent: usize,
    }

    let mut nodes = vec![Node {
        name: String::new(),
        value: None,
        section: true,
        indent: -1,
        parent: 0,
    }];
    let mut current = 0usize;

    for raw_line in text.split_inclusive('\n').flat_map(|line| {
        let line = line.strip_suffix('\n').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        line.split('\r')
    }) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(**byte, b' ' | b'\t'))
            .count();
        let bytes = raw_line.as_bytes();
        let mut position = indent;
        let section = bytes.get(position) == Some(&b'[')
            && bytes.get(position + 1).is_some_and(u8::is_ascii_alphabetic);
        if section {
            position += 1;
        } else if !bytes.get(position).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let name_start = position;
        while bytes
            .get(position)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b' ' | b'_'))
        {
            position += 1;
        }
        let name = &raw_line[name_start..position];
        while bytes
            .get(position)
            .is_some_and(|byte| matches!(*byte, b' ' | b'\t'))
        {
            position += 1;
        }
        let expected = if section { b']' } else { b'=' };
        if bytes.get(position) != Some(&expected) {
            continue;
        }
        position += 1;
        let node_indent = (indent + usize::from(!section)) as isize;
        while current != 0 && nodes[current].indent >= node_indent {
            current = nodes[current].parent;
        }
        let index = nodes.len();
        nodes.push(Node {
            name: name.to_string(),
            value: (!section).then(|| raw_line[position..].to_string()),
            section,
            indent: node_indent,
            parent: current,
        });
        if section {
            current = index;
        }
    }

    let head = nodes
        .iter()
        .enumerate()
        .find(|(_, node)| node.parent == 0 && node.section && node.name == "Head")
        .map(|(index, _)| index)?;
    let raw = nodes
        .iter()
        .find(|node| node.parent == head && !node.section && node.name == "MissionAccess")?
        .value
        .as_deref()?
        .trim_start_matches([' ', '\t']);
    if raw.is_empty() {
        return None;
    }
    let mut bytes = clonk_script::c4_string_bytes(raw);
    bytes.truncate(512); // C4MaxTitle, the fixed MissionAccess buffer.
    Some(clonk_script::c4_string_from_bytes(&bytes))
}

fn parse_legacy_core_info(text: &str) -> LegacyCoreInfo {
    let mut info = LegacyCoreInfo::default();
    let mut current_section = String::from("head");
    let mut in_first_definitions_section = false;
    let mut saw_definitions_section = false;
    let mut saw_local_only = false;
    let mut saw_allow_user_change = false;
    let mut definition_list = None;
    let mut numbered_definitions: [Option<String>; 10] = Default::default();

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
            let Some(section) = stdcompiler_ini_name(&line[1..line.len() - 1]) else {
                // CreateNameTree ignores a malformed section header without
                // leaving the current section.
                continue;
            };
            in_first_definitions_section = section == "Definitions" && !saw_definitions_section;
            if section == "Definitions" {
                saw_definitions_section = true;
            }
            current_section = section.to_ascii_lowercase();
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim();
        let value = raw_value.trim();
        if in_first_definitions_section {
            let definition_key = stdcompiler_ini_name(raw_key);
            // C4SDefinitions::CompileFunc (C4Scenario.cpp:482-493): the
            // modern Definitions container wins; numbered fields are only a
            // fallback when that container key is absent. StdCompiler's INI
            // name tree is case-sensitive and consumes only the first exact
            // section/key occurrence.
            if definition_key == Some("LocalOnly") && !saw_local_only {
                saw_local_only = true;
                info.local_only = parse_bool_flag(value);
            } else if definition_key == Some("AllowUserChange") && !saw_allow_user_change {
                saw_allow_user_change = true;
                info.allow_user_change = parse_bool_flag(value);
            } else if definition_key == Some("Definitions") {
                if definition_list.is_none() {
                    // String(std::string) chooses RCT_Escaped vs RCT_All at
                    // the byte immediately after '='. Preserve leading
                    // whitespace here because ReadString skips it only after
                    // making that choice (StdCompiler.cpp:734-741).
                    definition_list = Some(parse_c4s_string_list(raw_value));
                }
            } else if let Some(index) = definition_key.and_then(definition_number) {
                if numbered_definitions[index].is_none() {
                    numbered_definitions[index] = Some(value.to_string());
                }
            }
            continue;
        }
        if value.is_empty() {
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

    info.definition_modules = match definition_list {
        Some(definitions) => definitions,
        None => numbered_definitions
            .into_iter()
            .flatten()
            .filter(|definition| !definition.is_empty())
            .collect(),
    };

    info
}

/// Extracts a name the same way `StdCompilerINIRead::CreateNameTree` does.
/// Spaces are valid name characters (and thus a trailing space changes the
/// name); a tab terminates the name and is skipped before `]`/`=`.
fn stdcompiler_ini_name(raw: &str) -> Option<&str> {
    let bytes = raw.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = 0;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b' ' || *byte == b'_')
    {
        end += 1;
    }
    bytes[end..]
        .iter()
        .all(|byte| matches!(byte, b' ' | b'\t'))
        .then(|| &raw[..end])
}

fn definition_number(key: &str) -> Option<usize> {
    const KEYS: [&str; 10] = [
        "Definition1",
        "Definition2",
        "Definition3",
        "Definition4",
        "Definition5",
        "Definition6",
        "Definition7",
        "Definition8",
        "Definition9",
        "Definition10",
    ];
    KEYS.iter().position(|candidate| *candidate == key)
}

/// Parses an INI value compiled through
/// `mkSTLContainerAdapt(vector<string>)` by `StdCompilerINIRead`.
///
/// The first string decides the representation: an unquoted value falls
/// back to `RCT_All` and therefore consumes the whole line, commas included.
/// A quoted value starts the comma-separated escaped-string representation.
/// Order, duplicates, and empty quoted entries are retained.
pub fn parse_c4s_string_list(raw: &str) -> Vec<String> {
    if !raw.starts_with('"') {
        let value = raw.trim_start_matches([' ', '\t']);
        return vec![value.to_string()];
    }

    let mut chars = raw.chars().peekable();
    let mut values = Vec::new();
    loop {
        // Every container element is requested as RCT_Escaped. As in
        // StdCompilerINIRead::String, a non-quote at the current position
        // switches that element to RCT_All and consumes the remaining line.
        if chars.peek() != Some(&'"') {
            let remainder: String = chars.collect();
            values.push(remainder.trim_start_matches([' ', '\t']).to_string());
            break;
        }
        chars.next();

        let mut value = String::new();
        let mut terminated = false;
        while let Some(ch) = chars.next() {
            match ch {
                '"' => {
                    terminated = true;
                    break;
                }
                '\\' => {
                    if let Some(escaped) = parse_c4s_escaped_char(&mut chars) {
                        value.push(escaped);
                    }
                }
                other => value.push(other),
            }
        }
        values.push(value);
        if !terminated {
            break;
        }

        // Separator(SEP_SEP) skips spaces/tabs before requiring a comma.
        while matches!(chars.peek(), Some(' ' | '\t')) {
            chars.next();
        }
        if chars.next_if_eq(&',').is_none() {
            break;
        }
        // Do not skip whitespace here: String checks for a quote before its
        // own ReadString call skips leading whitespace. Consequently a space
        // after the comma selects the one-item RCT_All fallback for the rest.
    }

    values
}

fn parse_c4s_escaped_char(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> Option<char> {
    let escaped = chars.next()?;
    Some(match escaped {
        'a' => '\u{0007}',
        'b' => '\u{0008}',
        'f' => '\u{000c}',
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        'v' => '\u{000b}',
        '\'' => '\'',
        '"' => '"',
        '\\' => '\\',
        '?' => '?',
        'x' => {
            let mut code = 0u32;
            let mut found = false;
            while let Some(digit) = chars.peek().and_then(|next| next.to_digit(16)) {
                found = true;
                code = code.wrapping_mul(16).wrapping_add(digit);
                chars.next();
            }
            if found {
                char::from_u32(code & 0xff).unwrap_or('\0')
            } else {
                'x'
            }
        }
        first @ '0'..='7' => {
            let mut code = first.to_digit(8).unwrap_or(0);
            while let Some(digit) = chars.peek().and_then(|next| next.to_digit(8)) {
                code = code.wrapping_mul(8).wrapping_add(digit);
                chars.next();
            }
            char::from_u32(code & 0xff).unwrap_or('\0')
        }
        // StdCompiler drops the backslash for unknown escapes.
        other => other,
    })
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
    filename: &[u8],
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_name_bytes(filename);
    let manifest = scenario_manifest_info(group)?;
    let legacy = legacy_core_info(group)?;
    // Startup-list entries keep their parsed C4Scenario separate from the
    // process-global Game.C4S consulted by C4Language::GetPackGroups. The
    // entry's own Origin therefore does not remap its Title/Desc lookup.
    let components = language_packs.component_groups(group, None, None);
    // Name precedence mirrors C4ScenarioListLoader::Entry::Load
    // (C4StartupScenSelDlg.cpp:477-515): the language-resolved Title.txt wins,
    // then the Scenario.txt [Head] Title fallback (Scenario::LoadCustom,
    // :712-714), then the filename. The Scenario.json manifest is a Rust-port
    // extension slotted between Title.txt and the legacy core.
    let mut title = title_from_title_files(group, &components, languages)?;

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
        .or_else(|| description_from_desc_files(&components, languages))
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
        mission_access: legacy.as_ref().and_then(|info| info.mission_access.clone()),
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
        definition_modules: legacy
            .as_ref()
            .map(|info| info.definition_modules.clone())
            .unwrap_or_default(),
    })
}

fn build_folder_entry(
    group: &Group,
    identifier: String,
    filename: &[u8],
    languages: &[String],
    language_packs: &LanguagePacks,
    context: &mut DiscoveryContext<'_>,
) -> Result<ScenarioEntry, ScenarioDiscoveryError> {
    let fallback = fallback_title_for_name_bytes(filename);
    let components = language_packs.component_groups(group, None, None);
    let mut title = title_from_title_files(group, &components, languages)?;
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
        collect_from_directory(
            group.root(),
            &identifier,
            languages,
            language_packs,
            context,
        )?
    } else {
        collect_children_from_group(group, &identifier, languages, language_packs, context)?
    };
    let folder_index = folder_info.and_then(|info| info.index);

    Ok(ScenarioEntry {
        identifier,
        path: group.root().to_path_buf(),
        title,
        description: description_from_desc_files(&components, languages),
        kind: ScenarioEntryKind::Folder,
        is_editable: group.is_directory(),
        is_playable: false,
        mission_access: None,
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
        definition_modules: Vec::new(),
    })
}

/// The right-page description per `C4CFN_ScenarioDesc` = "Desc{}.rtf"
/// (C4Components.h:74): the first nonempty `Desc<code>.rtf` loaded from the
/// language sequence is converted from RTF to plain text
/// (C4StartupScenSelDlg.cpp:523-531).
fn description_from_desc_files(
    components: &ComponentGroups,
    languages: &[String],
) -> Option<String> {
    for candidate in languages.iter().map(|code| format!("Desc{code}.rtf")) {
        if let Some(component) = components.read(candidate).ok().flatten() {
            let visible = component
                .bytes
                .split(|byte| *byte == 0)
                .next()
                .unwrap_or_default();
            let text = crate::rtf::rtf_to_plain_text(visible);
            if !text.is_empty() {
                return Some(text);
            }
            return None;
        }
    }
    None
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
        let image = match load_image_from_memory(&bytes) {
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
/// (C4ComponentHost.cpp:56-95, C4Components.h:67): the first nonempty loaded
/// candidate — "Title<code>.txt" per language code, then the plain
/// "Title.txt" — is loaded, and the language string is looked up in it.
fn title_from_title_files(
    group: &Group,
    components: &ComponentGroups,
    languages: &[String],
) -> Result<Option<String>, ScenarioDiscoveryError> {
    let candidates = languages
        .iter()
        .map(|code| format!("Title{code}.txt"))
        .chain(std::iter::once("Title.txt".to_string()));
    for candidate in candidates {
        let Some(component) = components
            .read(&candidate)
            .map_err(|err| group_error(&group.root().join(&candidate), err))?
        else {
            continue;
        };
        let text = decode_legacy_text(&component.bytes);
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
    languages
        .iter()
        .find_map(|code| component_language_string(text, code).map(str::to_string))
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

fn fallback_title_for_name_bytes(name: &[u8]) -> String {
    let stem = name
        .iter()
        .rposition(|byte| *byte == b'.')
        .filter(|index| *index > 0)
        .map_or(name, |index| &name[..index]);
    let stem = if stem.is_empty() { name } else { stem };
    decode_legacy_text(stem).replace('_', " ")
}

fn join_identifier_bytes(parent: &str, child: &[u8]) -> String {
    let child = clonk_script::c4_string_from_bytes(child);
    if parent.is_empty() {
        child
    } else {
        format!("{parent}/{child}")
    }
}

fn should_ignore_name_bytes(name: &[u8]) -> bool {
    name.starts_with(b".")
        || name.eq_ignore_ascii_case(b"CVS")
        || name.eq_ignore_ascii_case(b"Thumbs.db")
}

fn is_scenario_filename_bytes(name: &[u8]) -> bool {
    has_extension_bytes(name, b"c4s")
}

fn is_folder_filename_bytes(name: &[u8]) -> bool {
    has_extension_bytes(name, b"c4f")
}

fn has_extension_bytes(name: &[u8], extension: &[u8]) -> bool {
    name.iter()
        .rposition(|byte| *byte == b'.')
        .is_some_and(|dot| name[dot + 1..].eq_ignore_ascii_case(extension))
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

    compare_case_insensitive_bytes(
        &clonk_script::c4_string_bytes(&a.identifier),
        &clonk_script::c4_string_bytes(&b.identifier),
    )
}

fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
}

fn compare_case_insensitive_bytes(a: &[u8], b: &[u8]) -> Ordering {
    a.iter()
        .map(u8::to_ascii_lowercase)
        .cmp(b.iter().map(u8::to_ascii_lowercase))
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
    fn tempdir() -> std::io::Result<tempfile::TempDir> {
        tempfile::Builder::new().prefix("lc-test-").tempdir()
    }

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
    fn discovery_progress_percent_is_overflow_safe_and_bounded() {
        let complete = ScenarioDiscoveryProgress {
            current: usize::MAX,
            total: usize::MAX,
            percent_floor: 0,
        };
        assert_eq!(complete.percent(), 100);

        let over_complete = ScenarioDiscoveryProgress {
            current: usize::MAX,
            total: 1,
            percent_floor: 0,
        };
        assert_eq!(over_complete.percent(), 100);

        let empty = ScenarioDiscoveryProgress {
            current: 0,
            total: 0,
            percent_floor: 0,
        };
        assert_eq!(empty.percent(), 0);
    }

    #[test]
    fn recursive_discovery_reports_progress_and_can_cancel() {
        let dir = tempdir().unwrap();
        let root_scenario = dir.path().join("Alpha.c4s");
        fs::create_dir(&root_scenario).unwrap();
        fs::write(
            root_scenario.join("Scenario.txt"),
            "[Head]\nTitle=Root Mission\n",
        )
        .unwrap();
        let folder = dir.path().join("Missions.c4f");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("Folder.txt"), "[Head]\nTitle=Missions\n").unwrap();
        for name in ["Alpha.c4s", "Beta.c4s", "Gamma.c4s"] {
            let scenario = folder.join(name);
            fs::create_dir(&scenario).unwrap();
            fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=Mission\n").unwrap();
        }
        let languages = default_language_sequence();
        let packs = LanguagePacks::default();
        let mut progress_updates = Vec::new();

        let entries = discover_with_languages_and_packs_with_progress(
            dir.path(),
            &languages,
            &packs,
            |progress| {
                progress_updates.push(progress);
                ControlFlow::Continue(())
            },
        )
        .expect("discover with recursive progress");

        let percentages = progress_updates
            .iter()
            .map(|progress| progress.percent())
            .collect::<Vec<_>>();
        assert_eq!(entries.len(), 2);
        assert_eq!(percentages.first(), Some(&0));
        assert_eq!(percentages.last(), Some(&100));
        assert!(percentages.iter().all(|percent| *percent <= 100));
        assert!(percentages.iter().any(|percent| (1..100).contains(percent)));
        assert!(progress_updates
            .iter()
            .all(|progress| progress.current <= progress.total));
        assert!(
            percentages.windows(2).all(|pair| pair[0] <= pair[1]),
            "reported percentages must not move backwards as nested work expands the total: {percentages:?}"
        );
        assert!(
            progress_updates.iter().any(|progress| {
                let raw_percent = (progress.current as u128) * 100
                    / (progress.total.max(1) as u128);
                raw_percent < u128::from(progress.percent())
            }),
            "the fixture must exercise a nested total expansion behind the monotonic high-water mark"
        );

        let mut cancellation_percentages = Vec::new();
        let mut cancelled_during_nested_work = false;
        let error = discover_with_languages_and_packs_with_progress(
            dir.path(),
            &languages,
            &packs,
            |progress| {
                cancellation_percentages.push(progress.percent());
                if progress.total > 3 && progress.current >= 2 {
                    cancelled_during_nested_work = true;
                    ControlFlow::Break(())
                } else {
                    ControlFlow::Continue(())
                }
            },
        )
        .expect_err("cancel recursive discovery");
        assert!(matches!(error, ScenarioDiscoveryError::Cancelled));
        assert!(cancelled_during_nested_work);
        assert_ne!(cancellation_percentages.last(), Some(&100));
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
        fs::write(&scenario_path, gzip_group_image(&scenario_bytes)).unwrap();

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
        let mut folder_bytes = build_group(&[
            ("Folder.txt", b"Title=Campaign".to_vec()),
            ("Child.c4s", child_scenario),
        ]);
        // This entry is a packed child group, so its C4GroupEntryCore must
        // carry ChildGroup=1 (C4Group.cpp:1858-1862).
        mark_group_entry_child(&mut folder_bytes, 1);
        fs::write(&folder_path, gzip_group_image(&folder_bytes)).unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries.len(), 1);
        let folder_entry = &entries[0];
        assert_eq!(folder_entry.title, "Campaign");
        assert_eq!(folder_entry.kind, ScenarioEntryKind::Folder);
        assert_eq!(folder_entry.children.len(), 1);
        assert_eq!(folder_entry.children[0].title, "Packed Child");
    }

    #[cfg(unix)]
    #[test]
    fn scenario_discovery_preserves_native_byte_names_for_directory_and_packed_children() {
        use std::os::unix::ffi::OsStrExt as _;

        let dir = tempdir().unwrap();
        let sibling = dir.path().join("Sibling.c4s");
        fs::create_dir(&sibling).unwrap();
        fs::write(
            sibling.join("Scenario.json"),
            br#"{"name":"ASCII sibling"}"#,
        )
        .unwrap();

        // Packed C4Group names are legacy byte strings on every host. Include
        // an unrelated invalid-byte file and two scenario names that both
        // collapse to the same lossy Unicode path; each child has a distinct
        // payload so reopening either one through that lossy path cannot pass.
        let packed_u = build_group(&[("Scenario.txt", b"[Head]\nTitle=Packed U child\n".to_vec())]);
        let packed_o = build_group(&[("Scenario.txt", b"[Head]\nTitle=Packed O child\n".to_vec())]);
        let mut campaign = build_group(&[
            ("Folder.txt", b"Title=Campaign".to_vec()),
            ("ignored.bin", b"not a scenario".to_vec()),
            ("U.c4s", packed_u),
            ("O.c4s", packed_o),
        ]);
        set_group_entry_name(&mut campaign, 1, b"\xff.bin");
        set_group_entry_name(&mut campaign, 2, b"\xfc.c4s");
        set_group_entry_name(&mut campaign, 3, b"\xf6.c4s");
        mark_group_entry_child(&mut campaign, 2);
        mark_group_entry_child(&mut campaign, 3);
        fs::write(dir.path().join("Campaign.c4f"), gzip_group_image(&campaign)).unwrap();

        // Darwin filesystems reject these physical basenames, but the same
        // discovery path is exercised on Unix hosts that admit arbitrary
        // bytes. The packed assertions above remain active on macOS.
        #[cfg(not(target_os = "macos"))]
        {
            let native_folder_name = std::ffi::OsStr::from_bytes(b"\xe4.c4f");
            let native_folder = dir.path().join(native_folder_name);
            fs::create_dir(&native_folder).unwrap();
            let nested = native_folder.join("Nested.c4s");
            fs::create_dir(&nested).unwrap();
            fs::write(
                nested.join("Scenario.json"),
                br#"{"name":"Native folder child"}"#,
            )
            .unwrap();
            fs::write(
                dir.path().join(std::ffi::OsStr::from_bytes(b"0-\xff.bin")),
                b"unrelated",
            )
            .unwrap();
        }

        let entries = discover(dir.path()).expect("discover native-byte entries");
        assert!(entries.iter().any(|entry| entry.title == "ASCII sibling"));

        let campaign = entries
            .iter()
            .find(|entry| entry.title == "Campaign")
            .expect("packed campaign remains discoverable");
        assert_eq!(campaign.children.len(), 2);
        for (title, raw_name) in [
            ("Packed U child", b"\xfc.c4s".as_slice()),
            ("Packed O child", b"\xf6.c4s".as_slice()),
        ] {
            let child = campaign
                .children
                .iter()
                .find(|child| child.title == title)
                .expect("exact packed child payload is opened");
            assert_eq!(
                child.path.file_name().unwrap().as_bytes(),
                raw_name,
                "logical child path retains the exact packed entry bytes"
            );
            let mut expected_identifier = b"Campaign.c4f/".to_vec();
            expected_identifier.extend_from_slice(raw_name);
            assert_eq!(
                clonk_script::c4_string_bytes(&child.identifier),
                expected_identifier,
                "menu identity retains the exact packed entry bytes"
            );
        }

        #[cfg(not(target_os = "macos"))]
        {
            let folder = entries
                .iter()
                .find(|entry| entry.title == "ä")
                .expect("native-byte directory folder is discovered");
            assert_eq!(folder.path.file_name().unwrap().as_bytes(), b"\xe4.c4f");
            assert_eq!(
                clonk_script::c4_string_bytes(&folder.identifier),
                b"\xe4.c4f"
            );
            assert_eq!(folder.children.len(), 1);
            assert_eq!(folder.children[0].title, "Native folder child");
        }
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
        fs::write(
            scenario_dir.join("Title.txt"),
            "DE:Lernrunden\r\nUS:Tutorial",
        )
        .unwrap();

        let us = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(us[0].title, "Tutorial");

        let de = discover_with_languages(dir.path(), &langs(&["DE", "US"])).expect("discover");
        assert_eq!(de[0].title, "Lernrunden");

        // Unknown language falls back through the sequence to the next code.
        let fr = discover_with_languages(dir.path(), &langs(&["FR", "DE"])).expect("discover");
        assert_eq!(fr[0].title, "Lernrunden");
    }

    #[test]
    fn language_string_line_end_prefers_any_cr_before_lf_fallback() {
        let languages = langs(&["US", "DE"]);

        assert_eq!(
            (
                resolve_language_string("US:Cabin\nDE:Huette\r\n", &languages),
                resolve_language_string("US:Cabin\nDE:Huette\n", &languages),
            ),
            (
                Some("Cabin\nDE:Huette".to_string()),
                Some("Cabin".to_string()),
            )
        );
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

    #[test]
    fn empty_language_specific_title_falls_through_to_plain_title() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(scenario_dir.join("Scenario.txt"), "[Head]\nTitle=Core\n").unwrap();
        fs::write(scenario_dir.join("TitleUS.txt"), []).unwrap();
        fs::write(scenario_dir.join("Title.txt"), "US:From Title").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US"])).expect("discover");
        assert_eq!(entries[0].title, "From Title");
    }

    #[test]
    fn unreadable_title_candidate_falls_through_without_dropping_siblings() {
        let dir = tempdir().unwrap();

        let broken_path = dir.path().join("Broken.c4s");
        let mut broken = build_group(&[
            ("Scenario.txt", b"[Head]\nTitle=Core fallback\n".to_vec()),
            ("TitleUS.txt", b"unreadable".to_vec()),
        ]);
        make_group_entry_unreadable(&mut broken, 1);
        fs::write(&broken_path, gzip_group_image(&broken)).unwrap();

        let plain_path = dir.path().join("Plain.c4s");
        let mut plain = build_group(&[
            ("Scenario.txt", b"[Head]\nTitle=Wrong core title\n".to_vec()),
            ("Title.txt", b"US:Plain title\n".to_vec()),
            ("TitleUS.txt", b"unreadable".to_vec()),
        ]);
        make_group_entry_unreadable(&mut plain, 2);
        fs::write(&plain_path, gzip_group_image(&plain)).unwrap();

        let sibling_path = dir.path().join("Sibling.c4s");
        fs::create_dir(&sibling_path).unwrap();
        fs::write(sibling_path.join("Scenario.txt"), "[Head]\nTitle=Sibling\n").unwrap();

        for path in [&broken_path, &plain_path] {
            let group = Group::open(path).expect("open corrupt-entry fixture");
            assert!(group.exists("TitleUS.txt"));
            assert!(matches!(
                group.read_file("TitleUS.txt"),
                Err(GroupError::InvalidGroup(_))
            ));
        }

        let entries = discover_with_languages(dir.path(), &langs(&["US"]))
            .expect("one unreadable title component must not abort its root");
        assert_eq!(entries.len(), 3);
        let title = |identifier: &str| {
            entries
                .iter()
                .find(|entry| entry.identifier == identifier)
                .map(|entry| entry.title.as_str())
        };
        assert_eq!(title("Broken.c4s"), Some("Core fallback"));
        assert_eq!(title("Plain.c4s"), Some("Plain title"));
        assert_eq!(title("Sibling.c4s"), Some("Sibling"));
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
        fs::write(
            scenario_dir.join("Title.txt"),
            "DE:Goldmine\nUS:Gold Mine\n",
        )
        .unwrap();

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
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=CoreTitle\n",
        )
        .unwrap();
        fs::write(scenario_dir.join("Title.txt"), "DE:Nur Deutsch\n").unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US"])).expect("discover");
        assert_eq!(entries[0].title, "CoreTitle");
    }

    fn langs(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| code.to_string()).collect()
    }

    #[test]
    fn language_pack_title_is_used_but_same_candidate_local_title_wins() {
        let temp = tempdir().unwrap();
        let install = temp.path().join("install");
        let scenarios = install.join("Scenarios");
        let scenario = scenarios.join("Alpha.c4s");
        fs::create_dir_all(&scenario).unwrap();
        fs::write(scenario.join("Scenario.txt"), "[Head]\nTitle=Core title\n").unwrap();

        let language_container = install.join("Language.c4g");
        let packed_scenario = language_container.join("Finnish.c4g/Scenarios/Alpha.c4s");
        fs::create_dir_all(&packed_scenario).unwrap();
        fs::write(packed_scenario.join("TitleFI.txt"), "FI:Pack title\n").unwrap();
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            std::slice::from_ref(&install),
        );

        let packed = discover_with_languages_and_packs(&scenarios, &langs(&["FI", "US"]), &packs)
            .expect("discover pack-localized scenario");
        assert_eq!(packed[0].title, "Pack title");

        fs::write(scenario.join("TitleFI.txt"), "FI:Local title\n").unwrap();
        let local = discover_with_languages_and_packs(&scenarios, &langs(&["FI", "US"]), &packs)
            .expect("discover locally localized scenario");
        assert_eq!(local[0].title, "Local title");
    }

    #[test]
    fn startup_catalog_ignores_the_entrys_own_origin_for_pack_lookup() {
        let temp = tempdir().unwrap();
        let install = temp.path().join("install");
        let scenarios = install.join("Scenarios");
        let scenario = scenarios.join("Actual.c4s");
        fs::create_dir_all(&scenario).unwrap();
        fs::write(
            scenario.join("Scenario.txt"),
            "[Head]\nTitle=Core title\nOrigin=Scenarios\\Original.c4s\n",
        )
        .unwrap();

        let language_container = install.join("Language.c4g");
        let actual_scenario = language_container.join("Finnish.c4g/Scenarios/Actual.c4s");
        let origin_scenario = language_container.join("Finnish.c4g/Scenarios/Original.c4s");
        fs::create_dir_all(&actual_scenario).unwrap();
        fs::create_dir_all(&origin_scenario).unwrap();
        fs::write(actual_scenario.join("TitleFI.txt"), "FI:Actual title\n").unwrap();
        fs::write(
            origin_scenario.join("TitleFI.txt"),
            "FI:Wrong Origin title\n",
        )
        .unwrap();
        let packs = LanguagePacks::discover(
            std::slice::from_ref(&language_container),
            std::slice::from_ref(&install),
        );

        let entries = discover_with_languages_and_packs(&scenarios, &langs(&["FI", "US"]), &packs)
            .expect("discover scenario without applying its private Origin");
        assert_eq!(entries[0].title, "Actual title");
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
        fs::write(scenario_dir.join("DescFR.rtf"), []).unwrap();

        let us = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(us[0].description.as_deref(), Some("English\n"));

        let de = discover_with_languages(dir.path(), &langs(&["DE", "US"])).expect("discover");
        assert_eq!(de[0].description.as_deref(), Some("Deutsch\n"));

        // A zero-byte component fails LoadEntryString and falls through to
        // the next language just like a missing component.
        let fr = discover_with_languages(dir.path(), &langs(&["FR", "DE"])).expect("discover");
        assert_eq!(fr[0].description.as_deref(), Some("Deutsch\n"));
    }

    #[test]
    fn scenario_description_ignores_bytes_after_native_nul() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("NativeNul.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Native NUL\n",
        )
        .unwrap();
        fs::write(
            scenario_dir.join("DescUS.rtf"),
            b"{\\rtf1 Visible description.\\par}\0}",
        )
        .unwrap();

        let entries = discover_with_languages(dir.path(), &langs(&["US"])).expect("discover");
        assert_eq!(
            entries[0].description.as_deref(),
            Some("Visible description.\n")
        );

        fs::write(scenario_dir.join("DescUS.rtf"), b"\0ignored suffix").unwrap();
        fs::write(
            scenario_dir.join("DescDE.rtf"),
            br"{\rtf1 Later language must not win.\par}",
        )
        .unwrap();
        let entries = discover_with_languages(dir.path(), &langs(&["US", "DE"])).expect("discover");
        assert_eq!(entries[0].description, None);
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
    fn discovers_native_byte_mission_access_without_changing_playability() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Locked.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            b"[Head]\nTitle=Locked\nMissionAccess=Secr\x80t\n",
        )
        .unwrap();

        let entries = discover(dir.path()).expect("discover locked scenario");
        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_playable, "access does not change row actions");
        assert_eq!(
            clonk_script::c4_string_bytes(
                entries[0]
                    .mission_access
                    .as_deref()
                    .expect("mission access metadata"),
            ),
            b"Secr\x80t"
        );
    }

    #[test]
    fn discovery_ignores_mission_access_after_the_native_nul() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Nul.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            b"[Head]\nTitle=Visible\0\nMissionAccess=Invisible\n",
        )
        .unwrap();

        let entries = discover(dir.path()).expect("discover NUL-terminated scenario");
        assert_eq!(entries[0].mission_access, None);
    }

    #[test]
    fn mission_access_uses_the_first_head_key_even_when_empty() {
        let empty_first =
            parse_legacy_mission_access("[Head]\nMissionAccess=\nMissionAccess=MustNotWin\n");
        assert_eq!(empty_first, None);

        let first =
            parse_legacy_mission_access("[Head]\nMissionAccess=Secret\nMissionAccess=Ignored\n");
        assert_eq!(first.as_deref(), Some("Secret"));
    }

    #[test]
    fn mission_access_uses_exact_names_and_preserves_rct_all_bytes() {
        assert_eq!(
            parse_legacy_mission_access("MissionAccess=Root\n[head]\nMissionAccess=WrongSection\n",),
            None
        );
        assert_eq!(
            parse_legacy_mission_access("\u{feff}[Head]\nMissionAccess=BomMustNotOpenHead\n",),
            None
        );
        assert_eq!(
            parse_legacy_mission_access(
                "[Head]\nmissionaccess=WrongKey\nMissionAccess=Secret//part  \n",
            )
            .as_deref(),
            Some("Secret//part  ")
        );
        assert_eq!(
            parse_legacy_mission_access("    [Head]\nMissionAccess=NestedRootKey\n"),
            None,
            "a dedented key is not a child of an indented Head"
        );
        assert_eq!(
            parse_legacy_mission_access("[Head]\n [Nested]\nMissionAccess=BackInHead\n",)
                .as_deref(),
            Some("BackInHead"),
            "dedenting from a nested section restores Head ownership"
        );

        let oversized = format!("[Head]\nMissionAccess={}\n", "A".repeat(520));
        assert_eq!(
            clonk_script::c4_string_bytes(
                &parse_legacy_mission_access(&oversized).expect("capped access")
            )
            .len(),
            512
        );
    }

    #[test]
    fn reads_definitions_flags_and_version() {
        let dir = tempdir().unwrap();
        let scenario_dir = dir.path().join("Alpha.c4s");
        fs::create_dir(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.txt"),
            "[Head]\nTitle=Alpha\n\n[Definitions]\nLocalOnly=1\nAllowUserChange=1\nDefinition2=Ignored.c4d\nDefinitions=\"Objects.c4d\",\"Knights.c4d\"\nDefinition1=AlsoIgnored.c4d\n",
        )
        .unwrap();
        fs::write(scenario_dir.join("Version.txt"), "4.9.8.2\n").unwrap();

        let entries = discover(dir.path()).expect("discover");
        assert_eq!(entries[0].local_only, Some(true));
        assert_eq!(entries[0].allow_user_change, Some(true));
        assert_eq!(
            entries[0].definition_modules,
            ["Objects.c4d", "Knights.c4d"]
        );
        assert_eq!(entries[0].version.as_deref(), Some("4.9.8.2"));
    }

    #[test]
    fn c4s_string_list_decodes_cpp_quoted_escapes_and_keeps_duplicates() {
        let definitions = parse_c4s_string_list(
            r#""Dir\\Pack.c4d","Quote\"Pack.c4d","\x41\101\n\q","Dir\\Pack.c4d""#,
        );
        assert_eq!(
            definitions,
            ["Dir\\Pack.c4d", "Quote\"Pack.c4d", "AA\nq", "Dir\\Pack.c4d",]
        );
    }

    #[test]
    fn c4s_string_list_keeps_an_unquoted_comma_in_one_rct_all_value() {
        assert_eq!(
            parse_c4s_string_list("Objects.c4d, Knights.c4d"),
            ["Objects.c4d, Knights.c4d"]
        );
    }

    #[test]
    fn definitions_whitespace_after_equals_selects_rct_all_before_trim() {
        let info =
            parse_legacy_core_info("[Definitions]\nDefinitions= \"First.c4d\",\"Second.c4d\"\n");
        assert_eq!(info.definition_modules, [r#""First.c4d","Second.c4d""#]);
    }

    #[test]
    fn first_modern_definitions_key_wins_and_preserves_duplicates() {
        let info = parse_legacy_core_info(
            "[Definitions]\nDefinitions=\"First.c4d\",\"First.c4d\"\nDefinitions=\"Ignored.c4d\"\nDefinition1=AlsoIgnored.c4d\n",
        );
        assert_eq!(info.definition_modules, ["First.c4d", "First.c4d"]);
    }

    #[test]
    fn numbered_definition_fallback_uses_cpp_slot_order_and_limit() {
        let info = parse_legacy_core_info(
            "[Definitions]\nDefinition10=Ten.c4d\nDefinition2=Two;Literal.c4d\nDefinition11=Ignored.c4d\nDefinition01=AlsoIgnored.c4d\nDefinition1=One.c4d\n",
        );
        assert_eq!(
            info.definition_modules,
            ["One.c4d", "Two;Literal.c4d", "Ten.c4d"]
        );
    }

    #[test]
    fn present_bare_modern_definitions_suppresses_numbered_fallback() {
        let info = parse_legacy_core_info(
            "[Definitions]\nDefinitions=\nDefinition1=MustNotFallback.c4d\n",
        );
        assert_eq!(info.definition_modules, [""]);

        let whitespace = parse_legacy_core_info(
            "[Definitions]\nDefinitions=   \nDefinition1=MustNotFallback.c4d\n",
        );
        assert_eq!(whitespace.definition_modules, [""]);
    }

    #[test]
    fn definitions_use_first_exact_section_and_exact_key_names() {
        let info = parse_legacy_core_info(
            "[definitions]\nLocalOnly=1\nAllowUserChange=1\nDefinitions=WrongCaseSection.c4d\n\
             [Definitions]\nlocalonly=1\nallowuserchange=1\ndefinitions=WrongCaseKey.c4d\nDefinition01=Alias.c4d\nDefinition2=Exact.c4d\n\
             [Definitions]\nLocalOnly=1\nAllowUserChange=1\nDefinitions=Repeated.c4d\nDefinition1=RepeatedFallback.c4d\n",
        );
        assert_eq!(info.local_only, None);
        assert_eq!(info.allow_user_change, None);
        assert_eq!(info.definition_modules, ["Exact.c4d"]);
    }

    #[test]
    fn first_exact_scalar_key_is_consumed_even_when_its_value_is_invalid() {
        let info = parse_legacy_core_info(
            "[Definitions]\nLocalOnly=invalid\nLocalOnly=1\nAllowUserChange=\nAllowUserChange=1\n",
        );
        assert_eq!(info.local_only, None);
        assert_eq!(info.allow_user_change, None);
    }

    #[test]
    fn definitions_names_keep_spaces_but_skip_tabs_like_stdcompiler() {
        let info = parse_legacy_core_info(
            "[Head]\nTitle=Before invalid header\n\
             [ Definitions ]\nLocalOnly=1\n\
             [Definitions ]\nAllowUserChange=1\n\
             [Definitions\t ]\nLocalOnly =1\nAllowUserChange\t =1\nDefinitions =Wrong.c4d\nDefinition1 =Wrong.c4d\nDefinition2\t =Exact.c4d\n\
             [Definitions]\nLocalOnly=1\nDefinitions=Repeated.c4d\n",
        );
        assert_eq!(info.local_only, None);
        assert_eq!(info.allow_user_change, Some(true));
        assert_eq!(info.definition_modules, ["Exact.c4d"]);
    }

    fn encode_test_png() -> Vec<u8> {
        let image = image::RgbaImage::from_raw(2, 2, vec![255u8; 16]).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
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
        fs::write(&packed_path, gzip_group_image(&packed_bytes)).unwrap();

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

    fn gzip_group_image(image: &[u8]) -> Vec<u8> {
        let mut compressed = Vec::new();
        let mut encoder =
            flate2::write::GzEncoder::new(&mut compressed, flate2::Compression::default());
        encoder.write_all(image).unwrap();
        encoder.finish().unwrap();
        compressed
    }

    fn mark_group_entry_child(group: &mut [u8], entry_index: usize) {
        let child_flag = GROUP_HEADER_SIZE + entry_index * GROUP_ENTRY_SIZE + 260 + 4;
        group[child_flag..child_flag + 4].copy_from_slice(&1_i32.to_le_bytes());
    }

    fn set_group_entry_name(group: &mut [u8], entry_index: usize, name: &[u8]) {
        assert!(name.len() < 260);
        assert!(!name.contains(&0));
        let start = GROUP_HEADER_SIZE + entry_index * GROUP_ENTRY_SIZE;
        group[start..start + 260].fill(0);
        group[start..start + name.len()].copy_from_slice(name);
    }

    fn make_group_entry_unreadable(group: &mut [u8], entry_index: usize) {
        let size = GROUP_HEADER_SIZE + entry_index * GROUP_ENTRY_SIZE + 260 + 4 + 4;
        group[size..size + 4].copy_from_slice(&i32::MAX.to_le_bytes());
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
