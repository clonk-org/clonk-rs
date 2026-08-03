use std::fs::{self, OpenOptions};
use std::io::{self, Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clonk_core::std_config::Config;
use clonk_engine::{
    player_file::{CrewInfo, PlayerFile},
    scenario::ScenarioError,
};
use clonk_frontend::{
    startup_plrsel::{PlrSelCrew, PlrSelCrewPromotion, PlrSelPlayer},
    ImageData,
};
use clonk_platform::AppPaths;
use clonk_resources::{
    Group, GroupEntry, GroupError, MutableGroup, MutableGroupChildMut, MutableGroupError,
};
use png::{BitDepth, ColorType, Transformations};

/// One requested update to a player-group picture entry.
#[derive(Clone, Debug, PartialEq)]
pub enum PlayerImageWrite {
    /// Preserve the existing entry. For a new group this writes nothing.
    Keep,
    /// Replace the entry with this RGBA image.
    Replace(ImageData),
    /// Remove the entry if it exists.
    Clear,
}

/// Result of a successful player-properties save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedStartupPlayer {
    pub path: PathBuf,
    pub file_name: String,
}

/// One checked player that the native fixed-size Participants buffer refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayerActivationRefusal {
    pub index: usize,
    pub player_name: String,
}

/// Typed failures from player name validation and `.c4p` persistence.
///
/// Every storage variant names its failing step and path so the classic
/// screen-owned error modal can compose per-branch text the way
/// `C4StartupPlrPropertiesDlg::OnClosed` does (`IDS_FAIL_RENAME`,
/// `IDS_FAIL_MODIFY`, and the step-prefixed `C4Group` error strings).
#[derive(Debug, thiserror::Error)]
pub enum PlayerPropertiesSaveError {
    #[error("You must specify a player name!")]
    EmptyName,
    #[error("{name} is already taken")]
    NameTaken { name: String, path: PathBuf },
    /// Moving the player group onto its new filename failed
    /// (C++ `PlayerListItem::MoveFilename` -> `IDS_FAIL_RENAME`).
    #[error("rename \"{}\" to \"{}\": {detail}", from.display(), to.display())]
    Rename {
        from: PathBuf,
        to: PathBuf,
        detail: String,
    },
    /// Opening the player group, scanning its directory, or reading the
    /// configuration failed (C++ `C4Group::Open` -> "Open:" errors).
    #[error("open \"{}\": {detail}", path.display())]
    Open { path: PathBuf, detail: String },
    /// Rewriting the info core inside the group failed
    /// (C++ `PlayerListItem::UpdateCore` -> `IDS_FAIL_MODIFY`).
    #[error("write core \"{}/{entry}\": {detail}", path.display())]
    WriteCore {
        path: PathBuf,
        entry: &'static str,
        detail: String,
    },
    /// Encoding or storing a portrait/big-icon entry failed
    /// (C++ `SavePNG` into the group -> group error).
    #[error("write image \"{}/{entry}\": {detail}", path.display())]
    WriteImage {
        path: PathBuf,
        entry: &'static str,
        detail: String,
    },
    /// Flushing the rewritten group back to disk failed
    /// (C++ `C4Group::Close` -> "Close:" errors).
    #[error("close \"{}\": {detail}", path.display())]
    Close { path: PathBuf, detail: String },
}

/// Sanitizes the player core name into the filename used by
/// `C4StartupPlrPropertiesDlg::CheckPlayerName`.
pub fn player_group_filename(name: &str) -> Result<String, PlayerPropertiesSaveError> {
    if name.is_empty() {
        return Err(PlayerPropertiesSaveError::EmptyName);
    }
    // `ClonkToSystem` converts the native C4 byte string before applying
    // filesystem sanitization. Preserve ordinary UTF-8 while projecting
    // raw legacy bytes through the same Windows-1252 fallback used by loads.
    let system_name =
        clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(name));
    let mut filename = system_name
        .chars()
        .map(|character| {
            if matches!(
                character,
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            ) {
                '_'
            } else {
                character
            }
        })
        .collect::<String>();
    if filename.starts_with('.') {
        filename.replace_range(..1, "_");
    }
    filename.push_str(".c4p");
    Ok(filename)
}

/// One player file shown by the startup player-selection dialog.
pub struct StartupPlayerFile {
    /// Resolved on-disk `.c4p` group.
    pub path: PathBuf,
    /// C++ `Config.AtExeRelativePath`-shaped participant reference.
    pub file_name: String,
    /// Simulation-facing player data.
    pub player_file: PlayerFile,
    /// Presentation-facing player data.
    pub render_model: PlrSelPlayer,
}

impl StartupPlayerFile {
    pub fn set_activated(&mut self, activated: bool) {
        self.render_model.activated = activated;
    }
}

/// One direct `*.c4i` child shown in startup crew mode.
pub struct StartupCrewFile {
    /// Physical player group containing this entry.
    pub player_path: PathBuf,
    /// Child basename, without the player path.
    pub file_name: String,
    /// Simulation-facing `C4ObjectInfoCore` data.
    pub crew_info: CrewInfo,
    /// Presentation-facing crew data.
    pub render_model: PlrSelCrew,
}

/// Discovers the direct `*.c4i` children of one startup player and applies
/// the visible C++ crew ordering: descending maximum experience for the
/// definition type, then descending individual experience within that type.
pub fn discover_crew_files(player: &StartupPlayerFile) -> io::Result<Vec<StartupCrewFile>> {
    let group = Group::open(&player.path).map_err(group_error_to_io)?;
    let mut crew = Vec::new();
    for entry in group.entries().map_err(group_error_to_io)? {
        if !has_crew_extension(&entry.name_bytes) {
            continue;
        }
        let Ok(child) = open_direct_child(&group, &entry) else {
            continue;
        };
        let Ok(source) = child.load_entry_string("ObjectInfo.txt") else {
            continue;
        };
        if !has_object_info_section(&source) {
            continue;
        }
        let Ok(crew_info) = CrewInfo::load(&child) else {
            continue;
        };
        let rank_icon = load_group_png(&child, "Rank.png");
        let portrait = load_group_png(&child, "Portrait.png");
        let render_model = crew_render_model(
            &crew_info,
            player.player_file.normalized_preferred_color(),
            rank_icon,
            portrait,
        );
        crew.push(StartupCrewFile {
            player_path: player.path.clone(),
            file_name: clonk_script::c4_string_from_bytes(&entry.name_bytes),
            crew_info,
            render_model,
        });
    }

    let mut type_maximum = std::collections::HashMap::<String, i32>::new();
    for entry in &crew {
        type_maximum
            .entry(entry.crew_info.id.clone())
            .and_modify(|maximum| *maximum = (*maximum).max(entry.crew_info.experience))
            .or_insert(entry.crew_info.experience);
    }
    crew.sort_by(|left, right| {
        type_maximum[&right.crew_info.id]
            .cmp(&type_maximum[&left.crew_info.id])
            .then_with(|| right.crew_info.experience.cmp(&left.crew_info.experience))
    });
    Ok(crew)
}

fn crew_render_model(
    crew: &CrewInfo,
    color_dw: u32,
    rank_icon: Option<ImageData>,
    portrait: Option<ImageData>,
) -> PlrSelCrew {
    let display = |value: &str| {
        clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(value))
    };
    let next_rank =
        (crew.core.next_rank_exp > 0 && !crew.core.next_rank_name.is_empty()).then(|| {
            PlrSelCrewPromotion {
                rank_name: display(&crew.core.next_rank_name),
                experience: crew.core.next_rank_exp,
            }
        });
    PlrSelCrew {
        name: display(&crew.name),
        participating: crew.participation != 0,
        rank_icon,
        portrait,
        color_dw,
        rank: crew.rank,
        rank_name: display(&crew.rank_name),
        type_name: display(&crew.core.type_name),
        experience: crew.experience,
        rounds: crew.rounds,
        death_count: crew.death_count,
        total_playing_time: crew.total_playing_time,
        birthday: String::new(),
        next_rank,
        physical: crew.physical,
    }
}

fn open_direct_child(parent: &Group, entry: &GroupEntry) -> Result<Group, GroupError> {
    if parent.is_directory() {
        parent.open_child(&entry.relative_path)
    } else {
        let bytes = parent.read_entry_bytes_exact(entry)?;
        Group::from_raw_memory(PathBuf::from(&entry.relative_path), bytes)
    }
}

fn has_crew_extension(name: &[u8]) -> bool {
    name.get(name.len().saturating_sub(4)..)
        .is_some_and(|extension| extension.eq_ignore_ascii_case(b".c4i"))
}

fn has_object_info_section(source: &[u8]) -> bool {
    source
        .split(|byte| matches!(*byte, b'\r' | b'\n'))
        .map(trim_c4_whitespace)
        .any(|line| line == b"[ObjectInfo]")
}

fn group_error_to_io(error: GroupError) -> io::Error {
    match error {
        GroupError::Io(error) => error,
        error => io::Error::new(io::ErrorKind::InvalidData, error),
    }
}

/// Failure while changing one startup crew child.
#[derive(Debug, thiserror::Error)]
pub enum StartupCrewMutationError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Group(#[from] GroupError),
    #[error(transparent)]
    MutableGroup(#[from] MutableGroupError),
    #[error(transparent)]
    CrewInfo(#[from] ScenarioError),
    #[error("crew entry not found: {file_name}")]
    EntryNotFound { file_name: String },
    #[error("crew entry has no valid ObjectInfo core: {file_name}")]
    InvalidCrewCore { file_name: String },
    #[error("crew name must not be empty")]
    EmptyName,
    #[error("crew filename is too long: {file_name}")]
    FilenameTooLong { file_name: String },
    #[error("crew filename already exists: {file_name}")]
    NameCollision { file_name: String },
    #[error("crew rename to {file_name} was accepted but its core rewrite failed: {source}")]
    RenameAcceptedCoreRewriteFailed {
        file_name: String,
        #[source]
        source: Box<StartupCrewMutationError>,
    },
}

/// Immediately rewrites `C4ObjectInfoCore::Participation` for one direct
/// crew child. The default value (`true`) is omitted like the native compiler.
pub fn set_crew_participation(
    player_path: &Path,
    child_name: &str,
    participating: bool,
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    rewrite_crew_value(
        player_path,
        child_name,
        b"Participation",
        (!participating).then_some(b"0".as_slice()),
        group_maker,
    )
}

/// Rewrites the crew death message, capped to the native 75-byte C string.
pub fn set_crew_death_message(
    player_path: &Path,
    child_name: &str,
    message: &str,
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    let mut message = c4_input_bytes(message);
    message.truncate(75);
    rewrite_crew_value(
        player_path,
        child_name,
        b"DeathMessage",
        (!message.is_empty()).then_some(message.as_slice()),
        group_maker,
    )
}

/// Renames a crew child from its display name and rewrites `Core.Name`.
/// Returns the new child basename (`*.c4i`).
pub fn rename_crew(
    player_path: &Path,
    child_name: &str,
    new_display_name: &str,
    group_maker: &[u8],
) -> Result<String, StartupCrewMutationError> {
    let mut requested_name = c4_input_bytes(new_display_name);
    if requested_name.is_empty() {
        return Err(StartupCrewMutationError::EmptyName);
    }

    let player_group = Group::open(player_path)?;
    let entry = find_crew_entry(&player_group, child_name)?;
    let actual_name = entry_name(&entry);
    let child = open_direct_child(&player_group, &entry)?;
    let crew_info = CrewInfo::load(&child)?;
    if requested_name == clonk_script::c4_string_bytes(&crew_info.name) {
        return Ok(actual_name);
    }

    let new_name_bytes = crew_filename_from_title(&requested_name);
    let new_file_name = clonk_script::c4_string_from_bytes(&new_name_bytes);
    let max_name = if cfg!(windows) { 256 } else { 255 };
    if new_name_bytes.len() > max_name {
        return Err(StartupCrewMutationError::FilenameTooLong {
            file_name: new_file_name,
        });
    }
    let rename_due = !item_identical(&entry.name_bytes, &new_name_bytes);
    if rename_due
        && player_group
            .entries()?
            .iter()
            .any(|candidate| candidate.name_bytes.eq_ignore_ascii_case(&new_name_bytes))
    {
        return Err(StartupCrewMutationError::NameCollision {
            file_name: new_file_name,
        });
    }

    requested_name.truncate(30);
    let source = child.load_entry_string("ObjectInfo.txt")?;
    let rewritten =
        rewrite_object_info_value(&source, b"Name", Some(&requested_name)).ok_or_else(|| {
            StartupCrewMutationError::InvalidCrewCore {
                file_name: actual_name.clone(),
            }
        })?;

    if player_group.is_directory() {
        let persisted_name = if rename_due {
            let target = player_path.join(path_from_name_bytes(&new_name_bytes));
            fs::rename(player_path.join(&entry.relative_path), &target)?;
            new_file_name.clone()
        } else {
            actual_name
        };
        let rewrite_result = (|| {
            let reopened = Group::open(player_path)?;
            let renamed_entry = find_crew_entry(&reopened, &persisted_name)?;
            let renamed_child = open_direct_child(&reopened, &renamed_entry)?;
            persist_object_info_to_standalone_child(&renamed_child, &rewritten, group_maker)
        })();
        if let Err(source) = rewrite_result {
            return Err(StartupCrewMutationError::RenameAcceptedCoreRewriteFailed {
                file_name: persisted_name,
                source: Box::new(source),
            });
        }
    } else {
        let actual_utf8 = std::str::from_utf8(&entry.name_bytes).map_err(|_| {
            StartupCrewMutationError::InvalidCrewCore {
                file_name: actual_name.clone(),
            }
        })?;
        let new_utf8 = std::str::from_utf8(&new_name_bytes).map_err(|_| {
            StartupCrewMutationError::InvalidCrewCore {
                file_name: new_file_name.clone(),
            }
        })?;
        let mut mutable = MutableGroup::from_group(&player_group)?;
        if rename_due && !mutable.rename_entry(actual_utf8, new_utf8) {
            return Err(StartupCrewMutationError::EntryNotFound {
                file_name: actual_name,
            });
        }
        let lookup = if rename_due { new_utf8 } else { actual_utf8 };
        replace_mutable_child_core(&mut mutable, lookup, rewritten, group_maker)?;
        persist_packed_group(player_path, &mutable)?;
    }
    Ok(new_file_name)
}

/// Permanently removes one direct crew child from a directory or packed
/// player group.
pub fn delete_crew_file(
    player_path: &Path,
    child_name: &str,
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    let player_group = Group::open(player_path)?;
    let entry = find_crew_entry(&player_group, child_name)?;
    if player_group.is_directory() {
        let path = player_path.join(entry.relative_path);
        let file_type = fs::symlink_metadata(&path)?.file_type();
        if file_type.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        return Ok(());
    }

    let actual_name = std::str::from_utf8(&entry.name_bytes).map_err(|_| {
        StartupCrewMutationError::InvalidCrewCore {
            file_name: entry_name(&entry),
        }
    })?;
    let mut mutable = MutableGroup::from_group(&player_group)?;
    if !mutable.remove_entry(actual_name) {
        return Err(StartupCrewMutationError::EntryNotFound {
            file_name: child_name.to_string(),
        });
    }
    stamp_nonempty_group_maker(&mut mutable, group_maker);
    persist_packed_group(player_path, &mutable)
}

fn rewrite_crew_value(
    player_path: &Path,
    child_name: &str,
    key: &[u8],
    value: Option<&[u8]>,
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    let player_group = Group::open(player_path)?;
    let entry = find_crew_entry(&player_group, child_name)?;
    let child = open_direct_child(&player_group, &entry)?;
    let source = child.load_entry_string("ObjectInfo.txt")?;
    let rewritten = rewrite_object_info_value(&source, key, value).ok_or_else(|| {
        StartupCrewMutationError::InvalidCrewCore {
            file_name: entry_name(&entry),
        }
    })?;
    if player_group.is_directory() {
        persist_object_info_to_standalone_child(&child, &rewritten, group_maker)
    } else {
        let actual_name = std::str::from_utf8(&entry.name_bytes).map_err(|_| {
            StartupCrewMutationError::InvalidCrewCore {
                file_name: entry_name(&entry),
            }
        })?;
        let mut mutable = MutableGroup::from_group(&player_group)?;
        replace_mutable_child_core(&mut mutable, actual_name, rewritten, group_maker)?;
        persist_packed_group(player_path, &mutable)
    }
}

fn replace_mutable_child_core(
    parent: &mut MutableGroup,
    child_name: &str,
    source: Vec<u8>,
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    match parent.child_mut(child_name)? {
        MutableGroupChildMut::Child(child) => {
            child.add_file("ObjectInfo.txt", source)?;
            stamp_nonempty_group_maker(child, group_maker);
        }
        MutableGroupChildMut::Missing | MutableGroupChildMut::File => {
            return Err(StartupCrewMutationError::EntryNotFound {
                file_name: child_name.to_string(),
            });
        }
    }
    Ok(())
}

fn persist_object_info_to_standalone_child(
    child: &Group,
    source: &[u8],
    group_maker: &[u8],
) -> Result<(), StartupCrewMutationError> {
    if child.is_directory() {
        let entry = child
            .entries()?
            .into_iter()
            .find(|entry| entry.name_bytes.eq_ignore_ascii_case(b"ObjectInfo.txt"))
            .ok_or_else(|| StartupCrewMutationError::InvalidCrewCore {
                file_name: child.root().display().to_string(),
            })?;
        return replace_existing_file(&child.root().join(entry.relative_path), source);
    }
    let mut mutable = MutableGroup::from_group(child)?;
    mutable.add_file("ObjectInfo.txt", source.to_vec())?;
    stamp_nonempty_group_maker(&mut mutable, group_maker);
    persist_packed_group(child.root(), &mutable)
}

fn stamp_nonempty_group_maker(group: &mut MutableGroup, group_maker: &[u8]) {
    // C4Group::Close only copies the process maker when its first byte is
    // nonzero. An empty maker therefore retains an imported header (or a new
    // group's native "New C4Group" default) instead of clearing it.
    if group_maker.first().is_some_and(|byte| *byte != 0) {
        group.set_maker_bytes(group_maker);
    }
}

fn persist_packed_group(path: &Path, group: &MutableGroup) -> Result<(), StartupCrewMutationError> {
    let packed = group.pack()?;
    replace_existing_file(path, &packed)
}

fn find_crew_entry(
    player: &Group,
    child_name: &str,
) -> Result<GroupEntry, StartupCrewMutationError> {
    let requested = c4_input_bytes(child_name);
    player
        .entries()?
        .into_iter()
        .find(|entry| {
            has_crew_extension(&entry.name_bytes)
                && entry.name_bytes.eq_ignore_ascii_case(&requested)
        })
        .ok_or_else(|| StartupCrewMutationError::EntryNotFound {
            file_name: child_name.to_string(),
        })
}

fn entry_name(entry: &GroupEntry) -> String {
    clonk_script::c4_string_from_bytes(&entry.name_bytes)
}

fn c4_input_bytes(value: &str) -> Vec<u8> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
}

fn crew_filename_from_title(title: &[u8]) -> Vec<u8> {
    const MAX_PATH: usize = if cfg!(windows) { 260 } else { 1024 };
    let mut title = title[..title.len().min(MAX_PATH)].to_vec();
    let mut filename = Vec::with_capacity(title.len() + 4);
    let mut index = 0;
    while index < title.len() {
        if title[index..].starts_with("§".as_bytes()) {
            index += "§".len();
            continue;
        }
        let byte = title[index];
        index += 1;
        let leading_whitespace = filename.is_empty() && is_c4_whitespace(byte);
        let stripped = b"!\"\xa7%&/=?+*#:;<>\\.".contains(&byte);
        if !leading_whitespace && !stripped {
            filename.push(byte);
        }
    }
    while filename.last().is_some_and(|byte| is_c4_whitespace(*byte)) {
        filename.pop();
    }
    if filename.is_empty() {
        filename.extend_from_slice(b"unnamed");
    }
    let remaining = MAX_PATH.saturating_sub(filename.len());
    filename.extend_from_slice(&b".c4i"[..remaining.min(4)]);
    title.clear();
    filename
}

pub(crate) fn crew_file_name_for_title(title: &str) -> String {
    let title = c4_input_bytes(title);
    clonk_script::c4_string_from_bytes(&crew_filename_from_title(&title))
}

fn item_identical(old: &[u8], new: &[u8]) -> bool {
    if cfg!(windows) {
        old.eq_ignore_ascii_case(new)
    } else {
        old == new
    }
}

fn rewrite_object_info_value(source: &[u8], key: &[u8], value: Option<&[u8]>) -> Option<Vec<u8>> {
    let newline: &[u8] = if source.windows(2).any(|window| window == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let mut lines = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let end = source[start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(source.len(), |offset| start + offset + 1);
        lines.push((start, end));
        start = end;
    }
    if lines.is_empty() {
        return None;
    }

    let mut in_object_info = false;
    let mut found_section = false;
    let mut insertion = source.len();
    let mut matching_line = None;
    for (line_start, line_end) in &lines {
        let content_end = (*line_end).saturating_sub(usize::from(
            source.get(line_end.saturating_sub(1)) == Some(&b'\n'),
        ));
        let content_end = content_end.saturating_sub(usize::from(
            source.get(content_end.saturating_sub(1)) == Some(&b'\r'),
        ));
        let content = &source[*line_start..content_end];
        let trimmed = trim_c4_whitespace(content);
        if trimmed.starts_with(b"[") && trimmed.ends_with(b"]") {
            if in_object_info {
                insertion = *line_start;
                break;
            }
            in_object_info = trimmed == b"[ObjectInfo]";
            found_section |= in_object_info;
            continue;
        }
        if !in_object_info {
            continue;
        }
        insertion = *line_end;
        if let Some(equal) = content.iter().position(|byte| *byte == b'=') {
            if trim_c4_whitespace(&content[..equal]) == key {
                matching_line = Some((*line_start, *line_end));
                break;
            }
        }
    }
    if !found_section {
        return None;
    }

    let replacement = value.map(|value| {
        let mut line = Vec::with_capacity(key.len() + value.len() + newline.len() + 1);
        line.extend_from_slice(key);
        line.push(b'=');
        line.extend_from_slice(value);
        line.extend_from_slice(newline);
        line
    });
    let mut result = Vec::with_capacity(source.len() + replacement.as_ref().map_or(0, Vec::len));
    if let Some((line_start, line_end)) = matching_line {
        result.extend_from_slice(&source[..line_start]);
        if let Some(replacement) = replacement {
            result.extend_from_slice(&replacement);
        }
        result.extend_from_slice(&source[line_end..]);
    } else if let Some(replacement) = replacement {
        result.extend_from_slice(&source[..insertion]);
        if insertion > 0 && !matches!(source[insertion - 1], b'\r' | b'\n') {
            result.extend_from_slice(newline);
        }
        result.extend_from_slice(&replacement);
        result.extend_from_slice(&source[insertion..]);
    } else {
        return Some(source.to_vec());
    }
    Some(result)
}

fn trim_c4_whitespace(mut value: &[u8]) -> &[u8] {
    while value.first().is_some_and(|byte| is_c4_whitespace(*byte)) {
        value = &value[1..];
    }
    while value.last().is_some_and(|byte| is_c4_whitespace(*byte)) {
        value = &value[..value.len() - 1];
    }
    value
}

fn is_c4_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

#[cfg(unix)]
fn path_from_name_bytes(bytes: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes.to_vec())
}

#[cfg(not(unix))]
fn path_from_name_bytes(bytes: &[u8]) -> std::ffi::OsString {
    std::ffi::OsString::from(String::from_utf8_lossy(bytes).into_owned())
}

static NEXT_CREW_STAGED_PATH: AtomicU64 = AtomicU64::new(0);

fn replace_existing_file(path: &Path, data: &[u8]) -> Result<(), StartupCrewMutationError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let staged = create_staged_file(parent, "new", data)?;
    let backup = unused_staged_path(parent, "old")?;
    if let Err(error) = fs::rename(path, &backup) {
        let _ = fs::remove_file(&staged);
        return Err(error.into());
    }
    if let Err(error) = fs::rename(&staged, path) {
        let rollback = fs::rename(&backup, path);
        let _ = fs::remove_file(&staged);
        if let Err(rollback_error) = rollback {
            return Err(io::Error::other(format!(
                "crew rewrite failed ({error}); rollback failed ({rollback_error}); original remains at '{}'",
                backup.display()
            ))
            .into());
        }
        return Err(error.into());
    }
    fs::remove_file(backup)?;
    Ok(())
}

fn create_staged_file(parent: &Path, purpose: &str, data: &[u8]) -> io::Result<PathBuf> {
    for _ in 0..1_000 {
        let path = next_staged_path(parent, purpose);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data).and_then(|()| file.sync_all()) {
                    drop(file);
                    let _ = fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a staged crew filename",
    ))
}

fn unused_staged_path(parent: &Path, purpose: &str) -> io::Result<PathBuf> {
    for _ in 0..1_000 {
        let path = next_staged_path(parent, purpose);
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(path),
            Ok(_) => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a crew backup filename",
    ))
}

fn next_staged_path(parent: &Path, purpose: &str) -> PathBuf {
    let unique = NEXT_CREW_STAGED_PATH.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".clonk-rust-crew-{purpose}-{}-{unique}",
        std::process::id()
    ))
}

/// Permanently deletes a physical player group, matching
/// `C4Group_DeleteItem(path, false)` (C4Group.cpp:233-255).
pub(crate) fn delete_player_file(path: &Path) -> io::Result<()> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// Discovers the player files visible to the startup player-selection dialog.
pub fn discover_player_files(paths: &AppPaths) -> io::Result<Vec<StartupPlayerFile>> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    discover_player_files_in(paths.install_root(), &config)
}

/// Testable core of [`discover_player_files`].
pub fn discover_player_files_in(
    install_root: &Path,
    config: &Config,
) -> io::Result<Vec<StartupPlayerFile>> {
    let player_path = configured_player_path(config);
    let participants = config
        .get_in(Some("General"), "Participants")
        .map(participant_modules)
        .unwrap_or_default();

    let roots = player_roots(install_root, &player_path);
    let mut candidates = Vec::new();
    for (root_index, root) in roots.into_iter().enumerate() {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || !has_player_extension(&name) {
                continue;
            }
            let path = entry.path();
            let file_name = participant_reference(&player_path, &path, &name);
            // AppPaths represents several possible C++ ExePath locations.
            // If the same relative player exists in more than one, the first
            // root is the one an installed executable would see.
            candidates.push((root_index, file_name, path));
        }
    }

    candidates.sort_by(|left, right| {
        left.1
            .to_ascii_lowercase()
            .cmp(&right.1.to_ascii_lowercase())
            .then_with(|| left.0.cmp(&right.0))
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
    candidates.dedup_by(|left, right| left.1.eq_ignore_ascii_case(&right.1));
    candidates.sort_by(|left, right| {
        left.1
            .to_ascii_lowercase()
            .cmp(&right.1.to_ascii_lowercase())
            .then_with(|| left.1.cmp(&right.1))
    });

    let mut players = Vec::new();
    for (_, file_name, path) in candidates {
        let group = match Group::open(&path) {
            Ok(group) => group,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %path.display(),
                    "failed to open startup player file"
                );
                continue;
            }
        };
        let player_file = match PlayerFile::load(&group) {
            Ok(player_file) => player_file,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    path = %path.display(),
                    "failed to load startup player file"
                );
                continue;
            }
        };
        let exact_core = player_file.exact_info_core();
        let activated = participants
            .iter()
            .any(|participant| participant.eq_ignore_ascii_case(&file_name));
        let render_name = if player_file.name.is_empty() {
            Path::new(&file_name)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(&file_name)
                .to_string()
        } else {
            clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(
                &player_file.name,
            ))
        };
        let render_model = PlrSelPlayer {
            name: render_name,
            activated,
            big_icon: load_group_png(&group, "BigIcon.png"),
            portrait: load_group_png(&group, "Portrait.png"),
            color_dw: normalized_player_color(&player_file),
            score: exact_core.score,
            rounds: exact_core.rounds,
            rounds_won: exact_core.rounds_won,
            rounds_lost: exact_core.rounds_lost,
            total_playing_time: exact_core.total_playing_time,
            comment: clonk_resources::decode_legacy_script_text(&clonk_script::c4_string_bytes(
                &exact_core.comment,
            )),
        };
        players.push(StartupPlayerFile {
            path,
            file_name,
            player_file,
            render_model,
        });
    }

    Ok(players)
}

/// Rewrites `Config.General.Participants` from the checked entries and saves it.
///
/// C++ reserves `CFG_MaxString + 1` bytes and, before each `SAddModule`,
/// requires `current_len + 1 + filename_len < sizeof(Participants)`. The
/// unconditional separator byte means the first filename is limited to 1023
/// bytes, while a populated list may reach the full 1024-byte payload.
pub fn persist_activations(
    config_path: &Path,
    players: &mut [StartupPlayerFile],
) -> io::Result<Vec<PlayerActivationRefusal>> {
    const CFG_MAX_STRING: usize = 1024;

    let mut config = match Config::load(config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let mut participant_keys = Vec::new();
    let mut participants = Vec::new();
    let mut participants_byte_len = 0_usize;
    let mut refusals = Vec::new();
    for (index, player) in players.iter().enumerate() {
        if !player.render_model.activated {
            continue;
        }
        let filename_byte_len = clonk_script::c4_string_byte_len(&player.file_name);
        if participants_byte_len
            .saturating_add(1)
            .saturating_add(filename_byte_len)
            > CFG_MAX_STRING
        {
            refusals.push(PlayerActivationRefusal {
                index,
                player_name: player.render_model.name.clone(),
            });
            continue;
        }
        // SAddModule accepts the guard above but does not append empty or
        // case-insensitively duplicate module names.
        if player.file_name.is_empty() {
            continue;
        }
        let key = player.file_name.to_ascii_lowercase();
        if participant_keys.iter().any(|known| known == &key) {
            continue;
        }
        participant_keys.push(key);
        if !participants.is_empty() {
            participants_byte_len += 1;
        }
        participants_byte_len += filename_byte_len;
        participants.push(player.file_name.as_str());
    }
    config.set_in(Some("General"), "Participants", participants.join(";"));
    if let Some(parent) = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    crate::save_config_preserving_native_general_booleans(&config, config_path, None, None)?;
    for refusal in &refusals {
        players[refusal.index].set_activated(false);
    }
    Ok(refusals)
}

/// Validates and saves the editable subset of `C4PlayerInfoCore`, preserving
/// every unmodeled `Player.txt` field and every unrelated group entry.
pub fn save_player_properties(
    paths: &AppPaths,
    existing_path: Option<&Path>,
    player: &PlayerFile,
    comment: &str,
    portrait: &PlayerImageWrite,
    big_icon: &PlayerImageWrite,
    group_maker: &[u8],
) -> Result<SavedStartupPlayer, PlayerPropertiesSaveError> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => {
            return Err(PlayerPropertiesSaveError::Open {
                path: paths.config_file().to_path_buf(),
                detail: error.to_string(),
            });
        }
    };
    save_player_properties_in(
        paths.install_root(),
        &config,
        existing_path,
        player,
        comment,
        portrait,
        big_icon,
        group_maker,
    )
}

/// Testable filesystem core of [`save_player_properties`].
pub fn save_player_properties_in(
    install_root: &Path,
    config: &Config,
    existing_path: Option<&Path>,
    player: &PlayerFile,
    comment: &str,
    portrait: &PlayerImageWrite,
    big_icon: &PlayerImageWrite,
    group_maker: &[u8],
) -> Result<SavedStartupPlayer, PlayerPropertiesSaveError> {
    let filename = player_group_filename(&player.name)?;
    let player_path = configured_player_path(config);
    let parent = existing_path
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| {
            if player_path.is_absolute() {
                player_path.clone()
            } else {
                install_root.join(&player_path)
            }
        });
    let target = parent.join(&filename);

    if let Some(occupant) = find_case_insensitive_entry(&parent, &filename).map_err(|error| {
        PlayerPropertiesSaveError::Open {
            path: parent.clone(),
            detail: error.to_string(),
        }
    })? {
        let owns_occupant =
            existing_path.is_some_and(|existing| paths_identify_same_item(existing, &occupant));
        if !owns_occupant {
            return Err(PlayerPropertiesSaveError::NameTaken {
                name: player.name.clone(),
                path: occupant,
            });
        }
    }

    let encoded_portrait =
        encode_image_write(portrait).map_err(|detail| PlayerPropertiesSaveError::WriteImage {
            path: target.clone(),
            entry: "Portrait.png",
            detail,
        })?;
    let encoded_big_icon =
        encode_image_write(big_icon).map_err(|detail| PlayerPropertiesSaveError::WriteImage {
            path: target.clone(),
            entry: "BigIcon.png",
            detail,
        })?;

    if let Some(existing) = existing_path {
        if existing != target {
            fs::rename(existing, &target).map_err(|error| PlayerPropertiesSaveError::Rename {
                from: existing.to_path_buf(),
                to: target.clone(),
                detail: error.to_string(),
            })?;
        }
        let source = Group::open(&target).map_err(|error| PlayerPropertiesSaveError::Open {
            path: target.clone(),
            detail: error.to_string(),
        })?;
        let original_core = source.read_file("Player.txt").ok();
        let core = rewrite_player_core(original_core.as_deref(), player, comment);
        if source.is_directory() {
            replace_directory_file(&target, "Player.txt", Some(&core)).map_err(|error| {
                PlayerPropertiesSaveError::WriteCore {
                    path: target.clone(),
                    entry: "Player.txt",
                    detail: error.to_string(),
                }
            })?;
            replace_directory_file(&target, "C4Player.c4b", None).map_err(|error| {
                PlayerPropertiesSaveError::WriteCore {
                    path: target.clone(),
                    entry: "C4Player.c4b",
                    detail: error.to_string(),
                }
            })?;
            apply_directory_image(&target, "Portrait.png", &encoded_portrait)?;
            apply_directory_image(&target, "BigIcon.png", &encoded_big_icon)?;
        } else {
            let mut mutable = MutableGroup::from_group(&source).map_err(|error| {
                PlayerPropertiesSaveError::Open {
                    path: target.clone(),
                    detail: error.to_string(),
                }
            })?;
            mutable.remove_entry("Player.txt");
            mutable.add_file("Player.txt", core).map_err(|error| {
                PlayerPropertiesSaveError::WriteCore {
                    path: target.clone(),
                    entry: "Player.txt",
                    detail: error.to_string(),
                }
            })?;
            mutable.remove_entry("C4Player.c4b");
            apply_packed_image(&mut mutable, &target, "Portrait.png", &encoded_portrait)?;
            apply_packed_image(&mut mutable, &target, "BigIcon.png", &encoded_big_icon)?;
            stamp_nonempty_group_maker(&mut mutable, group_maker);
            write_packed_group(&mutable, &target)?;
        }
    } else {
        fs::create_dir_all(&parent).map_err(|error| PlayerPropertiesSaveError::Open {
            path: parent.clone(),
            detail: error.to_string(),
        })?;
        let mut mutable = MutableGroup::new(filename.clone());
        mutable
            .add_file("Player.txt", rewrite_player_core(None, player, comment))
            .map_err(|error| PlayerPropertiesSaveError::WriteCore {
                path: target.clone(),
                entry: "Player.txt",
                detail: error.to_string(),
            })?;
        apply_packed_image(&mut mutable, &target, "Portrait.png", &encoded_portrait)?;
        apply_packed_image(&mut mutable, &target, "BigIcon.png", &encoded_big_icon)?;
        stamp_nonempty_group_maker(&mut mutable, group_maker);
        write_packed_group(&mutable, &target)?;
    }

    Ok(SavedStartupPlayer {
        file_name: participant_reference(&player_path, &target, &filename),
        path: target,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EncodedImageWrite {
    Keep,
    Replace(Vec<u8>),
    Clear,
}

fn encode_image_write(update: &PlayerImageWrite) -> Result<EncodedImageWrite, String> {
    match update {
        PlayerImageWrite::Keep => Ok(EncodedImageWrite::Keep),
        PlayerImageWrite::Clear => Ok(EncodedImageWrite::Clear),
        PlayerImageWrite::Replace(image) => {
            let expected = usize::try_from(image.width())
                .ok()
                .and_then(|width| {
                    usize::try_from(image.height())
                        .ok()
                        .and_then(|height| width.checked_mul(height))
                })
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| "image dimensions overflow".to_string())?;
            if image.pixels().len() != expected {
                return Err(format!(
                    "RGBA image has {} bytes, expected {expected}",
                    image.pixels().len()
                ));
            }
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
                encoder.set_color(ColorType::Rgba);
                encoder.set_depth(BitDepth::Eight);
                // Preserve png 0.17's defaults: these bytes feed player archives and CRCs.
                encoder.set_compression(png::Compression::Fast);
                encoder.set_filter(png::Filter::Sub);
                let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
                writer
                    .write_image_data(image.pixels())
                    .map_err(|error| error.to_string())?;
                writer.finish().map_err(|error| error.to_string())?;
            }
            Ok(EncodedImageWrite::Replace(bytes))
        }
    }
}

fn apply_packed_image(
    group: &mut MutableGroup,
    path: &Path,
    entry: &'static str,
    update: &EncodedImageWrite,
) -> Result<(), PlayerPropertiesSaveError> {
    match update {
        EncodedImageWrite::Keep => Ok(()),
        EncodedImageWrite::Clear => {
            group.remove_entry(entry);
            Ok(())
        }
        EncodedImageWrite::Replace(bytes) => {
            group.remove_entry(entry);
            group.add_file(entry, bytes.clone()).map_err(|error| {
                PlayerPropertiesSaveError::WriteImage {
                    path: path.to_path_buf(),
                    entry,
                    detail: error.to_string(),
                }
            })
        }
    }
}

fn apply_directory_image(
    directory: &Path,
    entry: &'static str,
    update: &EncodedImageWrite,
) -> Result<(), PlayerPropertiesSaveError> {
    let replacement = match update {
        EncodedImageWrite::Keep => return Ok(()),
        EncodedImageWrite::Clear => None,
        EncodedImageWrite::Replace(bytes) => Some(bytes.as_slice()),
    };
    replace_directory_file(directory, entry, replacement).map_err(|error| {
        PlayerPropertiesSaveError::WriteImage {
            path: directory.to_path_buf(),
            entry,
            detail: error.to_string(),
        }
    })
}

fn replace_directory_file(
    directory: &Path,
    name: &str,
    replacement: Option<&[u8]>,
) -> io::Result<()> {
    if let Some(existing) = find_case_insensitive_entry(directory, name)? {
        let kind = fs::symlink_metadata(&existing)?.file_type();
        if kind.is_dir() {
            fs::remove_dir_all(existing)?;
        } else {
            fs::remove_file(existing)?;
        }
    }
    if let Some(bytes) = replacement {
        fs::write(directory.join(name), bytes)?;
    }
    Ok(())
}

/// The packed-group close step: repack the rewritten group and flush it back
/// to disk (C++ `C4Group::Close` rewriting the group file).
fn write_packed_group(
    group: &MutableGroup,
    target: &Path,
) -> Result<(), PlayerPropertiesSaveError> {
    let close_error = |detail: String| PlayerPropertiesSaveError::Close {
        path: target.to_path_buf(),
        detail,
    };
    let bytes = group
        .pack()
        .map_err(|error| close_error(error.to_string()))?;
    fs::write(target, bytes).map_err(|error| close_error(error.to_string()))
}

fn configured_player_path(config: &Config) -> PathBuf {
    config
        .get_in(Some("General"), "PlayerPath")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn find_case_insensitive_entry(directory: &Path, name: &str) -> io::Result<Option<PathBuf>> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(name)
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

fn paths_identify_same_item(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn rewrite_player_core(original: Option<&[u8]>, player: &PlayerFile, comment: &str) -> Vec<u8> {
    let exact_core = player.exact_info_core();
    let line_value = |value: &str| {
        value
            .chars()
            .map(|character| {
                if matches!(character, '\r' | '\n' | '\0') {
                    ' '
                } else {
                    character
                }
            })
            .collect::<String>()
    };
    let player_values = vec![
        ("Name", line_value(&player.name)),
        ("Comment", line_value(comment)),
        ("Score", player.score.to_string()),
        ("Rounds", player.rounds.to_string()),
        ("RoundsWon", player.rounds_won.to_string()),
        ("RoundsLost", player.rounds_lost.to_string()),
        ("TotalPlayingTime", player.total_playing_time.to_string()),
    ];
    let preference_values = vec![
        ("Color", player.pref_color.to_string()),
        ("ColorDw", (player.pref_color_dw & 0x00ff_ffff).to_string()),
        ("Position", player.pref_position.to_string()),
        ("Control", player.pref_control.to_string()),
        ("Mouse", exact_core.pref_mouse_value.to_string()),
        (
            "AutoStopControl",
            exact_core.pref_control_style_value.to_string(),
        ),
        (
            "AutoContextMenu",
            exact_core.pref_auto_context_menu_value.to_string(),
        ),
    ];
    let source = original
        .map(clonk_script::c4_string_from_bytes)
        .unwrap_or_default();
    let mut output = Vec::<String>::new();
    let mut section = CoreSection::Other;
    let mut player_seen = false;
    let mut preferences_seen = false;
    let mut player_written = vec![false; player_values.len()];
    let mut preferences_written = vec![false; preference_values.len()];

    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            append_missing_core_values(
                &mut output,
                section,
                &player_values,
                &preference_values,
                &mut player_written,
                &mut preferences_written,
            );
            let name = &trimmed[1..trimmed.len() - 1];
            section = if name == "Player" {
                player_seen = true;
                CoreSection::Player
            } else if name == "Preferences" {
                preferences_seen = true;
                CoreSection::Preferences
            } else {
                CoreSection::Other
            };
            output.push(line.to_string());
            continue;
        }

        let values = match section {
            CoreSection::Player => Some((&player_values, &mut player_written)),
            CoreSection::Preferences => Some((&preference_values, &mut preferences_written)),
            CoreSection::Other => None,
        };
        if let (Some((key, _)), Some((values, written))) = (line.split_once('='), values) {
            let key = key.trim_start_matches([' ', '\t']);
            if let Some(index) = values.iter().position(|(known, _)| *known == key) {
                if !written[index] {
                    output.push(format!("{}={}", values[index].0, values[index].1));
                    written[index] = true;
                }
                continue;
            }
        }
        output.push(line.to_string());
    }
    append_missing_core_values(
        &mut output,
        section,
        &player_values,
        &preference_values,
        &mut player_written,
        &mut preferences_written,
    );
    if !player_seen {
        if !output.is_empty() && output.last().is_some_and(|line| !line.is_empty()) {
            output.push(String::new());
        }
        output.push("[Player]".to_string());
        for (name, value) in &player_values {
            output.push(format!("{name}={value}"));
        }
    }
    if !preferences_seen {
        if output.last().is_some_and(|line| !line.is_empty()) {
            output.push(String::new());
        }
        output.push("[Preferences]".to_string());
        for (name, value) in &preference_values {
            output.push(format!("{name}={value}"));
        }
    }
    let mut text = output.join("\n");
    text.push('\n');
    clonk_script::c4_string_bytes(&text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CoreSection {
    Other,
    Player,
    Preferences,
}

fn append_missing_core_values(
    output: &mut Vec<String>,
    section: CoreSection,
    player_values: &[(&str, String)],
    preference_values: &[(&str, String)],
    player_written: &mut [bool],
    preferences_written: &mut [bool],
) {
    let (values, written) = match section {
        CoreSection::Player => (player_values, player_written),
        CoreSection::Preferences => (preference_values, preferences_written),
        CoreSection::Other => return,
    };
    for (index, (name, value)) in values.iter().enumerate() {
        if !written[index] {
            output.push(format!("{name}={value}"));
            written[index] = true;
        }
    }
}

fn player_roots(install_root: &Path, player_path: &Path) -> Vec<PathBuf> {
    if player_path.is_absolute() {
        return vec![player_path.to_path_buf()];
    }
    [
        install_root.to_path_buf(),
        install_root.join("build"),
        install_root.join("build-arm64-native"),
    ]
    .into_iter()
    .map(|root| root.join(player_path))
    .collect()
}

fn participant_reference(player_path: &Path, path: &Path, name: &str) -> String {
    if player_path.is_absolute() {
        path.to_string_lossy().into_owned()
    } else {
        player_path.join(name).to_string_lossy().into_owned()
    }
}

fn participant_modules(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(str::trim)
        .filter(|module| !module.is_empty())
        .map(str::to_string)
        .collect()
}

fn has_player_extension(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("c4p"))
}

fn normalized_player_color(player: &PlayerFile) -> u32 {
    player.normalized_preferred_color()
}

fn load_group_png(group: &Group, name: &str) -> Option<ImageData> {
    let bytes = group.read_file(name).ok()?;
    decode_png(bytes)
}

fn load_player_big_icon_from_group_with_limit(
    group: &Group,
    max_size: Option<u64>,
) -> Option<ImageData> {
    let entry = group.entries().ok()?.into_iter().find(|entry| {
        !entry.is_directory
            && entry.relative_path.components().count() == 1
            && entry.name_bytes.eq_ignore_ascii_case(b"BigIcon.png")
    })?;
    if max_size.is_some_and(|max_size| entry.size > max_size) {
        return None;
    }
    decode_png(group.read_entry_bytes_exact(&entry).ok()?)
}

/// Loads the root `BigIcon.png` from an already-open player group. The runtime
/// `C4Player::Load` path has no size cap; network inputs must use one of the
/// capped loaders below after their optimizer/strip boundary.
pub(crate) fn load_player_big_icon_from_group(group: &Group) -> Option<ImageData> {
    load_player_big_icon_from_group_with_limit(group, None)
}

/// Loads the root player `BigIcon.png` directly from a local player file.
/// This mirrors runtime `C4Player::Load`, which does not apply the lobby/network
/// 20 KiB cap to the author's original file.
pub(crate) fn load_local_player_big_icon(path: &Path) -> Option<ImageData> {
    let group = Group::open(path).ok()?;
    load_player_big_icon_from_group(&group)
}

/// Loads the root player `BigIcon.png` retained by the network-resource
/// optimizer. Missing, oversized, nested, or malformed icons use the active
/// game fallback graphic instead.
pub(crate) fn load_network_player_big_icon(path: &Path) -> Option<ImageData> {
    let group = Group::open(path).ok()?;
    load_player_big_icon_from_group_with_limit(
        &group,
        Some(clonk_network::MAX_PLAYER_BIG_ICON_SIZE),
    )
}

/// Loads a stripped `CID_JoinPlr` player group carried as packed bytes. The
/// label is diagnostic only; packed controls use the same 20 KiB BigIcon cap
/// as network player resources.
pub(crate) fn load_packed_network_player_big_icon(
    label: PathBuf,
    data: &[u8],
) -> Option<ImageData> {
    let group = Group::from_memory(label, data.to_vec()).ok()?;
    load_player_big_icon_from_group_with_limit(
        &group,
        Some(clonk_network::MAX_PLAYER_BIG_ICON_SIZE),
    )
}

fn decode_png(bytes: Vec<u8>) -> Option<ImageData> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let bytes = &buffer[..info.buffer_size()];
    let pixels = match info.color_type {
        ColorType::Rgba => bytes.to_vec(),
        ColorType::Rgb => bytes
            .chunks_exact(3)
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        ColorType::Grayscale => bytes
            .iter()
            .flat_map(|value| [*value, *value, *value, 255])
            .collect(),
        ColorType::GrayscaleAlpha => bytes
            .chunks_exact(2)
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        ColorType::Indexed => return None,
    };
    Some(ImageData::new(info.width, info.height, pixels))
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    fn write_player(path: &Path, name: &str, color: u32) {
        fs::create_dir_all(path).expect("create player group");
        fs::write(
            path.join("Player.txt"),
            format!(
                "[Player]\nName={name}\nComment={name} comment\nScore=17\nRounds=5\nRoundsWon=3\nRoundsLost=2\nTotalPlayingTime=3661\n\n[Preferences]\nColorDw={color}\n"
            ),
        )
        .expect("write player core");
    }

    fn write_crew(player: &Path, file_name: &str, core: &str) {
        let crew = player.join(file_name);
        fs::create_dir_all(&crew).expect("create crew group");
        fs::write(crew.join("ObjectInfo.txt"), core).expect("write crew core");
    }

    fn load_crew(player: &Path, file_name: &str) -> CrewInfo {
        let player = Group::open(player).expect("open player group");
        let child = player.open_child(file_name).expect("open crew child");
        CrewInfo::load(&child).expect("load crew info")
    }

    fn synthetic_player(file_name: impl Into<String>, name: &str) -> StartupPlayerFile {
        StartupPlayerFile {
            path: PathBuf::new(),
            file_name: file_name.into(),
            player_file: PlayerFile::default(),
            render_model: PlrSelPlayer {
                name: name.to_string(),
                activated: true,
                big_icon: None,
                portrait: None,
                color_dw: 0,
                score: 0,
                rounds: 0,
                rounds_won: 0,
                rounds_lost: 0,
                total_playing_time: 0,
                comment: String::new(),
            },
        }
    }

    fn test_png(image: ImageData) -> Vec<u8> {
        match encode_image_write(&PlayerImageWrite::Replace(image)).expect("encode test png") {
            EncodedImageWrite::Replace(bytes) => bytes,
            EncodedImageWrite::Keep | EncodedImageWrite::Clear => {
                unreachable!("replace must encode png bytes")
            }
        }
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
    }

    #[test]
    fn player_png_encoding_preserves_png_017_bytes() {
        let encoded = test_png(tiny_image(73));

        assert_eq!(encoded.len(), 77);
        assert_eq!(fnv1a64(&encoded), 8_220_305_507_732_473_095);
    }

    #[test]
    fn big_icon_loaders_only_accept_a_direct_root_file() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Player.c4p");
        fs::create_dir_all(player.join("Nested")).expect("nested player directory");
        let icon = tiny_image(73);
        let png = test_png(icon.clone());
        fs::write(player.join("Nested/BigIcon.png"), &png).expect("nested icon");

        let group = Group::open(&player).expect("open directory player");
        assert!(load_player_big_icon_from_group(&group).is_none());
        assert!(load_local_player_big_icon(&player).is_none());

        fs::write(player.join("bIgIcOn.PnG"), png).expect("root icon");
        assert_eq!(load_local_player_big_icon(&player), Some(icon));
    }

    #[test]
    fn big_icon_network_loaders_enforce_the_cap_but_local_load_does_not() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Player.c4p");
        fs::create_dir_all(&player).expect("directory player");
        let icon = tiny_image(91);
        let small_png = test_png(icon.clone());

        let mut oversized_png = small_png.clone();
        oversized_png.resize(
            usize::try_from(clonk_network::MAX_PLAYER_BIG_ICON_SIZE).expect("cap fits usize") + 1,
            0,
        );
        fs::write(player.join("BigIcon.png"), &oversized_png).expect("oversized local icon");
        assert_eq!(load_local_player_big_icon(&player), Some(icon.clone()));
        assert!(load_network_player_big_icon(&player).is_none());

        let mut packed = MutableGroup::new("Player.c4p");
        packed
            .add_file("BigIcon.png", oversized_png)
            .expect("oversized packed icon");
        let bytes = packed.pack().expect("pack oversized player");
        assert!(load_packed_network_player_big_icon(PathBuf::from("Player.c4p"), &bytes).is_none());

        let mut packed = MutableGroup::new("Player.c4p");
        packed
            .add_file("BigIcon.png", small_png)
            .expect("small packed icon");
        let bytes = packed.pack().expect("pack small player");
        assert_eq!(
            load_packed_network_player_big_icon(PathBuf::from("Player.c4p"), &bytes),
            Some(icon)
        );
    }

    #[test]
    fn discovery_uses_cpp_player_path_references_and_marks_participants() {
        // C4StartupPlrSelDlg::UpdatePlayerList (C4StartupPlrSelDlg.cpp:678-733)
        // searches ExePath+PlayerPath, keeps top-level *.c4p entries, and checks
        // their AtExeRelativePath names against Participants case-insensitively.
        let install = tempdir().expect("install root");
        write_player(
            &install.path().join("build/Players/zulu.c4p"),
            "Zulu",
            0x112233,
        );
        write_player(&install.path().join("Players/Alpha.C4P"), "Alpha", 0x445566);
        write_player(
            &install.path().join("build-arm64-native/Players/bravo.c4p"),
            "Bravo",
            0x778899,
        );
        write_player(
            &install.path().join("Players/Nested/ignored.c4p"),
            "Ignored",
            0,
        );
        write_player(&install.path().join("Players/.private.c4p"), "Private", 0);
        fs::write(install.path().join("Players/not-a-player.txt"), b"ignored")
            .expect("write irrelevant file");

        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        config.set_in(
            Some("General"),
            "Participants",
            "players/BRAVO.C4P;Players/alpha.c4p",
        );

        let players = discover_player_files_in(install.path(), &config).expect("discover players");
        assert_eq!(
            players
                .iter()
                .map(|entry| entry.file_name.as_str())
                .collect::<Vec<_>>(),
            ["Players/Alpha.C4P", "Players/bravo.c4p", "Players/zulu.c4p"]
        );
        assert_eq!(
            players
                .iter()
                .map(|entry| entry.render_model.activated)
                .collect::<Vec<_>>(),
            [true, true, false]
        );
        assert_eq!(players[0].player_file.name, "Alpha");
        assert_eq!(players[0].render_model.name, "Alpha");
        assert_eq!(players[0].render_model.color_dw, 0x445566);
        assert_eq!(players[0].render_model.score, 17);
        assert_eq!(players[0].render_model.rounds, 5);
        assert_eq!(players[0].render_model.rounds_won, 3);
        assert_eq!(players[0].render_model.rounds_lost, 2);
        assert_eq!(players[0].render_model.total_playing_time, 3661);
        assert_eq!(players[0].render_model.comment, "Alpha comment");
        assert!(players[0].render_model.big_icon.is_none());
        assert!(players[0].render_model.portrait.is_none());
    }

    #[test]
    fn discovery_render_uses_exact_player_core_names() {
        // C4StartupPlrSelDlg renders the same exact-name C4PlayerInfoCore
        // loaded for gameplay; it does not parse a second, permissive view of
        // Player.txt (C4StartupPlrSelDlg.cpp:216-243,293-301).
        let install = tempdir().expect("install root");
        let player = install.path().join("Case.c4p");
        fs::create_dir_all(&player).expect("create player group");
        fs::write(
            player.join("Player.txt"),
            "[player]\n\
             Comment=Wrong section\n\
             Score=101\n\
             Rounds=102\n\
             RoundsWon=103\n\
             RoundsLost=104\n\
             TotalPlayingTime=105\n\
             [Player]\n\
             Name=Case\n\
             comment=Wrong key\n\
             score=201\n\
             rounds=202\n\
             roundswon=203\n\
             roundslost=204\n\
             totalplayingtime=205\n",
        )
        .expect("write player core");

        let players =
            discover_player_files_in(install.path(), &Config::new()).expect("discover player");

        assert_eq!(players.len(), 1);
        assert_eq!(players[0].render_model.name, "Case");
        assert_eq!(
            (
                players[0].render_model.score,
                players[0].render_model.rounds,
                players[0].render_model.rounds_won,
                players[0].render_model.rounds_lost,
                players[0].render_model.total_playing_time,
            ),
            (0, 0, 0, 0, 0)
        );
        assert!(players[0].render_model.comment.is_empty());
    }

    #[test]
    fn discovery_decodes_native_player_text_only_for_presentation() {
        let install = tempdir().expect("install root");
        let player = install.path().join("Native.c4p");
        fs::create_dir_all(&player).expect("create player group");
        fs::write(
            player.join("Player.txt"),
            [
                b"[Player]\nName=Andr".as_slice(),
                &[0xe9],
                b"\nComment=Gr",
                &[0xfc, 0xdf],
                b"e\n",
            ]
            .concat(),
        )
        .expect("write native player core");

        let players = discover_player_files_in(install.path(), &Config::new())
            .expect("discover native player");
        assert_eq!(players.len(), 1);
        assert_eq!(
            clonk_script::c4_string_bytes(&players[0].player_file.name),
            b"Andr\xe9"
        );
        assert_eq!(players[0].render_model.name, "Andr\u{e9}");
        assert_eq!(players[0].render_model.comment, "Gr\u{fc}\u{df}e");
    }

    #[test]
    fn persistence_rebuilds_participants_in_visible_order() {
        // C4StartupPlrSelDlg::UpdateActivatedPlayers
        // (C4StartupPlrSelDlg.cpp:821-837) clears Participants and walks the
        // visible list, adding each checked filename as a semicolon module.
        let install = tempdir().expect("install root");
        write_player(&install.path().join("Alpha.c4p"), "Alpha", 1);
        write_player(&install.path().join("Bravo.c4p"), "Bravo", 2);

        let config_path = install.path().join("Config/clonk-rust.config");
        fs::create_dir_all(config_path.parent().expect("config parent"))
            .expect("create config parent");
        fs::write(
            &config_path,
            "[General]\nParticipants = Stale.c4p\nPlayerPath = \nFairCrew = true\n",
        )
        .expect("write config");
        let config = Config::load(&config_path).expect("load config");
        let mut players =
            discover_player_files_in(install.path(), &config).expect("discover players");
        players[0].set_activated(true);
        players[1].set_activated(false);

        persist_activations(&config_path, &mut players).expect("save activation");

        let saved = Config::load(&config_path).expect("reload config");
        assert_eq!(
            saved.get_in(Some("General"), "Participants"),
            Some("Alpha.c4p")
        );
        assert_eq!(
            saved.get_in(Some("General"), "FairCrew"),
            Some("true"),
            "unrelated config survives the rewrite"
        );
    }

    #[test]
    fn l063_persist_activations_accepts_exact_cpp_buffer_payload() {
        let root = tempdir().expect("config root");
        let config_path = root.path().join("clonk-rust.config");
        let tail = "b".repeat(1022);
        let mut players = vec![
            synthetic_player("A", "Alpha"),
            synthetic_player(tail.clone(), "Exact"),
        ];

        let refusals =
            persist_activations(&config_path, &mut players).expect("save exact-size list");

        assert!(refusals.is_empty());
        let saved = Config::load(&config_path).expect("reload exact-size config");
        let participants = saved
            .get_in(Some("General"), "Participants")
            .expect("participants value");
        assert_eq!(participants, format!("A;{tail}"));
        assert_eq!(clonk_script::c4_string_byte_len(participants), 1024);
        assert!(players.iter().all(|player| player.render_model.activated));
    }

    #[test]
    fn l063_persist_activations_refuses_overflow_and_continues() {
        let root = tempdir().expect("config root");
        let config_path = root.path().join("clonk-rust.config");
        fs::write(
            &config_path,
            "[General]\nParticipants=Stale.c4p\nFairCrew=true\n",
        )
        .expect("write existing config");
        let mut players = vec![
            synthetic_player("A", "Alpha"),
            synthetic_player("b".repeat(1023), "Overflow"),
            synthetic_player("C", "Charlie"),
        ];

        let refusals = persist_activations(&config_path, &mut players).expect("save bounded list");

        assert_eq!(
            refusals,
            vec![PlayerActivationRefusal {
                index: 1,
                player_name: "Overflow".to_string(),
            }]
        );
        assert!(players[0].render_model.activated);
        assert!(!players[1].render_model.activated);
        assert!(players[2].render_model.activated);
        let saved = Config::load(&config_path).expect("reload bounded config");
        assert_eq!(saved.get_in(Some("General"), "Participants"), Some("A;C"));
        assert_eq!(saved.get_in(Some("General"), "FairCrew"), Some("true"));
    }

    #[test]
    fn l063_persist_activations_reserves_separator_for_first_player() {
        let root = tempdir().expect("config root");
        let accepted_path = root.path().join("accepted.config");
        let mut accepted = vec![synthetic_player("a".repeat(1023), "Accepted")];
        assert!(persist_activations(&accepted_path, &mut accepted)
            .expect("save 1023-byte first player")
            .is_empty());

        let refused_path = root.path().join("refused.config");
        let mut refused = vec![synthetic_player("b".repeat(1024), "Refused")];
        assert_eq!(
            persist_activations(&refused_path, &mut refused)
                .expect("save list without oversized first player"),
            vec![PlayerActivationRefusal {
                index: 0,
                player_name: "Refused".to_string(),
            }]
        );
        assert!(!refused[0].render_model.activated);
        assert_eq!(
            Config::load(&refused_path)
                .expect("reload refused config")
                .get_in(Some("General"), "Participants"),
            Some("")
        );
    }

    #[test]
    fn permanent_delete_removes_packed_file_and_directory_group() {
        let root = tempdir().expect("player root");

        let packed = root.path().join("Packed.c4p");
        fs::write(&packed, b"packed player group").expect("write packed group");
        delete_player_file(&packed).expect("delete packed group");
        assert!(!packed.exists());

        let directory = root.path().join("Directory.c4p");
        fs::create_dir_all(directory.join("Nested")).expect("create directory group");
        fs::write(directory.join("Nested/Player.txt"), b"[Player]\nName=Ada\n")
            .expect("write nested player file");
        delete_player_file(&directory).expect("delete directory group");
        assert!(!directory.exists());
    }

    #[test]
    fn permanent_delete_reports_a_missing_player() {
        let root = tempdir().expect("player root");
        let error = delete_player_file(&root.path().join("Missing.c4p"))
            .expect_err("missing player must not look deleted");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    fn tiny_image(red: u8) -> ImageData {
        ImageData::new(2, 1, vec![red, 20, 30, 255, red, 40, 50, 128])
    }

    #[test]
    fn player_name_validation_sanitizes_and_rejects_collisions_except_self() {
        assert!(matches!(
            player_group_filename(""),
            Err(PlayerPropertiesSaveError::EmptyName)
        ));
        assert_eq!(
            player_group_filename(".A/B\\C:D*E?F\"G<H>I|J").expect("sanitize"),
            "_A_B_C_D_E_F_G_H_I_J.c4p"
        );
        assert_eq!(
            player_group_filename("Name.c4p").expect("append suffix"),
            "Name.c4p.c4p"
        );
        assert_eq!(
            player_group_filename("A.B").expect("non-leading dot"),
            "A.B.c4p"
        );
        assert_eq!(
            player_group_filename(&clonk_script::c4_string_from_bytes(b"Andr\xe9"))
                .expect("native name conversion"),
            "André.c4p"
        );

        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        let players = root.path().join("Players");
        fs::create_dir_all(players.join("Taken.c4p")).expect("taken player");
        let core = PlayerFile {
            name: "Taken".to_string(),
            ..PlayerFile::default()
        };
        assert!(matches!(
            save_player_properties_in(
                root.path(),
                &config,
                None,
                &core,
                "",
                &PlayerImageWrite::Keep,
                &PlayerImageWrite::Keep,
                b"",
            ),
            Err(PlayerPropertiesSaveError::NameTaken { .. })
        ));

        fs::write(
            players.join("Taken.c4p/Player.txt"),
            b"[Player]\nName=Taken\n",
        )
        .expect("self core");
        let saved = save_player_properties_in(
            root.path(),
            &config,
            Some(&players.join("Taken.c4p")),
            &core,
            "self",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Keep,
            b"",
        )
        .expect("own filename is allowed");
        assert_eq!(saved.path, players.join("Taken.c4p"));
    }

    #[test]
    fn new_player_save_creates_packed_group_with_core_and_images() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        let player = PlayerFile {
            name: "Ada".to_string(),
            pref_color: 7,
            pref_color_dw: 0xf08050,
            pref_control_style: true,
            pref_auto_context_menu: true,
            ..PlayerFile::default()
        };
        let portrait = tiny_image(100);
        let icon = tiny_image(200);

        let saved = save_player_properties_in(
            root.path(),
            &config,
            None,
            &player,
            "I'm new.",
            &PlayerImageWrite::Replace(portrait.clone()),
            &PlayerImageWrite::Replace(icon.clone()),
            b"",
        )
        .expect("create player");
        assert_eq!(saved.file_name, "Players/Ada.c4p");
        assert!(saved.path.is_file(), "new .c4p is packed, not a folder");

        let group = Group::open(&saved.path).expect("open packed player");
        let loaded = PlayerFile::load(&group).expect("load saved core");
        assert_eq!(loaded.name, "Ada");
        assert_eq!(loaded.pref_color, 7);
        assert_eq!(loaded.pref_color_dw, 0xf08050);
        assert!(loaded.pref_control_style);
        assert!(loaded.pref_auto_context_menu);
        assert!(group.read_file("Portrait.png").is_ok());
        assert!(group.read_file("BigIcon.png").is_ok());

        let players = discover_player_files_in(root.path(), &config).expect("rediscover");
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].render_model.comment, "I'm new.");
        assert_eq!(players[0].render_model.portrait, Some(portrait));
        assert_eq!(players[0].render_model.big_icon, Some(icon));
    }

    #[test]
    fn player_core_rewrite_preserves_noncanonical_integer_preferences() {
        let player = PlayerFile {
            info_core: clonk_engine::player_file::PlayerInfoCoreState {
                pref_mouse: true,
                pref_mouse_value: 7,
                pref_control_style: true,
                pref_control_style_value: 2,
                pref_auto_context_menu: true,
                pref_auto_context_menu_value: -2,
                ..clonk_engine::player_file::PlayerInfoCoreState::default()
            },
            pref_mouse: true,
            pref_control_style: true,
            pref_auto_context_menu: true,
            ..PlayerFile::default()
        };

        let rewritten = rewrite_player_core(None, &player, "");
        for expected in [
            b"Mouse=7\n".as_slice(),
            b"AutoStopControl=2\n",
            b"AutoContextMenu=-2\n",
        ] {
            assert!(rewritten
                .windows(expected.len())
                .any(|window| window == expected));
        }
    }

    #[test]
    fn player_core_rewrite_preserves_wrong_case_names_as_unmodeled() {
        // C4PlayerInfoCore only recognizes exact names. The preserving Rust
        // editor may retain malformed lines, but must write edited values to
        // canonical sections and keys instead of treating those lines as the
        // canonical save targets.
        let player = PlayerFile {
            name: "Edited".to_string(),
            score: 17,
            rounds: 8,
            rounds_won: 5,
            rounds_lost: 3,
            total_playing_time: 3_661,
            pref_color: 4,
            pref_color_dw: 0x12_34_56,
            pref_position: 2,
            pref_control: 3,
            pref_mouse: false,
            pref_control_style: true,
            pref_auto_context_menu: true,
            ..PlayerFile::default()
        };
        let original = b"[player]\n\
Name=Wrong section\n\
Score=91\n\
[Player]\n\
name=Wrong key\n\
score=92\n\
[preferences]\n\
Color=9\n\
Control=8\n\
[Preferences]\n\
color=7\n\
control=6\n";

        let rewritten = rewrite_player_core(Some(original), &player, "Edited comment");
        let rewritten_text = clonk_script::c4_string_from_bytes(&rewritten);
        for malformed in [
            "[player]\nName=Wrong section\nScore=91",
            "[Player]\nname=Wrong key\nscore=92",
            "[preferences]\nColor=9\nControl=8",
            "[Preferences]\ncolor=7\ncontrol=6",
        ] {
            assert!(rewritten_text.contains(malformed), "missing `{malformed}`");
        }

        let root = tempdir().expect("player root");
        let path = root.path().join("Edited.c4p");
        fs::create_dir_all(&path).expect("create player group");
        fs::write(path.join("Player.txt"), rewritten).expect("write rewritten core");
        let loaded = PlayerFile::load_from_path(&path).expect("load rewritten player");

        assert_eq!(loaded.name, "Edited");
        assert_eq!(loaded.info_core.comment, "Edited comment");
        assert_eq!(
            (
                loaded.score,
                loaded.rounds,
                loaded.rounds_won,
                loaded.rounds_lost,
                loaded.total_playing_time,
            ),
            (17, 8, 5, 3, 3_661)
        );
        assert_eq!(
            (
                loaded.pref_color,
                loaded.pref_color_dw,
                loaded.pref_position,
                loaded.pref_control,
            ),
            (4, 0x12_34_56, 2, 3)
        );
        assert!(!loaded.pref_mouse);
        assert!(loaded.pref_control_style);
        assert!(loaded.pref_auto_context_menu);
    }

    #[test]
    fn existing_directory_save_renames_and_preserves_unmodeled_core_and_entries() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        let old = root.path().join("Players/Old.c4p");
        fs::create_dir_all(old.join("Crew.c4i")).expect("directory player");
        fs::write(
            old.join("Player.txt"),
            "[Player]\nName=Old\nComment=old comment\nRank=7\nScore=17\nRounds=5\nRoundsWon=3\nRoundsLost=2\nTotalPlayingTime=3661\nMystery=retain\n\n[Preferences]\nColorDw=1\n\n[Extra]\nValue=keep\n",
        )
        .expect("old core");
        fs::write(old.join("Portrait.png"), b"old portrait").expect("old portrait");
        fs::write(old.join("BigIcon.png"), b"old icon").expect("old icon");
        fs::write(old.join("C4Player.c4b"), b"obsolete runtime").expect("old runtime");
        fs::write(old.join("Crew.c4i/ObjectInfo.txt"), b"crew").expect("crew entry");

        let source = Group::open(&old).expect("open source");
        let mut player = PlayerFile::load(&source).expect("load source");
        player.name = "New".to_string();
        player.pref_color = 11;
        player.pref_color_dw = 0xbc00c0;
        player.pref_control_style = true;
        player.pref_auto_context_menu = true;
        let replacement = tiny_image(77);
        let saved = save_player_properties_in(
            root.path(),
            &config,
            Some(&old),
            &player,
            "old comment",
            &PlayerImageWrite::Replace(replacement.clone()),
            &PlayerImageWrite::Clear,
            b"",
        )
        .expect("edit directory player");

        assert!(!old.exists());
        assert!(saved.path.is_dir());
        assert!(saved.path.join("Crew.c4i/ObjectInfo.txt").is_file());
        assert!(!saved.path.join("BigIcon.png").exists());
        assert!(!saved.path.join("C4Player.c4b").exists());
        let core = fs::read_to_string(saved.path.join("Player.txt")).expect("rewritten core");
        assert!(core.contains("Rank=7"));
        assert!(core.contains("Mystery=retain"));
        assert!(core.contains("[Extra]\nValue=keep"));
        let loaded = PlayerFile::load_from_path(&saved.path).expect("load edited player");
        assert_eq!((loaded.score, loaded.rounds), (17, 5));
        assert_eq!(loaded.pref_color, 11);
        assert_eq!(loaded.pref_color_dw, 0xbc00c0);
        let edited = Group::open(&saved.path).expect("open edited group");
        assert_eq!(load_group_png(&edited, "Portrait.png"), Some(replacement));
    }

    #[test]
    fn existing_packed_save_preserves_unrelated_entries_and_keep_picture() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        fs::create_dir_all(root.path().join("Players")).expect("player directory");
        let old = root.path().join("Players/Old.c4p");
        let portrait = tiny_image(33);
        let portrait_bytes = match encode_image_write(&PlayerImageWrite::Replace(portrait))
            .expect("encode portrait")
        {
            EncodedImageWrite::Replace(bytes) => bytes,
            _ => unreachable!(),
        };
        let mut group = MutableGroup::new("Old.c4p");
        group
            .add_file(
                "Player.txt",
                b"[Player]\nName=Old\nScore=9\nRankName=Captain\n\n[Preferences]\nColorDw=2\n"
                    .to_vec(),
            )
            .expect("core");
        group
            .add_file("Portrait.png", portrait_bytes.clone())
            .expect("portrait");
        group
            .add_file("Untouched.bin", b"retain".to_vec())
            .expect("extra");
        group
            .add_file("C4Player.c4b", b"obsolete runtime".to_vec())
            .expect("old runtime");
        fs::write(&old, group.pack().expect("pack source")).expect("write source");

        let source = Group::open(&old).expect("open source");
        let mut player = PlayerFile::load(&source).expect("load source");
        player.name = "Renamed".to_string();
        let saved = save_player_properties_in(
            root.path(),
            &config,
            Some(&old),
            &player,
            "packed",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Clear,
            b"",
        )
        .expect("edit packed player");
        let edited = Group::open(&saved.path).expect("open edited");
        assert_eq!(
            edited.read_file("Untouched.bin").expect("untouched entry"),
            b"retain"
        );
        assert_eq!(
            edited.read_file("Portrait.png").expect("kept portrait"),
            portrait_bytes
        );
        assert!(edited.read_file("C4Player.c4b").is_err());
        assert!(
            clonk_script::c4_string_from_bytes(&edited.read_file("Player.txt").expect("core"))
                .contains("RankName=Captain")
        );
    }

    #[test]
    fn save_failure_steps_carry_their_paths() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        let players = root.path().join("Players");
        fs::create_dir_all(&players).expect("player directory");
        let core = PlayerFile {
            name: "Renamed".to_string(),
            ..PlayerFile::default()
        };
        let save = |existing: Option<&Path>, portrait: &PlayerImageWrite| {
            save_player_properties_in(
                root.path(),
                &config,
                existing,
                &core,
                "",
                portrait,
                &PlayerImageWrite::Keep,
                b"",
            )
        };
        let missing = players.join("Old.c4p");
        let target = players.join("Renamed.c4p");

        // Rename: the source group vanished, so the move has nothing to rename.
        assert!(matches!(
            save(Some(&missing), &PlayerImageWrite::Keep),
            Err(PlayerPropertiesSaveError::Rename { from, to, .. })
                if from == missing && to == target
        ));

        // Open: the destination is not a valid player group.
        fs::write(&target, b"not a C4Group").expect("corrupt packed group");
        assert!(matches!(
            save(Some(&target), &PlayerImageWrite::Keep),
            Err(PlayerPropertiesSaveError::Open { path, .. }) if path == target
        ));
        fs::remove_file(&target).expect("drop corrupt group");

        // WriteImage: a broken RGBA payload names the entry it was meant for.
        let broken = ImageData::new(2, 1, vec![0, 0, 0]);
        assert!(matches!(
            save(None, &PlayerImageWrite::Replace(broken)),
            Err(PlayerPropertiesSaveError::WriteImage {
                path,
                entry: "Portrait.png",
                ..
            }) if path == target
        ));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            // Close: the packed destination file cannot be rewritten.
            let mut group = MutableGroup::new("Renamed.c4p");
            group
                .add_file("Player.txt", b"[Player]\nName=Renamed\n".to_vec())
                .expect("core entry");
            fs::write(&target, group.pack().expect("pack source")).expect("write source");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o444))
                .expect("make packed group read-only");
            assert!(matches!(
                save(Some(&target), &PlayerImageWrite::Keep),
                Err(PlayerPropertiesSaveError::Close { path, .. }) if path == target
            ));
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
                .expect("restore packed group permissions");
            fs::remove_file(&target).expect("drop packed group");

            // WriteCore: the directory group's Player.txt cannot be replaced.
            fs::create_dir(&target).expect("directory group");
            fs::write(target.join("Player.txt"), b"[Player]\nName=Renamed\n")
                .expect("directory core");
            fs::set_permissions(&target, fs::Permissions::from_mode(0o555))
                .expect("make directory group read-only");
            assert!(matches!(
                save(Some(&target), &PlayerImageWrite::Keep),
                Err(PlayerPropertiesSaveError::WriteCore {
                    path,
                    entry: "Player.txt",
                    ..
                }) if path == target
            ));
            fs::set_permissions(&target, fs::Permissions::from_mode(0o755))
                .expect("restore directory group permissions");
            fs::remove_dir_all(&target).expect("drop directory group");
        }

        // Open: the players directory itself cannot be scanned for occupants.
        fs::remove_dir_all(&players).expect("remove player root");
        fs::write(&players, b"not a directory").expect("occupy player root path");
        assert!(matches!(
            save(None, &PlayerImageWrite::Keep),
            Err(PlayerPropertiesSaveError::Open { path, .. }) if path == players
        ));
    }

    #[test]
    fn startup_player_properties_stamps_nonempty_configured_group_maker() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        fs::create_dir_all(root.path().join("Players")).expect("player directory");
        let configured_maker = b"Configured \x81 Maker";

        let new_player = PlayerFile {
            name: "New".to_string(),
            ..PlayerFile::default()
        };
        let saved = save_player_properties_in(
            root.path(),
            &config,
            None,
            &new_player,
            "",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Keep,
            configured_maker,
        )
        .expect("save new player");
        assert_eq!(
            Group::open(&saved.path)
                .expect("open new player")
                .maker_bytes(),
            Some(configured_maker.as_slice())
        );

        let existing_path = root.path().join("Players/Existing.c4p");
        let mut existing = MutableGroup::new("Existing.c4p");
        existing.set_maker_bytes(b"Original Maker");
        existing
            .add_file("Player.txt", b"[Player]\nName=Existing\n".to_vec())
            .expect("existing core");
        fs::write(&existing_path, existing.pack().expect("pack existing")).expect("write existing");
        let existing_player =
            PlayerFile::load(&Group::open(&existing_path).expect("open existing player"))
                .expect("load existing core");
        let saved = save_player_properties_in(
            root.path(),
            &config,
            Some(&existing_path),
            &existing_player,
            "updated",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Keep,
            configured_maker,
        )
        .expect("rewrite existing player");
        assert_eq!(
            Group::open(&saved.path)
                .expect("open rewritten player")
                .maker_bytes(),
            Some(configured_maker.as_slice())
        );
    }

    #[test]
    fn startup_player_properties_empty_maker_preserves_new_and_existing_defaults() {
        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        fs::create_dir_all(root.path().join("Players")).expect("player directory");

        let new_player = PlayerFile {
            name: "Default".to_string(),
            ..PlayerFile::default()
        };
        let saved = save_player_properties_in(
            root.path(),
            &config,
            None,
            &new_player,
            "",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Keep,
            b"",
        )
        .expect("save new player");
        assert_eq!(
            Group::open(&saved.path)
                .expect("open new player")
                .maker_bytes(),
            Some(b"New C4Group".as_slice())
        );

        let existing_path = root.path().join("Players/Existing.c4p");
        let mut existing = MutableGroup::new("Existing.c4p");
        existing.set_maker_bytes(b"Original Maker");
        existing
            .add_file("Player.txt", b"[Player]\nName=Existing\n".to_vec())
            .expect("existing core");
        fs::write(&existing_path, existing.pack().expect("pack existing")).expect("write existing");
        let existing_player =
            PlayerFile::load(&Group::open(&existing_path).expect("open existing player"))
                .expect("load existing core");
        let saved = save_player_properties_in(
            root.path(),
            &config,
            Some(&existing_path),
            &existing_player,
            "updated",
            &PlayerImageWrite::Keep,
            &PlayerImageWrite::Keep,
            b"",
        )
        .expect("rewrite existing player");
        assert_eq!(
            Group::open(&saved.path)
                .expect("open rewritten player")
                .maker_bytes(),
            Some(b"Original Maker".as_slice())
        );
    }

    #[test]
    fn crew_discovery_is_direct_and_uses_visible_descending_type_order() {
        let root = tempdir().expect("player root");
        let player_path = root.path().join("CrewSort.c4p");
        write_player(&player_path, "CrewSort", 0x123456);
        write_crew(
            &player_path,
            "Novice.c4i",
            "[ObjectInfo]\nid=CLNK\nName=Novice\nRankName=Private\nTypeName=Clonk\nExperience=10\n",
        );
        write_crew(
            &player_path,
            "Veteran.c4i",
            "[ObjectInfo]\nid=CLNK\nName=Veteran\nRankName=Captain\nTypeName=Clonk\nExperience=100\nParticipation=0\n",
        );
        write_crew(
            &player_path,
            "Wipf.c4i",
            "[ObjectInfo]\nid=WIPF\nName=Wipf\nTypeName=Animal\nExperience=50\n",
        );
        write_crew(
            &player_path.join("Roster.c4f"),
            "Nested.c4i",
            "[ObjectInfo]\nid=CLNK\nName=Nested\nExperience=1000\n",
        );
        fs::create_dir(player_path.join("Broken.c4i")).expect("broken crew group");

        let player = discover_player_files_in(root.path(), &Config::new())
            .expect("discover player")
            .remove(0);
        let crew = discover_crew_files(&player).expect("discover direct crew");

        assert_eq!(
            crew.iter()
                .map(|entry| entry.file_name.as_str())
                .collect::<Vec<_>>(),
            ["Veteran.c4i", "Novice.c4i", "Wipf.c4i"]
        );
        assert_eq!(crew[0].crew_info.experience, 100);
        assert!(!crew[0].render_model.participating);
        assert_eq!(crew[0].render_model.name, "Veteran");
        assert_eq!(crew[0].render_model.rank_name, "Captain");
        assert_eq!(crew[0].render_model.type_name, "Clonk");
        assert_eq!(crew[0].render_model.color_dw, 0x123456);
    }

    #[test]
    fn directory_crew_mutations_rewrite_rename_collide_and_delete() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Directory.c4p");
        write_player(&player, "Directory", 1);
        write_crew(
            &player,
            "Scout.c4i",
            "[ObjectInfo]\nid=CLNK\nName=Scout\nExperience=7\nParticipation=1\n\n[Physical]\nWalk=80000\n",
        );
        write_crew(
            &player,
            "Target.c4i",
            "[ObjectInfo]\nid=CLNK\nName=Target\n",
        );

        set_crew_participation(&player, "SCOUT.C4I", false, b"").expect("disable crew");
        assert_eq!(load_crew(&player, "Scout.c4i").participation, 0);
        set_crew_participation(&player, "Scout.c4i", true, b"").expect("enable crew");
        assert_eq!(load_crew(&player, "Scout.c4i").participation, 1);

        set_crew_death_message(&player, "Scout.c4i", &"x".repeat(90), b"")
            .expect("set death message");
        assert_eq!(load_crew(&player, "Scout.c4i").death_message.len(), 75);

        let renamed = rename_crew(&player, "Scout.c4i", "A!li.ce§", b"")
            .expect("rename sanitized crew filename");
        assert_eq!(renamed, "Alice.c4i");
        assert!(!player.join("Scout.c4i").exists());
        assert_eq!(load_crew(&player, "Alice.c4i").name, "A!li.ce§");
        assert!(matches!(
            rename_crew(&player, "Alice.c4i", "Target", b""),
            Err(StartupCrewMutationError::NameCollision { file_name })
                if file_name == "Target.c4i"
        ));

        delete_crew_file(&player, "Alice.c4i", b"").expect("delete crew");
        assert!(!player.join("Alice.c4i").exists());
        assert!(player.join("Target.c4i").exists());
    }

    #[test]
    fn packed_player_crew_mutations_keep_the_parent_group_valid() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Packed.c4p");
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Crew\nExperience=5\n".to_vec(),
        )
        .expect("crew core");
        let mut packed = MutableGroup::new("Packed.c4p");
        packed
            .add_file("Player.txt", b"[Player]\nName=Packed\n".to_vec())
            .expect("player core");
        packed
            .add_file("Keep.txt", b"untouched".to_vec())
            .expect("opaque sibling");
        packed.add_child("Crew.c4i", crew).expect("crew child");
        fs::write(&player, packed.pack().expect("pack player")).expect("write player");

        set_crew_participation(&player, "Crew.c4i", false, b"").expect("disable packed crew");
        set_crew_death_message(&player, "Crew.c4i", "Farewell", b"").expect("packed death message");
        let renamed = rename_crew(&player, "Crew.c4i", "Pack?ed", b"").expect("rename packed crew");
        assert_eq!(renamed, "Packed.c4i");

        let reopened = Group::open(&player).expect("rewritten player remains valid");
        assert_eq!(reopened.read_file("Keep.txt").unwrap(), b"untouched");
        let crew = load_crew(&player, "Packed.c4i");
        assert_eq!(crew.name, "Pack?ed");
        assert_eq!(crew.participation, 0);
        assert_eq!(crew.death_message, "Farewell");

        delete_crew_file(&player, "Packed.c4i", b"").expect("delete packed crew");
        let reopened = Group::open(&player).expect("player remains valid after delete");
        assert!(!reopened.exists("Packed.c4i"));
        assert_eq!(reopened.read_file("Keep.txt").unwrap(), b"untouched");
    }

    #[test]
    fn startup_crew_core_rewrite_stamps_child_maker_but_preserves_parent_maker() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Packed.c4p");
        let configured_maker = b"Configured \x81 Maker";
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.set_maker_bytes(b"Original Child Maker");
        crew.add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Crew\n".to_vec(),
        )
        .expect("crew core");
        let mut packed = MutableGroup::new("Packed.c4p");
        packed.set_maker_bytes(b"Original Parent Maker");
        packed
            .add_file("Player.txt", b"[Player]\nName=Packed\n".to_vec())
            .expect("player core");
        packed.add_child("Crew.c4i", crew).expect("crew child");
        fs::write(&player, packed.pack().expect("pack player")).expect("write player");

        set_crew_participation(&player, "Crew.c4i", false, configured_maker)
            .expect("rewrite crew core");
        let parent = Group::open(&player).expect("open rewritten parent");
        assert_eq!(
            parent.maker_bytes(),
            Some(b"Original Parent Maker".as_slice())
        );
        assert_eq!(
            parent
                .open_child("Crew.c4i")
                .expect("open rewritten child")
                .maker_bytes(),
            Some(configured_maker.as_slice())
        );

        let renamed = rename_crew(&player, "Crew.c4i", "Renamed", configured_maker)
            .expect("rename and rewrite crew");
        let parent = Group::open(&player).expect("open renamed parent");
        assert_eq!(renamed, "Renamed.c4i");
        assert_eq!(
            parent.maker_bytes(),
            Some(b"Original Parent Maker".as_slice())
        );
        assert_eq!(
            parent
                .open_child("Renamed.c4i")
                .expect("open renamed child")
                .maker_bytes(),
            Some(configured_maker.as_slice())
        );
    }

    #[test]
    fn startup_crew_core_rewrite_with_empty_maker_preserves_both_headers() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Packed.c4p");
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.set_maker_bytes(b"Original Child Maker");
        crew.add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Crew\n".to_vec(),
        )
        .expect("crew core");
        let mut packed = MutableGroup::new("Packed.c4p");
        packed.set_maker_bytes(b"Original Parent Maker");
        packed
            .add_file("Player.txt", b"[Player]\nName=Packed\n".to_vec())
            .expect("player core");
        packed.add_child("Crew.c4i", crew).expect("crew child");
        fs::write(&player, packed.pack().expect("pack player")).expect("write player");

        set_crew_death_message(&player, "Crew.c4i", "Farewell", b"").expect("rewrite crew core");
        let parent = Group::open(&player).expect("open rewritten parent");
        assert_eq!(
            parent.maker_bytes(),
            Some(b"Original Parent Maker".as_slice())
        );
        assert_eq!(
            parent
                .open_child("Crew.c4i")
                .expect("open rewritten child")
                .maker_bytes(),
            Some(b"Original Child Maker".as_slice())
        );
    }

    #[test]
    fn startup_crew_rewrite_stamps_standalone_packed_child_in_directory_player() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Directory.c4p");
        fs::create_dir_all(&player).expect("directory player");
        fs::write(player.join("Player.txt"), b"[Player]\nName=Directory\n").expect("player core");
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.set_maker_bytes(b"Original Child Maker");
        crew.add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Crew\n".to_vec(),
        )
        .expect("crew core");
        fs::write(
            player.join("Crew.c4i"),
            crew.pack().expect("pack crew child"),
        )
        .expect("write crew child");

        set_crew_participation(&player, "Crew.c4i", false, b"Configured Maker")
            .expect("rewrite crew core");
        assert_eq!(
            Group::open(player.join("Crew.c4i"))
                .expect("open rewritten child")
                .maker_bytes(),
            Some(b"Configured Maker".as_slice())
        );
    }

    #[test]
    fn startup_crew_delete_stamps_parent_when_native_group_close_persists_it() {
        let root = tempdir().expect("player root");
        let player = root.path().join("Packed.c4p");
        let mut crew = MutableGroup::new("Crew.c4i");
        crew.add_file(
            "ObjectInfo.txt",
            b"[ObjectInfo]\nid=CLNK\nName=Crew\n".to_vec(),
        )
        .expect("crew core");
        let mut packed = MutableGroup::new("Packed.c4p");
        packed.set_maker_bytes(b"Original Parent Maker");
        packed
            .add_file("Player.txt", b"[Player]\nName=Packed\n".to_vec())
            .expect("player core");
        packed.add_child("Crew.c4i", crew).expect("crew child");
        let mut other = MutableGroup::new("Other.c4i");
        other
            .add_file(
                "ObjectInfo.txt",
                b"[ObjectInfo]\nid=CLNK\nName=Other\n".to_vec(),
            )
            .expect("other crew core");
        packed
            .add_child("Other.c4i", other)
            .expect("other crew child");
        fs::write(&player, packed.pack().expect("pack player")).expect("write player");

        delete_crew_file(&player, "Crew.c4i", b"").expect("delete with empty maker");
        let parent = Group::open(&player).expect("open first rewritten parent");
        assert!(!parent.exists("Crew.c4i"));
        assert!(parent.exists("Other.c4i"));
        assert_eq!(
            parent.maker_bytes(),
            Some(b"Original Parent Maker".as_slice())
        );

        delete_crew_file(&player, "Other.c4i", b"Configured Delete Maker").expect("delete crew");
        let parent = Group::open(&player).expect("open rewritten parent");
        assert!(!parent.exists("Other.c4i"));
        assert_eq!(
            parent.maker_bytes(),
            Some(b"Configured Delete Maker".as_slice())
        );
    }

    #[test]
    fn crew_filename_sanitizer_matches_make_filename_from_title() {
        assert_eq!(crew_filename_from_title(b"  !Bad.Name?  "), b"BadName.c4i");
        assert_eq!(
            crew_filename_from_title(b"!\"\xa7%&/=?+*#:;<>\\."),
            b"unnamed.c4i"
        );
        assert_eq!(crew_filename_from_title(b"A  B"), b"A  B.c4i");
    }
}
