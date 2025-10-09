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

pub const COM_SINGLE: u8 = 64;
pub const COM_DOUBLE: u8 = 128;
pub const COM_RELEASE_OFFSET: u8 = 16;

pub const COM_LEFT: u8 = 1;
pub const COM_RIGHT: u8 = 2;
pub const COM_UP: u8 = 3;
pub const COM_DOWN: u8 = 4;
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
    ClearPressed,
}

pub fn interpret_player_control_command(command: i32) -> Option<ControlEvent> {
    if command == i32::from(COM_CLEAR_PRESSED_COMS) {
        return Some(ControlEvent::ClearPressed);
    }
    if command < 0 || command > u8::MAX as i32 {
        return None;
    }
    let mut raw = command as u8;
    let is_release = raw >= COM_RELEASE_FIRST && raw <= COM_RELEASE_LAST;
    if is_release {
        raw = raw.saturating_sub(COM_RELEASE_OFFSET);
    } else {
        raw &= !(COM_SINGLE | COM_DOUBLE);
    }
    let button = match raw {
        COM_LEFT => ControlButton::Left,
        COM_RIGHT => ControlButton::Right,
        COM_UP => ControlButton::Up,
        COM_DOWN => ControlButton::Down,
        _ => return None,
    };
    Some(if is_release {
        ControlEvent::Release(button)
    } else {
        ControlEvent::Press(button)
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawPacket {
    id: Option<u8>,
    name: Option<String>,
    fields: HashMap<String, String>,
}

impl RawPacket {
    fn new() -> Self {
        Self {
            id: None,
            name: None,
            fields: HashMap::new(),
        }
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
                        active.name = Some(section.to_string());
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
    fn ignores_unhandled_commands() {
        assert!(interpret_player_control_command(999).is_none());
        assert!(interpret_player_control_command(-5).is_none());

        // Menu commands should be ignored.
        assert!(interpret_player_control_command(38).is_none());
    }
}
