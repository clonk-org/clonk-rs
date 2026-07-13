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
    /// Direct synchronized client-list insertion (`CID_ClientJoin`,
    /// C4Control.cpp:552-573).
    ClientJoin(ClientJoinControlData),
    /// Synchronized client activation/observer update (`CID_ClientUpdate`,
    /// C4Control.cpp:578-633).
    ClientUpdate(ClientUpdateControlData),
    /// Synchronized client removal (`CID_ClientRemove`,
    /// C4Control.cpp:637-687).
    ClientRemove(ClientRemoveControlData),
    /// Player control command (`CID_PlrControl`).
    PlayerControl(PlayerControlData),
    /// Queued team choice that resumes a player waiting in
    /// `PS_TeamSelectionPending` (`CID_InitScenarioPlayer`).
    InitScenarioPlayer(InitScenarioPlayerControlData),
    /// Deterministic state checksum used for desync detection (`CID_SyncCheck`).
    SyncCheck(SyncCheckPacket),
    /// Deterministic game-state synchronization (`CID_Synchronize`,
    /// C4Control.cpp:537-550).
    Synchronize(SynchronizeControlData),
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

pub const CLIENT_UPDATE_ACTIVATE: u8 = 0;
pub const CLIENT_UPDATE_SET_OBSERVER: u8 = 1;

/// Binary `C4ClientCore` fields carried by `C4ControlClientJoin`
/// (`src/C4Client.cpp:75-83`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientCoreControlData {
    pub client_id: i32,
    pub activated: bool,
    pub observer: bool,
    pub name: LegacyCString,
    pub nick: LegacyCString,
    pub lobby_ready: bool,
}

impl Default for ClientCoreControlData {
    fn default() -> Self {
        Self {
            client_id: -1,
            activated: false,
            observer: false,
            name: LegacyCString::default(),
            nick: LegacyCString::default(),
            lobby_ready: false,
        }
    }
}

/// Body of `C4ControlClientJoin` (`src/C4Control.cpp:570-573`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientJoinControlData {
    pub core: ClientCoreControlData,
    pub by_client: i32,
}

/// Body of `C4ControlClientUpdate` (`src/C4Control.cpp:626-633`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientUpdateControlData {
    pub update_type: u8,
    pub client_id: i32,
    pub data: i32,
    pub by_client: i32,
}

/// Body of `C4ControlClientRemove` (`src/C4Control.cpp:682-687`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRemoveControlData {
    pub client_id: i32,
    pub reason: LegacyCString,
    pub by_client: i32,
}

/// Body of a `PlayerControl` packet describing one direct input command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerControlData {
    pub player: i32,
    pub command: i32,
    pub data: i32,
    pub by_client: i32,
}

/// Body of `C4ControlInitScenarioPlayer` and its control bases.
///
/// The C++ compiler writes `Team`, inherited `Plr`, then inherited `ByClient`
/// (`src/C4Control.cpp:1684-1688,1566-1570,53-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InitScenarioPlayerControlData {
    pub team: i32,
    pub player: i32,
    pub by_client: i32,
}

impl Default for InitScenarioPlayerControlData {
    fn default() -> Self {
        Self {
            team: 0,
            player: -1,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlSurrenderPlayer` and its control bases.
///
/// The C++ compiler writes inherited `Plr`, then inherited `ByClient`
/// (`src/C4Control.cpp:1566-1570,53-57`; `src/C4Control.h:589-594`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurrenderPlayerControlData {
    pub player: i32,
    pub by_client: i32,
}

/// Body of `C4ControlSynchronize` (`src/C4Control.cpp:537-550`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynchronizeControlData {
    pub save_player_files: bool,
    pub sync_clearance: bool,
    pub by_client: i32,
}

impl Default for SynchronizeControlData {
    fn default() -> Self {
        Self {
            save_player_files: false,
            sync_clearance: false,
            by_client: -1,
        }
    }
}

/// NUL-terminated legacy wire string, stored without its terminator.
///
/// `StdCompilerBinRead::String` preserves arbitrary bytes through the first
/// NUL (src/StdCompiler.cpp:194-210), so this type deliberately does not
/// require UTF-8.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct LegacyCString {
    bytes: Vec<u8>,
}

impl LegacyCString {
    /// Construct a wire string body. Interior NUL would terminate the C++
    /// value early and is therefore rejected.
    pub fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        (!bytes.contains(&0)).then_some(Self { bytes })
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn to_str(&self) -> Result<&str, std::str::Utf8Error> {
        std::str::from_utf8(&self.bytes)
    }

    pub fn to_string_lossy(&self) -> std::borrow::Cow<'_, str> {
        String::from_utf8_lossy(&self.bytes)
    }
}

pub const NETWORK_RESOURCE_TYPE_NULL: u8 = 0;
pub const NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE: u32 = 100 * 1024;

/// Full synchronized `C4Network2ResCore` value
/// (`src/C4Network2Res.h:58-94`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkResourceCore {
    pub resource_type: u8,
    pub id: i32,
    pub derived_id: i32,
    pub loadable: bool,
    pub file_size: u32,
    pub file_crc: u32,
    pub chunk_size: u32,
    pub contents_crc: u32,
    pub file_sha: Option<[u8; 20]>,
    pub filename: LegacyCString,
    pub author: LegacyCString,
}

impl Default for NetworkResourceCore {
    fn default() -> Self {
        // C4Network2ResCore::C4Network2ResCore
        // (src/C4Network2Res.cpp:75-80).
        Self {
            resource_type: NETWORK_RESOURCE_TYPE_NULL,
            id: -1,
            derived_id: -1,
            loadable: false,
            file_size: u32::MAX,
            file_crc: u32::MAX,
            chunk_size: NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE,
            contents_crc: u32::MAX,
            file_sha: None,
            filename: LegacyCString::default(),
            author: LegacyCString::default(),
        }
    }
}

/// The one `ByRes`-selected payload branch serialized by
/// `C4ControlJoinPlayer::CompileFunc` (`src/C4Control.cpp:852-863`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinPlayerSource {
    Embedded(Vec<u8>),
    Resource(NetworkResourceCore),
}

/// `C4ControlJoinPlayer` (CompileFunc at C4Control.cpp:852-863).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPlayerControlData {
    pub filename: LegacyCString,
    pub at_client: i32,
    pub info_id: i32,
    pub source: JoinPlayerSource,
    pub by_client: i32,
}

impl Default for JoinPlayerControlData {
    fn default() -> Self {
        Self {
            filename: LegacyCString::default(),
            at_client: -1,
            info_id: -1,
            source: JoinPlayerSource::Embedded(Vec::new()),
            by_client: -1,
        }
    }
}

pub const CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS: u32 = 1 << 0;
pub const CLIENT_PLAYER_INFO_FLAG_UPDATED: u32 = 1 << 1;
pub const CLIENT_PLAYER_INFO_FLAG_INITIAL: u32 = 1 << 2;

pub const PLAYER_INFO_TYPE_NONE: u8 = 0;
pub const PLAYER_INFO_TYPE_USER: u8 = 1;
pub const PLAYER_INFO_TYPE_SCRIPT: u8 = 2;

pub const PLAYER_INFO_FLAG_JOINED: u16 = 1 << 0;
pub const PLAYER_INFO_FLAG_REMOVED: u16 = 1 << 2;
pub const PLAYER_INFO_FLAG_HAS_RESOURCE: u16 = 1 << 3;
pub const PLAYER_INFO_FLAG_IN_SCENARIO_FILE: u16 = 1 << 6;
pub const PLAYER_INFO_FLAG_SAVEGAME_JOIN: u16 = 1 << 7;
pub const PLAYER_INFO_FLAG_DISCONNECTED: u16 = 1 << 8;
pub const PLAYER_INFO_FLAG_WON: u16 = 1 << 9;
pub const PLAYER_INFO_FLAG_VOTED_OUT: u16 = 1 << 10;
pub const PLAYER_INFO_FLAG_ATTRIBUTES_FIXED: u16 = 1 << 11;
pub const PLAYER_INFO_FLAG_NO_SCENARIO_INIT: u16 = 1 << 12;
pub const PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK: u16 = 1 << 13;
pub const PLAYER_INFO_FLAG_INVISIBLE: u16 = 1 << 14;

/// Complete synchronized `C4PlayerInfo` value serialized inside a
/// `C4ClientPlayerInfos` (`src/C4PlayerInfo.cpp:177-268`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlPlayerInfoEntry {
    pub name: LegacyCString,
    pub forced_name: LegacyCString,
    pub filename: LegacyCString,
    pub flags: u16,
    pub id: i32,
    pub player_type: u8,
    pub color: u32,
    pub original_color: u32,
    pub savegame_player: i32,
    pub team: i32,
    pub auth_id: LegacyCString,
    pub game_number: i32,
    pub game_join_frame: i32,
    pub game_part_frame: i32,
    pub extra_data: [u8; 4],
    pub league_account: LegacyCString,
    pub league_score: i32,
    pub league_rank: i32,
    pub league_rank_symbol: i32,
    pub league_projected_gain: i32,
    pub clan_tag: LegacyCString,
    pub league_performance: i32,
    pub league_progress_data: LegacyCString,
    pub resource: Option<NetworkResourceCore>,
}

impl ControlPlayerInfoEntry {
    pub fn is_script_player(&self) -> bool {
        self.player_type == PLAYER_INFO_TYPE_SCRIPT
    }

    pub fn no_scenario_init(&self) -> bool {
        self.flags & PLAYER_INFO_FLAG_NO_SCENARIO_INIT != 0
    }
}

impl Default for ControlPlayerInfoEntry {
    fn default() -> Self {
        Self {
            name: LegacyCString::default(),
            forced_name: LegacyCString::default(),
            filename: LegacyCString::default(),
            flags: 0,
            id: 0,
            player_type: PLAYER_INFO_TYPE_USER,
            color: 0x00ff_ffff,
            original_color: 0x00ff_ffff,
            savegame_player: 0,
            team: 0,
            auth_id: LegacyCString::default(),
            game_number: -1,
            game_join_frame: -1,
            game_part_frame: -1,
            extra_data: *b"NONE",
            league_account: LegacyCString::default(),
            league_score: 0,
            league_rank: 0,
            league_rank_symbol: 0,
            league_projected_gain: -1,
            clan_tag: LegacyCString::default(),
            league_performance: 0,
            league_progress_data: LegacyCString::default(),
            resource: None,
        }
    }
}

/// `C4PacketPlayerInfoUpdRequest` body (`C4ClientPlayerInfos`,
/// C4PlayerInfo.cpp:601-633,1800-1803).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoUpdateRequest {
    pub client_id: i32,
    pub flags: u32,
    pub players: Vec<ControlPlayerInfoEntry>,
}

/// `C4ControlPlayerInfo` body (C4ClientPlayerInfos,
/// C4PlayerInfo.cpp:601-633).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoControlData {
    pub client_id: i32,
    pub flags: u32,
    pub players: Vec<ControlPlayerInfoEntry>,
    pub by_client: i32,
}

impl Default for PlayerInfoControlData {
    fn default() -> Self {
        Self {
            client_id: -1,
            flags: 0,
            players: Vec::new(),
            by_client: -1,
        }
    }
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

pub const COM_NONE: u8 = 0;
pub const COM_LEFT: u8 = 1;
pub const COM_RIGHT: u8 = 2;
pub const COM_UP: u8 = 3;
pub const COM_DOWN: u8 = 4;
pub const COM_THROW: u8 = 5;
pub const COM_DIG: u8 = 6;
pub const COM_SPECIAL: u8 = 7;
pub const COM_SPECIAL2: u8 = 8;
/// COM_Contents (C4Constants.h:187).
pub const COM_CONTENTS: u8 = 9;
pub const COM_WHEEL_UP: u8 = 10;
pub const COM_WHEEL_DOWN: u8 = 11;
pub const COM_CURSOR_LEFT: u8 = 12;
pub const COM_CURSOR_RIGHT: u8 = 13;
pub const COM_CURSOR_TOGGLE: u8 = 14;
pub const COM_CURSOR_FIRST: u8 = COM_CURSOR_LEFT;
pub const COM_CURSOR_LAST: u8 = COM_CURSOR_TOGGLE;
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
/// C4MN_AdjustPosition (C4Menu.h:71), ORed into
/// C4ControlPlayerControl::Data for pointer-driven menu selection.
pub const C4MN_ADJUST_POSITION: i32 = i32::MIN;

/// COM_MenuFirst..COM_MenuLast (C4Constants.h:249-250).
pub const COM_MENU_FIRST: u8 = COM_MENU_ENTER;
pub const COM_MENU_LAST: u8 = COM_MENU_SELECT;
/// COM_MenuNavigation1/2 (C4Constants.h:252-253): the menu-only coms a
/// closed menu leaves behind in the control queue.
pub const COM_MENU_NAVIGATION1: u8 = COM_MENU_SHOW_TEXT;
pub const COM_MENU_NAVIGATION2: u8 = COM_MENU_SELECT;

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
    /// An exact `C4ControlPlayerControl` payload for commands whose `Data`
    /// slot is semantically significant (for example pointer-driven object
    /// menu selection).
    RawPlayerControl {
        command: u8,
        data: i32,
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
        // C4PacketType::PID_None (src/C4PacketBase.h).
        const PID_NONE: u8 = 0xff;
        const CID_PLR_CONTROL: u8 = 0xA1;

        if id == PID_NONE {
            return Ok(None);
        }

        const CID_JOIN_PLR: u8 = 0x91; // CID_First|0x11 (C4PacketBase.h:160)
        const CID_PLR_INFO: u8 = 0x90; // CID_First|0x10 (C4PacketBase.h:159)

        if id == CID_JOIN_PLR {
            // C4ControlJoinPlayer::CompileFunc (C4Control.cpp:852-863).
            let filename = self.fields.get("Filename").cloned().unwrap_or_default();
            let at_client = parse_int_field(&self.fields, "AtClient").unwrap_or(-1);
            let info_id = parse_int_field(&self.fields, "InfoID").unwrap_or(-1);
            let by_res = self
                .fields
                .get("ByRes")
                .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
                .unwrap_or(false);
            if by_res {
                return Err(ControlParseError::UnsupportedResourceJoin);
            }
            let player_data = self
                .fields
                .get("PlrData")
                .map(|value| parse_std_buf(value))
                .unwrap_or_default();
            let filename = LegacyCString::from_bytes(filename.into_bytes()).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Filename".to_string(),
                },
            )?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename,
                at_client,
                info_id,
                source: JoinPlayerSource::Embedded(player_data),
                by_client,
            })));
        }

        if id == CID_PLR_INFO {
            // C4ClientPlayerInfos (C4PlayerInfo.cpp:601-633): client ID and
            // flags in the packet body, one [Player] section per info
            // (C4PlayerInfo CompileFunc keys, C4PlayerInfo.cpp:177-268).
            let body = self.section_fields("Player Info").unwrap_or(&[]);
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
                .map(
                    |(_, fields)| -> Result<ControlPlayerInfoEntry, ControlParseError> {
                        let int = |key: &str| -> i32 {
                            field(fields, key)
                                .and_then(|value| value.parse::<i64>().ok())
                                .unwrap_or(0) as i32
                        };
                        let name = LegacyCString::from_bytes(
                            field(fields, "Name").unwrap_or_default().into_bytes(),
                        )
                        .ok_or(ControlParseError::InteriorNulString {
                            field: "Player.Name".to_string(),
                        })?;
                        let mut entry = ControlPlayerInfoEntry {
                            name,
                            id: int("ID"),
                            team: int("Team"),
                            color: field(fields, "Color")
                                .and_then(|value| value.parse::<i64>().ok())
                                .unwrap_or(0) as u32,
                            player_type: field(fields, "Type")
                                .filter(|value| value.eq_ignore_ascii_case("Script"))
                                .map(|_| PLAYER_INFO_TYPE_SCRIPT)
                                .unwrap_or(PLAYER_INFO_TYPE_USER),
                            ..ControlPlayerInfoEntry::default()
                        };
                        if field(fields, "Flags")
                            .is_some_and(|value| value.contains("NoScenarioInit"))
                        {
                            entry.flags |= PLAYER_INFO_FLAG_NO_SCENARIO_INIT;
                        }
                        Ok(entry)
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Some(ControlPacket::PlayerInfo(PlayerInfoControlData {
                client_id,
                players,
                ..PlayerInfoControlData::default()
            })));
        }

        if id == CID_PLR_CONTROL {
            // The writer omits default-valued fields (StdCompilerINIWrite):
            // defaults per C4ControlPlayerControl::CompileFunc
            // (C4Control.cpp:397-403) and C4ControlPacket::CompileFunc
            // (ByClient -1, C4Control.cpp:53-57).
            let player = parse_int_field_or(&self.fields, "Player", -1)?;
            let command = parse_int_field_or(&self.fields, "Com", 0)?;
            let data = parse_int_field_or(&self.fields, "Data", 0)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
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
    #[error("field `{field}` contained an interior NUL byte")]
    InteriorNulString { field: String },
    #[error("resource-backed JoinPlayer INI parsing is not implemented")]
    UnsupportedResourceJoin,
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

/// Missing fields take their CompileFunc default (the INI writer omits
/// default-valued entries); present-but-malformed fields still error.
fn parse_int_field_or(
    fields: &HashMap<String, String>,
    name: &str,
    default: i32,
) -> Result<i32, ControlParseError> {
    match fields.get(name) {
        None => Ok(default),
        Some(raw) => raw
            .parse::<i32>()
            .map_err(|_| ControlParseError::InvalidIntegerField {
                field: name.to_string(),
                value: raw.clone(),
            }),
    }
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
    fn player_info_model_defaults_match_cpp_clear() {
        // C4PlayerInfo::Clear and C4ClientPlayerInfos construction establish
        // the synchronized state that CompileFunc serializes
        // (src/C4PlayerInfo.cpp:35-54,177-268,357-359,601-633).
        let player = ControlPlayerInfoEntry::default();
        assert!(player.name.is_empty());
        assert!(player.forced_name.is_empty());
        assert!(player.filename.is_empty());
        assert_eq!(player.flags, 0);
        assert_eq!(player.id, 0);
        assert_eq!(player.player_type, PLAYER_INFO_TYPE_USER);
        assert_eq!(
            (player.color, player.original_color),
            (0x00ff_ffff, 0x00ff_ffff)
        );
        assert_eq!((player.savegame_player, player.team), (0, 0));
        assert!(player.auth_id.is_empty());
        assert_eq!(
            (
                player.game_number,
                player.game_join_frame,
                player.game_part_frame,
            ),
            (-1, -1, -1)
        );
        assert_eq!(player.extra_data, *b"NONE");
        assert!(player.league_account.is_empty());
        assert_eq!(
            (
                player.league_score,
                player.league_rank,
                player.league_rank_symbol,
                player.league_projected_gain,
                player.league_performance,
            ),
            (0, 0, 0, -1, 0)
        );
        assert!(player.clan_tag.is_empty());
        assert!(player.league_progress_data.is_empty());
        assert_eq!(player.resource, None);
        assert!(!player.is_script_player());
        assert!(!player.no_scenario_init());

        let packet = PlayerInfoControlData::default();
        assert_eq!(packet.client_id, -1);
        assert_eq!(packet.flags, 0);
        assert!(packet.players.is_empty());
        assert_eq!(packet.by_client, -1);
    }

    #[test]
    fn join_player_model_is_canonical_and_filename_is_byte_preserving() {
        // C4ControlJoinPlayer stores exactly one ByRes-selected payload branch
        // and StdCompiler binary strings retain bytes through their NUL
        // terminator (src/C4Control.cpp:852-863;
        // src/StdCompiler.cpp:113-121,194-210).
        let join = JoinPlayerControlData {
            filename: LegacyCString::from_bytes(b"P\x80.c4p".to_vec())
                .expect("filename has no interior NUL"),
            at_client: 2,
            info_id: 9,
            source: JoinPlayerSource::Embedded(vec![0xaa, 0xbb, 0xcc]),
            by_client: 4,
        };

        assert_eq!(join.filename.as_bytes(), b"P\x80.c4p");
        assert_eq!(
            join.source,
            JoinPlayerSource::Embedded(vec![0xaa, 0xbb, 0xcc])
        );
        assert_eq!(join.by_client, 4);
    }

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
    ID=255\r\n";

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
    fn player_control_omits_default_fields_like_the_real_writer() {
        // StdCompilerINIWrite skips values that equal their CompileFunc
        // default: real CID_PlrControl packets omit Data (default 0) and
        // ByClient (default -1) — C4Control.cpp:397-403, 53-57. A live
        // GoldRush record drops whole control frames if these are treated
        // as required.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=161\n\
    [Player Control]\n\
      Player=0\n\
      Com=24\n";
        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(
            packets,
            vec![ControlPacket::PlayerControl(PlayerControlData {
                player: 0,
                command: 24,
                data: 0,
                by_client: -1,
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
                assert_eq!(join.filename.to_str(), Ok("Tyler.c4p"));
                assert_eq!(join.at_client, -1);
                assert_eq!(join.info_id, 1);
                assert_eq!(
                    join.source,
                    JoinPlayerSource::Embedded(vec![b'a', b'b', 0x00, 0xff, b'c'])
                );
                assert_eq!(join.by_client, 0);
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
                assert_eq!(info.players[0].name.to_str(), Ok("Tyler"));
                assert_eq!(info.players[0].color, 15997440);
                assert_eq!(info.players[0].team, 0);
                assert!(!info.players[0].is_script_player());
                assert_eq!(info.players[1].id, 2);
                assert_eq!(info.players[1].name.to_str(), Ok("Rival"));
                assert_eq!(info.players[1].team, 7);
                assert!(info.players[1].is_script_player());
            }
            other => panic!("expected PlayerInfo, got {other:?}"),
        }
    }

    #[test]
    fn records_unknown_packets() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=0\n\
    [Mystery]\n\
      Foo=\"bar\"\n";

        let packets = parse_control_ini(input).expect("parse control log");
        assert_eq!(packets.len(), 1);
        match &packets[0] {
            ControlPacket::Unknown { id, name, fields } => {
                assert_eq!(id.raw(), 0);
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
