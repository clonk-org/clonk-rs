use std::fs;
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};

use lc_core::std_config::Config;
use lc_engine::player_file::PlayerFile;
use lc_frontend::{startup_plrsel::PlrSelPlayer, ImageData};
use lc_platform::AppPaths;
use lc_resources::Group;
use png::{ColorType, Transformations};

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
    let player_path = config
        .get_in(Some("General"), "PlayerPath")
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default();
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
            player_file.name.clone()
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
    if player.pref_color_dw != 0 {
        return player.pref_color_dw & 0x00ff_ffff;
    }
    const PLAYER_COLORS: [u32; 12] = [
        0x0000e8, 0xf40000, 0x00c800, 0xfcf41c, 0xc48444, 0x784830, 0xa04400, 0xf08050, 0x848484,
        0xffffff, 0x0094f8, 0xbc00c0,
    ];
    usize::try_from(player.pref_color)
        .ok()
        .and_then(|index| PLAYER_COLORS.get(index))
        .copied()
        .unwrap_or(0xaaaaaa)
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
        let text = String::from_utf8_lossy(&bytes);
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
}
