//! `Game.txt` serialization for an initial C++ network save.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{Engine, EnvironmentSettings, GammaControlState, NextMissionState};

/// The shipped C++ build's `C4SyncCheckRate` default.
pub const INITIAL_NETWORK_DEFAULT_SYNC_RATE: i32 = 100;

/// `C4Landscape::CompileFunc`'s default `FIXED100(20)`, stored through
/// `mkCastIntAdapt` as the fixed-point value's raw signed 32-bit word.
pub const LANDSCAPE_DEFAULT_GRAVITY_RAW: i32 = 13_107;

/// The exact `[Landscape]` runtime block compiled into `Game.txt`.
///
/// `gravity` is the raw `C4Fixed` representation, not the script-facing
/// `GetGravity()` integer. `mat_modulation` remains unsigned like C++'s
/// `uint32_t Modulation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LandscapeGameData {
    pub map_seed: i32,
    pub left_open: i32,
    pub right_open: i32,
    pub top_open: bool,
    pub bottom_open: bool,
    pub gravity: i32,
    pub mat_modulation: u32,
    pub mode: i32,
}

impl Default for LandscapeGameData {
    fn default() -> Self {
        Self {
            map_seed: 0,
            left_open: 0,
            right_open: 0,
            top_open: false,
            bottom_open: false,
            gravity: LANDSCAPE_DEFAULT_GRAVITY_RAW,
            mat_modulation: 0,
            mode: 0,
        }
    }
}

/// Canonical `StdCompilerINIWrite` blocks that an existing C++ savegame has
/// already compiled. Runtime staging interprets these separately while this
/// copy remains authoritative for byte-exact dynamic serialization.
///
/// Each retained value includes its `[Section]` header and terminating CRLF,
/// but not the blank line that separates top-level sections. Keeping the
/// native bytes opaque is intentional: in particular, `GlobalEffects` may
/// contain the complete recursive C4Value syntax. Re-encoding that graph from
/// runtime values would change `Game.txt` bytes and the C4Group contents CRC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InitialNetworkCompiledSections {
    pub(crate) script_engine: Option<Vec<u8>>,
    pub(crate) sky: Option<Vec<u8>>,
    pub(crate) effects: Option<Vec<u8>>,
    pub(crate) scoreboard: Option<Vec<u8>>,
}

/// Runtime fields that cannot be installed without changing the saved C++
/// state or making the engine's derived tick gates disagree with `Frame`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InitialNetworkGameApplyError {
    #[error("saved Frame value {value} is negative")]
    NegativeFrame { value: i32 },
    #[error("saved ObjectEnumerationIndex value {value} is negative")]
    NegativeObjectEnumerationIndex { value: i32 },
    #[error(
        "saved {field} value {actual} disagrees with Frame {frame} modulo {modulus} (expected {expected})"
    )]
    TickMismatch {
        field: &'static str,
        frame: i32,
        modulus: i32,
        expected: i32,
        actual: i32,
    },
}

impl InitialNetworkCompiledSections {
    pub fn script_engine(&self) -> Option<&[u8]> {
        self.script_engine.as_deref()
    }

    pub fn sky(&self) -> Option<&[u8]> {
        self.sky.as_deref()
    }

    pub fn effects(&self) -> Option<&[u8]> {
        self.effects.as_deref()
    }

    pub fn scoreboard(&self) -> Option<&[u8]> {
        self.scoreboard.as_deref()
    }
}

/// The state serialized by `C4Game::CompileFunc` before runtime-join data is
/// added to a network dynamic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialNetworkGameData {
    pub time: i32,
    pub frame: i32,
    pub control_tick: i32,
    pub sync_rate: i32,
    pub tick2: i32,
    pub tick3: i32,
    pub tick5: i32,
    pub tick10: i32,
    pub tick35: i32,
    pub tick255: i32,
    pub tick500: i32,
    pub tick1000: i32,
    pub object_enumeration_index: i32,
    pub rules: i32,
    pub play_list: String,
    pub current_scenario_section: String,
    pub resort_any_object: bool,
    pub music_enabled: bool,
    pub music_level: i32,
    pub next_mission: NextMissionState,
    /// Entries are emitted in this order. This makes the byte order explicit
    /// instead of pretending Rust's map order equals C++ `unordered_map`.
    pub message_board_commands: Vec<InitialNetworkMessageBoardCommand>,
    pub script_go: bool,
    pub script_counter: i32,
    pub environment: EnvironmentSettings,
    pub gamma: GammaControlState,
    /// `None` before a landscape exists (notably the pristine initial host
    /// dynamic). A present all-default block still decompiles to no section.
    pub landscape: Option<LandscapeGameData>,
    /// Exact non-player runtime blocks retained from a canonical C++
    /// savegame. They are emitted in `C4Game::CompileFunc` order.
    pub compiled_sections: InitialNetworkCompiledSections,
}

impl Default for InitialNetworkGameData {
    fn default() -> Self {
        Self {
            time: 0,
            frame: 0,
            control_tick: 0,
            sync_rate: INITIAL_NETWORK_DEFAULT_SYNC_RATE,
            tick2: 0,
            tick3: 0,
            tick5: 0,
            tick10: 0,
            tick35: 0,
            tick255: 0,
            tick500: 0,
            tick1000: 0,
            object_enumeration_index: 0,
            rules: 0,
            play_list: String::new(),
            current_scenario_section: String::new(),
            resort_any_object: false,
            music_enabled: false,
            music_level: 100,
            next_mission: NextMissionState::default(),
            // C4Game::InitSystem calls C4MessageInput::Init before any game
            // or network initialization; an empty command map receives this
            // stock command (C4Game.cpp:3516-3519;
            // C4MessageInput.cpp:252-260).
            message_board_commands: vec![InitialNetworkMessageBoardCommand::speed()],
            script_go: false,
            script_counter: 0,
            environment: EnvironmentSettings::default(),
            gamma: GammaControlState::default(),
            landscape: None,
            compiled_sections: InitialNetworkCompiledSections::default(),
        }
    }
}

impl InitialNetworkGameData {
    /// Captures the runtime fields present at initial-record start without
    /// requiring the full exact-save object/effect surface. In particular,
    /// landscape map identity must be frozen after creation so replay does
    /// not regenerate a different dynamic map.
    pub fn for_initial_record(engine: &Engine) -> Self {
        Self {
            landscape: landscape_game_data(engine),
            ..Self::default()
        }
    }

    /// Captures fields the Rust engine currently represents without inventing
    /// encodings for unported C++ save components.
    pub fn from_engine(engine: &Engine) -> Result<Self, InitialNetworkGameError> {
        if !engine.capture_script_globals().is_empty() {
            return Err(UnsupportedInitialNetworkGameState::ScriptGlobals.into());
        }
        if engine.sky.is_some() {
            return Err(UnsupportedInitialNetworkGameState::Sky.into());
        }
        if !engine.global_effects.is_empty() {
            return Err(UnsupportedInitialNetworkGameState::GlobalEffects.into());
        }
        if !engine.scoreboard.borrow().is_default() {
            return Err(UnsupportedInitialNetworkGameState::Scoreboard.into());
        }

        Self::from_engine_live(engine)
    }

    /// Capture the scalar `C4Game::CompileFunc` fields while a live-save
    /// caller supplies typed Script/Sky/Effects/Scoreboard blocks itself.
    pub(crate) fn from_engine_live(engine: &Engine) -> Result<Self, InitialNetworkGameError> {
        let frame = i32::try_from(engine.frame).map_err(|_| {
            InitialNetworkGameError::IntegerOutOfRange {
                field: "Frame",
                value: engine.frame,
            }
        })?;
        let object_enumeration_index = engine.next_object_id.saturating_sub(1);
        let object_enumeration_index = i32::try_from(object_enumeration_index).map_err(|_| {
            InitialNetworkGameError::IntegerOutOfRange {
                field: "ObjectEnumerationIndex",
                value: object_enumeration_index,
            }
        })?;

        let mut rules = 0;
        if engine.structures_need_energy {
            rules |= 1;
        }
        if engine.construction_needs_material {
            rules |= 2;
        }
        // Game.Rules is a cached C4Game field. C4Game::UpdateRules refreshes
        // FGRV only on frame one and Tick255; a save between those refreshes
        // must retain the cached bit rather than re-counting live objects.
        if engine.cached_flag_removeable_rule() {
            rules |= 4;
        }
        if engine.structures_snow_in {
            rules |= 8;
        }
        if engine.team_home_base_rule {
            rules |= 16;
        }

        let landscape = landscape_game_data(engine);

        Ok(Self {
            time: engine.game_time,
            frame,
            control_tick: engine.control_tick,
            sync_rate: engine.sync_rate,
            tick2: frame % 2,
            tick3: frame % 3,
            tick5: frame % 5,
            tick10: frame % 10,
            tick35: frame % 35,
            tick255: frame % 255,
            tick500: frame % 500,
            tick1000: frame % 1000,
            object_enumeration_index,
            rules,
            play_list: engine.music_playlist().to_owned(),
            current_scenario_section: if engine.scenario_section_state.last_flags.is_some() {
                engine.scenario_section_state.current.clone()
            } else {
                Default::default()
            },
            resort_any_object: engine.resort_any_object_pending(),
            music_enabled: false,
            music_level: i32::from(engine.music_level()),
            next_mission: engine.next_mission.clone(),
            message_board_commands: engine.message_board_commands.clone(),
            script_go: engine.scenario_script_go,
            script_counter: engine.scenario_script_counter,
            environment: engine.environment,
            gamma: engine.gamma,
            landscape,
            compiled_sections: InitialNetworkCompiledSections::default(),
        })
    }

    /// Checks the invariants required before applying a compiled `Game.txt`
    /// to a live engine. C++ stores the individual tick counters, while the
    /// Rust engine derives them from `Frame`; accepting a disagreement would
    /// make the very first post-load frame take different tick branches.
    pub fn validate_runtime_application(&self) -> Result<(), InitialNetworkGameApplyError> {
        if self.frame < 0 {
            return Err(InitialNetworkGameApplyError::NegativeFrame { value: self.frame });
        }
        if self.object_enumeration_index < 0 {
            return Err(
                InitialNetworkGameApplyError::NegativeObjectEnumerationIndex {
                    value: self.object_enumeration_index,
                },
            );
        }
        for (field, modulus, actual) in [
            ("Tick2", 2, self.tick2),
            ("Tick3", 3, self.tick3),
            ("Tick5", 5, self.tick5),
            ("Tick10", 10, self.tick10),
            ("Tick35", 35, self.tick35),
            ("Tick255", 255, self.tick255),
            ("Tick500", 500, self.tick500),
            ("Tick1000", 1000, self.tick1000),
        ] {
            let expected = self.frame % modulus;
            if actual != expected {
                return Err(InitialNetworkGameApplyError::TickMismatch {
                    field,
                    frame: self.frame,
                    modulus,
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }
}

/// Compiles the C4Game/C4Script/C4Weather/C4Landscape fields represented by
/// [`InitialNetworkGameData`] from a saved `Game.txt`. A subsequent call to
/// [`serialize_initial_network_game`] performs the same canonical decompile
/// used by the initial network save. Canonical Sky/Effects/Scoreboard blocks
/// and the original `[Player...` tail are retained byte-for-byte because their
/// complete runtime graphs are not yet represented by this data model.
pub fn parse_initial_network_game_data(source: &[u8]) -> InitialNetworkGameData {
    if source.is_empty() {
        return InitialNetworkGameData::default();
    }
    let effective = &source[..source
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(source.len())];

    let mut data = InitialNetworkGameData::default();
    // A present nonempty component is compiled even when it contains only
    // whitespace. Missing map/weather fields take compiler defaults rather
    // than preserving InitSystem/C4Weather::Default live values.
    data.message_board_commands.clear();
    data.environment.no_gamma = false;
    if let Some(game) = first_top_level_section_values(effective, "Game") {
        let i32_value = |name: &str, default| {
            game.get(name)
                .and_then(|value| parse_i32_prefix(value))
                .unwrap_or(default)
        };
        let bool_value = |name: &str, default| {
            game.get(name)
                .and_then(|value| parse_bool_prefix(value))
                .unwrap_or(default)
        };
        data.time = i32_value("Time", 0);
        data.frame = i32_value("Frame", 0);
        data.control_tick = i32_value("ControlTick", 0);
        data.sync_rate = i32_value("SyncRate", INITIAL_NETWORK_DEFAULT_SYNC_RATE);
        data.tick2 = i32_value("Tick2", 0);
        data.tick3 = i32_value("Tick3", 0);
        data.tick5 = i32_value("Tick5", 0);
        data.tick10 = i32_value("Tick10", 0);
        data.tick35 = i32_value("Tick35", 0);
        data.tick255 = i32_value("Tick255", 0);
        data.tick500 = i32_value("Tick500", 0);
        data.tick1000 = i32_value("Tick1000", 0);
        data.object_enumeration_index = i32_value("ObjectEnumerationIndex", 0);
        data.rules = i32_value("Rules", 0);
        data.play_list = game
            .get("PlayList")
            .map(|value| decode_legacy_game_string(value))
            .unwrap_or_default();
        data.current_scenario_section = game
            .get("CurrentScenarioSection")
            .map(|value| decode_raw_game_string(value, 30))
            .unwrap_or_default();
        data.resort_any_object = bool_value("ResortAnyObj", false);
        data.music_enabled = bool_value("MusicEnabled", false);
        data.music_level = i32_value("MusicLevel", 100);
        data.next_mission.path = game
            .get("NextMission")
            .map(|value| decode_legacy_game_string(value))
            .unwrap_or_default();
        data.next_mission.text = game
            .get("NextMissionText")
            .map(|value| decode_legacy_game_string(value))
            .unwrap_or_default();
        data.next_mission.description = game
            .get("NextMissionDesc")
            .map(|value| decode_legacy_game_string(value))
            .unwrap_or_default();
        if let Some(commands) = game
            .get("MessageBoardCommands")
            .and_then(|value| parse_message_board_commands(value))
        {
            data.message_board_commands = commands;
        }
    }

    if let Some(script) = first_top_level_section_values(effective, "Script") {
        data.script_go = script
            .get("Go")
            .and_then(|value| parse_bool_prefix(value))
            .unwrap_or(false);
        data.script_counter = script
            .get("Counter")
            .and_then(|value| parse_i32_prefix(value))
            .unwrap_or(0);
    }

    if let Some(weather) = first_top_level_section_values(effective, "Weather") {
        let i32_value = |name: &str, default| {
            weather
                .get(name)
                .and_then(|value| parse_i32_prefix(value))
                .unwrap_or(default)
        };
        data.environment.season = i32_value("Season", 0);
        data.environment.year_speed = i32_value("YearSpeed", 0);
        data.environment.season_delay = i32_value("SeasonDelay", 0);
        data.environment.wind = i32_value("Wind", 0);
        data.environment.wind_target = i32_value("TargetWind", 0);
        data.environment.temperature = i32_value("Temperature", 0);
        data.environment.temperature_range = i32_value("TemperatureRange", 30);
        data.environment.climate = i32_value("Climate", 0);
        data.environment.meteorite = i32_value("MeteoriteLevel", 0);
        data.environment.volcano = i32_value("VolcanoLevel", 0);
        data.environment.earthquake = i32_value("EarthquakeLevel", 0);
        data.environment.lightning = i32_value("LightningLevel", 0);
        data.environment.no_gamma = weather
            .get("NoGamma")
            .and_then(|value| parse_bool_prefix(value))
            .unwrap_or(false);
        if let Some(gamma) = weather.get("Gamma").and_then(|value| parse_gamma(value)) {
            data.gamma = gamma;
        }
    }

    if first_top_level_section_values(effective, "Landscape").is_some() {
        data.landscape = Some(parse_landscape_game_data(effective));
    }
    data.compiled_sections.sky = canonical_compiled_section(effective, b"Sky");
    data.compiled_sections.effects = canonical_compiled_section(effective, b"Effects");
    data.compiled_sections.scoreboard = canonical_compiled_section(effective, b"Scoreboard");
    data.compiled_sections.script_engine = canonical_script_engine_state(effective);
    data
}

/// Extracts a first root-level section and normalizes only its line framing to
/// the CRLF form emitted by `StdCompilerINIWrite`. C++ savegames are already
/// canonical, so every value byte (including recursive effect syntax) remains
/// byte-for-byte unchanged. A header-only block stays represented so runtime
/// compilation can apply member defaults; serialization omits it again like
/// `StdCompilerINIWrite`.
fn canonical_compiled_section(source: &[u8], target: &[u8]) -> Option<Vec<u8>> {
    let mut sections: Vec<usize> = Vec::new();
    let mut found_indent = None;
    let mut body_start = 0;
    let mut body_end = source.len();
    let mut offset = 0;

    while offset < source.len() {
        let (line, next) = next_ini_line(source, offset);
        let indent = line
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        if let Some(name) = ini_section_name_bytes(&line[indent..]) {
            while sections.last().is_some_and(|level| *level >= indent) {
                sections.pop();
            }
            let root = sections.is_empty();
            if let Some(target_indent) = found_indent {
                if root && indent <= target_indent {
                    body_end = offset;
                    break;
                }
            } else if root && name == target {
                found_indent = Some(indent);
                body_start = next;
            }
            sections.push(indent);
        }
        offset = next;
    }

    found_indent?;
    let body = &source[body_start..body_end];
    let mut lines = Vec::new();
    let mut offset = 0;
    while offset < body.len() {
        let (line, next) = next_ini_line(body, offset);
        lines.push(line);
        offset = next;
    }
    while lines
        .last()
        .is_some_and(|line| line.trim_ascii().is_empty())
    {
        lines.pop();
    }
    let mut section = Vec::new();
    section.push(b'[');
    section.extend_from_slice(target);
    section.extend_from_slice(b"]\r\n");
    for line in lines {
        section.extend_from_slice(line);
        section.extend_from_slice(b"\r\n");
    }
    Some(section)
}

/// Retains the complete Script block whenever `mkInsertAdapt(Script,
/// ScriptEngine)` compiled more than the two C4GameScriptHost scalars. The
/// extra data is normally `Globals` and/or `GlobalNamed`; keeping the native
/// bytes lets canonical dynamic serialization remain lossless while typed
/// runtime staging interprets the values independently.
fn canonical_script_engine_state(source: &[u8]) -> Option<Vec<u8>> {
    let section = canonical_compiled_section(source, b"Script")?;
    let mut lines = section.split(|byte| *byte == b'\n');
    let _header = lines.next();
    let unsupported = lines.any(|line| {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let trimmed = line.trim_ascii();
        if trimmed.is_empty() {
            return false;
        }
        if line.len() != line.trim_ascii_start().len() {
            return true;
        }
        let Some(equals) = trimmed.iter().position(|byte| *byte == b'=') else {
            return true;
        };
        !matches!(trimmed[..equals].trim_ascii_end(), b"Go" | b"Counter")
    });
    unsupported.then_some(section)
}

fn next_ini_line(source: &[u8], start: usize) -> (&[u8], usize) {
    let end = source[start..]
        .iter()
        .position(|byte| matches!(byte, b'\r' | b'\n'))
        .map_or(source.len(), |relative| start + relative);
    let mut next = end;
    if next < source.len() {
        let first = source[next];
        next += 1;
        if next < source.len()
            && ((first == b'\r' && source[next] == b'\n')
                || (first == b'\n' && source[next] == b'\r'))
        {
            next += 1;
        }
    }
    (&source[start..end], next)
}

fn ini_section_name_bytes(line: &[u8]) -> Option<&[u8]> {
    let rest = line.strip_prefix(b"[")?;
    if !rest.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let name_end = 1 + rest[1..]
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
        .count();
    let mut delimiter = name_end;
    while rest
        .get(delimiter)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        delimiter += 1;
    }
    (rest.get(delimiter) == Some(&b']')).then_some(&rest[..name_end])
}

fn first_top_level_section_values(source: &[u8], target: &str) -> Option<HashMap<String, String>> {
    let source = clonk_script::c4_string_from_bytes(source);
    let mut values = HashMap::new();
    let mut found = false;
    let mut sections: Vec<(usize, bool)> = Vec::new();
    for raw_line in source.split(['\r', '\n']) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        if let Some(section) = ini_section_name(line) {
            while sections.last().is_some_and(|(level, _)| *level >= indent) {
                sections.pop();
            }
            let is_target = sections.is_empty() && !found && section == target;
            found |= is_target;
            sections.push((indent, is_target));
            continue;
        }
        let Some((name, value)) = ini_named_value(line) else {
            continue;
        };
        let value_indent = indent + 1;
        while sections
            .last()
            .is_some_and(|(level, _)| *level >= value_indent)
        {
            sections.pop();
        }
        if sections.last().is_some_and(|(_, target)| *target) {
            values
                .entry(name.to_string())
                .or_insert_with(|| value.to_string());
        }
    }
    found.then_some(values)
}

pub(crate) fn decode_legacy_game_string(value: &str) -> String {
    let raw = clonk_script::c4_string_bytes(value);
    let Some(mut remaining) = raw.strip_prefix(b"\"") else {
        return clonk_script::c4_string_from_bytes(skip_horizontal_bytes(&raw));
    };
    let mut decoded = Vec::new();
    while let Some((&byte, rest)) = remaining.split_first() {
        remaining = rest;
        if byte == b'"' {
            break;
        }
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let Some((&escape, rest)) = remaining.split_first() else {
            break;
        };
        remaining = rest;
        decoded.push(decode_escaped_game_byte(escape, &mut remaining));
    }
    clonk_script::c4_string_from_bytes(&decoded)
}

fn decode_raw_game_string(value: &str, max_length: usize) -> String {
    let raw = clonk_script::c4_string_bytes(value);
    let raw = skip_horizontal_bytes(&raw);
    clonk_script::c4_string_from_bytes(&raw[..raw.len().min(max_length)])
}

fn parse_message_board_commands(value: &str) -> Option<Vec<InitialNetworkMessageBoardCommand>> {
    let raw = clonk_script::c4_string_bytes(value);
    let (count, mut remaining) = parse_i32_game_bytes(&raw)?;
    if count <= 0 {
        return Some(Vec::new());
    }
    let mut commands = Vec::new();
    for index in 0..count {
        remaining = strip_game_separator(remaining, b';')?;
        let (name, rest) = parse_quoted_game_bytes(remaining)?;
        remaining = strip_game_separator(rest, b'=')?;
        let (script, rest) = parse_quoted_game_bytes(remaining)?;
        remaining = strip_game_separator(rest, b',')?;
        let (restriction, rest) = parse_message_command_restriction(remaining)?;
        let command = InitialNetworkMessageBoardCommand {
            name: clonk_script::c4_string_from_bytes(&name),
            script: clonk_script::c4_string_from_bytes(&script),
            restriction,
        };
        if let Some(existing) = commands
            .iter_mut()
            .find(|existing: &&mut InitialNetworkMessageBoardCommand| existing.name == command.name)
        {
            *existing = command;
        } else {
            commands.push(command);
        }
        remaining = rest;
        if index + 1 == count {
            break;
        }
    }
    Some(commands)
}

fn parse_quoted_game_bytes(raw: &[u8]) -> Option<(Vec<u8>, &[u8])> {
    let mut remaining = raw.strip_prefix(b"\"")?;
    let mut decoded = Vec::new();
    while let Some((&byte, rest)) = remaining.split_first() {
        remaining = rest;
        if byte == b'"' {
            return Some((decoded, remaining));
        }
        if byte != b'\\' {
            decoded.push(byte);
            continue;
        }
        let (&escape, rest) = remaining.split_first()?;
        remaining = rest;
        decoded.push(decode_escaped_game_byte(escape, &mut remaining));
    }
    None
}

fn decode_escaped_game_byte(escape: u8, remaining: &mut &[u8]) -> u8 {
    match escape {
        b'a' => 0x07,
        b'b' => 0x08,
        b'f' => 0x0c,
        b'n' => b'\n',
        b'r' => b'\r',
        b't' => b'\t',
        b'v' => 0x0b,
        b'\'' | b'"' | b'\\' | b'?' => escape,
        b'x' if remaining.first().is_some_and(u8::is_ascii_hexdigit) => {
            let mut code = 0_i32;
            while let Some((&next, rest)) = remaining.split_first() {
                if !next.is_ascii_hexdigit() {
                    break;
                }
                let digit = if next.is_ascii_digit() {
                    i32::from(next - b'0')
                } else {
                    // StdCompilerINIRead applies this lowercase conversion
                    // formula to every isxdigit byte, including uppercase.
                    i32::from(next) - i32::from(b'a') + 10
                };
                code = code.wrapping_mul(16).wrapping_add(digit);
                *remaining = rest;
            }
            code as u8
        }
        b'x' => b'x',
        digit @ b'0'..=b'7' => {
            let mut code = u32::from(digit - b'0');
            while let Some((&next @ b'0'..=b'7', rest)) = remaining.split_first() {
                code = code.wrapping_mul(8).wrapping_add(u32::from(next - b'0'));
                *remaining = rest;
            }
            code as u8
        }
        other => other,
    }
}

fn parse_message_command_restriction(
    raw: &[u8],
) -> Option<(MessageBoardCommandRestriction, &[u8])> {
    let raw = skip_horizontal_bytes(raw);
    if let Some((value, remaining)) = parse_i32_game_bytes(raw) {
        let restriction = match value {
            0 => MessageBoardCommandRestriction::Escaped,
            1 => MessageBoardCommandRestriction::Plain,
            2 => MessageBoardCommandRestriction::Identifier,
            _ => return None,
        };
        return Some((restriction, remaining));
    }
    let length = raw
        .iter()
        .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        .count();
    let restriction = match &raw[..length] {
        b"Escaped" => MessageBoardCommandRestriction::Escaped,
        b"Plain" => MessageBoardCommandRestriction::Plain,
        b"Identifier" => MessageBoardCommandRestriction::Identifier,
        _ => return None,
    };
    Some((restriction, &raw[length..]))
}

fn parse_gamma(value: &str) -> Option<GammaControlState> {
    let raw = clonk_script::c4_string_bytes(value);
    let mut remaining = raw.as_slice();
    let mut values = Vec::with_capacity(GammaControlState::RAMP_COUNT * 3);
    for index in 0..GammaControlState::RAMP_COUNT * 3 {
        if index != 0 {
            remaining = strip_game_separator(remaining, b',')?;
        }
        let (value, rest) = parse_u32_game_bytes(remaining)?;
        values.push(value);
        remaining = rest;
    }
    let mut gamma = GammaControlState::default();
    for (index, ramp) in values.chunks_exact(3).enumerate() {
        gamma.ramps[index].copy_from_slice(ramp);
    }
    Some(gamma)
}

fn skip_horizontal_bytes(mut raw: &[u8]) -> &[u8] {
    while raw.first().is_some_and(|byte| matches!(byte, b' ' | b'\t')) {
        raw = &raw[1..];
    }
    raw
}

fn strip_game_separator(raw: &[u8], separator: u8) -> Option<&[u8]> {
    skip_horizontal_bytes(raw).strip_prefix(&[separator])
}

fn parse_i32_game_bytes(raw: &[u8]) -> Option<(i32, &[u8])> {
    let (number, remaining) = game_number_prefix(raw)?;
    let number = std::str::from_utf8(number).ok()?;
    Some((parse_i32_prefix(number)?, remaining))
}

fn parse_u32_game_bytes(raw: &[u8]) -> Option<(u32, &[u8])> {
    let (number, remaining) = game_number_prefix(raw)?;
    let number = std::str::from_utf8(number).ok()?;
    Some((parse_u32_prefix(number)?, remaining))
}

fn game_number_prefix(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let raw = skip_horizontal_bytes(raw);
    let mut end = 0;
    if raw.starts_with(b"0x") || raw.starts_with(b"0X") {
        let digits = raw[2..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if digits != 0 {
            end = 2 + digits;
        }
    }
    if end == 0 {
        end += usize::from(matches!(raw.first(), Some(b'+') | Some(b'-')));
        let digits = raw[end..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits == 0 {
            return None;
        }
        end += digits;
    }
    Some((&raw[..end], &raw[end..]))
}

fn landscape_game_data(engine: &Engine) -> Option<LandscapeGameData> {
    engine
        .landscape
        .as_ref()
        .map(|landscape| LandscapeGameData {
            map_seed: landscape
                .raster_state()
                .map(|state| state.map_seed())
                .unwrap_or(0),
            left_open: landscape.left_open(),
            right_open: landscape.right_open(),
            top_open: landscape.top_open(),
            bottom_open: landscape.bottom_open(),
            gravity: engine.physics().gravity_as_c4fixed().val(),
            mat_modulation: landscape.modulation(),
            mode: landscape.mode(),
        })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitialNetworkMessageBoardCommand {
    pub name: String,
    pub script: String,
    pub restriction: MessageBoardCommandRestriction,
}

impl InitialNetworkMessageBoardCommand {
    pub fn speed() -> Self {
        Self {
            name: "speed".to_owned(),
            script: "SetGameSpeed(%d)".to_owned(),
            restriction: MessageBoardCommandRestriction::Escaped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageBoardCommandRestriction {
    Escaped,
    Plain,
    Identifier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UnsupportedInitialNetworkGameState {
    #[error("script globals")]
    ScriptGlobals,
    #[error("landscape runtime fields")]
    Landscape,
    #[error("sky runtime fields")]
    Sky,
    #[error("global effects")]
    GlobalEffects,
    #[error("scoreboard")]
    Scoreboard,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum InitialNetworkGameError {
    #[error("initial Game.txt cannot yet encode {0}")]
    Unsupported(UnsupportedInitialNetworkGameState),
    #[error("{field} value {value} does not fit the C++ signed 32-bit field")]
    IntegerOutOfRange { field: &'static str, value: u64 },
    #[error("duplicate message-board command `{name}`")]
    DuplicateMessageBoardCommand { name: String },
}

impl From<UnsupportedInitialNetworkGameState> for InitialNetworkGameError {
    fn from(state: UnsupportedInitialNetworkGameState) -> Self {
        Self::Unsupported(state)
    }
}

/// Serializes the initial network `Game.txt` bytes. `None` means C++
/// `C4Game::SaveData` would delete an all-default component.
///
/// `original_game_text` is consulted only for C++'s initial-save compatibility
/// hack: everything from the first byte-exact `[Player` marker is appended.
pub fn serialize_initial_network_game(
    data: &InitialNetworkGameData,
    original_game_text: Option<&[u8]>,
) -> Result<Option<Vec<u8>>, InitialNetworkGameError> {
    let mut writer = IniWriter::default();
    writer.push_section("Game", game_lines(data)?);
    if data.compiled_sections.script_engine().is_some() {
        writer.push_compiled_section(data.compiled_sections.script_engine());
    } else {
        writer.push_section("Script", script_lines(data));
    }
    writer.push_section("Weather", weather_lines(data));
    writer.push_section(
        "Landscape",
        data.landscape
            .as_ref()
            .map(landscape_lines)
            .unwrap_or_default(),
    );
    writer.push_compiled_section(data.compiled_sections.sky());
    writer.push_compiled_section(data.compiled_sections.effects());
    writer.push_compiled_section(data.compiled_sections.scoreboard());

    let mut output = writer.finish();
    if let Some(player_sections) = original_game_text.and_then(player_section_tail) {
        // C4Game::SaveData appends two line breaks even though the decompiler
        // output already ends in CRLF. Preserve the resulting two blank lines.
        output.extend_from_slice(b"\r\n\r\n");
        output.extend_from_slice(player_sections);
    }

    Ok((!output.is_empty()).then_some(output))
}

#[derive(Default)]
struct IniWriter {
    output: Vec<u8>,
}

impl IniWriter {
    fn push_section(&mut self, name: &str, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        if !self.output.is_empty() {
            self.output.extend_from_slice(b"\r\n");
        }
        self.output.push(b'[');
        self.output.extend_from_slice(name.as_bytes());
        self.output.extend_from_slice(b"]\r\n");
        for line in lines {
            self.output
                .extend_from_slice(&clonk_script::c4_string_bytes(&line));
            self.output.extend_from_slice(b"\r\n");
        }
    }

    fn finish(self) -> Vec<u8> {
        self.output
    }

    fn push_compiled_section(&mut self, section: Option<&[u8]>) {
        let Some(section) = section.filter(|section| !section.is_empty()) else {
            return;
        };
        // A present empty compiler block is runtime-significant on input
        // (notably [Sky], whose member defaults overwrite scenario state),
        // but StdCompilerINIWrite omits that empty naming block again.
        let mut lines = section.split(|byte| *byte == b'\n');
        let _header = lines.next();
        if !lines.any(|line| !line.trim_ascii().is_empty()) {
            return;
        }
        if !self.output.is_empty() {
            self.output.extend_from_slice(b"\r\n");
        }
        self.output.extend_from_slice(section);
        if !self.output.ends_with(b"\r\n") {
            self.output.extend_from_slice(b"\r\n");
        }
    }
}

fn game_lines(data: &InitialNetworkGameData) -> Result<Vec<String>, InitialNetworkGameError> {
    let mut lines = Vec::new();
    push_i32(&mut lines, "Time", data.time, 0);
    push_i32(&mut lines, "Frame", data.frame, 0);
    push_i32(&mut lines, "ControlTick", data.control_tick, 0);
    push_i32(
        &mut lines,
        "SyncRate",
        data.sync_rate,
        INITIAL_NETWORK_DEFAULT_SYNC_RATE,
    );
    push_i32(&mut lines, "Tick2", data.tick2, 0);
    push_i32(&mut lines, "Tick3", data.tick3, 0);
    push_i32(&mut lines, "Tick5", data.tick5, 0);
    push_i32(&mut lines, "Tick10", data.tick10, 0);
    push_i32(&mut lines, "Tick35", data.tick35, 0);
    push_i32(&mut lines, "Tick255", data.tick255, 0);
    push_i32(&mut lines, "Tick500", data.tick500, 0);
    push_i32(&mut lines, "Tick1000", data.tick1000, 0);
    push_i32(
        &mut lines,
        "ObjectEnumerationIndex",
        data.object_enumeration_index,
        0,
    );
    push_i32(&mut lines, "Rules", data.rules, 0);
    push_escaped_string(&mut lines, "PlayList", &data.play_list, "");
    push_raw_string(
        &mut lines,
        "CurrentScenarioSection",
        &data.current_scenario_section,
        "",
    );
    push_bool(&mut lines, "ResortAnyObj", data.resort_any_object, false);
    push_bool(&mut lines, "MusicEnabled", data.music_enabled, false);
    push_i32(&mut lines, "MusicLevel", data.music_level, 100);
    push_escaped_string(&mut lines, "NextMission", &data.next_mission.path, "");
    push_escaped_string(&mut lines, "NextMissionText", &data.next_mission.text, "");
    push_escaped_string(
        &mut lines,
        "NextMissionDesc",
        &data.next_mission.description,
        "",
    );
    if !data.message_board_commands.is_empty() {
        let mut names = HashSet::with_capacity(data.message_board_commands.len());
        let mut encoded = data.message_board_commands.len().to_string();
        for command in &data.message_board_commands {
            let name = c4_legacy_string_bytes(&command.name);
            if !names.insert(name.clone()) {
                return Err(InitialNetworkGameError::DuplicateMessageBoardCommand {
                    name: command.name.clone(),
                });
            }
            encoded.push(';');
            encoded.push_str(&quote_legacy_bytes(&name));
            encoded.push('=');
            encoded.push_str(&quote_legacy_bytes(&c4_legacy_string_bytes(
                &command.script,
            )));
            encoded.push(',');
            encoded.push_str(command.restriction.as_cpp_name());
        }
        lines.push(format!("MessageBoardCommands={encoded}"));
    }
    Ok(lines)
}

fn script_lines(data: &InitialNetworkGameData) -> Vec<String> {
    let mut lines = Vec::new();
    push_bool(&mut lines, "Go", data.script_go, false);
    push_i32(&mut lines, "Counter", data.script_counter, 0);
    lines
}

fn weather_lines(data: &InitialNetworkGameData) -> Vec<String> {
    let weather = data.environment;
    let mut lines = Vec::new();
    push_i32(&mut lines, "Season", weather.season, 0);
    push_i32(&mut lines, "YearSpeed", weather.year_speed, 0);
    push_i32(&mut lines, "SeasonDelay", weather.season_delay, 0);
    push_i32(&mut lines, "Wind", weather.wind, 0);
    push_i32(&mut lines, "TargetWind", weather.wind_target, 0);
    push_i32(&mut lines, "Temperature", weather.temperature, 0);
    push_i32(
        &mut lines,
        "TemperatureRange",
        weather.temperature_range,
        30,
    );
    push_i32(&mut lines, "Climate", weather.climate, 0);
    push_i32(&mut lines, "MeteoriteLevel", weather.meteorite, 0);
    push_i32(&mut lines, "VolcanoLevel", weather.volcano, 0);
    push_i32(&mut lines, "EarthquakeLevel", weather.earthquake, 0);
    push_i32(&mut lines, "LightningLevel", weather.lightning, 0);
    // Deliberately false: C4Weather::CompileFunc's default differs from
    // C4Weather::Default(), which initializes the live value to true.
    push_bool(&mut lines, "NoGamma", weather.no_gamma, false);
    if !data.gamma.is_default() {
        let values = data
            .gamma
            .ramps
            .iter()
            .flatten()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        lines.push(format!("Gamma={values}"));
    }
    lines
}

fn landscape_lines(landscape: &LandscapeGameData) -> Vec<String> {
    let defaults = LandscapeGameData::default();
    let mut lines = Vec::new();
    push_i32(&mut lines, "MapSeed", landscape.map_seed, defaults.map_seed);
    push_i32(
        &mut lines,
        "LeftOpen",
        landscape.left_open,
        defaults.left_open,
    );
    push_i32(
        &mut lines,
        "RightOpen",
        landscape.right_open,
        defaults.right_open,
    );
    push_bool(&mut lines, "TopOpen", landscape.top_open, defaults.top_open);
    push_bool(
        &mut lines,
        "BottomOpen",
        landscape.bottom_open,
        defaults.bottom_open,
    );
    push_i32(&mut lines, "Gravity", landscape.gravity, defaults.gravity);
    push_u32(
        &mut lines,
        "MatModulation",
        landscape.mat_modulation,
        defaults.mat_modulation,
    );
    push_i32(&mut lines, "Mode", landscape.mode, defaults.mode);
    lines
}

/// Parses the first `[Landscape]` block from C++ `Game.txt` input.
///
/// Section and field names use StdCompilerINIRead's exact spelling. Missing
/// or malformed fields (and a missing or empty section) take the same
/// per-field defaults as `C4Landscape::CompileFunc`'s default adaptors. Like
/// C4's string-backed compiler, bytes after the first NUL are not visible.
pub fn parse_landscape_game_data(source: &[u8]) -> LandscapeGameData {
    let source = source.split(|byte| *byte == 0).next().unwrap_or_default();
    let source = String::from_utf8_lossy(source);
    let mut landscape = LandscapeGameData::default();
    let mut found_landscape = false;
    let mut sections: Vec<(usize, bool)> = Vec::new();
    let mut seen = HashSet::new();

    for raw_line in source.split(['\r', '\n']) {
        let indent = raw_line
            .as_bytes()
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count();
        let line = &raw_line[indent..];
        if let Some(section) = ini_section_name(line) {
            while sections.last().is_some_and(|(level, _)| *level >= indent) {
                sections.pop();
            }
            let is_landscape = sections.is_empty() && !found_landscape && section == "Landscape";
            found_landscape |= is_landscape;
            sections.push((indent, is_landscape));
            continue;
        }
        let Some((name, value)) = ini_named_value(line) else {
            continue;
        };
        let value_indent = indent + 1;
        while sections
            .last()
            .is_some_and(|(level, _)| *level >= value_indent)
        {
            sections.pop();
        }
        if !sections.last().is_some_and(|(_, target)| *target) {
            continue;
        }
        if !seen.insert(name.to_owned()) {
            continue;
        }
        match name {
            "MapSeed" => {
                landscape.map_seed = parse_i32_prefix(value).unwrap_or_default();
            }
            "LeftOpen" => {
                landscape.left_open = parse_i32_prefix(value).unwrap_or_default();
            }
            "RightOpen" => {
                landscape.right_open = parse_i32_prefix(value).unwrap_or_default();
            }
            "TopOpen" => {
                landscape.top_open = parse_bool_prefix(value).unwrap_or_default();
            }
            "BottomOpen" => {
                landscape.bottom_open = parse_bool_prefix(value).unwrap_or_default();
            }
            "Gravity" => {
                landscape.gravity =
                    parse_i32_prefix(value).unwrap_or(LANDSCAPE_DEFAULT_GRAVITY_RAW);
            }
            "MatModulation" => {
                landscape.mat_modulation = parse_u32_prefix(value).unwrap_or_default();
            }
            "Mode" => {
                landscape.mode = parse_i32_prefix(value).unwrap_or_default();
            }
            _ => {}
        }
    }

    landscape
}

fn ini_section_name(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let name_end = ini_name_end(rest)?;
    let mut delimiter = name_end;
    while rest
        .as_bytes()
        .get(delimiter)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        delimiter += 1;
    }
    (rest.as_bytes().get(delimiter) == Some(&b']')).then_some(&rest[..name_end])
}

fn ini_named_value(line: &str) -> Option<(&str, &str)> {
    let name_end = ini_name_end(line)?;
    let mut delimiter = name_end;
    while line
        .as_bytes()
        .get(delimiter)
        .is_some_and(|byte| matches!(byte, b' ' | b'\t'))
    {
        delimiter += 1;
    }
    // `then_some` evaluates its argument EAGERLY, so the value slice was taken
    // even when the byte is not `=` — and when the whitespace scan above stops
    // at the end of the line, `delimiter + 1` is past it and the slice panics.
    // `then` defers it to the branch that can be taken
    // (clonk-org/clonk-rs#961).
    (line.as_bytes().get(delimiter) == Some(&b'='))
        .then(|| (&line[..name_end], &line[delimiter + 1..]))
}

fn ini_name_end(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut end = 1;
    while bytes
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b' ' | b'_'))
    {
        end += 1;
    }
    Some(end)
}

fn numeric_prefix(value: &str, radix: u32) -> Option<(&str, bool)> {
    let bytes = value.as_bytes();
    let negative = bytes.first() == Some(&b'-');
    let mut start = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    let digits_start = start;
    while bytes.get(start).is_some_and(|byte| match radix {
        10 => byte.is_ascii_digit(),
        16 => byte.is_ascii_hexdigit(),
        _ => false,
    }) {
        start += 1;
    }
    (start > digits_start).then_some((&value[digits_start..start], negative))
}

fn digit_prefix(value: &str, radix: u32) -> &str {
    let count = value
        .bytes()
        .take_while(|byte| match radix {
            10 => byte.is_ascii_digit(),
            16 => byte.is_ascii_hexdigit(),
            _ => false,
        })
        .count();
    &value[..count]
}

fn parse_saturating_u64(digits: &str, radix: u32) -> (u64, bool) {
    let mut value = 0_u64;
    let mut overflow = false;
    for byte in digits.bytes() {
        let digit = (byte as char)
            .to_digit(radix)
            .expect("numeric_prefix admits only radix digits") as u64;
        match value
            .checked_mul(u64::from(radix))
            .and_then(|value| value.checked_add(digit))
        {
            Some(next) if !overflow => value = next,
            _ => {
                value = u64::MAX;
                overflow = true;
            }
        }
    }
    (value, overflow)
}

fn parse_i32_prefix(value: &str) -> Option<i32> {
    let value = value.trim_start_matches([' ', '\t']);
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        let digits = digit_prefix(hex, 16);
        if digits.is_empty() {
            return Some(0);
        }
        let (magnitude, overflow) = parse_saturating_u64(digits, 16);
        let value = if overflow || magnitude > i64::MAX as u64 {
            i64::MAX
        } else {
            magnitude as i64
        };
        return Some(value as i32);
    }
    let (digits, negative) = numeric_prefix(value, 10)?;
    let (magnitude, overflow) = parse_saturating_u64(digits, 10);
    let value = if negative {
        if overflow || magnitude > (i64::MAX as u64) + 1 {
            i64::MIN
        } else if magnitude == (i64::MAX as u64) + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else if overflow || magnitude > i64::MAX as u64 {
        i64::MAX
    } else {
        magnitude as i64
    };
    Some(value as i32)
}

fn parse_u32_prefix(value: &str) -> Option<u32> {
    let value = value.trim_start_matches([' ', '\t']);
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        let digits = digit_prefix(hex, 16);
        if digits.is_empty() {
            return Some(0);
        }
        return Some(parse_saturating_u64(digits, 16).0 as u32);
    }
    let (digits, negative) = numeric_prefix(value, 10)?;
    let (magnitude, overflow) = parse_saturating_u64(digits, 10);
    let value = if overflow {
        u64::MAX
    } else if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    Some(value as u32)
}

fn parse_bool_prefix(value: &str) -> Option<bool> {
    let bytes = value.as_bytes();
    if (bytes.first() == Some(&b'1') && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit()))
        || value.get(..4) == Some("true")
    {
        Some(true)
    } else if (bytes.first() == Some(&b'0')
        && bytes.get(1).is_none_or(|byte| !byte.is_ascii_digit()))
        || value.get(..5) == Some("false")
    {
        Some(false)
    } else {
        None
    }
}

fn push_i32(lines: &mut Vec<String>, name: &str, value: i32, default: i32) {
    if value != default {
        lines.push(format!("{name}={value}"));
    }
}

fn push_u32(lines: &mut Vec<String>, name: &str, value: u32, default: u32) {
    if value != default {
        lines.push(format!("{name}={value}"));
    }
}

fn push_bool(lines: &mut Vec<String>, name: &str, value: bool, default: bool) {
    if value != default {
        lines.push(format!("{name}={}", if value { "true" } else { "false" }));
    }
}

fn push_escaped_string(lines: &mut Vec<String>, name: &str, value: &str, default: &str) {
    if value != default {
        lines.push(format!("{name}={}", quote_legacy_text(value)));
    }
}

fn push_raw_string(lines: &mut Vec<String>, name: &str, value: &str, default: &str) {
    if value != default {
        let bytes = legacy_c_string_bytes(value);
        let value =
            std::str::from_utf8(bytes).expect("a NUL boundary cannot split a UTF-8 code point");
        lines.push(format!("{name}={value}"));
    }
}

fn quote_legacy_text(value: &str) -> String {
    quote_legacy_bytes(&c4_legacy_string_bytes(value))
}

fn quote_legacy_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() + 2);
    output.push('"');
    let mut last_was_numeric_escape = false;
    for &byte in bytes {
        let printable = (b' '..=b'~').contains(&byte);
        if !printable
            || byte == b'\\'
            || byte == b'"'
            || (last_was_numeric_escape && byte.is_ascii_digit())
        {
            last_was_numeric_escape = false;
            match byte {
                0x07 => output.push_str("\\a"),
                0x08 => output.push_str("\\b"),
                0x0c => output.push_str("\\f"),
                b'\n' => output.push_str("\\n"),
                b'\r' => output.push_str("\\r"),
                b'\t' => output.push_str("\\t"),
                0x0b => output.push_str("\\v"),
                b'"' => output.push_str("\\\""),
                b'\\' => output.push_str("\\\\"),
                _ => {
                    output.push('\\');
                    output.push_str(&format!("{byte:o}"));
                    last_was_numeric_escape = true;
                }
            }
        } else {
            output.push(char::from(byte));
            last_was_numeric_escape = false;
        }
    }
    output.push('"');
    output
}

fn c4_legacy_string_bytes(value: &str) -> Vec<u8> {
    let mut bytes = clonk_script::c4_string_bytes(value);
    if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
        bytes.truncate(nul);
    }
    bytes
}

fn legacy_c_string_bytes(value: &str) -> &[u8] {
    let bytes = value.as_bytes();
    &bytes[..bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len())]
}

fn player_section_tail(source: &[u8]) -> Option<&[u8]> {
    let source = &source[..source
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(source.len())];
    source
        .windows(b"[Player".len())
        .position(|window| window == b"[Player")
        .map(|position| &source[position..])
}

impl MessageBoardCommandRestriction {
    const fn as_cpp_name(self) -> &'static str {
        match self {
            Self::Escaped => "Escaped",
            Self::Plain => "Plain",
            Self::Identifier => "Identifier",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key with no `=`, padded to the end of the line, must not panic
    /// (clonk-org/clonk-rs#961).
    ///
    /// An INI name may contain spaces (`ini_name_end` accepts them alongside
    /// alphanumerics and underscores), so a line that is *only* a padded name
    /// runs `name_end` all the way to the end of the line. `ini_named_value`
    /// then leaves `delimiter == line.len()`, the guard
    /// `get(delimiter) == Some(&b'=')` is false — and `then_some` evaluates its
    /// argument EAGERLY, so `&line[delimiter + 1..]` was still taken and sliced
    /// one past the end.
    ///
    /// Scenario and save components reach this parser from downloads, records
    /// and peers, so a malformed line was a reachable panic. `then` defers the
    /// slice to the branch that can actually be taken.
    #[test]
    fn a_name_that_runs_to_the_end_of_line_has_no_value() {
        assert_eq!(ini_named_value("abc  "), None);
        assert_eq!(ini_named_value("abc"), None);
        assert_eq!(ini_named_value("a"), None);
        assert_eq!(ini_named_value("a_b 1"), None);
        // The ordinary forms still parse. Note the name keeps its padding —
        // spaces are name characters here, which is the same reason the scan
        // above can reach the end of the line.
        assert_eq!(ini_named_value("abc=x"), Some(("abc", "x")));
        assert_eq!(ini_named_value("abc  =x"), Some(("abc  ", "x")));
        assert_eq!(ini_named_value("abc="), Some(("abc", "")));
    }

    #[test]
    fn pristine_initial_state_matches_clean_cpp_dynamic_payload() {
        let bytes = serialize_initial_network_game(&InitialNetworkGameData::default(), None)
            .expect("pristine state serializes")
            .expect("C++ emits the speed command and NoGamma");

        assert_eq!(
            bytes,
            b"[Game]\r\nMessageBoardCommands=1;\"speed\"=\"SetGameSpeed(%d)\",Escaped\r\n\r\n[Weather]\r\nNoGamma=true\r\n"
        );
    }

    #[test]
    fn resort_any_object_ignores_unrelated_resort_proc_nodes() {
        use crate::compat::ObjectOrderCommand;

        let mut engine = Engine::new();
        engine
            .execution
            .pending_object_order_commands
            .push(ObjectOrderCommand::SetRelative {
                relative_to: crate::ObjectId::new(1),
                object: crate::ObjectId::new(2),
                after: false,
            });
        engine
            .execution
            .pending_object_order_commands
            .push(ObjectOrderCommand::SortByCategory);
        assert!(
            !InitialNetworkGameData::from_engine_live(&engine)
                .expect("relative/category ResortProc captures")
                .resort_any_object,
            "ResortAnyObj serializes only C4Game::fResortAnyObject"
        );

        engine
            .execution
            .pending_object_order_commands
            .push(ObjectOrderCommand::ResortUnsortedSweep);
        assert!(
            InitialNetworkGameData::from_engine_live(&engine)
                .expect("native resort trigger captures")
                .resort_any_object
        );
    }

    #[test]
    fn clean_source_rule_payload_uses_map_and_command_compilers() {
        let data = InitialNetworkGameData::default();

        let bytes = serialize_initial_network_game(&data, None)
            .expect("command serializes")
            .expect("nondefault command emits Game.txt");

        let expected = b"[Game]\r\nMessageBoardCommands=1;\"speed\"=\"SetGameSpeed(%d)\",Escaped\r\n\r\n[Weather]\r\nNoGamma=true\r\n";
        assert_eq!(bytes, expected);
        assert_eq!(bytes.len(), 94);
        // SHA-256: ffc2e1631afa3d2bbe91402d6782dd1c3bc52be269b2bcdc04c2df35b71e97fe
    }

    #[test]
    fn escaped_text_preserves_utf8_bytes_as_octal() {
        let mut data = InitialNetworkGameData::default();
        data.next_mission.text = "café".to_owned();

        let bytes = serialize_initial_network_game(&data, None)
            .expect("UTF-8 text serializes")
            .expect("nondefault text emits Game.txt");
        let text = std::str::from_utf8(&bytes).expect("output syntax is ASCII");

        assert!(text.contains("NextMissionText=\"caf\\303\\251\"\r\n"));
    }

    #[test]
    fn game_script_and_weather_fields_keep_cpp_order_and_defaults() {
        let mut data = InitialNetworkGameData {
            time: 1,
            frame: 2,
            control_tick: 3,
            sync_rate: 4,
            tick2: 5,
            tick3: 6,
            tick5: 7,
            tick10: 8,
            tick35: 9,
            tick255: 10,
            tick500: 11,
            tick1000: 12,
            object_enumeration_index: 13,
            rules: 14,
            play_list: "Music*.ogg".to_owned(),
            current_scenario_section: "Main".to_owned(),
            resort_any_object: true,
            music_enabled: true,
            music_level: 99,
            next_mission: NextMissionState {
                path: "Next.c4s".to_owned(),
                text: "Continue".to_owned(),
                description: "Next round".to_owned(),
            },
            message_board_commands: Vec::new(),
            script_go: true,
            script_counter: 15,
            ..Default::default()
        };
        data.environment.season = 16;
        data.environment.year_speed = 17;
        data.environment.season_delay = 18;
        data.environment.wind = 19;
        data.environment.wind_target = 20;
        data.environment.temperature = 21;
        data.environment.temperature_range = 22;
        data.environment.climate = 23;
        data.environment.meteorite = 24;
        data.environment.volcano = 25;
        data.environment.earthquake = 26;
        data.environment.lightning = 27;

        let bytes = serialize_initial_network_game(&data, None)
            .expect("modeled fields serialize")
            .expect("nondefault fields emit Game.txt");

        assert_eq!(
            bytes,
            b"[Game]\r\nTime=1\r\nFrame=2\r\nControlTick=3\r\nSyncRate=4\r\nTick2=5\r\nTick3=6\r\nTick5=7\r\nTick10=8\r\nTick35=9\r\nTick255=10\r\nTick500=11\r\nTick1000=12\r\nObjectEnumerationIndex=13\r\nRules=14\r\nPlayList=\"Music*.ogg\"\r\nCurrentScenarioSection=Main\r\nResortAnyObj=true\r\nMusicEnabled=true\r\nMusicLevel=99\r\nNextMission=\"Next.c4s\"\r\nNextMissionText=\"Continue\"\r\nNextMissionDesc=\"Next round\"\r\n\r\n[Script]\r\nGo=true\r\nCounter=15\r\n\r\n[Weather]\r\nSeason=16\r\nYearSpeed=17\r\nSeasonDelay=18\r\nWind=19\r\nTargetWind=20\r\nTemperature=21\r\nTemperatureRange=22\r\nClimate=23\r\nMeteoriteLevel=24\r\nVolcanoLevel=25\r\nEarthquakeLevel=26\r\nLightningLevel=27\r\nNoGamma=true\r\n"
        );
    }

    #[test]
    fn changed_gamma_emits_all_27_values() {
        let mut data = InitialNetworkGameData::default();
        data.message_board_commands.clear();
        data.gamma.ramps[1][1] = 7;

        let bytes = serialize_initial_network_game(&data, None)
            .expect("gamma serializes")
            .expect("changed gamma emits Game.txt");

        assert_eq!(
            bytes,
            b"[Weather]\r\nNoGamma=true\r\nGamma=0,8421504,16777215,0,7,16777215,0,8421504,16777215,0,8421504,16777215,0,8421504,16777215,0,8421504,16777215,0,8421504,16777215,0,8421504,16777215,0,8421504,16777215\r\n"
        );
    }

    #[test]
    fn initial_save_reinserts_player_tail_byte_for_byte() {
        let source = b"[Game]\nTime=999\n[Player2]\nName=Legacy\n";
        let mut data = InitialNetworkGameData::default();
        data.message_board_commands.clear();
        let bytes = serialize_initial_network_game(&data, Some(source))
            .expect("player tail serializes")
            .expect("NoGamma and player tail emit Game.txt");

        assert_eq!(
            bytes,
            b"[Weather]\r\nNoGamma=true\r\n\r\n\r\n[Player2]\nName=Legacy\n"
        );
    }

    #[test]
    fn runtime_parser_obeys_ini_tree_and_legacy_string_rules() {
        let source = b" [Game]\r\n Time=7\r\n PlayList=\"A\\x42\"\r\n NextMission=  Path with trailing spaces  \r\n CurrentScenarioSection=  123456789012345678901234567890TAIL\r\n  [Nested]\r\n  Frame=8\r\n ControlTick=9\r\n [Weather]\r\n Wind=4\r\n";

        let data = parse_initial_network_game_data(source);

        assert_eq!(data.time, 7, "an indented first section remains root-level");
        assert_eq!(data.frame, 0, "nested values are not direct Game children");
        assert_eq!(data.control_tick, 9);
        assert_eq!(data.play_list, "AB");
        assert_eq!(data.next_mission.path, "Path with trailing spaces  ");
        assert_eq!(
            data.current_scenario_section, "123456789012345678901234567890",
            "the fixed C4MaxName buffer keeps only its first 30 native bytes"
        );
        assert_eq!(data.environment.wind, 4);

        let spaced_quote = parse_initial_network_game_data(
            b"[Game]\r\nPlayList=  \"quotes are raw after whitespace\"\r\n",
        );
        assert_eq!(
            spaced_quote.play_list,
            "\"quotes are raw after whitespace\""
        );
    }

    #[test]
    fn canonical_cpp_runtime_sections_roundtrip_in_compile_order() {
        // Frozen `StdCompilerINIWrite` shapes from C4Sky::CompileFunc,
        // C4Effect::CompileFunc and C4Scoreboard::CompileFunc. Effects stay
        // opaque because their optional C4Value graph is recursive.
        let source = b"[Game]\r\nTime=7\r\n\r\n\
[Scoreboard]\r\nRows=2\r\nCols=2\r\nDlgShow=1\r\nCell0_0String=\"Scores\"\r\nCell0_0Value=-1\r\nCell1_0String=\"Round\"\r\nCell1_0Value=1234\r\nCell0_1String=\"Alice\"\r\nCell0_1Value=7\r\nCell1_1String=\"42\"\r\nCell1_1Value=42\r\n\r\n\
[Sky]\r\nX=65536\r\nY=-65536\r\nXDir=32768\r\nYDir=-32768\r\nModulation=4278255360\r\nParX=12\r\nParY=13\r\nParMode=1\r\nBackClr=-16711936\r\nBackClrEnabled=true\r\n\r\n\
[Effects]\r\nGlobalEffects=Fog(1,100,7,3,0,FOGG)\r\n";
        let expected = b"[Game]\r\nTime=7\r\n\r\n\
[Sky]\r\nX=65536\r\nY=-65536\r\nXDir=32768\r\nYDir=-32768\r\nModulation=4278255360\r\nParX=12\r\nParY=13\r\nParMode=1\r\nBackClr=-16711936\r\nBackClrEnabled=true\r\n\r\n\
[Effects]\r\nGlobalEffects=Fog(1,100,7,3,0,FOGG)\r\n\r\n\
[Scoreboard]\r\nRows=2\r\nCols=2\r\nDlgShow=1\r\nCell0_0String=\"Scores\"\r\nCell0_0Value=-1\r\nCell1_0String=\"Round\"\r\nCell1_0Value=1234\r\nCell0_1String=\"Alice\"\r\nCell0_1Value=7\r\nCell1_1String=\"42\"\r\nCell1_1Value=42\r\n";

        let data = parse_initial_network_game_data(source);
        assert_eq!(
            data.compiled_sections.sky(),
            Some(
                b"[Sky]\r\nX=65536\r\nY=-65536\r\nXDir=32768\r\nYDir=-32768\r\nModulation=4278255360\r\nParX=12\r\nParY=13\r\nParMode=1\r\nBackClr=-16711936\r\nBackClrEnabled=true\r\n"
                    .as_slice()
            ),
            "the retained section includes its exact canonical native bytes"
        );
        assert_eq!(
            serialize_initial_network_game(&data, None),
            Ok(Some(expected.to_vec())),
            "C4Game emits opaque runtime blocks in CompileFunc order"
        );
    }

    #[test]
    fn explicit_empty_compiled_sections_keep_runtime_presence_but_redecompile_away() {
        let data =
            parse_initial_network_game_data(b"[Sky]\r\n\r\n[Effects]\r\n\r\n[Scoreboard]\r\n");
        assert_eq!(data.compiled_sections.sky(), Some(b"[Sky]\r\n".as_slice()));
        assert_eq!(
            data.compiled_sections.effects(),
            Some(b"[Effects]\r\n".as_slice())
        );
        assert_eq!(
            data.compiled_sections.scoreboard(),
            Some(b"[Scoreboard]\r\n".as_slice())
        );
        assert_eq!(
            serialize_initial_network_game(&data, None),
            Ok(None),
            "the C++ decompiler omits empty naming blocks"
        );
    }

    #[test]
    fn runtime_parser_distinguishes_an_absent_component_from_a_nul_source() {
        let absent = parse_initial_network_game_data(b"");
        assert_eq!(
            absent.message_board_commands,
            vec![InitialNetworkMessageBoardCommand::speed()]
        );
        assert!(absent.environment.no_gamma);

        let present = parse_initial_network_game_data(b"\0ignored");
        assert!(present.message_board_commands.is_empty());
        assert!(!present.environment.no_gamma);
    }

    #[test]
    fn runtime_parser_compiles_message_map_separators_counts_and_duplicates() {
        let data = parse_initial_network_game_data(
            b"[Game]\r\nMessageBoardCommands=0x3 ;\"dup\" =\"One()\" , Escaped ;\"dup\" =\"\\x54wo()\" , Plain ;\"id\" =\"Arg(%s)\" , 2 ignored-at-end\r\n",
        );

        assert_eq!(
            data.message_board_commands,
            vec![
                InitialNetworkMessageBoardCommand {
                    name: "dup".to_owned(),
                    script: "Two()".to_owned(),
                    restriction: MessageBoardCommandRestriction::Plain,
                },
                InitialNetworkMessageBoardCommand {
                    name: "id".to_owned(),
                    script: "Arg(%s)".to_owned(),
                    restriction: MessageBoardCommandRestriction::Identifier,
                },
            ],
            "unordered_map assignment keeps one key and the last value"
        );

        let wrapped_negative = parse_initial_network_game_data(
            b"[Game]\r\nMessageBoardCommands=2147483648;\"ignored\"=\"Ignored()\",Plain\r\n",
        );
        assert!(wrapped_negative.message_board_commands.is_empty());

        let whitespace_after_separator = parse_initial_network_game_data(
            b"[Game]\r\nMessageBoardCommands=1; \"ignored\"=\"Ignored()\",Plain\r\n",
        );
        assert!(whitespace_after_separator.message_board_commands.is_empty());
    }

    #[test]
    fn runtime_parser_gamma_requires_internal_separators_but_ignores_a_tail() {
        let mut values = GammaControlState::default()
            .ramps
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        values[0] = 1;
        let encoded = values
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let source = format!("[Weather]\r\nGamma={encoded},999 ignored\r\n");
        let data = parse_initial_network_game_data(source.as_bytes());
        assert_eq!(data.gamma.ramps[0][0], 1);

        let malformed = format!("[Weather]\r\nGamma=1junk,{}\r\n", &encoded[2..]);
        let data = parse_initial_network_game_data(malformed.as_bytes());
        assert_eq!(data.gamma, GammaControlState::default());
    }

    #[test]
    fn parsed_native_string_bytes_recompile_as_the_same_octal_bytes() {
        let data = parse_initial_network_game_data(
            b"[Game]\r\nPlayList=\"\\377\"\r\nCurrentScenarioSection=\xff\r\n",
        );
        assert_eq!(clonk_script::c4_string_bytes(&data.play_list), vec![0xff]);
        assert_eq!(
            clonk_script::c4_string_bytes(&data.current_scenario_section),
            vec![0xff]
        );

        let bytes = serialize_initial_network_game(&data, None)
            .expect("parsed native byte serializes")
            .expect("playlist emits Game.txt");
        assert!(bytes
            .windows(b"PlayList=\"\\377\"".len())
            .any(|window| window == b"PlayList=\"\\377\""));
        assert!(bytes
            .windows(b"CurrentScenarioSection=\xff\r\n".len())
            .any(|window| window == b"CurrentScenarioSection=\xff\r\n"));
    }

    #[test]
    fn numeric_escape_disambiguates_following_digits_like_std_compiler() {
        let mut data = InitialNetworkGameData::default();
        data.play_list = "\u{1}12\n\"\\".to_owned();

        let bytes = serialize_initial_network_game(&data, None)
            .expect("control bytes serialize")
            .expect("playlist emits Game.txt");
        let text = std::str::from_utf8(&bytes).expect("output syntax is ASCII");

        assert!(text.contains("PlayList=\"\\1\\61\\62\\n\\\"\\\\\"\r\n"));
    }

    #[test]
    fn landscape_block_keeps_cpp_order_widths_and_raw_gravity() {
        let mut data = InitialNetworkGameData::default();
        data.landscape = Some(LandscapeGameData {
            map_seed: -7,
            left_open: 11,
            right_open: 12,
            top_open: true,
            bottom_open: true,
            gravity: -123_456,
            mat_modulation: 4_000_000_000,
            mode: 3,
        });

        let bytes = serialize_initial_network_game(&data, None)
            .expect("landscape fields serialize")
            .expect("nondefault landscape emits Game.txt");

        assert_eq!(
            bytes,
            b"[Game]\r\nMessageBoardCommands=1;\"speed\"=\"SetGameSpeed(%d)\",Escaped\r\n\r\n[Weather]\r\nNoGamma=true\r\n\r\n[Landscape]\r\nMapSeed=-7\r\nLeftOpen=11\r\nRightOpen=12\r\nTopOpen=true\r\nBottomOpen=true\r\nGravity=-123456\r\nMatModulation=4000000000\r\nMode=3\r\n"
        );
        assert_eq!(
            parse_landscape_game_data(&bytes),
            data.landscape.expect("landscape data remains present")
        );
    }

    #[test]
    fn landscape_parser_matches_exact_names_lowercase_bools_and_hex_numbers() {
        let source = b"[lAnDsCaPe]\r\nMapSeed=999\r\n\r\n[Landscape]\r\nmapseed=888\r\nMapSeed =777\r\nMapSeed=0xFFFFFFED trailing\r\nMapSeed=99\r\nLeftOpen=0XFFFFFFFF\r\nRightOpen=42\r\ntopopen=TRUE\r\nTopOpen=true\r\nBottomOpen=1\r\nGravity=0xFFFFE21A\r\nMatModulation=0xFFFFFFFF\r\nMode=0X2\r\n\r\n[Weather]\r\nWind=8\r\n";

        assert_eq!(
            parse_landscape_game_data(source),
            LandscapeGameData {
                map_seed: -19,
                left_open: -1,
                right_open: 42,
                top_open: true,
                bottom_open: true,
                gravity: -7_654,
                mat_modulation: u32::MAX,
                mode: 2,
            }
        );
    }

    #[test]
    fn empty_landscape_section_uses_cpp_compile_defaults() {
        assert_eq!(
            parse_landscape_game_data(b"[Landscape]\r\n"),
            LandscapeGameData::default()
        );
        assert_eq!(
            parse_landscape_game_data(b"[LANDSCAPE]\r\nMapSeed=7\r\n"),
            LandscapeGameData::default(),
            "wrong-case section names remain invisible"
        );
        assert_eq!(
            LandscapeGameData::default().gravity,
            LANDSCAPE_DEFAULT_GRAVITY_RAW
        );
        assert_eq!(LANDSCAPE_DEFAULT_GRAVITY_RAW, 13_107);
    }

    #[test]
    fn recognized_invalid_landscape_values_take_compile_defaults() {
        assert_eq!(
            parse_landscape_game_data(
                b"[Landscape]\r\nMapSeed=oops\r\nTopOpen= true\r\nBottomOpen=TRUE\r\nGravity=\r\nMode=3\r\n",
            ),
            LandscapeGameData {
                mode: 3,
                ..LandscapeGameData::default()
            }
        );
    }

    #[test]
    fn landscape_parser_matches_ini_tree_root_and_libc_number_edges() {
        let source = b"[Game]\r\n [Landscape]\r\n MapSeed=7\r\n\xef\xbb\xbf[Landscape]\r\nMapSeed=8\r\n[Landscape]\r\nMapSeed=9223372036854775808\r\nGravity=0x\r\nMatModulation=18446744073709551616\r\n";
        assert_eq!(
            parse_landscape_game_data(source),
            LandscapeGameData {
                map_seed: -1,
                gravity: 0,
                mat_modulation: u32::MAX,
                ..LandscapeGameData::default()
            }
        );
        assert_eq!(
            parse_landscape_game_data(b"[Landscape]\rMapSeed=7"),
            LandscapeGameData {
                map_seed: 7,
                ..LandscapeGameData::default()
            }
        );
    }

    #[test]
    fn engine_capture_includes_represented_landscape_runtime_fields() {
        let mut engine = Engine::new();
        let mut landscape = crate::Landscape::flat(2, 2);
        assert!(landscape.set_mode(crate::LANDSCAPE_MODE_STATIC));
        landscape.set_modulation(0xaabb_ccdd);
        landscape.set_border_open(7, 9, false, true);
        engine.set_physics(crate::PhysicsSettings::new(77, 12, -12));
        let gravity = engine.physics().gravity_as_c4fixed().val();
        engine.set_landscape(landscape);

        assert_eq!(
            InitialNetworkGameData::from_engine(&engine)
                .expect("represented landscape captures")
                .landscape,
            Some(LandscapeGameData {
                map_seed: 0,
                left_open: 7,
                right_open: 9,
                top_open: false,
                bottom_open: true,
                gravity,
                mat_modulation: 0xaabb_ccdd,
                mode: crate::LANDSCAPE_MODE_STATIC,
            })
        );
        assert_eq!(
            InitialNetworkGameData::for_initial_record(&engine).landscape,
            InitialNetworkGameData::from_engine(&engine)
                .unwrap()
                .landscape
        );
    }

    #[test]
    fn live_capture_retains_true_cached_flag_rule_after_fgrv_disappears() {
        let mut state = Engine::new().capture_state();
        state.flag_removeable = true;
        let mut engine = Engine::new();
        engine
            .restore_state(&state)
            .expect("cached rule state restores");

        assert!(engine.objects.is_empty(), "fixture has no current FGRV");
        assert_eq!(
            InitialNetworkGameData::from_engine_live(&engine)
                .expect("cached-rule engine captures")
                .rules
                & 4,
            4,
            "a save before UpdateRules must retain the cached true bit"
        );
    }

    #[test]
    fn live_capture_retains_false_cached_flag_rule_after_fgrv_appears() {
        let mut engine = Engine::new();
        engine
            .register_definition(
                crate::Definition::from_script("FGRV", "Flag-removal rule", "")
                    .expect("fixture definition compiles"),
            )
            .expect("fixture definition registers");
        engine
            .spawn_object(crate::SpawnConfig::new("FGRV"))
            .expect("fixture FGRV spawns");

        assert!(
            !engine.capture_state().flag_removeable,
            "the cached bit remains false until UpdateRules"
        );
        assert_eq!(
            InitialNetworkGameData::from_engine_live(&engine)
                .expect("cached-rule engine captures")
                .rules
                & 4,
            0,
            "a save before UpdateRules must retain the cached false bit"
        );
    }

    #[test]
    fn pristine_engine_capture_produces_pristine_source_rule_payload() {
        let data = InitialNetworkGameData::from_engine(&Engine::new())
            .expect("a pristine engine uses only modeled fields");

        assert_eq!(
            serialize_initial_network_game(&data, None),
            Ok(Some(
                b"[Game]\r\nMessageBoardCommands=1;\"speed\"=\"SetGameSpeed(%d)\",Escaped\r\n\r\n[Weather]\r\nNoGamma=true\r\n".to_vec()
            ))
        );
    }

    #[test]
    fn duplicate_message_command_is_rejected_before_ambiguous_output() {
        let mut data = InitialNetworkGameData::default();
        let command = InitialNetworkMessageBoardCommand {
            name: "same".to_owned(),
            script: "First()".to_owned(),
            restriction: MessageBoardCommandRestriction::Plain,
        };
        data.message_board_commands = vec![
            command.clone(),
            InitialNetworkMessageBoardCommand {
                script: "Second()".to_owned(),
                ..command
            },
        ];

        assert_eq!(
            serialize_initial_network_game(&data, None),
            Err(InitialNetworkGameError::DuplicateMessageBoardCommand {
                name: "same".to_owned()
            })
        );
    }

    #[test]
    fn message_commands_serialize_c4_projected_bytes_not_private_use_utf8() {
        let mut data = InitialNetworkGameData::default();
        data.message_board_commands = vec![InitialNetworkMessageBoardCommand {
            name: clonk_script::c4_string_from_bytes(&[0xff]),
            script: clonk_script::c4_string_from_bytes(&[0xfe]),
            restriction: MessageBoardCommandRestriction::Identifier,
        }];

        assert_eq!(
            serialize_initial_network_game(&data, None),
            Ok(Some(
                b"[Game]\r\nMessageBoardCommands=1;\"\\377\"=\"\\376\",Identifier\r\n\r\n[Weather]\r\nNoGamma=true\r\n"
                    .to_vec()
            ))
        );
    }

    #[test]
    fn every_message_command_restriction_uses_cpp_name() {
        assert_eq!(
            MessageBoardCommandRestriction::Escaped.as_cpp_name(),
            "Escaped"
        );
        assert_eq!(MessageBoardCommandRestriction::Plain.as_cpp_name(), "Plain");
        assert_eq!(
            MessageBoardCommandRestriction::Identifier.as_cpp_name(),
            "Identifier"
        );
    }

    #[test]
    fn compiler_defaults_can_delete_game_component() {
        let mut data = InitialNetworkGameData::default();
        data.environment.no_gamma = false;
        data.message_board_commands.clear();

        assert_eq!(serialize_initial_network_game(&data, None), Ok(None));
    }

    #[test]
    fn runtime_validation_rejects_negative_identity_but_accepts_unbounded_music() {
        let mut data = InitialNetworkGameData::default();
        data.frame = -1;
        assert_eq!(
            data.validate_runtime_application(),
            Err(InitialNetworkGameApplyError::NegativeFrame { value: -1 })
        );

        data.frame = 0;
        data.object_enumeration_index = -7;
        assert_eq!(
            data.validate_runtime_application(),
            Err(InitialNetworkGameApplyError::NegativeObjectEnumerationIndex { value: -7 })
        );

        data.object_enumeration_index = 0;
        data.music_level = 101;
        assert_eq!(data.validate_runtime_application(), Ok(()));
        data.music_level = i32::MIN;
        assert_eq!(data.validate_runtime_application(), Ok(()));
        data.music_level = i32::MAX;
        assert_eq!(data.validate_runtime_application(), Ok(()));
    }

    #[test]
    fn runtime_application_clamps_compiled_music_level_like_cpp() {
        for (compiled, expected) in [(i32::MIN, 0), (-1, 0), (101, 100), (i32::MAX, 100)] {
            let mut data = InitialNetworkGameData::default();
            data.music_level = compiled;
            let mut engine = Engine::with_seed(0);
            engine
                .apply_initial_network_game_data(&data)
                .expect("arbitrary compiled MusicLevel applies");
            assert_eq!(engine.music_level(), expected, "compiled {compiled}");
        }
    }

    #[test]
    fn runtime_validation_checks_every_independent_tick_counter() {
        let valid = InitialNetworkGameData {
            frame: 1_001,
            tick2: 1,
            tick3: 2,
            tick5: 1,
            tick10: 1,
            tick35: 21,
            tick255: 236,
            tick500: 1,
            tick1000: 1,
            ..InitialNetworkGameData::default()
        };
        valid
            .validate_runtime_application()
            .expect("matching tick counters validate");

        macro_rules! assert_tick_mismatch {
            ($member:ident, $field:literal) => {{
                let mut invalid = valid.clone();
                invalid.$member += 1;
                assert!(matches!(
                    invalid.validate_runtime_application(),
                    Err(InitialNetworkGameApplyError::TickMismatch { field: $field, .. })
                ));
            }};
        }
        assert_tick_mismatch!(tick2, "Tick2");
        assert_tick_mismatch!(tick3, "Tick3");
        assert_tick_mismatch!(tick5, "Tick5");
        assert_tick_mismatch!(tick10, "Tick10");
        assert_tick_mismatch!(tick35, "Tick35");
        assert_tick_mismatch!(tick255, "Tick255");
        assert_tick_mismatch!(tick500, "Tick500");
        assert_tick_mismatch!(tick1000, "Tick1000");
    }

    #[test]
    fn script_engine_globals_are_preserved_and_accepted_for_staged_restore() {
        let source = b"[Script]\r\nGo=true\r\nGlobals=2;i17,b1\r\nGlobalNamed=1;saved=i23\r\n\r\n[Weather]\r\nNoGamma=true\r\n";
        let data = parse_initial_network_game_data(source);

        data.validate_runtime_application()
            .expect("typed compiled sections are staged by Scenario");
        let serialized = serialize_initial_network_game(&data, None)
            .expect("opaque Script block serializes")
            .expect("Script block remains present");
        assert!(serialized
            .windows(b"Globals=2;i17,b1\r\n".len())
            .any(|window| window == b"Globals=2;i17,b1\r\n"));
        assert!(serialized
            .windows(b"GlobalNamed=1;saved=i23\r\n".len())
            .any(|window| window == b"GlobalNamed=1;saved=i23\r\n"));
    }
}
