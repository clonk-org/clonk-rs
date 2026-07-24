use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clonk_app_core::native_config::{
    decode_general_config_string, trim_ascii, trim_horizontal_start, CFG_MAX_STRING,
};
use clonk_engine::{player_file::PlayerFile, LegacyCString};
use clonk_network::HostInitialResourceSource;
use clonk_platform::AppPaths;
use thiserror::Error;

use crate::resource_path_identity::{
    open_group_path, opened_group_name, path_from_wire_bytes, path_wire_bytes,
};
use crate::SelectedClientPlayer;

pub use clonk_app_core::native_config::{
    configured_native_boolean, configured_native_dynamic_value, configured_native_scalar,
    configured_native_value, update_configured_native_values, NativeConfigValue,
};

const C4_MAX_NAME: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredClientPlayers {
    players: Vec<SelectedClientPlayer>,
    group_maker: LegacyCString,
}

/// The raw `C4Game::PlayerFilenames`/maker values frozen before networking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredClientPlayerSelection {
    participants: Vec<u8>,
    group_maker: LegacyCString,
}

impl ConfiguredClientPlayerSelection {
    pub fn group_maker(&self) -> &LegacyCString {
        &self.group_maker
    }

    /// Replaces the configured participant modules with command-line modules.
    ///
    /// Classic feeds `.c4p` arguments through case-insensitive `SAddModule`,
    /// preserving the first occurrence in argument order and using the same
    /// semicolon-separated format as `Config.General.Participants`. The
    /// configured maker remains the owner of the resulting player info list.
    pub fn replace_participant_modules(&mut self, modules: &[PathBuf]) {
        let mut unique_modules: Vec<Vec<u8>> = Vec::new();
        for module in modules {
            let module = path_bytes(module);
            if module.is_empty()
                || unique_modules
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(&module))
            {
                continue;
            }
            unique_modules.push(module);
        }

        let mut participants = Vec::new();
        for (index, module) in unique_modules.iter().enumerate() {
            if index != 0 {
                participants.push(b';');
            }
            participants.extend_from_slice(module);
        }
        self.participants = participants;
    }
}

impl ConfiguredClientPlayers {
    #[cfg(test)]
    pub(crate) fn from_parts(
        players: Vec<SelectedClientPlayer>,
        group_maker: LegacyCString,
    ) -> Self {
        Self {
            players,
            group_maker,
        }
    }

    pub fn players(&self) -> &[SelectedClientPlayer] {
        &self.players
    }

    pub fn group_maker(&self) -> &LegacyCString {
        &self.group_maker
    }

    pub fn host_initial_resource_sources(&self) -> Vec<HostInitialResourceSource> {
        self.players
            .iter()
            .map(|player| HostInitialResourceSource {
                path: player.source_path().to_path_buf(),
                lookup_name: player.resource_lookup_name().clone(),
                opened_name: player.resource_opened_name().clone(),
                wire_name: player.resource_wire_name().clone(),
                virtual_group_bytes: None,
            })
            .collect()
    }
}

#[derive(Debug, Error)]
pub enum ConfiguredClientPlayersError {
    #[error("failed to read client configuration: {0}")]
    Config(#[from] io::Error),
}

pub fn load_configured_client_players(
    paths: &AppPaths,
) -> Result<ConfiguredClientPlayers, ConfiguredClientPlayersError> {
    let selection = snapshot_configured_client_player_selection(paths)?;
    Ok(load_snapshotted_client_players(paths, &selection))
}

pub fn load_configured_mission_access(
    paths: &AppPaths,
) -> Result<String, ConfiguredClientPlayersError> {
    load_configured_mission_access_from_path(&paths.config_file())
}

fn load_configured_mission_access_from_path(
    config_path: &Path,
) -> Result<String, ConfiguredClientPlayersError> {
    let config = match fs::read(config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    Ok(clonk_script::c4_string_from_bytes(
        &raw_general_config(&config).mission_access,
    ))
}

pub fn snapshot_configured_client_player_selection(
    paths: &AppPaths,
) -> Result<ConfiguredClientPlayerSelection, ConfiguredClientPlayersError> {
    snapshot_configured_client_player_selection_from_path(&paths.config_file())
}

fn snapshot_configured_client_player_selection_from_path(
    config_path: &Path,
) -> Result<ConfiguredClientPlayerSelection, ConfiguredClientPlayersError> {
    let config = match fs::read(config_path) {
        Ok(config) => config,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(error.into()),
    };
    let general = raw_general_config(&config);
    Ok(ConfiguredClientPlayerSelection {
        participants: general.participants,
        group_maker: legacy_string(&general.name),
    })
}

pub fn load_snapshotted_client_players(
    paths: &AppPaths,
    selection: &ConfiguredClientPlayerSelection,
) -> ConfiguredClientPlayers {
    let roots = [
        paths.install_root().to_path_buf(),
        paths.install_root().join("build"),
        paths.install_root().join("build-arm64-native"),
    ];
    load_client_players_from_selection(selection, &roots)
}

#[cfg(test)]
fn load_configured_client_players_from_roots(
    config_path: &Path,
    exe_roots: &[PathBuf],
) -> Result<ConfiguredClientPlayers, ConfiguredClientPlayersError> {
    let config = fs::read(config_path)?;
    let general = raw_general_config(&config);
    let selection = ConfiguredClientPlayerSelection {
        participants: general.participants,
        group_maker: legacy_string(&general.name),
    };
    Ok(load_client_players_from_selection(&selection, exe_roots))
}

fn load_client_players_from_selection(
    selection: &ConfiguredClientPlayerSelection,
    exe_roots: &[PathBuf],
) -> ConfiguredClientPlayers {
    let players = split_modules(&selection.participants)
        .filter_map(|module| load_module(module, exe_roots))
        .collect();
    ConfiguredClientPlayers {
        players,
        group_maker: selection.group_maker.clone(),
    }
}

fn load_module(module: &[u8], exe_roots: &[PathBuf]) -> Option<SelectedClientPlayer> {
    let module_filename = LegacyCString::from_bytes(module.to_vec())?;
    let module_path = path_from_bytes(module);
    let module_is_absolute = module_path.is_absolute();
    let (group, executable_root) = if module_is_absolute {
        let group = open_group_path(&module_path).ok()?;
        let executable_root = matching_executable_root(&module_path, exe_roots)
            .unwrap_or_else(|| Path::new(""))
            .to_path_buf();
        (group, executable_root)
    } else {
        exe_roots.iter().find_map(|root| {
            open_group_path(&root.join(&module_path))
                .ok()
                .map(|group| (group, root.clone()))
        })?
    };
    let resource_name = if module_is_absolute {
        exe_relative_name(&module_path, module, exe_roots)
    } else {
        module.to_vec()
    };
    let resource_lookup_name = LegacyCString::from_bytes(resource_name.clone())?;
    let resource_opened_name_bytes =
        opened_group_name(group.root(), &resource_name, &executable_root);
    let resource_wire_name = resource_lookup_name.clone();
    let resource_opened_name = LegacyCString::from_bytes(resource_opened_name_bytes)?;
    let player_file = PlayerFile::load(&group).ok()?;
    let player_text = group.read_file("Player.txt").ok()?;
    let player_name = player_name_from_core(&player_text);
    let (network_color, alternate_color) = player_colors_from_core(&player_text);
    Some(SelectedClientPlayer::from_configured(
        group.root().to_path_buf(),
        module_filename,
        resource_lookup_name,
        resource_wire_name,
        resource_opened_name,
        legacy_string(&player_name),
        network_color,
        alternate_color,
        player_file,
    ))
}

fn matching_executable_root<'a>(source_path: &Path, exe_roots: &'a [PathBuf]) -> Option<&'a Path> {
    exe_roots
        .iter()
        .filter(|root| source_path.strip_prefix(root).is_ok())
        .max_by_key(|root| root.components().count())
        .map(PathBuf::as_path)
}

fn exe_relative_name(source_path: &Path, module: &[u8], exe_roots: &[PathBuf]) -> Vec<u8> {
    matching_executable_root(source_path, exe_roots)
        .and_then(|root| source_path.strip_prefix(root).ok())
        .map(path_bytes)
        .unwrap_or_else(|| module.to_vec())
}

struct RawGeneralConfig {
    participants: Vec<u8>,
    name: Vec<u8>,
    mission_access: Vec<u8>,
}

fn raw_general_config(config: &[u8]) -> RawGeneralConfig {
    let mut in_general = false;
    let mut selected_general = false;
    let mut participants = None;
    let mut name = None;
    let mut mission_access = None;
    for raw_line in config.split(|byte| *byte == b'\n') {
        let line = raw_line
            .split(|byte| *byte == b'\r')
            .next()
            .unwrap_or_default();
        let structural = trim_ascii(line);
        if structural.starts_with(b"[") && structural.ends_with(b"]") {
            if in_general {
                break;
            }
            let is_general = &structural[1..structural.len() - 1] == b"General";
            in_general = is_general && !selected_general;
            selected_general |= is_general;
            continue;
        }
        if !in_general || structural.starts_with(b"#") || structural.starts_with(b";") {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = trim_ascii(&line[..equals]);
        if participants.is_none() && key == b"Participants" {
            participants = Some(decode_general_config_string(
                &line[equals + 1..],
                CFG_MAX_STRING,
            ));
        } else if name.is_none() && key == b"Name" {
            name = Some(decode_general_config_string(
                &line[equals + 1..],
                CFG_MAX_STRING,
            ));
        } else if mission_access.is_none() && key == b"MissionAccess" {
            mission_access = Some(decode_general_config_string(
                &line[equals + 1..],
                CFG_MAX_STRING,
            ));
        }
    }
    RawGeneralConfig {
        participants: participants.unwrap_or_default(),
        name: name.unwrap_or_default(),
        mission_access: mission_access.unwrap_or_default(),
    }
}

fn split_modules(modules: &[u8]) -> impl Iterator<Item = &[u8]> {
    let capacity = modules
        .iter()
        .take_while(|byte| **byte != 0)
        .fold((0_usize, true), |(count, new_module), byte| match byte {
            b' ' => (count, new_module),
            b';' => (count, true),
            _ => (count + usize::from(new_module), false),
        })
        .0;
    modules
        .split(|byte| *byte == b';')
        .map(trim_spaces)
        .take(capacity)
}

fn player_name_from_core(player_text: &[u8]) -> Vec<u8> {
    let mut in_player = false;
    let mut selected_player = false;
    let mut name = None;
    for raw_line in player_text.split(|byte| *byte == b'\n') {
        let raw_line = raw_line
            .strip_prefix(&[0xef, 0xbb, 0xbf])
            .unwrap_or(raw_line);
        let line = raw_line
            .split(|byte| *byte == b'\r')
            .next()
            .unwrap_or_default();
        let structural = trim_ascii(line);
        if structural.starts_with(b"[") && structural.ends_with(b"]") {
            if in_player {
                break;
            }
            let is_player = &structural[1..structural.len() - 1] == b"Player";
            in_player = is_player && !selected_player;
            selected_player |= is_player;
            continue;
        }
        if !in_player || structural.starts_with(b"#") || structural.starts_with(b";") {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        if name.is_none() && trim_ascii(&line[..equals]) == b"Name" {
            name = Some(decode_cpp_all_string(&line[equals + 1..], C4_MAX_NAME));
        }
    }
    strip_c4_markup(name.as_deref().unwrap_or(b"Neuling"))
}

fn player_colors_from_core(player_text: &[u8]) -> (u32, u32) {
    let mut in_preferences = false;
    let mut selected_preferences = false;
    let mut pref_color = None;
    let mut pref_color_dw = None;
    let mut pref_color2_dw = None;
    for raw_line in player_text.split(|byte| *byte == b'\n') {
        let line = raw_line
            .split(|byte| *byte == b'\r')
            .next()
            .unwrap_or_default();
        let structural = trim_ascii(line);
        if structural.starts_with(b"[") && structural.ends_with(b"]") {
            if in_preferences {
                break;
            }
            let is_preferences = &structural[1..structural.len() - 1] == b"Preferences";
            in_preferences = is_preferences && !selected_preferences;
            selected_preferences |= is_preferences;
            continue;
        }
        if !in_preferences || structural.starts_with(b"#") || structural.starts_with(b";") {
            continue;
        }
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let key = trim_ascii(&line[..equals]);
        if key == b"Color" && pref_color.is_none() {
            pref_color = Some(parse_cpp_u32(&line[equals + 1..]).unwrap_or(0));
        } else if key == b"ColorDw" && pref_color_dw.is_none() {
            pref_color_dw = Some(parse_cpp_u32(&line[equals + 1..]).unwrap_or(0));
        } else if key == b"AlternateColorDw" && pref_color2_dw.is_none() {
            pref_color2_dw = Some(parse_cpp_u32(&line[equals + 1..]).unwrap_or(0));
        }
    }
    let pref_color_dw = pref_color_dw.unwrap_or(0xff);
    let network_color = if pref_color_dw == 0 {
        cpp_preferred_color_value(pref_color.unwrap_or(0))
    } else {
        pref_color_dw & 0x00ff_ffff
    };
    (network_color, pref_color2_dw.unwrap_or(0) & 0x00ff_ffff)
}

fn cpp_preferred_color_value(index: u32) -> u32 {
    const PLAYER_COLORS: [u32; 12] = [
        0x0000_00e8,
        0x00f4_0000,
        0x0000_c800,
        0x00fc_f41c,
        0x00c4_8444,
        0x0078_4830,
        0x00a0_4400,
        0x00f0_8050,
        0x0084_8484,
        0x00ff_ffff,
        0x0000_94f8,
        0x00bc_00c0,
    ];

    usize::try_from(index)
        .ok()
        .and_then(|index| PLAYER_COLORS.get(index))
        .copied()
        .unwrap_or(0x00aa_aaaa)
}

fn parse_cpp_u32(value: &[u8]) -> Option<u32> {
    let value = trim_ascii(value);
    let (negative, value) = match value.first() {
        Some(b'-') => (true, &value[1..]),
        Some(b'+') => (false, &value[1..]),
        _ => (false, value),
    };
    let (radix, digits) = if value.starts_with(b"0x") || value.starts_with(b"0X") {
        (16_u32, &value[2..])
    } else {
        (10_u32, value)
    };
    let mut parsed = 0_u64;
    let mut consumed = false;
    for byte in digits.iter().copied() {
        let digit = match byte {
            b'0'..=b'9' => u32::from(byte - b'0'),
            b'a'..=b'f' if radix == 16 => u32::from(byte - b'a') + 10,
            b'A'..=b'F' if radix == 16 => u32::from(byte - b'A') + 10,
            _ => break,
        };
        if digit >= radix {
            break;
        }
        parsed = parsed
            .wrapping_mul(u64::from(radix))
            .wrapping_add(u64::from(digit));
        consumed = true;
    }
    consumed.then(|| {
        if negative {
            0_u32.wrapping_sub(parsed as u32)
        } else {
            parsed as u32
        }
    })
}

fn decode_cpp_all_string(value: &[u8], max_length: usize) -> Vec<u8> {
    trim_horizontal_start(value)
        .iter()
        .copied()
        .take_while(|byte| *byte != 0)
        .take(max_length)
        .collect()
}

fn strip_c4_markup(input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'<' {
            if let Some(close) = input[index + 1..].iter().position(|byte| *byte == b'>') {
                let close = index + close + 1;
                if valid_markup_tag(&input[index + 1..close]) {
                    index = close + 1;
                    continue;
                }
            }
        }
        if input[index..].starts_with(b"{{")
            && input.get(index + 2).is_some_and(|byte| *byte != b'{')
        {
            if let Some(close) = input[index + 2..]
                .windows(2)
                .position(|window| window == b"}}")
            {
                index += close + 4;
                continue;
            }
            index += 2;
            continue;
        }
        if input[index..].starts_with(b"}}") {
            index += 2;
            continue;
        }
        output.push(input[index]);
        index += 1;
    }
    output
}

fn valid_markup_tag(tag: &[u8]) -> bool {
    if tag == b"i" || (tag.starts_with(b"/") && !tag.contains(&b' ')) {
        return true;
    }
    let Some(color) = tag.strip_prefix(b"c ") else {
        return false;
    };
    color.len() <= 8
}

fn legacy_string(bytes: &[u8]) -> LegacyCString {
    let bytes = bytes
        .split(|byte| *byte == 0)
        .next()
        .unwrap_or_default()
        .to_vec();
    LegacyCString::from_bytes(bytes).unwrap_or_default()
}

fn trim_spaces(value: &[u8]) -> &[u8] {
    let start = value
        .iter()
        .position(|byte| *byte != b' ')
        .unwrap_or(value.len());
    let end = value
        .iter()
        .rposition(|byte| *byte != b' ')
        .map_or(start, |index| index + 1);
    &value[start..end]
}

fn path_from_bytes(bytes: &[u8]) -> PathBuf {
    path_from_wire_bytes(bytes)
}

fn path_bytes(path: &Path) -> Vec<u8> {
    path_wire_bytes(path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use clonk_resources::MutableGroup;
    use tempfile::tempdir;

    #[test]
    fn command_line_participant_modules_replace_config_in_argument_order() {
        let directory = tempdir().expect("absolute module directory");
        let modules = vec![
            PathBuf::from("Players/Bravo.c4p"),
            directory.path().join("Absolute.c4p"),
            PathBuf::from("players/BRAVO.C4P"),
            PathBuf::from("Players/Alpha.c4p"),
        ];
        let mut selection = super::ConfiguredClientPlayerSelection {
            participants: b"Players/Configured.c4p".to_vec(),
            group_maker: super::legacy_string(b"Configured maker"),
        };

        selection.replace_participant_modules(&modules);

        let split = super::split_modules(&selection.participants)
            .map(<[u8]>::to_vec)
            .collect::<Vec<_>>();
        let expected = [&modules[0], &modules[1], &modules[3]]
            .into_iter()
            .map(|path| super::path_bytes(path))
            .collect::<Vec<_>>();
        assert_eq!(split, expected);
        assert_eq!(
            selection
                .participants
                .iter()
                .filter(|byte| **byte == b';')
                .count(),
            expected.len() - 1
        );
    }

    #[test]
    fn command_line_participant_modules_preserve_configured_group_maker() {
        let mut selection = super::ConfiguredClientPlayerSelection {
            participants: b"Configured.c4p".to_vec(),
            group_maker: super::legacy_string(b"Maker\twith raw bytes \x81"),
        };

        selection.replace_participant_modules(&[PathBuf::from("CommandLine.c4p")]);

        assert_eq!(
            selection.group_maker().as_bytes(),
            b"Maker\twith raw bytes \x81"
        );
    }

    #[test]
    fn configured_modules_load_directly_in_order_without_deduplication() {
        // Game.PlayerFilenames retains Config.General.Participants verbatim;
        // C4ClientPlayerInfos then walks every semicolon module in order and
        // opens that module directly, including nested paths and duplicates
        // (pristine 9ffa0a5d src/C4Game.cpp:361-364;
        // src/C4PlayerInfo.cpp:357-395; src/C4Strings.cpp:435-440).
        let install = tempdir().expect("install root");
        let nested = install.path().join("Players/Deep/Bravo.c4p");
        let alpha = install.path().join("Players/Alpha.c4p");
        write_player(&nested, b"Bravo");
        write_player(&alpha, b"Alpha");
        let config = install.path().join("clonk-rust.config");
        fs::write(
            &config,
            b"[General]\nName=\"Maker\"\nParticipants=\"Players/Deep/Bravo.c4p;Players/Alpha.c4p;Players/Deep/Bravo.c4p\"\n",
        )
        .expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured players");

        assert_eq!(
            loaded
                .players()
                .iter()
                .map(|player| player.module_filename().as_bytes())
                .collect::<Vec<_>>(),
            vec![
                b"Players/Deep/Bravo.c4p".as_slice(),
                b"Players/Alpha.c4p".as_slice(),
                b"Players/Deep/Bravo.c4p".as_slice(),
            ]
        );
        assert_eq!(loaded.players()[0].source_path(), nested);
        assert_eq!(loaded.players()[1].source_path(), alpha);
        assert_eq!(
            loaded
                .players()
                .iter()
                .map(|player| player.player_name().as_bytes())
                .collect::<Vec<_>>(),
            vec![
                b"Bravo".as_slice(),
                b"Alpha".as_slice(),
                b"Bravo".as_slice()
            ]
        );
    }

    #[test]
    fn teamless_offline_initial_packet_skips_unreadable_keeps_duplicates_and_assigns_dense_ids() {
        // Offline InitLocal walks every configured module in order, omits
        // unreadable player cores, retains duplicate modules, and immediately
        // assigns dense player IDs subject to the scenario capacity. Without
        // multiple teams, it does not run team assignment (pristine 9ffa0a5d
        // src/C4PlayerInfo.cpp:357-395,781-807,834-875,1273-1290).
        let install = tempdir().expect("install root");
        write_player(&install.path().join("Bravo.c4p"), b"Bravo");
        write_player(&install.path().join("Alpha.c4p"), b"Alpha");
        write_player(&install.path().join("Excess.c4p"), b"Excess");
        let broken = install.path().join("Broken.c4p");
        let mut broken_group = MutableGroup::new("Broken.c4p");
        broken_group
            .add_file_with_metadata("Other.txt", b"not a player".to_vec(), 1, false)
            .expect("add non-player file");
        fs::write(&broken, broken_group.pack().expect("pack broken group"))
            .expect("write broken group");
        let config = install.path().join("clonk-rust.config");
        fs::write(
            &config,
            b"[General]\nParticipants=\"Bravo.c4p;Broken.c4p;Alpha.c4p;Bravo.c4p;Excess.c4p\"\n",
        )
        .expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured players");

        assert_eq!(
            loaded
                .players()
                .iter()
                .map(|player| player.player_name().as_bytes())
                .collect::<Vec<_>>(),
            vec![
                b"Bravo".as_slice(),
                b"Alpha".as_slice(),
                b"Bravo".as_slice(),
                b"Excess".as_slice(),
            ]
        );

        let packet = crate::build_teamless_offline_initial_player_info(&loaded, 3);
        let entry = |name: &[u8], filename: &[u8], id| clonk_engine::ControlPlayerInfoEntry {
            name: clonk_engine::LegacyCString::from_bytes(name.to_vec()).expect("NUL-free name"),
            filename: clonk_engine::LegacyCString::from_bytes(filename.to_vec())
                .expect("NUL-free filename"),
            id,
            color: 0x12_34_56,
            original_color: 0x12_34_56,
            ..Default::default()
        };
        assert_eq!(
            packet,
            clonk_engine::PlayerInfoControlData {
                client_id: 0,
                flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![
                    entry(b"Bravo", b"Bravo.c4p", 1),
                    entry(b"Alpha", b"Alpha.c4p", 2),
                    entry(b"Bravo", b"Bravo.c4p", 3),
                ],
                by_client: 0,
            }
        );
    }

    #[test]
    fn host_resource_sources_keep_cpp_participant_order_and_resource_names() {
        // C4Game copies Config.General.Participants verbatim before
        // C4ClientPlayerInfos walks the modules in that order. Each loaded
        // player publishes AtExeRelativePath(module) as its NRT_Player name;
        // startup file-browser sorting is not involved (pristine 9ffa0a5d
        // src/C4Game.cpp:361-364; src/C4PlayerInfo.cpp:70-104,357-395;
        // src/C4Config.cpp:759-763).
        let install = tempdir().expect("install root");
        let bravo = install.path().join("Players/Bravo.c4p");
        let alpha = install.path().join("Players/Alpha.c4p");
        write_player(&bravo, b"Bravo");
        write_player(&alpha, b"Alpha");
        let config = install.path().join("clonk-rust.config");
        fs::write(
            &config,
            b"[General]\nParticipants=\"Players/Bravo.c4p;Players/Alpha.c4p\"\n",
        )
        .expect("write config");
        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured players");

        let sources = loaded.host_initial_resource_sources();

        assert_eq!(
            sources
                .iter()
                .map(|source| (source.path.as_path(), source.wire_name.as_bytes()))
                .collect::<Vec<_>>(),
            vec![
                (bravo.as_path(), b"Players/Bravo.c4p".as_slice()),
                (alpha.as_path(), b"Players/Alpha.c4p".as_slice()),
            ]
        );
    }

    #[test]
    fn host_resource_sources_retain_the_group_opened_player_alias() {
        // AddByFile first probes the incoming alias, but stores the filename
        // retained by C4Group. A later exact spelling therefore reuses the
        // alias-opened resource (src/C4PlayerInfo.cpp:70-101;
        // src/C4Network2Res.cpp:377-422,1414-1449).
        let install = tempdir().expect("install root");
        let player = install.path().join("Players/Player.c4p");
        write_player(&player, b"Player");
        let config = install.path().join("clonk-rust.config");
        fs::write(
            &config,
            b"[General]\nParticipants=\"Players/P?ayer.c4p;Players/Player.c4p\"\n",
        )
        .expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load aliased players");
        let sources = loaded.host_initial_resource_sources();

        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0].path, player);
        assert_eq!(sources[0].lookup_name.as_bytes(), b"Players/P?ayer.c4p");
        assert_eq!(sources[0].opened_name.as_bytes(), b"Players/Player.c4p");
        assert_eq!(sources[0].wire_name.as_bytes(), b"Players/P?ayer.c4p");
        assert_eq!(sources[1].lookup_name.as_bytes(), b"Players/Player.c4p");
        assert_eq!(sources[1].opened_name.as_bytes(), b"Players/Player.c4p");
    }

    #[test]
    fn empty_module_segment_consumes_cpp_player_capacity() {
        // C4ClientPlayerInfos allocates SModuleCount slots but indexes the raw
        // semicolon segments. A leading empty segment therefore consumes the
        // only slot and the later valid module is never visited (pristine
        // 9ffa0a5d src/C4PlayerInfo.cpp:375-390;
        // src/C4Strings.cpp:435-440,513-525).
        let install = tempdir().expect("install root");
        write_player(&install.path().join("Alpha.c4p"), b"Alpha");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, b"[General]\nParticipants=\";Alpha.c4p\"\n").expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured players");

        assert!(loaded.players().is_empty());
    }

    #[test]
    fn general_fields_are_case_sensitive() {
        // StdCompilerINIRead compares section and value NameNode strings with
        // exact equality (pristine 9ffa0a5d src/StdCompiler.cpp:498-526,
        // 794-857; src/C4Config.cpp:63-74).
        let config = super::raw_general_config(
            b"[general]\nName=\"Wrong section\"\nParticipants=\"WrongSection.c4p\"\n\
[General]\nname=\"Wrong key\"\nparticipants=\"WrongKey.c4p\"\n\
Name=\"Right\"\nParticipants=\"Right.c4p\"\n",
        );

        assert_eq!(config.name, b"Right");
        assert_eq!(config.participants, b"Right.c4p");
    }

    #[test]
    fn first_empty_general_field_wins_over_later_duplicates() {
        // The INI name tree selects the first exact child and an empty value
        // still has a valid position; the node is then consumed rather than
        // replaced by a later duplicate (pristine 9ffa0a5d
        // src/StdCompiler.cpp:498-557,794-857).
        let config = super::raw_general_config(
            b"[General]\nName=\"\"\nName=\"Later\"\nParticipants=\"\"\nParticipants=\"Later.c4p\"\n",
        );

        assert!(config.name.is_empty());
        assert!(config.participants.is_empty());
    }

    #[test]
    fn first_general_section_wins_even_when_a_field_is_absent() {
        // Name("General") selects the first matching section node; the
        // compiler never falls through to a later duplicate section for a
        // missing child (pristine 9ffa0a5d src/StdCompiler.cpp:498-557,
        // 794-857).
        let config = super::raw_general_config(
            b"[General]\nName=\"First\"\n[Other]\nValue=1\n\
[General]\nName=\"Later\"\nParticipants=\"Later.c4p\"\n",
        );

        assert_eq!(config.name, b"First");
        assert!(config.participants.is_empty());
    }

    #[test]
    fn mission_access_preserves_escaped_bytes_and_first_exact_value_with_cpp_cap() {
        let config = super::raw_general_config(
            b"[general]\nMissionAccess=\"Wrong section\"\n\
[General]\nmissionaccess=\"Wrong key\"\n\
MissionAccess=\"M\\151ss\\x80\"\nMissionAccess=\"Later key\"\n\
[Other]\nValue=1\n[General]\nMissionAccess=\"Later section\"\n",
        );

        assert_eq!(config.mission_access, b"Miss\x80");

        let mut capped = b"[General]\nMissionAccess=\"".to_vec();
        capped.extend(std::iter::repeat_n(b'A', super::CFG_MAX_STRING + 6));
        capped.extend_from_slice(b"\"\n");
        let capped = super::raw_general_config(&capped);
        assert_eq!(capped.mission_access, vec![b'A'; super::CFG_MAX_STRING]);

        let directory = tempdir().expect("config directory");
        let config_path = directory.path().join("clonk-rust.config");
        fs::write(
            &config_path,
            b"[General]\nMissionAccess=\"M\\151ss\\x80\"\n",
        )
        .expect("write config");
        let loaded = super::load_configured_mission_access_from_path(&config_path)
            .expect("mission access loads");
        assert_eq!(clonk_script::c4_string_bytes(&loaded), b"Miss\x80");
    }

    #[test]
    fn missing_config_snapshots_cpp_empty_player_selection() {
        // C4Config initializes fixed General.Name/Participants buffers empty;
        // a missing persisted config therefore still permits a zero-player
        // client join (pristine 9ffa0a5d src/C4Config.cpp:48-75;
        // src/C4Network2Players.cpp:38-49,124-136).
        let directory = tempdir().expect("config directory");
        let selection = super::snapshot_configured_client_player_selection_from_path(
            &directory.path().join("missing.config"),
        )
        .expect("missing config uses C++ defaults");

        assert!(selection.participants.is_empty());
        assert!(selection.group_maker.as_bytes().is_empty());
        assert!(super::load_configured_mission_access_from_path(
            &directory.path().join("missing.config")
        )
        .expect("missing config uses empty mission access")
        .is_empty());
    }

    #[test]
    fn unquoted_general_fields_recover_values_written_by_this_rust_port() {
        // Compatibility only, not C++ parity: fixed-buffer RCT_Escaped does
        // not have the std::string RCT_All fallback (pristine 9ffa0a5d
        // src/StdCompiler.cpp:726-743), but the current Rust writer emits
        // whitespace-free values without quotes (rust/crates/clonk-core/src/
        // std_config.rs:194-233; startup_player_files.rs:167-188).
        let config = super::raw_general_config(
            b"[General]\nName = RustMaker\nParticipants = Players/Alice.c4p\n",
        );

        assert_eq!(config.name, b"RustMaker");
        assert_eq!(config.participants, b"Players/Alice.c4p");
    }

    #[test]
    fn escaped_general_fields_follow_cpp_string_decoding() {
        // mkStringAdaptM uses RCT_Escaped; quoted config strings decode C/C++
        // escapes, including control, quote, slash, octal, and hexadecimal
        // forms (pristine 9ffa0a5d src/StdAdaptors.h:182-202;
        // src/StdCompiler.cpp:726-743,903-1049).
        let install = tempdir().expect("install root");
        let player = install.path().join("Players/Alice.c4p");
        write_player(&player, b"Alice");
        let config = install.path().join("clonk-rust.config");
        fs::write(
            &config,
            b"[General]\nName=\"M\\141ker\\t\\\"Q\\\"\\\\end\"\n\
Participants=\"Players\\057Alice.c4\\x70\"\n",
        )
        .expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured player");

        assert_eq!(loaded.group_maker().as_bytes(), b"Maker\t\"Q\"\\end");
        assert_eq!(loaded.players().len(), 1);
        assert_eq!(
            loaded.players()[0].module_filename().as_bytes(),
            b"Players/Alice.c4p"
        );
        assert_eq!(
            loaded.players()[0].resource_wire_name().as_bytes(),
            b"Players/Alice.c4p"
        );
    }

    #[test]
    fn general_fields_are_capped_before_participant_splitting() {
        // Name and Participants are CFG_MaxString+1 arrays adapted with a
        // maximum payload of CFG_MaxString (pristine 9ffa0a5d
        // src/StdConfig.h:19-21; src/C4Config.h:51-56;
        // src/StdAdaptors.h:196-202; src/StdCompiler.cpp:726-731).
        let mut bytes = b"[General]\nName=\"".to_vec();
        bytes.extend(std::iter::repeat_n(b'M', 1_030));
        bytes.extend_from_slice(b"\"\nParticipants=\"");
        bytes.extend(std::iter::repeat_n(b'P', 1_024));
        bytes.extend_from_slice(b";Ignored.c4p\"\n");

        let config = super::raw_general_config(&bytes);

        assert_eq!(config.name.len(), 1_024);
        assert_eq!(config.participants.len(), 1_024);
        assert_eq!(super::split_modules(&config.participants).count(), 1);
    }

    #[test]
    fn configured_player_name_strips_cpp_markup_from_raw_player_core() {
        // C4PlayerInfoCore strips valid angle-bracket markup and inline-image
        // tags before C4PlayerInfo copies PrefName into the synchronized entry
        // (pristine 9ffa0a5d src/C4InfoCore.cpp:103-125;
        // src/StdMarkup.cpp:36-112,131-162; src/C4PlayerInfo.cpp:70-89).
        let install = tempdir().expect("install root");
        let player = install.path().join("Marked.c4p");
        write_player(&player, b"<i>Al</i><c f>ice</c>{{X}}");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, b"[General]\nParticipants=\"Marked.c4p\"\n").expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured player");

        assert_eq!(loaded.players()[0].player_name().as_bytes(), b"Alice");
    }

    #[test]
    fn player_core_names_are_case_sensitive_and_rct_all() {
        // C4PlayerInfoCore names use exact INI NameNode lookup and toC4CStr,
        // which is fixed-buffer RCT_All: quote bytes are ordinary data
        // (pristine 9ffa0a5d src/C4InfoCore.cpp:146-154;
        // src/StdAdaptors.h:30-33,196-203; src/StdCompiler.cpp:498-526,
        // 726-731,936-998).
        assert_eq!(
            super::player_name_from_core(
                b"[player]\nName=Wrong section\n[Player]\nname=Wrong key\nName=\"Right\"\n"
            ),
            b"\"Right\""
        );
    }

    #[test]
    fn network_color_maps_zero_colordw_through_exact_pref_color() {
        // C4PlayerInfoCore::Load maps zero ColorDw through
        // GetPrefColorValue after exact-name INI compilation (pristine
        // 9ffa0a5d src/C4InfoCore.cpp:90-100,103-121,164-173).
        assert_eq!(
            super::player_colors_from_core(b"[Preferences]\nColor=3\nColorDw=0\n").0,
            0x00fc_f41c
        );
    }

    #[test]
    fn host_local_alternate_color_uses_exact_key_and_24_bit_mask() {
        // C4PlayerInfoCore compiles AlternateColorDw with exact INI names,
        // defaults it to zero and masks alpha before C4PlayerInfo retains it
        // as the non-synchronized conflict-resolution fallback.
        assert_eq!(
            super::player_colors_from_core(
                b"[Preferences]\nColorDw=1193046\nAlternateColorDw=4289449455\n"
            ),
            (0x0012_3456, 0x00ab_cdef)
        );
        assert_eq!(
            super::player_colors_from_core(
                b"[preferences]\nAlternateColorDw=11259375\n\
                  [Preferences]\nalternatecolordw=11259375\n"
            )
            .1,
            0
        );
    }

    #[test]
    fn network_color_ignores_wrong_case_player_core_names() {
        // StdCompilerINIRead matches section and value names exactly, and
        // C4PlayerInfo copies PrefColorDw's default 0xff directly into both
        // synchronized color fields (pristine 9ffa0a5d
        // src/StdCompiler.cpp:498-526; src/C4InfoCore.cpp:148-172;
        // src/C4PlayerInfo.cpp:70-89).
        let install = tempdir().expect("install root");
        let player = install.path().join("Alice.c4p");
        let mut group = MutableGroup::new("Alice.c4p");
        group
            .add_file_with_metadata(
                "Player.txt",
                b"[Player]\nName=Alice\n[preferences]\ncolordw=1193046\n".to_vec(),
                1,
                false,
            )
            .expect("add Player.txt");
        fs::write(&player, group.pack().expect("pack player")).expect("write player");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, b"[General]\nParticipants=\"Alice.c4p\"\n").expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured player");
        let request = loaded.players()[0]
            .initial_player_info_update(7, clonk_engine::NetworkResourceCore::default())
            .expect("build player info");

        assert_eq!(request.players[0].color, 0xff);
        assert_eq!(request.players[0].original_color, 0xff);
    }

    #[test]
    fn player_core_name_is_capped_before_markup_stripping() {
        // PrefName is C4MaxName+1 and its RCT_All adaptor reads at most
        // C4MaxName=30 bytes before CMarkup::StripMarkup runs (pristine
        // 9ffa0a5d src/C4Constants.h:25-27; src/C4InfoCore.h:198;
        // src/C4InfoCore.cpp:103-125,146-154).
        let mut core = b"[Player]\nName=".to_vec();
        core.extend(std::iter::repeat_n(b'A', 35));
        core.push(b'\n');

        assert_eq!(super::player_name_from_core(&core), vec![b'A'; 30]);
    }

    #[test]
    fn markup_strip_accepts_arbitrary_parameterless_closing_tags() {
        // CMarkup::Read skips every parameterless closing tag when fSkip is
        // true; stack and tag-name validation only run when applying markup
        // (pristine 9ffa0a5d src/StdMarkup.cpp:36-67,131-162).
        assert_eq!(
            super::strip_c4_markup(b"Before</future-tag>After"),
            b"BeforeAfter"
        );
    }

    #[test]
    fn markup_strip_skips_unvalidated_color_parameters() {
        // StripMarkup parses with fSkip=true, so C4Markup::Read skips color
        // parameters up to eight bytes without validating them as hexadecimal
        // (pristine 9ffa0a5d src/StdMarkup.cpp:36-106,131-152).
        assert_eq!(super::strip_c4_markup(b"<c G>A</c>"), b"A");
    }

    #[test]
    fn absolute_module_keeps_config_name_but_publishes_exe_relative_name() {
        // LoadFromLocalFile retains its input in C4PlayerInfo::szFilename,
        // while the resource lookup and AddByFile use AtExeRelativePath.
        // Nested paths below ExePath therefore have two intentionally
        // distinct names (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
        // src/C4Config.cpp:759-763).
        let install = tempdir().expect("install root");
        let player = install.path().join("Players/Deep/Alice.c4p");
        write_player(&player, b"Alice");
        let mut config_bytes = b"[General]\nParticipants=\"".to_vec();
        config_bytes.extend_from_slice(&super::path_bytes(&player));
        config_bytes.extend_from_slice(b"\"\n");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, config_bytes).expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured player");

        assert_eq!(loaded.players().len(), 1);
        assert_eq!(
            loaded.players()[0].module_filename().as_bytes(),
            super::path_bytes(&player)
        );
        assert_eq!(
            loaded.players()[0].resource_wire_name().as_bytes(),
            b"Players/Deep/Alice.c4p"
        );
    }

    #[test]
    fn developer_exe_root_keeps_its_own_relative_resource_name() {
        // AppPaths models alternate executable roots used by developer builds.
        // AtExeRelativePath is relative to that executable root, not to a
        // parent install directory (pristine 9ffa0a5d src/C4Config.cpp:618-650,
        // 759-763; src/C4PlayerInfo.cpp:87-96).
        let install = tempdir().expect("install root");
        let build = install.path().join("build");
        let player = build.join("Players/Alice.c4p");
        write_player(&player, b"Alice");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, b"[General]\nParticipants=\"Players/Alice.c4p\"\n")
            .expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf(), build],
        )
        .expect("load configured player");

        assert_eq!(loaded.players()[0].source_path(), player);
        assert_eq!(
            loaded.players()[0].resource_wire_name().as_bytes(),
            b"Players/Alice.c4p"
        );
    }

    #[test]
    fn configured_module_is_opened_directly_without_recursive_discovery() {
        // C4ClientPlayerInfos passes each SGetModule result straight to
        // LoadFromLocalFile/C4Group::Open; it does not search subdirectories
        // for a matching basename (pristine 9ffa0a5d
        // src/C4PlayerInfo.cpp:70-79,357-395; src/C4Strings.cpp:435-440).
        let install = tempdir().expect("install root");
        write_player(&install.path().join("Players/Deep/Alice.c4p"), b"Alice");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, b"[General]\nParticipants=\"Alice.c4p\"\n").expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured players");

        assert!(loaded.players().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn configured_bytes_survive_non_utf8_paths_maker_and_player_name() {
        use std::os::unix::ffi::OsStrExt;

        // Config and player-core strings are legacy byte strings. C4Game
        // copies Participants directly, C4Group takes General.Name as maker,
        // and C4PlayerInfo copies the markup-stripped PrefName without a UTF-8
        // conversion (pristine 9ffa0a5d src/C4Game.cpp:361-364;
        // src/C4Application.cpp:118-121; src/C4InfoCore.cpp:103-125;
        // src/C4PlayerInfo.cpp:70-104).
        let raw_config = super::raw_general_config(
            b"[General]\nName=\"M\x81ker\"\nParticipants=\"Players/\x80lice.c4p\"\n",
        );
        assert_eq!(raw_config.name, b"M\x81ker");
        assert_eq!(
            super::split_modules(&raw_config.participants).collect::<Vec<_>>(),
            vec![b"Players/\x80lice.c4p".as_slice()]
        );

        let install = tempdir().expect("install root");
        // Darwin rejects invalid byte sequences at the filesystem API even
        // though OsStr remains byte-preserving. Other Unix filesystems permit
        // the full non-UTF-8 module-path integration case.
        #[cfg(target_os = "macos")]
        let module = b"Players/Alice.c4p".as_slice();
        #[cfg(not(target_os = "macos"))]
        let module = b"Players/\x80lice.c4p".as_slice();
        let player = install.path().join(super::path_from_bytes(module));
        write_player(&player, b"Al\x82ce");
        let mut config_bytes = b"[General]\nName=\"M\x81ker\"\nParticipants=\"".to_vec();
        config_bytes.extend_from_slice(module);
        config_bytes.extend_from_slice(b"\"\n");
        let config = install.path().join("clonk-rust.config");
        fs::write(&config, config_bytes).expect("write config");

        let loaded = super::load_configured_client_players_from_roots(
            &config,
            &[install.path().to_path_buf()],
        )
        .expect("load configured player");

        assert_eq!(loaded.group_maker().as_bytes(), b"M\x81ker");
        assert_eq!(loaded.players().len(), 1);
        assert_eq!(loaded.players()[0].module_filename().as_bytes(), module);
        assert_eq!(loaded.players()[0].resource_wire_name().as_bytes(), module);
        assert_eq!(loaded.players()[0].player_name().as_bytes(), b"Al\x82ce");
        assert_eq!(loaded.players()[0].alternate_color(), 0x00ab_cdef);
        assert!(loaded.players()[0]
            .source_path()
            .as_os_str()
            .as_bytes()
            .ends_with(module));
    }

    fn write_player(path: &Path, name: &[u8]) {
        fs::create_dir_all(path.parent().expect("player parent")).expect("create player parent");
        let mut group = MutableGroup::new(
            path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Player.c4p"),
        );
        let mut player = b"[Player]\nName=".to_vec();
        player.extend_from_slice(name);
        player.extend_from_slice(b"\n[Preferences]\nColorDw=1193046\nAlternateColorDw=11259375\n");
        group
            .add_file_with_metadata("Player.txt", player, 1, false)
            .expect("add Player.txt");
        fs::write(path, group.pack().expect("pack player")).expect("write player");
    }
}
