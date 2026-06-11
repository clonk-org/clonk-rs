use std::collections::HashMap;

use thiserror::Error;

/// Unique identifier for a control packet inside the control log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlPacketId(pub u8);

impl ControlPacketId {
    pub const fn new(id: u8) -> Self {
        Self(id)
    }

    #[cfg(test)]
    pub const fn raw(self) -> u8 {
        self.0
    }
}

/// Parsed representation of a control packet contained in an `.ini` control log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlPacket {
    /// Player control command (`CID_PlrControl`).
    PlayerControl(PlayerControlData),
    /// Deterministic state checksum used for desync detection (`CID_SyncCheck`).
    SyncCheck(SyncCheckPacket),
    /// Player join (`CID_JoinPlr`, C4Control.cpp:689-786): executes
    /// C4Game::JoinPlayer with the carried player file.
    JoinPlayer(JoinPlayerControlData),
    /// Player info update (`CID_PlrInfo`, C4Control.cpp:1264-1282):
    /// registers C4PlayerInfo entries before the join references them.
    PlayerInfo(PlayerInfoControlData),
    /// A control packet that is not yet interpreted by the Rust runtime.
    Unknown {
        id: ControlPacketId,
        name: Option<String>,
        fields: HashMap<String, String>,
    },
}

/// Body of a `PlayerControl` packet describing one direct input command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerControlData {
    pub player: i32,
    pub command: i32,
    pub data: i32,
    pub by_client: i32,
}

/// `C4ControlJoinPlayer` (CompileFunc at C4Control.cpp:852-863).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPlayerControlData {
    pub filename: String,
    pub at_client: i32,
    pub info_id: i32,
    pub by_res: bool,
    /// The raw player file (`PlrData`, a StdBuf of the .c4p bytes) for
    /// non-resource joins.
    pub player_data: Vec<u8>,
}

/// One `C4PlayerInfo` of a `C4ClientPlayerInfos`
/// (C4PlayerInfo.cpp:177-268).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlayerInfoEntry {
    pub id: i32,
    pub name: String,
    pub team: i32,
    pub color: u32,
    /// `Type=Script` (C4PT_Script).
    pub script_player: bool,
    /// `NoScenarioInit` flag (PIF_NoScenarioInit).
    pub no_scenario_init: bool,
}

/// `C4ControlPlayerInfo` body (C4ClientPlayerInfos,
/// C4PlayerInfo.cpp:601-633).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoControlData {
    pub client_id: i32,
    pub players: Vec<ControlPlayerInfoEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncCheckPacket {
    pub frame: i32,
    pub control_tick: i32,
    pub random3: i32,
    pub random_count: i32,
    pub crew_positions_sum: i32,
    pub pxs_count: i32,
    pub mass_mover_index: i32,
    pub object_count: i32,
    pub object_enumeration_index: i32,
    pub sector_shape_sum: i32,
    pub by_client: i32,
}

impl SyncCheckPacket {
    pub fn matches(&self, other: &Self) -> bool {
        self.frame == other.frame
            && self.control_tick == other.control_tick
            && self.random3 == other.random3
            && self.random_count == other.random_count
            && self.crew_positions_sum == other.crew_positions_sum
            && self.pxs_count == other.pxs_count
            && self.mass_mover_index == other.mass_mover_index
            && self.object_count == other.object_count
            && self.object_enumeration_index == other.object_enumeration_index
            && self.sector_shape_sum == other.sector_shape_sum
    }
}

pub const COM_SINGLE: u8 = 64;
pub const COM_DOUBLE: u8 = 128;
pub const COM_RELEASE_OFFSET: u8 = 16;

pub const COM_LEFT: u8 = 1;
pub const COM_RIGHT: u8 = 2;
pub const COM_UP: u8 = 3;
pub const COM_DOWN: u8 = 4;
pub const COM_THROW: u8 = 5;
pub const COM_DIG: u8 = 6;
pub const COM_SPECIAL: u8 = 7;
pub const COM_SPECIAL2: u8 = 8;
pub const COM_CURSOR_LEFT: u8 = 12;
pub const COM_CURSOR_RIGHT: u8 = 13;
pub const COM_CURSOR_TOGGLE: u8 = 14;
pub const COM_PLAYER_MENU: u8 = 36;
pub const COM_MENU_ENTER: u8 = 38;
pub const COM_MENU_ENTER_ALL: u8 = 39;
pub const COM_MENU_CLOSE: u8 = 40;
pub const COM_MENU_SHOW_TEXT: u8 = 42;
pub const COM_MENU_LEFT: u8 = 52;
pub const COM_MENU_RIGHT: u8 = 53;
pub const COM_MENU_UP: u8 = 54;
pub const COM_MENU_DOWN: u8 = 55;
pub const COM_MENU_SELECT: u8 = 60;
pub const COM_CLEAR_PRESSED_COMS: u8 = 61;

pub const COM_RELEASE_FIRST: u8 = COM_LEFT + COM_RELEASE_OFFSET;
pub const COM_RELEASE_LAST: u8 = 14 + COM_RELEASE_OFFSET;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlButton {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlEvent {
    Press(ControlButton),
    Release(ControlButton),
    Command {
        command: ControlCommand,
        kind: CommandKind,
    },
    ClearPressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandKind {
    Press,
    Release,
    Single,
    Double,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ControlCommand {
    Throw,
    Dig,
    Special,
    Special2,
    CursorLeft,
    CursorRight,
    CursorToggle,
    PlayerMenu,
    MenuEnter,
    MenuEnterAll,
    MenuClose,
    MenuShowText,
    MenuLeft,
    MenuRight,
    MenuUp,
    MenuDown,
    MenuSelect,
}

pub fn interpret_player_control_command(command: i32) -> Option<ControlEvent> {
    if command == i32::from(COM_CLEAR_PRESSED_COMS) {
        return Some(ControlEvent::ClearPressed);
    }
    if command < 0 || command > u8::MAX as i32 {
        return None;
    }
    let mut raw = command as u8;
    if (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&raw) {
        let base = raw.saturating_sub(COM_RELEASE_OFFSET);
        return interpret_base_command(base, CommandKind::Release);
    }
    let mut kind = CommandKind::Press;
    if raw & COM_DOUBLE != 0 {
        raw &= !COM_DOUBLE;
        kind = CommandKind::Double;
    } else if raw & COM_SINGLE != 0 {
        raw &= !COM_SINGLE;
        kind = CommandKind::Single;
    }
    interpret_base_command(raw, kind)
}

fn interpret_base_command(base: u8, kind: CommandKind) -> Option<ControlEvent> {
    match base {
        COM_LEFT => Some(match kind {
            CommandKind::Release => ControlEvent::Release(ControlButton::Left),
            _ => ControlEvent::Press(ControlButton::Left),
        }),
        COM_RIGHT => Some(match kind {
            CommandKind::Release => ControlEvent::Release(ControlButton::Right),
            _ => ControlEvent::Press(ControlButton::Right),
        }),
        COM_UP => Some(match kind {
            CommandKind::Release => ControlEvent::Release(ControlButton::Up),
            _ => ControlEvent::Press(ControlButton::Up),
        }),
        COM_DOWN => Some(match kind {
            CommandKind::Release => ControlEvent::Release(ControlButton::Down),
            _ => ControlEvent::Press(ControlButton::Down),
        }),
        COM_THROW => Some(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind,
        }),
        COM_DIG => Some(ControlEvent::Command {
            command: ControlCommand::Dig,
            kind,
        }),
        COM_SPECIAL => Some(ControlEvent::Command {
            command: ControlCommand::Special,
            kind,
        }),
        COM_SPECIAL2 => Some(ControlEvent::Command {
            command: ControlCommand::Special2,
            kind,
        }),
        COM_CURSOR_LEFT => Some(ControlEvent::Command {
            command: ControlCommand::CursorLeft,
            kind,
        }),
        COM_CURSOR_RIGHT => Some(ControlEvent::Command {
            command: ControlCommand::CursorRight,
            kind,
        }),
        COM_CURSOR_TOGGLE => Some(ControlEvent::Command {
            command: ControlCommand::CursorToggle,
            kind,
        }),
        COM_PLAYER_MENU => Some(ControlEvent::Command {
            command: ControlCommand::PlayerMenu,
            kind,
        }),
        COM_MENU_ENTER => Some(ControlEvent::Command {
            command: ControlCommand::MenuEnter,
            kind,
        }),
        COM_MENU_ENTER_ALL => Some(ControlEvent::Command {
            command: ControlCommand::MenuEnterAll,
            kind,
        }),
        COM_MENU_CLOSE => Some(ControlEvent::Command {
            command: ControlCommand::MenuClose,
            kind,
        }),
        COM_MENU_SHOW_TEXT => Some(ControlEvent::Command {
            command: ControlCommand::MenuShowText,
            kind,
        }),
        COM_MENU_LEFT => Some(ControlEvent::Command {
            command: ControlCommand::MenuLeft,
            kind,
        }),
        COM_MENU_RIGHT => Some(ControlEvent::Command {
            command: ControlCommand::MenuRight,
            kind,
        }),
        COM_MENU_UP => Some(ControlEvent::Command {
            command: ControlCommand::MenuUp,
            kind,
        }),
        COM_MENU_DOWN => Some(ControlEvent::Command {
            command: ControlCommand::MenuDown,
            kind,
        }),
        COM_MENU_SELECT => Some(ControlEvent::Command {
            command: ControlCommand::MenuSelect,
            kind,
        }),
        _ => None,
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RawPacket {
    id: Option<u8>,
    name: Option<String>,
    fields: HashMap<String, String>,
    /// Ordered nested sections with their own ordered fields — needed by
    /// packets whose bodies carry repeated subsections (C4ClientPlayerInfos
    /// writes one [Player] per info, C4PlayerInfo.cpp:629-630).
    sections: Vec<(String, Vec<(String, String)>)>,
}

#[allow(dead_code)]
impl RawPacket {
    fn new() -> Self {
        Self {
            id: None,
            name: None,
            fields: HashMap::new(),
            sections: Vec::new(),
        }
    }

    fn section_fields(&self, name: &str) -> Option<&[(String, String)]> {
        self.sections
            .iter()
            .find(|(section, _)| section.eq_ignore_ascii_case(name))
            .map(|(_, fields)| fields.as_slice())
    }

    fn into_control_packet(self) -> Result<Option<ControlPacket>, ControlParseError> {
        let Some(id) = self.id else {
            // Incomplete packets are ignored; they represent terminators emitted by the C++ side.
            return Ok(None);
        };

        // Packet names come from `PktHandlingData` on the C++ side. The values we care about are
        // a small subset so far; everything else is recorded as `Unknown`.
        const PID_NONE: u8 = 0x00;
        const CID_PLR_CONTROL: u8 = 0xA1;

        if id == PID_NONE {
            return Ok(None);
        }

        const CID_JOIN_PLR: u8 = 0x91; // CID_First|0x11 (C4PacketBase.h:160)
        const CID_PLR_INFO: u8 = 0x90; // CID_First|0x10 (C4PacketBase.h:159)

        if id == CID_JOIN_PLR {
            // C4ControlJoinPlayer::CompileFunc (C4Control.cpp:852-863).
            let filename = self
                .fields
                .get("Filename")
                .cloned()
                .unwrap_or_default();
            let at_client = parse_int_field(&self.fields, "AtClient").unwrap_or(-1);
            let info_id = parse_int_field(&self.fields, "InfoID").unwrap_or(-1);
            let by_res = self
                .fields
                .get("ByRes")
                .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
                .unwrap_or(false);
            let player_data = self
                .fields
                .get("PlrData")
                .map(|value| parse_std_buf(value))
                .unwrap_or_default();
            return Ok(Some(ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename,
                at_client,
                info_id,
                by_res,
                player_data,
            })));
        }

        if id == CID_PLR_INFO {
            // C4ClientPlayerInfos (C4PlayerInfo.cpp:601-633): client ID and
            // flags in the packet body, one [Player] section per info
            // (C4PlayerInfo CompileFunc keys, C4PlayerInfo.cpp:177-268).
            let body = self
                .section_fields("Player Info")
                .unwrap_or(&[]);
            let field = |fields: &[(String, String)], key: &str| -> Option<String> {
                fields
                    .iter()
                    .find(|(entry, _)| entry.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value.clone())
            };
            let client_id = field(body, "ID")
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or(-1);
            let players = self
                .sections
                .iter()
                .filter(|(name, _)| name.eq_ignore_ascii_case("Player"))
                .map(|(_, fields)| {
                    let int = |key: &str| -> i32 {
                        field(fields, key)
                            .and_then(|value| value.parse::<i64>().ok())
                            .unwrap_or(0) as i32
                    };
                    ControlPlayerInfoEntry {
                        id: int("ID"),
                        name: field(fields, "Name").unwrap_or_default(),
                        team: int("Team"),
                        color: field(fields, "Color")
                            .and_then(|value| value.parse::<i64>().ok())
                            .unwrap_or(0) as u32,
                        script_player: field(fields, "Type")
                            .map(|value| value.eq_ignore_ascii_case("Script"))
                            .unwrap_or(false),
                        no_scenario_init: field(fields, "Flags")
                            .map(|value| value.contains("NoScenarioInit"))
                            .unwrap_or(false),
                    }
                })
                .collect();
            return Ok(Some(ControlPacket::PlayerInfo(PlayerInfoControlData {
                client_id,
                players,
            })));
        }

        if id == CID_PLR_CONTROL {
            let player = parse_int_field(&self.fields, "Player")?;
            let command = parse_int_field(&self.fields, "Com")?;
            let data = parse_int_field(&self.fields, "Data")?;
            let by_client = parse_int_field(&self.fields, "ByClient")?;
            Ok(Some(ControlPacket::PlayerControl(PlayerControlData {
                player,
                command,
                data,
                by_client,
            })))
        } else {
            Ok(Some(ControlPacket::Unknown {
                id: ControlPacketId::new(id),
                name: self.name,
                fields: self.fields,
            }))
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Error)]
pub enum ControlParseError {
    #[error("control log did not start with [Control] section")]
    MissingControlSection,
    #[error("unexpected section [{section}] inside control log")]
    UnexpectedSection { section: String },
    #[error("control log contained malformed line `{line}`")]
    MalformedLine { line: String },
    #[error("field `{field}` missing from control packet")]
    MissingField { field: String },
    #[error("field `{field}` contained invalid integer `{value}`")]
    InvalidIntegerField { field: String, value: String },
}

/// Parse the `.ini` control payload emitted by the C++ runtime into structured packets.
///
/// The writer on the C++ side always produces CRLF separated output. The parser is permissive with
/// respect to whitespace and therefore also accepts LF-only line endings which is convenient for
/// unit tests.
#[allow(dead_code)]
pub fn parse_control_ini(input: &str) -> Result<Vec<ControlPacket>, ControlParseError> {
    enum ParserState {
        Start,
        ControlSection,
        InPacket,
    }

    let mut state = ParserState::Start;
    let mut packets = Vec::new();
    let mut current = None::<RawPacket>;

    for raw_line in input.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        if let Some(name) = trimmed
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let section = name.trim();
            match state {
                ParserState::Start => {
                    if section.eq_ignore_ascii_case("Control") {
                        state = ParserState::ControlSection;
                    } else {
                        return Err(ControlParseError::MissingControlSection);
                    }
                }
                ParserState::ControlSection | ParserState::InPacket => {
                    if section.eq_ignore_ascii_case("IDPacket") {
                        if let Some(packet) = current.take() {
                            if let Some(parsed) = packet.into_control_packet()? {
                                packets.push(parsed);
                            }
                        }
                        current = Some(RawPacket::new());
                        state = ParserState::InPacket;
                    } else if let Some(active) = current.as_mut() {
                        if active.name.is_none() {
                            active.name = Some(section.to_string());
                        }
                        active.sections.push((section.to_string(), Vec::new()));
                    } else {
                        return Err(ControlParseError::UnexpectedSection {
                            section: section.to_string(),
                        });
                    }
                }
            }
            continue;
        }

        let Some((key, value)) = trimmed.split_once('=') else {
            return Err(ControlParseError::MalformedLine {
                line: line.to_string(),
            });
        };

        let key = key.trim();
        let value = unescape_value(value.trim());

        if let Some(packet) = current.as_mut() {
            if key.eq_ignore_ascii_case("ID") && packet.id.is_none() {
                let parsed =
                    value
                        .parse::<u8>()
                        .map_err(|_| ControlParseError::InvalidIntegerField {
                            field: key.to_string(),
                            value: value.clone(),
                        })?;
                packet.id = Some(parsed);
            } else {
                if let Some((_, section_fields)) = packet.sections.last_mut() {
                    section_fields.push((key.to_string(), value.clone()));
                }
                packet.fields.insert(key.to_string(), value);
            }
        } else {
            return Err(ControlParseError::MalformedLine {
                line: line.to_string(),
            });
        }
    }

    if let Some(packet) = current.take() {
        if let Some(parsed) = packet.into_control_packet()? {
            packets.push(parsed);
        }
    }

    if matches!(state, ParserState::Start) {
        return Err(ControlParseError::MissingControlSection);
    }

    Ok(packets)
}

/// StdBuf textual form: `<size>:"escaped bytes"` (StdBuf::CompileFunc
/// writes the int-packed size, SEP_PART2 `:`, then the raw data escaped —
/// StdBuf.cpp:86-101, StdCompiler.cpp:79,383-396).
fn parse_std_buf(value: &str) -> Vec<u8> {
    let Some((size_text, rest)) = value.split_once(':') else {
        return Vec::new();
    };
    let size = size_text.trim().parse::<usize>().unwrap_or(0);
    let trimmed = rest.trim();
    let quoted = trimmed
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .unwrap_or(trimmed);
    let mut bytes = unescape_value_bytes(quoted);
    if bytes.len() > size {
        bytes.truncate(size);
    }
    bytes
}

/// Byte-level unescape of an INI escaped string (StdCompilerINIWrite
/// WriteEscaped): named escapes plus octal `\NNN` per byte. The OCTAL form
/// covers every non-printable byte, so escaped data round-trips as ASCII.
fn unescape_value_bytes(value: &str) -> Vec<u8> {
    let raw = value.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut index = 0;
    while index < raw.len() {
        let byte = raw[index];
        if byte != b'\\' {
            bytes.push(byte);
            index += 1;
            continue;
        }
        index += 1;
        let Some(&escape) = raw.get(index) else {
            break;
        };
        index += 1;
        match escape {
            b'"' => bytes.push(b'"'),
            b'\\' => bytes.push(b'\\'),
            b'n' => bytes.push(b'\n'),
            b'r' => bytes.push(b'\r'),
            b't' => bytes.push(b'\t'),
            b'b' => bytes.push(0x08),
            b'f' => bytes.push(0x0c),
            b'a' => bytes.push(0x07),
            b'v' => bytes.push(0x0b),
            digit @ b'0'..=b'7' => {
                let mut octal = u32::from(digit - b'0');
                for _ in 0..2 {
                    match raw.get(index) {
                        Some(&next @ b'0'..=b'7') => {
                            octal = octal * 8 + u32::from(next - b'0');
                            index += 1;
                        }
                        _ => break,
                    }
                }
                bytes.push(octal as u8);
            }
            other => bytes.push(other),
        }
    }
    bytes
}

#[allow(dead_code)]
fn parse_int_field(fields: &HashMap<String, String>, name: &str) -> Result<i32, ControlParseError> {
    let Some(raw) = fields.get(name) else {
        return Err(ControlParseError::MissingField {
            field: name.to_string(),
        });
    };
    raw.parse::<i32>()
        .map_err(|_| ControlParseError::InvalidIntegerField {
            field: name.to_string(),
            value: raw.clone(),
        })
}

#[allow(dead_code)]
fn unescape_value(value: &str) -> String {
    let trimmed = value.trim();
    if !trimmed.starts_with('"') || !trimmed.ends_with('"') {
        return trimmed.to_string();
    }

    let mut chars = trimmed[1..trimmed.len() - 1].chars();
    let mut result = String::with_capacity(trimmed.len());
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }

        match chars.next() {
            Some('"') => result.push('"'),
            Some('\\') => result.push('\\'),
            Some('n') => result.push('\n'),
            Some('r') => result.push('\r'),
            Some('t') => result.push('\t'),
            Some('b') => result.push('\u{0008}'),
            Some('f') => result.push('\u{000c}'),
            Some(other) => {
                // Octal escapes are written as \123. Collect consecutive octal digits.
                if ('0'..='7').contains(&other) {
                    let mut octal = String::new();
                    octal.push(other);
                    for _ in 0..2 {
                        match chars.clone().next() {
                            Some(next) if ('0'..='7').contains(&next) => {
                                octal.push(next);
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    if let Ok(value) = u8::from_str_radix(&octal, 8) {
                        result.push(value as char);
                    }
                } else {
                    result.push(other);
                }
            }
            None => break,
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_player_control_packet() {
        let input = "\
[Control]\r\n\
  [IDPacket]\r\n\
    ID=161\r\n\
    [Player Control]\r\n\
      Player=0\r\n\
      Com=1\r\n\
      Data=0\r\n\
      ByClient=1\r\n\
\r\n\
  [IDPacket]\r\n\
    ID=0\r\n";

        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(
            packets,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: 1,
                data: 0,
                by_client: 1
            })]
        );
    }

    #[test]
    fn parses_join_player_packet() {
        // C4ControlJoinPlayer::CompileFunc (C4Control.cpp:852-863):
        // Filename (mkNetFilenameAdapt), AtClient/InfoID (int-packed),
        // ByRes, and the raw player file as PlrData — a StdBuf, written as
        // `<size>:"escaped bytes"` (StdBuf::CompileFunc size + SEP_PART2
        // ':' + Raw RCT_Escaped, StdBuf.cpp:86-101, StdCompiler.cpp:79,
        // 383-396). CID_JoinPlr = 0x80|0x11 = 145 (C4PacketBase.h:160),
        // packet section name "Join Player" (C4Packet2.cpp:112).
        let input = "\
[Control]\r\n\
  [IDPacket]\r\n\
    ID=145\r\n\
    [Join Player]\r\n\
      Filename=\"Tyler.c4p\"\r\n\
      AtClient=-1\r\n\
      InfoID=1\r\n\
      ByRes=false\r\n\
      PlrData=5:\"ab\\000\\377c\"\r\n\
      ByClient=0\r\n";
        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(packets.len(), 1);
        match &packets[0] {
            ControlPacket::JoinPlayer(join) => {
                assert_eq!(join.filename, "Tyler.c4p");
                assert_eq!(join.at_client, -1);
                assert_eq!(join.info_id, 1);
                assert!(!join.by_res);
                assert_eq!(join.player_data, vec![b'a', b'b', 0x00, 0xff, b'c']);
            }
            other => panic!("expected JoinPlayer, got {other:?}"),
        }
    }

    #[test]
    fn parses_player_info_packet_with_nested_players() {
        // C4ControlPlayerInfo wraps a C4ClientPlayerInfos
        // (C4PlayerInfo.cpp:601-633): client ID, flags, then one [Player]
        // section per C4PlayerInfo (CompileFunc keys at
        // C4PlayerInfo.cpp:177-268). CID_PlrInfo = 144, section name
        // "Player Info" (C4Packet2.cpp:111). The nested [Player] sections
        // must not collapse into the packet's flat fields.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=144\n\
    [Player Info]\n\
      ID=0\n\
      Flags=Initial\n\
      [Player]\n\
        Name=\"Tyler\"\n\
        Flags=\n\
        ID=1\n\
        Type=User\n\
        Color=15997440\n\
        Team=0\n\
      [Player]\n\
        Name=\"Rival\"\n\
        ID=2\n\
        Type=Script\n\
        Color=255\n\
        Team=7\n\
    ByClient=0\n";
        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(packets.len(), 1);
        match &packets[0] {
            ControlPacket::PlayerInfo(info) => {
                assert_eq!(info.client_id, 0);
                assert_eq!(info.players.len(), 2);
                assert_eq!(info.players[0].id, 1);
                assert_eq!(info.players[0].name, "Tyler");
                assert_eq!(info.players[0].color, 15997440);
                assert_eq!(info.players[0].team, 0);
                assert!(!info.players[0].script_player);
                assert_eq!(info.players[1].id, 2);
                assert_eq!(info.players[1].name, "Rival");
                assert_eq!(info.players[1].team, 7);
                assert!(info.players[1].script_player);
            }
            other => panic!("expected PlayerInfo, got {other:?}"),
        }
    }

    #[test]
    fn records_unknown_packets() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=255\n\
    [Mystery]\n\
      Foo=\"bar\"\n";

        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(packets.len(), 1);
        match &packets[0] {
            ControlPacket::Unknown { id, name, fields } => {
                assert_eq!(id.raw(), 255);
                assert_eq!(name.as_deref(), Some("Mystery"));
                assert_eq!(fields.get("Foo").map(String::as_str), Some("bar"));
            }
            _ => panic!("expected unknown packet"),
        }
    }

    #[test]
    fn interprets_press_and_release_events() {
        let press =
            interpret_player_control_command(i32::from(COM_LEFT)).expect("press event detected");
        assert_eq!(press, ControlEvent::Press(ControlButton::Left));

        let release =
            interpret_player_control_command(i32::from(COM_LEFT) + i32::from(COM_RELEASE_OFFSET))
                .expect("release event detected");
        assert_eq!(release, ControlEvent::Release(ControlButton::Left));
    }

    #[test]
    fn interprets_clear_pressed_coms() {
        let event =
            interpret_player_control_command(i32::from(COM_CLEAR_PRESSED_COMS)).expect("event");
        assert_eq!(event, ControlEvent::ClearPressed);
    }

    #[test]
    fn interprets_cursor_toggle_command() {
        let event = interpret_player_control_command(i32::from(COM_CURSOR_TOGGLE))
            .expect("command event detected");
        assert_eq!(
            event,
            ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Press
            }
        );

        let double = interpret_player_control_command(i32::from(COM_CURSOR_TOGGLE | COM_DOUBLE))
            .expect("double command detected");
        assert_eq!(
            double,
            ControlEvent::Command {
                command: ControlCommand::CursorToggle,
                kind: CommandKind::Double
            }
        );
    }

    #[test]
    fn ignores_unhandled_commands() {
        assert!(interpret_player_control_command(999).is_none());
        assert!(interpret_player_control_command(-5).is_none());

        // Unknown menu-like command remains ignored.
        assert!(interpret_player_control_command(41).is_none());
    }
}
