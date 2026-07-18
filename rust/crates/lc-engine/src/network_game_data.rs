//! `Game.txt` serialization for an initial C++ network save.

use std::collections::HashSet;

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
        }
    }
}

impl InitialNetworkGameData {
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
        if engine.objects.iter().any(|object| {
            object.definition_id.eq_ignore_ascii_case("FGRV")
                && object.state.status.to_script_value() != 0
        }) {
            rules |= 4;
        }
        if engine.structures_snow_in {
            rules |= 8;
        }
        if engine.team_home_base_rule {
            rules |= 16;
        }

        let landscape = engine
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
            });

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
            current_scenario_section: engine
                .last_scenario_section_flags
                .is_some()
                .then(|| engine.current_scenario_section.clone())
                .unwrap_or_default(),
            resort_any_object: !engine.pending_object_order_commands.is_empty(),
            music_enabled: false,
            music_level: i32::from(engine.music_level()),
            next_mission: engine.next_mission.clone(),
            message_board_commands: engine.message_board_commands.clone(),
            script_go: engine.scenario_script_go,
            script_counter: engine.scenario_script_counter,
            environment: engine.environment,
            gamma: engine.gamma,
            landscape,
        })
    }
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
    writer.push_section("Script", script_lines(data));
    writer.push_section("Weather", weather_lines(data));
    writer.push_section(
        "Landscape",
        data.landscape
            .as_ref()
            .map(landscape_lines)
            .unwrap_or_default(),
    );

    let mut output = writer.finish().into_bytes();
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
    output: String,
}

impl IniWriter {
    fn push_section(&mut self, name: &str, lines: Vec<String>) {
        if lines.is_empty() {
            return;
        }
        if !self.output.is_empty() {
            self.output.push_str("\r\n");
        }
        self.output.push('[');
        self.output.push_str(name);
        self.output.push_str("]\r\n");
        for line in lines {
            self.output.push_str(&line);
            self.output.push_str("\r\n");
        }
    }

    fn finish(self) -> String {
        self.output
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
            let is_landscape = sections.is_empty()
                && !found_landscape
                && section == "Landscape";
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
    (line.as_bytes().get(delimiter) == Some(&b'='))
        .then_some((&line[..name_end], &line[delimiter + 1..]))
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
    quote_legacy_bytes(legacy_c_string_bytes(value))
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
    let mut bytes = lc_script::c4_string_bytes(value);
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
            environment: EnvironmentSettings::default(),
            gamma: GammaControlState::default(),
            landscape: None,
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
            name: lc_script::c4_string_from_bytes(&[0xff]),
            script: lc_script::c4_string_from_bytes(&[0xfe]),
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
}
