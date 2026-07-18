use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

use lc_core::std_config::Config;
use lc_engine::player_file::PlayerFile;
use lc_frontend::{startup_plrsel::PlrSelPlayer, ImageData};
use lc_platform::AppPaths;
use lc_resources::{Group, MutableGroup};
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

/// Typed failures from player name validation and `.c4p` persistence.
#[derive(Debug, thiserror::Error)]
pub enum PlayerPropertiesSaveError {
    #[error("You must specify a player name!")]
    EmptyName,
    #[error("{name} is already taken")]
    NameTaken { name: String, path: PathBuf },
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("failed to rewrite player group: {0}")]
    Group(String),
    #[error("failed to encode player image: {0}")]
    Image(String),
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
    let system_name = lc_resources::decode_legacy_script_text(&lc_script::c4_string_bytes(name));
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
        let metadata = PlayerRenderMetadata::load(&group);
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
            lc_resources::decode_legacy_script_text(&lc_script::c4_string_bytes(&player_file.name))
        };
        let render_model = PlrSelPlayer {
            name: render_name,
            activated,
            big_icon: load_group_png(&group, "BigIcon.png"),
            portrait: load_group_png(&group, "Portrait.png"),
            color_dw: normalized_player_color(&player_file),
            score: metadata.score,
            rounds: metadata.rounds,
            rounds_won: metadata.rounds_won,
            rounds_lost: metadata.rounds_lost,
            total_playing_time: metadata.total_playing_time,
            comment: metadata.comment,
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
pub fn persist_activations(config_path: &Path, players: &[StartupPlayerFile]) -> io::Result<()> {
    let mut config = match Config::load(config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error),
    };
    let mut participant_keys = Vec::new();
    let mut participants = Vec::new();
    for player in players
        .iter()
        .filter(|player| player.render_model.activated)
    {
        let key = player.file_name.to_ascii_lowercase();
        if participant_keys.iter().any(|known| known == &key) {
            continue;
        }
        participant_keys.push(key);
        participants.push(player.file_name.as_str());
    }
    config.set_in(Some("General"), "Participants", participants.join(";"));
    if let Some(parent) = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    config.save(config_path)
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
) -> Result<SavedStartupPlayer, PlayerPropertiesSaveError> {
    let config = match Config::load(paths.config_file()) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
        Err(error) => return Err(error.into()),
    };
    save_player_properties_in(
        paths.install_root(),
        &config,
        existing_path,
        player,
        comment,
        portrait,
        big_icon,
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

    if let Some(occupant) = find_case_insensitive_entry(&parent, &filename)? {
        let owns_occupant =
            existing_path.is_some_and(|existing| paths_identify_same_item(existing, &occupant));
        if !owns_occupant {
            return Err(PlayerPropertiesSaveError::NameTaken {
                name: player.name.clone(),
                path: occupant,
            });
        }
    }

    let encoded_portrait = encode_image_write(portrait)?;
    let encoded_big_icon = encode_image_write(big_icon)?;

    if let Some(existing) = existing_path {
        if existing != target {
            fs::rename(existing, &target)?;
        }
        let source = Group::open(&target).map_err(|error| {
            PlayerPropertiesSaveError::Group(format!("open {}: {error}", target.display()))
        })?;
        let original_core = source.read_file("Player.txt").ok();
        let core = rewrite_player_core(original_core.as_deref(), player, comment);
        if source.is_directory() {
            replace_directory_file(&target, "Player.txt", Some(&core))?;
            replace_directory_file(&target, "C4Player.c4b", None)?;
            apply_directory_image(&target, "Portrait.png", &encoded_portrait)?;
            apply_directory_image(&target, "BigIcon.png", &encoded_big_icon)?;
        } else {
            let mut mutable = MutableGroup::from_group(&source)
                .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))?;
            mutable.remove_entry("Player.txt");
            mutable
                .add_file("Player.txt", core)
                .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))?;
            mutable.remove_entry("C4Player.c4b");
            apply_packed_image(&mut mutable, "Portrait.png", &encoded_portrait)?;
            apply_packed_image(&mut mutable, "BigIcon.png", &encoded_big_icon)?;
            let bytes = mutable
                .pack()
                .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))?;
            fs::write(&target, bytes)?;
        }
    } else {
        fs::create_dir_all(&parent)?;
        let mut mutable = MutableGroup::new(filename.clone());
        mutable
            .add_file("Player.txt", rewrite_player_core(None, player, comment))
            .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))?;
        apply_packed_image(&mut mutable, "Portrait.png", &encoded_portrait)?;
        apply_packed_image(&mut mutable, "BigIcon.png", &encoded_big_icon)?;
        let bytes = mutable
            .pack()
            .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))?;
        fs::write(&target, bytes)?;
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

fn encode_image_write(
    update: &PlayerImageWrite,
) -> Result<EncodedImageWrite, PlayerPropertiesSaveError> {
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
                .ok_or_else(|| {
                    PlayerPropertiesSaveError::Image("image dimensions overflow".to_string())
                })?;
            if image.pixels().len() != expected {
                return Err(PlayerPropertiesSaveError::Image(format!(
                    "RGBA image has {} bytes, expected {expected}",
                    image.pixels().len()
                )));
            }
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, image.width(), image.height());
                encoder.set_color(ColorType::Rgba);
                encoder.set_depth(BitDepth::Eight);
                let mut writer = encoder
                    .write_header()
                    .map_err(|error| PlayerPropertiesSaveError::Image(error.to_string()))?;
                writer
                    .write_image_data(image.pixels())
                    .map_err(|error| PlayerPropertiesSaveError::Image(error.to_string()))?;
                writer
                    .finish()
                    .map_err(|error| PlayerPropertiesSaveError::Image(error.to_string()))?;
            }
            Ok(EncodedImageWrite::Replace(bytes))
        }
    }
}

fn apply_packed_image(
    group: &mut MutableGroup,
    name: &str,
    update: &EncodedImageWrite,
) -> Result<(), PlayerPropertiesSaveError> {
    match update {
        EncodedImageWrite::Keep => Ok(()),
        EncodedImageWrite::Clear => {
            group.remove_entry(name);
            Ok(())
        }
        EncodedImageWrite::Replace(bytes) => {
            group.remove_entry(name);
            group
                .add_file(name, bytes.clone())
                .map_err(|error| PlayerPropertiesSaveError::Group(error.to_string()))
        }
    }
}

fn apply_directory_image(
    directory: &Path,
    name: &str,
    update: &EncodedImageWrite,
) -> Result<(), PlayerPropertiesSaveError> {
    match update {
        EncodedImageWrite::Keep => Ok(()),
        EncodedImageWrite::Clear => replace_directory_file(directory, name, None),
        EncodedImageWrite::Replace(bytes) => replace_directory_file(directory, name, Some(bytes)),
    }
}

fn replace_directory_file(
    directory: &Path,
    name: &str,
    replacement: Option<&[u8]>,
) -> Result<(), PlayerPropertiesSaveError> {
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
        ("Mouse", i32::from(player.pref_mouse).to_string()),
        (
            "AutoStopControl",
            i32::from(player.pref_control_style).to_string(),
        ),
        (
            "AutoContextMenu",
            i32::from(player.pref_auto_context_menu).to_string(),
        ),
    ];
    let source = original
        .map(lc_script::c4_string_from_bytes)
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
            let name = trimmed[1..trimmed.len() - 1].trim();
            section = if name.eq_ignore_ascii_case("Player") {
                player_seen = true;
                CoreSection::Player
            } else if name.eq_ignore_ascii_case("Preferences") {
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
            if let Some(index) = values
                .iter()
                .position(|(known, _)| known.eq_ignore_ascii_case(key.trim()))
            {
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
    lc_script::c4_string_bytes(&text)
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

#[derive(Default)]
struct PlayerRenderMetadata {
    comment: String,
    score: i32,
    rounds: i32,
    rounds_won: i32,
    rounds_lost: i32,
    total_playing_time: i32,
}

impl PlayerRenderMetadata {
    fn load(group: &Group) -> Self {
        let Ok(bytes) = group.read_file("Player.txt") else {
            return Self::default();
        };
        let text = lc_resources::decode_legacy_script_text(&bytes);
        let mut metadata = Self::default();
        let mut in_player_section = false;
        for raw_line in text.lines() {
            let line = raw_line.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_player_section = line[1..line.len() - 1]
                    .trim()
                    .eq_ignore_ascii_case("Player");
                continue;
            }
            if !in_player_section {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let value = unquote(value.trim());
            if key.eq_ignore_ascii_case("Comment") {
                metadata.comment = value.to_string();
            } else if key.eq_ignore_ascii_case("Score") {
                metadata.score = parse_leading_i32(value).unwrap_or_default();
            } else if key.eq_ignore_ascii_case("Rounds") {
                metadata.rounds = parse_leading_i32(value).unwrap_or_default();
            } else if key.eq_ignore_ascii_case("RoundsWon") {
                metadata.rounds_won = parse_leading_i32(value).unwrap_or_default();
            } else if key.eq_ignore_ascii_case("RoundsLost") {
                metadata.rounds_lost = parse_leading_i32(value).unwrap_or_default();
            } else if key.eq_ignore_ascii_case("TotalPlayingTime") {
                metadata.total_playing_time = parse_leading_i32(value).unwrap_or_default();
            }
        }
        metadata
    }
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
}

fn parse_leading_i32(value: &str) -> Option<i32> {
    let value = value.trim_start();
    let digits = value
        .char_indices()
        .take_while(|(index, character)| {
            character.is_ascii_digit() || (*index == 0 && matches!(character, '+' | '-'))
        })
        .last()
        .map(|(index, character)| index + character.len_utf8())?;
    value[..digits].parse().ok()
}

fn load_group_png(group: &Group, name: &str) -> Option<ImageData> {
    let bytes = group.read_file(name).ok()?;
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()];
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

#[cfg(test)]
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
            lc_script::c4_string_bytes(&players[0].player_file.name),
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

        let config_path = install.path().join("Config/legacyclonk.config");
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

        persist_activations(&config_path, &players).expect("save activation");

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
            player_group_filename(&lc_script::c4_string_from_bytes(b"Andr\xe9"))
                .expect("native name conversion"),
            "André.c4p"
        );

        let root = tempdir().expect("player root");
        let mut config = Config::new();
        config.set_in(Some("General"), "PlayerPath", "Players");
        let players = root.path().join("Players");
        fs::create_dir_all(players.join("Taken.c4p")).expect("taken player");
        let mut core = PlayerFile::default();
        core.name = "Taken".to_string();
        assert!(matches!(
            save_player_properties_in(
                root.path(),
                &config,
                None,
                &core,
                "",
                &PlayerImageWrite::Keep,
                &PlayerImageWrite::Keep,
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
            lc_script::c4_string_from_bytes(&edited.read_file("Player.txt").expect("core"))
                .contains("RankName=Captain")
        );
    }
}
