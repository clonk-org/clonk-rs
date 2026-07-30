//! Runtime-network save player recreation.
//!
//! `C4GameSaveNetwork(false)` stores the live `C4Player` scalars in
//! `Game.txt`, while the inherited `C4PlayerInfoCore` and crew-info roster
//! live in root-level `.c4p` child groups.  SavePlayerInfos supplies the
//! authoritative recreation order and the current client association.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clonk_resources::{Group, GroupError};
use thiserror::Error;

use crate::player_file::{PersistedC4ValueResolution, PlayerFile};
use crate::scenario::ScenarioError;
use crate::{
    ControlPlayerInfoEntry, Engine, MessageBoardQuery, ObjectId, ObjectStatus, Player,
    PlayerAtClient, PlayerControlState, PlayerState, PlayerStatus, RgbColor, Vector2,
    PLAYER_INFO_FLAG_WON,
};

/// One joined SavePlayerInfos row, retaining its enclosing client packet.
/// Callers must preserve packet order and player order within each packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeJoinPlayerSource {
    pub client_id: i32,
    pub info: ControlPlayerInfoEntry,
    /// Native local loads adopt an otherwise unnamed embedded portrait;
    /// remote loads only resolve explicit portrait specifications.
    pub load_unnamed_portraits: bool,
}

/// The installed runtime number corresponding to one input source. Results
/// are returned in exactly the supplied SavePlayerInfos order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoredRuntimeJoinPlayer {
    pub client_id: i32,
    pub player_info_id: i32,
    pub number: i32,
}

#[derive(Debug, Error)]
pub enum RuntimeJoinPlayerRestoreError {
    #[error("runtime player recreation has no combined scenario path")]
    MissingScenarioPath,
    #[error(transparent)]
    Group(#[from] GroupError),
    #[error("Game.txt has no runtime section [Player{0}]")]
    MissingPlayerSection(i32),
    #[error("runtime [Player{player_info_id}] has unsupported Status={status}")]
    UnsupportedStatus { player_info_id: i32, status: i32 },
    #[error("runtime [Player{0}] compiled an invalid zero player-info ID")]
    ZeroPlayerInfoId(i32),
    #[error("runtime player number {0} is already in use")]
    DuplicatePlayerNumber(i32),
    #[error("SavePlayerInfos player {player_info_id} has no embedded player group `{filename}`")]
    MissingPlayerGroup {
        player_info_id: i32,
        filename: String,
    },
    #[error("failed to load embedded player group `{filename}`")]
    PlayerFile {
        filename: String,
        #[source]
        source: Box<ScenarioError>,
    },
}

#[derive(Debug)]
struct RuntimePlayerSection {
    name: String,
    fields: Vec<(String, String)>,
}

impl RuntimePlayerSection {
    fn value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, value)| value.as_str())
    }

    fn i32(&self, name: &str, default: i32) -> i32 {
        self.value(name)
            .and_then(parse_i32_prefix)
            .unwrap_or(default)
    }

    fn u32(&self, name: &str, default: u32) -> u32 {
        self.value(name)
            .and_then(parse_u32_prefix)
            .unwrap_or(default)
    }

    fn boolean(&self, name: &str, default: bool) -> bool {
        self.value(name)
            .and_then(parse_bool_prefix)
            .unwrap_or(default)
    }
}

fn runtime_sections(source: &[u8]) -> Vec<RuntimePlayerSection> {
    let source = &source[..source
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(source.len())];
    let source = clonk_script::c4_string_from_bytes(source);
    let mut sections = Vec::<RuntimePlayerSection>::new();
    let mut current = None;
    for physical_line in source.split('\n') {
        let line = physical_line.strip_suffix('\r').unwrap_or(physical_line);
        let line = line.trim_start_matches([' ', '\t']);
        if let Some(section_name) = line
            .strip_prefix('[')
            .and_then(|line| line.split_once(']').map(|(name, _)| name))
        {
            sections.push(RuntimePlayerSection {
                name: section_name.to_string(),
                fields: Vec::new(),
            });
            current = Some(sections.len() - 1);
            continue;
        }
        let Some(index) = current else {
            continue;
        };
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        sections[index].fields.push((
            name.trim_matches([' ', '\t']).to_string(),
            value.trim_start_matches([' ', '\t']).to_string(),
        ));
    }
    sections
}

fn number_prefix(value: &str) -> Option<&str> {
    let value = value.trim_start_matches([' ', '\t']);
    let bytes = value.as_bytes();
    let mut end = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    if bytes
        .get(end..end + 2)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"0x"))
    {
        end += 2;
        let digits = bytes[end..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        return (digits != 0).then_some(&value[..end + digits]);
    }
    let digits = bytes[end..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    (digits != 0).then_some(&value[..end + digits])
}

fn parse_i32_prefix(value: &str) -> Option<i32> {
    let value = number_prefix(value)?;
    let (negative, unsigned) = value
        .strip_prefix('-')
        .map_or((false, value), |value| (true, value));
    let unsigned = unsigned.strip_prefix('+').unwrap_or(unsigned);
    let magnitude = if let Some(hex) = unsigned
        .strip_prefix("0x")
        .or_else(|| unsigned.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).ok()?
    } else {
        unsigned.parse::<u64>().ok()?
    };
    Some(if negative {
        (0_u32.wrapping_sub(magnitude as u32)) as i32
    } else {
        magnitude as u32 as i32
    })
}

fn parse_u32_prefix(value: &str) -> Option<u32> {
    parse_i32_prefix(value).map(|value| value as u32)
}

fn parse_bool_prefix(value: &str) -> Option<bool> {
    let value = value.trim_start_matches([' ', '\t']);
    if value
        .get(..4)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("true"))
    {
        Some(true)
    } else if value
        .get(..5)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("false"))
    {
        Some(false)
    } else {
        parse_i32_prefix(value).map(|value| value != 0)
    }
}

fn parse_definition_entries(value: Option<&str>) -> Vec<(String, i32)> {
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter_map(|entry| {
            let (id, count) = entry.split_once('=')?;
            Some((id.to_string(), parse_i32_prefix(count)?))
        })
        .collect()
}

fn parse_hostility_entries(value: Option<&str>) -> Vec<(i32, i32)> {
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter_map(|entry| {
            let (id, count) = entry.split_once('=')?;
            Some((parse_i32_prefix(id)?, parse_i32_prefix(count)?))
        })
        .collect()
}

fn live_object_reference(value: i32, object_numbers: &HashSet<u64>) -> Option<ObjectId> {
    let number = u64::try_from(value).ok()?;
    (number != 0 && object_numbers.contains(&number)).then(|| ObjectId::new(number))
}

fn parse_object_list(value: Option<&str>, object_numbers: &HashSet<u64>) -> Vec<ObjectId> {
    value
        .into_iter()
        .flat_map(|value| value.split(';'))
        .filter_map(parse_i32_prefix)
        .filter_map(|number| live_object_reference(number, object_numbers))
        .collect()
}

fn decode_quoted_bytes(mut source: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    source = source.strip_prefix(b"\"")?;
    let mut decoded = Vec::new();
    while let Some((&byte, rest)) = source.split_first() {
        source = rest;
        if byte == b'"' {
            return Some((decoded, source));
        }
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let (&escape, rest) = source.split_first()?;
        source = rest;
        let decoded_byte = match escape {
            b'a' => 0x07,
            b'b' => 0x08,
            b'f' => 0x0c,
            b'n' => b'\n',
            b'r' => b'\r',
            b't' => b'\t',
            b'v' => 0x0b,
            b'\'' | b'"' | b'\\' | b'?' => escape,
            digit @ b'0'..=b'7' => {
                let mut value = u32::from(digit - b'0');
                while let Some((&next @ b'0'..=b'7', rest)) = source.split_first() {
                    value = value.wrapping_mul(8).wrapping_add(u32::from(next - b'0'));
                    source = rest;
                }
                value as u8
            }
            other => other,
        };
        decoded.push(decoded_byte);
    }
    None
}

fn skip_horizontal(mut source: &[u8]) -> &[u8] {
    while source
        .first()
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        source = &source[1..];
    }
    source
}

fn parse_message_board_query(
    value: Option<&str>,
    object_numbers: &HashSet<u64>,
) -> Option<MessageBoardQuery> {
    let bytes = value.map(clonk_script::c4_string_bytes)?;
    let mut source = skip_horizontal(&bytes).strip_prefix(b"(")?;
    let comma = source.iter().position(|byte| *byte == b',')?;
    let target = parse_i32_prefix(&clonk_script::c4_string_from_bytes(&source[..comma]))
        .and_then(|number| live_object_reference(number, object_numbers));
    source = skip_horizontal(&source[comma + 1..]);
    let (prompt, remaining) = decode_quoted_bytes(source)?;
    source = skip_horizontal(remaining).strip_prefix(b",")?;
    let uppercase =
        parse_i32_prefix(&clonk_script::c4_string_from_bytes(skip_horizontal(source)))? != 0;
    Some(MessageBoardQuery::new(
        target,
        clonk_script::c4_string_from_bytes(&prompt),
        uppercase,
    ))
}

fn parse_player_state(
    section: &RuntimePlayerSection,
    source_info_id: i32,
    object_numbers: &HashSet<u64>,
) -> Result<PlayerState, RuntimeJoinPlayerRestoreError> {
    let raw_status = section.i32("Status", 0);
    let mut status = match raw_status {
        0 => PlayerStatus::Inactive,
        1 => PlayerStatus::Active,
        2 => PlayerStatus::TeamSelection,
        3 => PlayerStatus::TeamSelectionPending,
        // C4Player::Status is an unconstrained int32 compiler word. Preserve
        // unknown nonzero values verbatim and use the truthy active projection
        // for Rust gameplay until a native transition replaces it.
        _ => PlayerStatus::Active,
    };
    let surrendered_value = section.i32("Surrendered", 0);
    let surrendered = surrendered_value != 0;
    let eliminated_value = section.i32("Eliminated", 0);
    if eliminated_value != 0 {
        status = if surrendered {
            PlayerStatus::Surrendered
        } else {
            PlayerStatus::Eliminated
        };
    }
    let color = section.u32("ColorDw", 0);
    let control_style_value = section.i32("AutoStopControl", 0);
    let auto_context_menu_value = section.i32("AutoContextMenu", 0);
    let message_board_queries =
        parse_message_board_query(section.value("MsgBoardQueries"), object_numbers)
            .into_iter()
            .collect();
    let object = |name: &str| live_object_reference(section.i32(name, 0), object_numbers);
    let color_index = section.i32("Color", -1);
    let position_index = section.i32("Position", 0);
    Ok(PlayerState {
        id: section.i32("Index", -1),
        player_info_id: section.i32("ID", 0),
        at_client: PlayerAtClient::new(section.i32("AtClient", -1)),
        at_client_name: section.value("AtClientName").map(ToOwned::to_owned),
        status,
        status_value: Some(raw_status),
        surrendered,
        surrendered_value,
        eliminated_value,
        evaluated: section.boolean("Evaluated", false),
        wealth: section.i32("Wealth", 0),
        points: section.i32("Points", 0),
        value: section.i32("Value", 0),
        initial_value: section.i32("InitialValue", 0),
        value_gain: section.i32("ValueGain", 0),
        objects_owned: section.u32("ObjectsOwned", 0),
        initial_value_set: true,
        knowledge_entries: parse_definition_entries(section.value("Knowledge")),
        magic_entries: parse_definition_entries(section.value("Magic")),
        cursor: object("Cursor"),
        view_mode: section.i32("ViewMode", 0),
        view_cursor: object("ViewCursor"),
        captain: object("Captain"),
        view_center: Some(Vector2::new(
            section.i32("ViewX", 0),
            section.i32("ViewY", 0),
        )),
        view_wealth: section.i32("ViewWealth", 0),
        view_value: section.i32("ViewValue", 0),
        crew: parse_object_list(section.value("Crew"), object_numbers),
        crew_created: section.i32("CrewCreated", 0),
        home_base_material_entries: parse_definition_entries(section.value("HomeBaseMaterial")),
        home_base_production_entries: parse_definition_entries(section.value("HomeBaseProduction")),
        production_delay: section.u32("ProductionDelay", 0),
        production_unit: section.u32("ProductionUnit", 0),
        color: Some(RgbColor::new(
            ((color >> 16) & 0xff) as u8,
            ((color >> 8) & 0xff) as u8,
            (color & 0xff) as u8,
        )),
        color_dw_raw: Some(color),
        color_index: (color_index != -1).then_some(color_index),
        position_index: (position_index != -1).then_some(position_index),
        control_set: section.i32("Control", 0),
        mouse_control: section.i32("MouseControl", 0),
        fog_of_war: section.boolean("FogOfWar", false),
        force_fog_of_war: section.boolean("ForceFogOfWar", false),
        show_startup: section.boolean("ShowStartup", false),
        select_count: section.i32("SelectCount", 0),
        message_status: section.i32("MessageStatus", 0),
        message_buf: section.value("MessageBuf").unwrap_or_default().to_string(),
        message_board_queries,
        show_control_position: section.i32("ShowControlPos", 0),
        show_control: section.i32("ShowControl", 0),
        hostility_entries: parse_hostility_entries(section.value("Hostile")),
        control: PlayerControlState {
            last_com: section.i32("LastCom", 0),
            last_com_delay: section.i32("LastComDel", 0),
            last_com_down_double: section.i32("LastComDownDouble", 0),
            pressed_coms: section.i32("PressedComs", 0),
            control_style: control_style_value != 0,
            control_style_value,
            auto_context_menu: auto_context_menu_value != 0,
            auto_context_menu_value,
            cursor_flash: section.i32("CursorFlash", 0),
            select_flash: section.i32("SelectFlash", 0),
            cursor_selection: section.i32("CursorSelection", 0),
            cursor_toggled: section.i32("CursorToggled", 0),
        },
        ..PlayerState::default()
    })
}

fn effective_player_name(info: &ControlPlayerInfoEntry) -> String {
    [&info.league_account, &info.forced_name, &info.name]
        .into_iter()
        .find(|name| !name.is_empty())
        .map(|name| clonk_script::c4_string_from_bytes(name.as_bytes()))
        .unwrap_or_default()
}

fn legacy_basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| matches!(*byte, b'/' | b'\\'))
        .map_or(path, |separator| &path[separator + 1..])
}

impl Engine {
    pub fn restore_runtime_join_players_from_path(
        &mut self,
        scenario_path: impl AsRef<Path>,
        sources: &[RuntimeJoinPlayerSource],
    ) -> Result<Vec<RestoredRuntimeJoinPlayer>, RuntimeJoinPlayerRestoreError> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let group = Group::open(scenario_path)?;
        let game_txt = group.read_file("Game.txt")?;
        self.restore_runtime_join_players(&group, &game_txt, sources)
    }

    /// Ordinary offline savegames keep user-player files outside the save
    /// group while embedding script-player files. Reuse the runtime-player
    /// compiler for both: an entry in `external_player_paths` replaces the
    /// embedded lookup for that saved player-info ID; all other sources keep
    /// the native embedded/script fallback.
    pub fn restore_offline_savegame_players_from_path(
        &mut self,
        scenario_path: impl AsRef<Path>,
        sources: &[RuntimeJoinPlayerSource],
        external_player_paths: &HashMap<i32, PathBuf>,
        save_game: bool,
    ) -> Result<Vec<RestoredRuntimeJoinPlayer>, RuntimeJoinPlayerRestoreError> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let group = Group::open(scenario_path)?;
        // `C4Game::OpenScenario` leaves `GameText` null for a scenario that
        // ships no Game.txt, and `LoadRuntimeData` simply reports failure
        // instead of aborting the open (C4Game.cpp:224; C4Player.cpp:1654-1655).
        let game_txt = group
            .exists("Game.txt")
            .then(|| group.read_file("Game.txt"));
        self.restore_runtime_join_players_with_external_paths(
            &group,
            &game_txt.transpose()?.unwrap_or_default(),
            sources,
            external_player_paths,
            save_game,
        )
    }

    pub fn restore_runtime_join_players(
        &mut self,
        scenario_group: &Group,
        game_txt: &[u8],
        sources: &[RuntimeJoinPlayerSource],
    ) -> Result<Vec<RestoredRuntimeJoinPlayer>, RuntimeJoinPlayerRestoreError> {
        self.restore_runtime_join_players_with_external_paths(
            scenario_group,
            game_txt,
            sources,
            &HashMap::new(),
            true,
        )
    }

    /// The state `C4Player::Init` is left holding when `LoadRuntimeData` finds
    /// no runtime section for a script player in a non-savegame: the
    /// pre-`Load` defaults (`Status = PS_Normal`, view centred on the world at
    /// C4Player.cpp:257,286) with Number/ColorDw/ID/Team re-seeded from the
    /// restore row (C4Player.cpp:363-369).
    fn default_script_player_state(&self, info: &ControlPlayerInfoEntry) -> PlayerState {
        let (world_width, world_height) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        PlayerState {
            id: info.game_number,
            player_info_id: info.id,
            status: PlayerStatus::Active,
            status_value: Some(1),
            color: Some(RgbColor::new(
                ((info.color >> 16) & 0xff) as u8,
                ((info.color >> 8) & 0xff) as u8,
                (info.color & 0xff) as u8,
            )),
            color_dw_raw: Some(info.color),
            view_center: Some(Vector2::new(world_width / 2, world_height / 2)),
            initial_value_set: true,
            ..PlayerState::default()
        }
    }

    fn restore_runtime_join_players_with_external_paths(
        &mut self,
        scenario_group: &Group,
        game_txt: &[u8],
        sources: &[RuntimeJoinPlayerSource],
        external_player_paths: &HashMap<i32, PathBuf>,
        save_game: bool,
    ) -> Result<Vec<RestoredRuntimeJoinPlayer>, RuntimeJoinPlayerRestoreError> {
        if sources.is_empty() {
            return Ok(Vec::new());
        }
        let sections = runtime_sections(game_txt);
        let root_entries = scenario_group.entries()?;
        let object_numbers = self
            .objects
            .iter()
            .filter(|object| !object.destroyed && object.state.status != ObjectStatus::Deleted)
            .map(|object| object.id.as_u64())
            .collect::<HashSet<_>>();
        let value_resolution = PersistedC4ValueResolution {
            strings: self.legacy_string_table_snapshot(),
            object_numbers: object_numbers.clone(),
        };
        let mut occupied = self.players.keys().copied().collect::<HashSet<_>>();
        let mut prepared = Vec::with_capacity(sources.len());

        for source in sources {
            let section_name = format!("Player{}", source.info.id);
            let mut state = match sections.iter().find(|section| section.name == section_name) {
                Some(section) => parse_player_state(section, source.info.id, &object_numbers)?,
                // "for script players in non-savegames, this is OK - it means
                // they get restored using default values"
                // (C4Player.cpp:359-371).
                None if !save_game && source.info.is_script_player() => {
                    self.default_script_player_state(&source.info)
                }
                None => {
                    return Err(RuntimeJoinPlayerRestoreError::MissingPlayerSection(
                        source.info.id,
                    ))
                }
            };
            if state.player_info_id == 0 {
                return Err(RuntimeJoinPlayerRestoreError::ZeroPlayerInfoId(
                    source.info.id,
                ));
            }
            if state.id == -1 {
                state.id = (0..)
                    .find(|number| !occupied.contains(number))
                    .unwrap_or_default();
            }
            if !occupied.insert(state.id) {
                return Err(RuntimeJoinPlayerRestoreError::DuplicatePlayerNumber(
                    state.id,
                ));
            }

            let filename = legacy_basename(source.info.filename.as_bytes());
            let player_file = if let Some(external_path) =
                external_player_paths.get(&source.info.id)
            {
                let child = Group::open(external_path)?;
                PlayerFile::load_with_portraits_and_value_resolution(
                    &child,
                    true,
                    &value_resolution,
                )
                .map_err(|error| RuntimeJoinPlayerRestoreError::PlayerFile {
                    filename: external_path.display().to_string(),
                    source: Box::new(error),
                })?
            } else if filename.is_empty() && source.info.is_script_player() {
                PlayerFile::default()
            } else {
                let entry = root_entries
                    .iter()
                    .find(|entry| entry.name_bytes.eq_ignore_ascii_case(filename))
                    .ok_or_else(|| RuntimeJoinPlayerRestoreError::MissingPlayerGroup {
                        player_info_id: source.info.id,
                        filename: clonk_script::c4_string_from_bytes(filename),
                    })?;
                let child = scenario_group.open_child(&entry.relative_path)?;
                PlayerFile::load_with_portraits_and_value_resolution(
                    &child,
                    source.load_unnamed_portraits,
                    &value_resolution,
                )
                .map_err(|error| RuntimeJoinPlayerRestoreError::PlayerFile {
                    filename: clonk_script::c4_string_from_bytes(filename),
                    source: Box::new(error),
                })?
            };
            let info_core = player_file.exact_info_core();
            state.name = effective_player_name(&source.info);
            state.team = (source.info.team != 0).then_some(source.info.team);
            state.script_player = source.info.is_script_player();
            state.no_elimination_check = source.info.no_elimination_check();
            state.won = source.info.flags & PLAYER_INFO_FLAG_WON != 0;
            state.score = player_file.score;
            state.rounds = player_file.rounds;
            state.rounds_won = player_file.rounds_won;
            state.rounds_lost = player_file.rounds_lost;
            state.total_playing_time = player_file.total_playing_time;
            state.pref_control = player_file.pref_control;
            state.pref_mouse = Some(player_file.pref_mouse);
            state.pref_control_style = player_file.pref_control_style;
            state.pref_auto_context_menu = player_file.pref_auto_context_menu;
            state.extra_data = info_core.extra_data.clone();
            state.player_info_core = Some(info_core);
            prepared.push((
                RestoredRuntimeJoinPlayer {
                    client_id: source.client_id,
                    player_info_id: source.info.id,
                    number: state.id,
                },
                state,
                player_file.crew,
            ));
        }

        let mut restored = Vec::with_capacity(prepared.len());
        for (binding, state, roster) in prepared {
            self.assign_player_info_id(state.player_info_id);
            let number = state.id;
            let mut player = Player::from_state(state);
            player.set_game_join_time(self.game_time);
            self.crew_info_control_counts
                .retain(|link, _| link.player_id != number);
            self.players.insert(number, player);
            self.append_and_recheck_player_order(number);
            self.crew_rosters.insert(number, roster);
            let roster_len = self.crew_rosters.get(&number).map_or(0, Vec::len);
            self.crew_info_order
                .insert(number, (0..roster_len).rev().collect());
            self.actualize_ownerless_fow_objects_for_new_player();
            restored.push(binding);
        }
        self.recheck_runtime_team_memberships();
        self.players_registered = !self.players.is_empty();
        Ok(restored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn offline_savegame_restores_user_profile_from_external_group() {
        let fixture = tempdir().expect("save tempdir");
        let save = fixture.path().join("Save.c4s");
        std::fs::create_dir(&save).expect("create save group");
        std::fs::write(
            save.join("Game.txt"),
            "[Player7]\nStatus=1\nIndex=2\nID=7\nWealth=19\n",
        )
        .expect("write Game.txt");
        let profile = fixture.path().join("Alice.c4p");
        std::fs::create_dir(&profile).expect("create profile group");
        std::fs::write(
            profile.join("Player.txt"),
            "[Player]\nName=Alice\nScore=31\n",
        )
        .expect("write Player.txt");
        let source = RuntimeJoinPlayerSource {
            client_id: 0,
            info: ControlPlayerInfoEntry {
                name: crate::LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                id: 7,
                ..Default::default()
            },
            load_unnamed_portraits: true,
        };
        let mut engine = Engine::new();

        let restored = engine
            .restore_offline_savegame_players_from_path(
                &save,
                &[source],
                &HashMap::from([(7, profile)]),
                true,
            )
            .expect("restore external user profile");

        assert_eq!(restored[0].number, 2);
        let player = engine.player(2).expect("restored player");
        assert_eq!(player.name(), "Alice");
        assert_eq!(player.wealth(), 19);
        assert_eq!(player.score(), 31);
    }

    /// `C4Player::LoadRuntimeData` bails when the scenario ships no `Game.txt`
    /// and again when that text carries no `[Player<ID>]` section
    /// (C4Player.cpp:1652-1657). For a script player in a non-savegame,
    /// `C4Player::Init` treats that as expected and re-seeds
    /// Number/ColorDw/ID/Team straight from the restore row instead of failing
    /// (C4Player.cpp:355-371).
    #[test]
    fn a_regular_scenario_restores_its_script_player_without_runtime_data() {
        let fixture = tempdir().expect("scenario tempdir");
        let scenario = fixture.path().join("Drachenfels.c4s");
        std::fs::create_dir(&scenario).expect("create scenario group");
        std::fs::write(scenario.join("Scenario.txt"), "[Head]\nMaxPlayer=4\n")
            .expect("write scenario core");
        let script_profile = scenario.join("ScriptPlr-1.c4p");
        std::fs::create_dir(&script_profile).expect("create script player group");
        std::fs::write(
            script_profile.join("Player.txt"),
            "[Player]\nName=Ala Kadabra\n",
        )
        .expect("write script profile");
        let source = RuntimeJoinPlayerSource {
            client_id: 0,
            info: ControlPlayerInfoEntry {
                name: crate::LegacyCString::from_bytes(b"Ala Kadabra".to_vec()).unwrap(),
                filename: crate::LegacyCString::from_bytes(b"ScriptPlr-1.c4p".to_vec()).unwrap(),
                flags: crate::PLAYER_INFO_FLAG_JOINED
                    | crate::PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
                id: 1,
                player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                color: 65535,
                game_number: 10,
                team: 2,
                ..Default::default()
            },
            load_unnamed_portraits: true,
        };
        let mut engine = Engine::new();

        let restored = engine
            .restore_offline_savegame_players_from_path(
                &scenario,
                &[source],
                &HashMap::new(),
                false,
            )
            .expect("a regular scenario restores its shipped script player");

        assert_eq!(restored.len(), 1);
        // `GetInGameNumber` owns the number, not a free-number scan.
        assert_eq!(restored[0].number, 10);
        assert_eq!(restored[0].player_info_id, 1);
        let player = engine.player(10).expect("script player 10 joined");
        assert!(player.is_script_player());
        assert!(player.no_elimination_check());
        assert_eq!(player.team(), Some(2));
        assert_eq!(player.name(), "Ala Kadabra");
    }

    /// The same missing section stays fatal for a real savegame and for user
    /// players, where C++ returns false from `C4Player::Init`
    /// (C4Player.cpp:370-371).
    #[test]
    fn a_savegame_still_fails_when_its_runtime_player_section_is_missing() {
        let fixture = tempdir().expect("scenario tempdir");
        let save = fixture.path().join("Save.c4s");
        std::fs::create_dir(&save).expect("create save group");
        std::fs::write(save.join("Game.txt"), "[Player9]\nStatus=1\nID=9\n")
            .expect("write Game.txt");
        let source = RuntimeJoinPlayerSource {
            client_id: 0,
            info: ControlPlayerInfoEntry {
                flags: crate::PLAYER_INFO_FLAG_JOINED,
                id: 1,
                player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                game_number: 10,
                ..Default::default()
            },
            load_unnamed_portraits: true,
        };
        let mut engine = Engine::new();

        assert!(matches!(
            engine.restore_offline_savegame_players_from_path(
                &save,
                &[source],
                &HashMap::new(),
                true,
            ),
            Err(RuntimeJoinPlayerRestoreError::MissingPlayerSection(1))
        ));
    }

    #[test]
    fn parser_applies_every_field_emitted_by_live_player_serializer() {
        let objects = [41_u64, 42, 43].into_iter().collect::<HashSet<_>>();
        let source = br#"[Player7]
Status=-4
AtClient=5
AtClientName=stale
Index=3
ID=7
Eliminated=-6
Surrendered=2
Evaluated=true
Color=4
ColorDw=1193046
Control=6
MouseControl=2
AutoContextMenu=3
AutoStopControl=4
Position=8
ViewMode=2
ViewX=-17
ViewY=29
ViewWealth=5
ViewValue=6
FogOfWar=true
ForceFogOfWar=true
ShowStartup=true
ShowControl=123
ShowControlPos=9
Wealth=100
Points=101
Value=102
InitialValue=103
ValueGain=-4
ObjectsOwned=12
Hostile=1=2;9=-3
ProductionDelay=-13
ProductionUnit=14
SelectCount=15
SelectFlash=16
CursorFlash=17
Cursor=41
ViewCursor=42
Captain=999
LastCom=18
LastComDel=19
PressedComs=20
LastComDownDouble=21
CursorSelection=22
CursorToggled=23
MessageStatus=24
MessageBuf=native bytes
HomeBaseMaterial=WOOD=3;METL=-1
HomeBaseProduction=ROCK=4
Knowledge=CLNK=1;CLNK=0
Magic=MAGC=2
Crew=41;999;42
CrewCreated=25
MsgBoardQueries=(43,"Ask\nnow\341",1)
"#;
        let sections = runtime_sections(source);
        let state = parse_player_state(&sections[0], 7, &objects).expect("parse runtime player");

        assert_eq!((state.id, state.player_info_id), (3, 7));
        assert_eq!(state.at_client, PlayerAtClient::new(5));
        assert_eq!(state.at_client_name.as_deref(), Some("stale"));
        assert_eq!(state.status, PlayerStatus::Surrendered);
        assert_eq!(state.status_value, Some(-4));
        assert_eq!(state.eliminated_value, -6);
        assert_eq!((state.surrendered, state.surrendered_value), (true, 2));
        assert!(state.evaluated);
        assert_eq!(state.color_index, Some(4));
        assert_eq!(state.color, Some(RgbColor::new(0x12, 0x34, 0x56)));
        assert_eq!((state.control_set, state.mouse_control), (6, 2));
        assert_eq!(state.position_index, Some(8));
        assert_eq!(state.view_mode, 2);
        assert_eq!(state.view_center, Some(Vector2::new(-17, 29)));
        assert_eq!((state.view_wealth, state.view_value), (5, 6));
        assert!(state.fog_of_war && state.force_fog_of_war && state.show_startup);
        assert_eq!((state.show_control, state.show_control_position), (123, 9));
        assert_eq!((state.wealth, state.points, state.value), (100, 101, 102));
        assert_eq!((state.initial_value, state.value_gain), (103, -4));
        assert_eq!(state.objects_owned, 12);
        assert_eq!(state.hostility_entries, [(1, 2), (9, -3)]);
        assert_eq!(
            (state.production_delay, state.production_unit),
            ((-13_i32) as u32, 14),
            "signed int32 runtime counters retain their u32 bit pattern"
        );
        assert_eq!(state.select_count, 15);
        assert_eq!(state.cursor, Some(ObjectId::new(41)));
        assert_eq!(state.view_cursor, Some(ObjectId::new(42)));
        assert_eq!(
            state.captain, None,
            "missing object pointers denumerate null"
        );
        assert_eq!(state.control.last_com, 18);
        assert_eq!(state.control.last_com_delay, 19);
        assert_eq!(state.control.pressed_coms, 20);
        assert_eq!(state.control.last_com_down_double, 21);
        assert_eq!(
            (state.control.select_flash, state.control.cursor_flash),
            (16, 17)
        );
        assert_eq!(
            (state.control.cursor_selection, state.control.cursor_toggled),
            (22, 23)
        );
        assert_eq!(
            (
                state.control.auto_context_menu_value,
                state.control.control_style_value
            ),
            (3, 4)
        );
        assert_eq!(state.message_status, 24);
        assert_eq!(state.message_buf, "native bytes");
        assert_eq!(
            state.home_base_material_entries,
            [("WOOD".into(), 3), ("METL".into(), -1)]
        );
        assert_eq!(state.home_base_production_entries, [("ROCK".into(), 4)]);
        assert_eq!(
            state.knowledge_entries,
            [("CLNK".into(), 1), ("CLNK".into(), 0)]
        );
        assert_eq!(state.magic_entries, [("MAGC".into(), 2)]);
        assert_eq!(state.crew, [ObjectId::new(41), ObjectId::new(42)]);
        assert_eq!(state.crew_created, 25);
        assert_eq!(state.message_board_queries.len(), 1);
        let query = &state.message_board_queries[0];
        assert_eq!(query.target, Some(ObjectId::new(43)));
        assert_eq!(
            clonk_script::c4_string_bytes(&query.prompt),
            b"Ask\nnow\xe1"
        );
        assert!(query.uppercase);
    }

    #[test]
    fn parser_retains_full_cpp_compiler_words_for_color_and_last_com() {
        let sections =
            runtime_sections(b"[Player7]\nID=7\nColorDw=4279383126\nLastCom=-2147483630\n");
        let state = parse_player_state(&sections[0], 7, &HashSet::new())
            .expect("parse full-width runtime player words");

        assert_eq!(state.color, Some(RgbColor::new(0x12, 0x34, 0x56)));
        assert_eq!(state.color_dw_raw, Some(0xff12_3456));
        assert_eq!(state.exact_color_dw(), 0xff12_3456);
        assert_eq!(state.control.last_com, -2_147_483_630);
    }
}
