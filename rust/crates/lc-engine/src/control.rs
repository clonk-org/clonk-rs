use std::collections::HashMap;

use thiserror::Error;

const MAX_PLAYER_SELECT_INI_OBJECTS: usize = 1_000_000;

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
    /// A client's league vote (`CID_Vote`, C4Control.cpp:1446-1451).
    Vote(VoteControlData),
    /// The authoritative league vote result (`CID_VoteEnd`,
    /// C4Control.cpp:1517-1520).
    VoteEnd(VoteControlData),
    /// Synchronized console/global/object-scoped script execution
    /// (`CID_Script`, C4Control.cpp:258-326).
    Script(ScriptControlData),
    /// Synchronized answer to a script-created message-board query
    /// (`CID_MessageBoardAnswer`, C4Control.cpp:1546-1594).
    MessageBoardAnswer(MessageBoardAnswerControlData),
    /// Synchronized mouse/object selection (`CID_PlrSelect`,
    /// C4Control.cpp:329-380).
    PlayerSelect(PlayerSelectControlData),
    /// Player control command (`CID_PlrControl`).
    PlayerControl(PlayerControlData),
    /// Mouse/object command (`CID_PlrCommand`, C4Control.cpp:405-439).
    PlayerCommand(PlayerCommandControlData),
    /// Queued team choice that resumes a player waiting in
    /// `PS_TeamSelectionPending` (`CID_InitScenarioPlayer`).
    InitScenarioPlayer(InitScenarioPlayerControlData),
    /// Queued player surrender (`CID_SurrenderPlayer`). C++ authenticates the
    /// player through the inherited `ByClient` field before executing it.
    SurrenderPlayer(SurrenderPlayerControlData),
    /// Deterministic state checksum used for desync detection (`CID_SyncCheck`).
    SyncCheck(SyncCheckPacket),
    /// Deterministic game-state synchronization (`CID_Synchronize`,
    /// C4Control.cpp:537-550).
    Synchronize(SynchronizeControlData),
    /// Player join (`CID_JoinPlr`, C4Control.cpp:689-786): executes
    /// C4Game::JoinPlayer with the carried player file.
    JoinPlayer(JoinPlayerControlData),
    /// Host-authored synchronized player removal (`CID_RemovePlr`,
    /// C4Control.cpp:1290-1305).
    RemovePlayer(RemovePlayerControlData),
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

/// Raw `C4ControlVoteType` values serialized by `C4ControlVote`.
///
/// The C++ binary compiler casts the enum through `uint8_t` without validating
/// it (`src/C4Control.cpp:1446-1451`), so [`VoteControlData::vote_type`] remains
/// a raw byte and unknown values survive a decode/encode cycle.
pub const VOTE_TYPE_NONE: u8 = u8::MAX;
pub const VOTE_TYPE_CANCEL: u8 = 0;
pub const VOTE_TYPE_KICK: u8 = 1;
pub const VOTE_TYPE_PAUSE: u8 = 2;

/// Special `C4ControlScript` target values (`src/C4Control.h:134`).
pub const SCRIPT_SCOPE_CONSOLE: i32 = -2;
pub const SCRIPT_SCOPE_GLOBAL: i32 = -1;

/// Valid `C4AulScriptStrict` values carried by `C4ControlScript`.
///
/// The binary compiler writes the scoped enum through its raw `uint8_t`
/// representation, then rejects values outside this range while decoding
/// (`src/C4Control.cpp:317-326,1708-1714`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ScriptStrictness {
    NonStrict = 0,
    Strict1 = 1,
    Strict2 = 2,
    #[default]
    Strict3 = 3,
}

impl ScriptStrictness {
    pub const fn raw(self) -> u8 {
        self as u8
    }

    /// Strictness representation used by `lc-script`: non-strict has no
    /// strict level, while the three strict modes carry levels 1 through 3.
    pub const fn level(self) -> Option<u8> {
        match self {
            Self::NonStrict => None,
            Self::Strict1 => Some(1),
            Self::Strict2 => Some(2),
            Self::Strict3 => Some(3),
        }
    }
}

impl TryFrom<u8> for ScriptStrictness {
    type Error = u8;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NonStrict),
            1 => Ok(Self::Strict1),
            2 => Ok(Self::Strict2),
            3 => Ok(Self::Strict3),
            other => Err(other),
        }
    }
}

impl TryFrom<i32> for ScriptStrictness {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::NonStrict),
            1 => Ok(Self::Strict1),
            2 => Ok(Self::Strict2),
            3 => Ok(Self::Strict3),
            other => Err(other),
        }
    }
}

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

/// Body of `C4ControlRemovePlr` (`CID_RemovePlr`).
///
/// The C++ compiler writes packed `Plr`, native bool `Disconnected`, then
/// inherited packed `ByClient` (`src/C4Control.cpp:1290-1305`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemovePlayerControlData {
    pub player: i32,
    pub disconnected: bool,
    pub by_client: i32,
}

impl Default for RemovePlayerControlData {
    fn default() -> Self {
        Self {
            player: -1,
            disconnected: false,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlScript` (`CID_Script`).
///
/// Script bytes remain NUL-free legacy bytes so binary controls round-trip
/// without requiring UTF-8, just like `StdStrBuf` on the C++ wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptControlData {
    pub target_object: i32,
    pub strictness: ScriptStrictness,
    pub script: LegacyCString,
    pub by_client: i32,
}

impl Default for ScriptControlData {
    fn default() -> Self {
        Self {
            target_object: SCRIPT_SCOPE_GLOBAL,
            strictness: ScriptStrictness::Strict3,
            script: LegacyCString::default(),
            by_client: -1,
        }
    }
}

/// Body of `C4ControlMessageBoardAnswer` (`CID_MessageBoardAnswer`).
///
/// The object number and both inherited author fields use the packed signed
/// integer codec. Answer bytes remain NUL-free legacy bytes so the binary
/// control can round-trip without assuming UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageBoardAnswerControlData {
    pub object: i32,
    pub answer: LegacyCString,
    pub player: i32,
    pub by_client: i32,
}

impl Default for MessageBoardAnswerControlData {
    fn default() -> Self {
        Self {
            object: 0,
            answer: LegacyCString::default(),
            player: -1,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlPlayerSelect` (`CID_PlrSelect`).
///
/// Object numbers remain signed and ordered until execution: invalid entries
/// are skipped by `SafeObjectPointer`, while valid entries contribute to the
/// iterative control checksum (`src/C4Control.cpp:341-368`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerSelectControlData {
    pub player: i32,
    pub objects: Vec<i32>,
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

/// Body of `C4ControlPlayerCommand` (`CID_PlrCommand`).
///
/// Object numbers and command/add-mode values remain raw until execution so
/// unknown values survive network and replay round trips exactly like C++.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerCommandControlData {
    pub player: i32,
    pub command: i32,
    pub x: i32,
    pub y: i32,
    pub target: i32,
    pub target2: i32,
    pub data: i32,
    pub add_mode: i32,
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

/// Shared body of `C4ControlVote` and `C4ControlVoteEnd`.
///
/// C++ writes `Type` as a raw byte, `Approve` as a native bool byte, `Data` as
/// a native `int32_t`, then the inherited packed signed `ByClient`
/// (`src/C4Control.cpp:1446-1451,1517-1520,53-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoteControlData {
    pub vote_type: u8,
    pub approve: bool,
    pub data: i32,
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
pub const PLAYER_INFO_FLAG_JOIN_ISSUED: u16 = 1 << 4;
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
    /// Whether `sLeagueProgressData` is a null `StdStrBuf`. The legacy wire
    /// format only carries C-string bytes, but freshly constructed/in-memory
    /// `C4PlayerInfo` values retain a distinct null state until compiled or
    /// explicitly assigned an empty string.
    pub league_progress_data_is_null: bool,
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

    pub fn no_elimination_check(&self) -> bool {
        self.flags & PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK != 0
    }

    pub fn is_joined(&self) -> bool {
        self.flags & PLAYER_INFO_FLAG_JOINED != 0 && self.flags & PLAYER_INFO_FLAG_REMOVED == 0
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
            league_progress_data_is_null: true,
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
    if command == i32::from(COM_NONE) {
        return None;
    }
    let original = command as u8;
    let mut raw = original;
    if (COM_RELEASE_FIRST..=COM_RELEASE_LAST).contains(&raw) {
        let base = raw.saturating_sub(COM_RELEASE_OFFSET);
        return interpret_base_command(base, CommandKind::Release).or(Some(
            ControlEvent::RawPlayerControl {
                command: original,
                data: 0,
            },
        ));
    }
    let mut kind = CommandKind::Press;
    if raw & COM_DOUBLE != 0 {
        raw &= !COM_DOUBLE;
        kind = CommandKind::Double;
    } else if raw & COM_SINGLE != 0 {
        raw &= !COM_SINGLE;
        kind = CommandKind::Single;
    }
    let interpreted = interpret_base_command(raw, kind);
    if matches!(raw, COM_LEFT | COM_RIGHT | COM_UP | COM_DOWN)
        && matches!(kind, CommandKind::Single | CommandKind::Double)
    {
        return Some(ControlEvent::RawPlayerControl {
            command: original,
            data: 0,
        });
    }
    interpreted.or(Some(ControlEvent::RawPlayerControl {
        command: original,
        data: 0,
    }))
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
        const CID_SYNC_CHECK: u8 = 0x85;
        const CID_SYNCHRONIZE: u8 = 0x86;
        const CID_SCRIPT: u8 = 0x88;
        const CID_MESSAGE_BOARD_ANSWER: u8 = 0xd0;
        const CID_PLR_SELECT: u8 = 0xA0;
        const CID_PLR_CONTROL: u8 = 0xA1;
        const CID_PLR_COMMAND: u8 = 0xA2;

        if id == PID_NONE {
            return Ok(None);
        }

        if id == CID_SYNC_CHECK {
            // C4ControlSyncCheck::CompileFunc serializes every diagnostic
            // ledger field followed by inherited ByClient. Keeping this
            // typed lets ordinary replay records carry their periodic sync
            // checks without being mistaken for unsupported simulation.
            return Ok(Some(ControlPacket::SyncCheck(SyncCheckPacket {
                frame: parse_int_field_or(&self.fields, "Frame", -1)?,
                control_tick: parse_int_field_or(&self.fields, "ControlTick", 0)?,
                random3: parse_int_field_or(&self.fields, "Random3", 0)?,
                random_count: parse_int_field_or(&self.fields, "RandomCount", 0)?,
                crew_positions_sum: parse_int_field_or(&self.fields, "AllCrewPosX", 0)?,
                pxs_count: parse_int_field_or(&self.fields, "PXSCount", 0)?,
                mass_mover_index: parse_int_field_or(&self.fields, "MassMoverIndex", 0)?,
                object_count: parse_int_field_or(&self.fields, "ObjectCount", 0)?,
                object_enumeration_index: parse_int_field_or(
                    &self.fields,
                    "ObjectEnumerationIndex",
                    0,
                )?,
                sector_shape_sum: parse_int_field_or(&self.fields, "SectShapeSum", 0)?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
        }

        if id == CID_SYNCHRONIZE {
            // C4ControlSynchronize::CompileFunc writes the two native bools
            // before inherited packed ByClient. The INI writer omits false
            // bools and the -1 author default (C4Control.cpp:545-550,53-57).
            let save_player_files = parse_bool_field_or(&self.fields, "SavePlrs", false)?;
            let sync_clearance = parse_bool_field_or(&self.fields, "SyncClear", false)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::Synchronize(
                SynchronizeControlData {
                    save_player_files,
                    sync_clearance,
                    by_client,
                },
            )));
        }

        const CID_JOIN_PLR: u8 = 0x91; // CID_First|0x11 (C4PacketBase.h:160)
        const CID_REMOVE_PLR: u8 = 0x92; // CID_First|0x12 (C4PacketBase.h:161)
        const CID_PLR_INFO: u8 = 0x90; // CID_First|0x10 (C4PacketBase.h:159)

        if id == CID_SCRIPT {
            // C4ControlScript::CompileFunc names the raw target object, raw
            // uint8 strictness, script string, and inherited packed author in
            // this order. The INI writer may omit all four defaults.
            let target_object = parse_int_field_or(&self.fields, "TargetObj", SCRIPT_SCOPE_GLOBAL)?;
            let strictness_value = parse_int_field_or(
                &self.fields,
                "Strict",
                i32::from(ScriptStrictness::Strict3.raw()),
            )?;
            let strictness = ScriptStrictness::try_from(strictness_value)
                .map_err(|value| ControlParseError::InvalidScriptStrictness { value })?;
            let script = self.fields.get("Script").cloned().unwrap_or_default();
            let script = LegacyCString::from_bytes(legacy_string_bytes(&script)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Script".to_string(),
                },
            )?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::Script(ScriptControlData {
                target_object,
                strictness,
                script,
                by_client,
            })));
        }

        if id == CID_MESSAGE_BOARD_ANSWER {
            // C4ControlMessageBoardAnswer writes packed Object, Answer,
            // inherited packed Plr, then inherited packed ByClient. The INI
            // writer may omit every field at its CompileFunc default.
            let object = parse_int_field_or(&self.fields, "Object", 0)?;
            let answer = self.fields.get("Answer").cloned().unwrap_or_default();
            let answer = LegacyCString::from_bytes(legacy_string_bytes(&answer)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Answer".to_string(),
                },
            )?;
            let player = parse_int_field_or(&self.fields, "Plr", -1)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::MessageBoardAnswer(
                MessageBoardAnswerControlData {
                    object,
                    answer,
                    player,
                    by_client,
                },
            )));
        }

        if id == CID_PLR_SELECT {
            // C4ControlPlayerSelect writes raw Player/ObjCnt/Objs fields,
            // followed by the inherited packed ByClient. The INI writer may
            // omit an all-zero object array through its naming default.
            let player = parse_int_field_or(&self.fields, "Player", -1)?;
            let object_count = parse_int_field_or(&self.fields, "ObjCnt", 0)?;
            let declared = usize::try_from(object_count).map_err(|_| {
                ControlParseError::InvalidPlayerSelectObjectCount {
                    value: object_count,
                }
            })?;
            if declared > MAX_PLAYER_SELECT_INI_OBJECTS {
                return Err(ControlParseError::InvalidPlayerSelectObjectCount {
                    value: object_count,
                });
            }
            let objects = match self.fields.get("Objs") {
                None => vec![0; declared],
                Some(raw) if raw.trim().is_empty() && declared == 0 => Vec::new(),
                Some(raw) => raw
                    .split(',')
                    .take(declared + 1)
                    .map(|value| {
                        value.trim().parse::<i32>().map_err(|_| {
                            ControlParseError::InvalidIntegerField {
                                field: "Objs".to_string(),
                                value: value.trim().to_string(),
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            };
            if objects.len() != declared {
                return Err(ControlParseError::PlayerSelectObjectCountMismatch {
                    declared,
                    actual: objects.len(),
                });
            }
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::PlayerSelect(PlayerSelectControlData {
                player,
                objects,
                by_client,
            })));
        }

        if id == CID_JOIN_PLR {
            // C4ControlJoinPlayer::CompileFunc (C4Control.cpp:852-863).
            // Read the outer section directly: a nested ResCore repeats ID
            // and Filename and therefore overwrites the flattened map.
            let body = self.section_fields("Join Player").unwrap_or(&[]);
            let field = |name: &str| {
                body.iter()
                    .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.as_str())
            };
            let int = |name: &str, default: i32| match field(name) {
                None => Ok(default),
                Some(value) => value.parse::<i32>().map_err(|_| {
                    ControlParseError::InvalidIntegerField {
                        field: name.to_string(),
                        value: value.to_string(),
                    }
                }),
            };
            let filename = normalize_network_filename(field("Filename").unwrap_or_default());
            let at_client = int("AtClient", -1)?;
            let info_id = int("InfoID", -1)?;
            let by_res = match field("ByRes") {
                None => false,
                Some(value) => parse_bool_value("ByRes", value)?,
            };
            let source = if by_res {
                let fields = self
                    .section_fields("ResCore")
                    .ok_or(ControlParseError::MissingJoinPlayerResource)?;
                JoinPlayerSource::Resource(parse_network_resource_core(fields)?)
            } else {
                JoinPlayerSource::Embedded(
                    field("PlrData")
                        .map(|value| parse_std_buf(value))
                        .unwrap_or_default(),
                )
            };
            let filename = LegacyCString::from_bytes(legacy_string_bytes(&filename)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Filename".to_string(),
                },
            )?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename,
                at_client,
                info_id,
                source,
                by_client,
            })));
        }

        if id == CID_REMOVE_PLR {
            let player = parse_int_field_or(&self.fields, "Plr", -1)?;
            let disconnected = parse_bool_field_or(&self.fields, "Disconnected", false)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::RemovePlayer(RemovePlayerControlData {
                player,
                disconnected,
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
            let int = |fields: &[(String, String)], key: &str, default: i32| {
                match field(fields, key) {
                    None => Ok(default),
                    Some(value) => value.parse::<i32>().map_err(|_| {
                        ControlParseError::InvalidIntegerField {
                            field: key.to_string(),
                            value,
                        }
                    }),
                }
            };
            let uint = |fields: &[(String, String)], key: &str, default: u32| {
                match field(fields, key) {
                    None => Ok(default),
                    Some(value) => value.parse::<u32>().map_err(|_| {
                        ControlParseError::InvalidIntegerField {
                            field: key.to_string(),
                            value,
                        }
                    }),
                }
            };
            let string = |fields: &[(String, String)], key: &str| {
                LegacyCString::from_bytes(legacy_string_bytes(
                    &field(fields, key).unwrap_or_default(),
                ))
                    .ok_or(ControlParseError::InteriorNulString {
                        field: format!("Player.{key}"),
                    })
            };
            let client_id = int(body, "ID", -1)?;
            let mut client_flags = 0;
            for token in field(body, "Flags")
                .unwrap_or_default()
                .split(['|', ','])
                .map(str::trim)
                .filter(|token| !token.is_empty())
            {
                if token.bytes().all(|byte| byte.is_ascii_digit()) {
                    client_flags |= token.parse::<u32>().map_err(|_| {
                        ControlParseError::InvalidIntegerField {
                            field: "Player Info.Flags".to_string(),
                            value: token.to_string(),
                        }
                    })?;
                } else if token.eq_ignore_ascii_case("AddPlayers") {
                    client_flags |= CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS;
                } else if token.eq_ignore_ascii_case("Updated") {
                    client_flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
                } else if token.eq_ignore_ascii_case("Initial") {
                    client_flags |= CLIENT_PLAYER_INFO_FLAG_INITIAL;
                }
            }
            let players = self
                .sections
                .iter()
                .enumerate()
                .filter(|(_, (name, _))| name.eq_ignore_ascii_case("Player"))
                .map(
                    |(index, (_, fields))| -> Result<ControlPlayerInfoEntry, ControlParseError> {
                        let mut flags = 0;
                        for token in field(fields, "Flags")
                            .unwrap_or_default()
                            .split(['|', ','])
                            .map(str::trim)
                            .filter(|token| !token.is_empty())
                        {
                            if token.bytes().all(|byte| byte.is_ascii_digit()) {
                                flags |= token.parse::<u16>().map_err(|_| {
                                    ControlParseError::InvalidIntegerField {
                                        field: "Player.Flags".to_string(),
                                        value: token.to_string(),
                                    }
                                })?;
                                continue;
                            }
                            for (name, bit) in [
                                ("Joined", PLAYER_INFO_FLAG_JOINED),
                                ("Removed", PLAYER_INFO_FLAG_REMOVED),
                                ("HasResource", PLAYER_INFO_FLAG_HAS_RESOURCE),
                                ("JoinIssued", PLAYER_INFO_FLAG_JOIN_ISSUED),
                                ("SavegameJoin", PLAYER_INFO_FLAG_SAVEGAME_JOIN),
                                ("Disconnected", PLAYER_INFO_FLAG_DISCONNECTED),
                                ("Won", PLAYER_INFO_FLAG_WON),
                                ("VotedOut", PLAYER_INFO_FLAG_VOTED_OUT),
                                ("AttributesFixed", PLAYER_INFO_FLAG_ATTRIBUTES_FIXED),
                                ("NoScenarioInit", PLAYER_INFO_FLAG_NO_SCENARIO_INIT),
                                (
                                    "NoEliminationCheck",
                                    PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
                                ),
                                ("Invisible", PLAYER_INFO_FLAG_INVISIBLE),
                            ] {
                                if token.eq_ignore_ascii_case(name) {
                                    flags |= bit;
                                    break;
                                }
                            }
                        }
                        let player_type = match field(fields, "Type") {
                            None => PLAYER_INFO_TYPE_USER,
                            Some(value) if value.eq_ignore_ascii_case("User") => {
                                PLAYER_INFO_TYPE_USER
                            }
                            Some(value) if value.eq_ignore_ascii_case("Script") => {
                                PLAYER_INFO_TYPE_SCRIPT
                            }
                            Some(value) => value.parse::<u8>().map_err(|_| {
                                ControlParseError::InvalidPlayerType { value }
                            })?,
                        };
                        if player_type != PLAYER_INFO_TYPE_SCRIPT {
                            flags &= !PLAYER_INFO_FLAG_INVISIBLE;
                        }
                        let color = uint(fields, "Color", 0)?;
                        let extra_data = field(fields, "ExtraData")
                            .unwrap_or_else(|| "NONE".to_string());
                        let extra_data: [u8; 4] = extra_data.as_bytes().try_into().map_err(|_| {
                            ControlParseError::InvalidC4IdField {
                                field: "ExtraData".to_string(),
                                value: extra_data,
                            }
                        })?;
                        let resource = if flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
                            let resource_fields = self
                                .sections
                                .get(index + 1)
                                .filter(|(name, _)| name.eq_ignore_ascii_case("ResCore"))
                                .map(|(_, fields)| fields.as_slice())
                                .ok_or(ControlParseError::MissingPlayerInfoResource)?;
                            Some(parse_network_resource_core(resource_fields)?)
                        } else {
                            None
                        };
                        Ok(ControlPlayerInfoEntry {
                            name: string(fields, "Name")?,
                            forced_name: string(fields, "ForcedName")?,
                            filename: string(fields, "Filename")?,
                            flags,
                            id: int(fields, "ID", 0)?,
                            player_type,
                            color,
                            original_color: uint(fields, "OriginalColor", color)?,
                            savegame_player: int(fields, "SavgamePlayer", 0)?,
                            team: int(fields, "Team", 0)?,
                            auth_id: string(fields, "AUID")?,
                            game_number: if flags & PLAYER_INFO_FLAG_JOINED != 0 {
                                int(fields, "GameNumber", -1)?
                            } else {
                                -1
                            },
                            game_join_frame: if flags & PLAYER_INFO_FLAG_JOINED != 0 {
                                int(fields, "GameJoinFrame", -1)?
                            } else {
                                -1
                            },
                            game_part_frame: if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
                                int(fields, "GamePartFrame", -1)?
                            } else {
                                -1
                            },
                            extra_data,
                            league_account: string(fields, "LeagueAccount")?,
                            league_score: int(fields, "LeagueScore", 0)?,
                            league_rank: int(fields, "LeagueRank", 0)?,
                            league_rank_symbol: int(fields, "LeagueRankSymbol", 0)?,
                            league_projected_gain: int(fields, "ProjectedGain", -1)?,
                            clan_tag: string(fields, "ClanTag")?,
                            league_performance: int(fields, "LeaguePerformance", 0)?,
                            // Compilation materializes a C-string value; the
                            // legacy wire shape cannot preserve StdStrBuf's
                            // pre-compilation null/allocated-empty distinction.
                            league_progress_data_is_null: false,
                            league_progress_data: string(fields, "LeagueProgressData")?,
                            resource,
                        })
                    },
                )
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(Some(ControlPacket::PlayerInfo(PlayerInfoControlData {
                client_id,
                flags: client_flags,
                players,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
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
            return Ok(Some(ControlPacket::PlayerControl(PlayerControlData {
                player,
                command,
                data,
                by_client,
            })));
        }

        if id == CID_PLR_COMMAND {
            // C4ControlPlayerCommand::CompileFunc writes these names in this
            // order; the INI compiler may omit every default-valued field.
            let player = parse_int_field_or(&self.fields, "Player", -1)?;
            let command = parse_int_field_or(&self.fields, "Cmd", 0)?;
            let x = parse_int_field_or(&self.fields, "X", 0)?;
            let y = parse_int_field_or(&self.fields, "Y", 0)?;
            let target = parse_int_field_or(&self.fields, "Target", 0)?;
            let target2 = parse_int_field_or(&self.fields, "Target2", 0)?;
            let data = parse_int_field_or(&self.fields, "Data", 0)?;
            let add_mode = parse_int_field_or(&self.fields, "AddMode", 0)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::PlayerCommand(
                PlayerCommandControlData {
                    player,
                    command,
                    x,
                    y,
                    target,
                    target2,
                    data,
                    add_mode,
                    by_client,
                },
            )));
        }

        Ok(Some(ControlPacket::Unknown {
            id: ControlPacketId::new(id),
            name: self.name,
            fields: self.fields,
        }))
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
    #[error("field `{field}` contained invalid boolean `{value}`")]
    InvalidBooleanField { field: String, value: String },
    #[error("field `{field}` contained an interior NUL byte")]
    InteriorNulString { field: String },
    #[error("field `{field}` contained invalid four-byte C4ID `{value}`")]
    InvalidC4IdField { field: String, value: String },
    #[error("resource core type `{value}` is not recognized")]
    InvalidResourceType { value: String },
    #[error("player type `{value}` is not recognized")]
    InvalidPlayerType { value: String },
    #[error("field `{field}` contained invalid SHA-1 `{value}`")]
    InvalidSha1Field { field: String, value: String },
    #[error("loadable resource core has zero chunk size")]
    ZeroResourceChunkSize,
    #[error("player info with HasResource is missing its ResCore section")]
    MissingPlayerInfoResource,
    #[error("resource-backed JoinPlayer is missing its ResCore section")]
    MissingJoinPlayerResource,
    #[error("script strictness {value} is outside the C++ range 0..=3")]
    InvalidScriptStrictness { value: i32 },
    #[error("PlayerSelect object count {value} is outside the supported INI range")]
    InvalidPlayerSelectObjectCount { value: i32 },
    #[error(
        "PlayerSelect declared {declared} objects but its INI array contained {actual} entries"
    )]
    PlayerSelectObjectCountMismatch { declared: usize, actual: usize },
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

fn parse_bool_value(field: &str, value: &str) -> Result<bool, ControlParseError> {
    if value.eq_ignore_ascii_case("true") || value == "1" {
        Ok(true)
    } else if value.eq_ignore_ascii_case("false") || value == "0" {
        Ok(false)
    } else {
        Err(ControlParseError::InvalidBooleanField {
            field: field.to_string(),
            value: value.to_string(),
        })
    }
}

fn parse_bool_field_or(
    fields: &HashMap<String, String>,
    name: &str,
    default: bool,
) -> Result<bool, ControlParseError> {
    match fields.get(name) {
        None => Ok(default),
        Some(value) => parse_bool_value(name, value),
    }
}

fn parse_network_resource_core(
    fields: &[(String, String)],
) -> Result<NetworkResourceCore, ControlParseError> {
    let field = |name: &str| {
        fields
            .iter()
            .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    };
    let int = |name: &str, default: i32| match field(name) {
        None => Ok(default),
        Some(value) => value.parse::<i32>().map_err(|_| {
            ControlParseError::InvalidIntegerField {
                field: format!("ResCore.{name}"),
                value: value.to_string(),
            }
        }),
    };
    let uint = |name: &str, default: u32| match field(name) {
        None => Ok(default),
        Some(value) => value.parse::<u32>().map_err(|_| {
            ControlParseError::InvalidIntegerField {
                field: format!("ResCore.{name}"),
                value: value.to_string(),
            }
        }),
    };
    let string = |name: &str| {
        let value = normalize_network_filename(field(name).unwrap_or_default());
        LegacyCString::from_bytes(legacy_string_bytes(&value)).ok_or(
            ControlParseError::InteriorNulString {
                field: format!("ResCore.{name}"),
            },
        )
    };

    let resource_type = match field("Type") {
        None => NETWORK_RESOURCE_TYPE_NULL,
        Some(value) if value.eq_ignore_ascii_case("Scenario") => 1,
        Some(value) if value.eq_ignore_ascii_case("Dynamic") => 2,
        Some(value) if value.eq_ignore_ascii_case("Player") => 3,
        Some(value) if value.eq_ignore_ascii_case("Definitions") => 4,
        Some(value) if value.eq_ignore_ascii_case("System") => 5,
        Some(value) if value.eq_ignore_ascii_case("Material") => 6,
        Some(value) => value.parse::<u8>().map_err(|_| {
            ControlParseError::InvalidResourceType {
                value: value.to_string(),
            }
        })?,
    };
    let loadable = match field("Loadable") {
        None => true,
        Some(value) => parse_bool_value("ResCore.Loadable", value)?,
    };
    let chunk_size = if loadable {
        uint("ChunkSize", NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE)?
    } else {
        NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE
    };
    if loadable && chunk_size == 0 {
        return Err(ControlParseError::ZeroResourceChunkSize);
    }
    let file_sha = match field("FileSHA") {
        None => None,
        Some(value) if value.len() == 40 => {
            let mut digest = [0u8; 20];
            for (index, byte) in digest.iter_mut().enumerate() {
                let offset = index * 2;
                let pair = std::str::from_utf8(&value.as_bytes()[offset..offset + 2])
                    .ok()
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .ok_or_else(|| ControlParseError::InvalidSha1Field {
                        field: "ResCore.FileSHA".to_string(),
                        value: value.to_string(),
                    })?;
                *byte = pair;
            }
            Some(digest)
        }
        Some(value) => {
            return Err(ControlParseError::InvalidSha1Field {
                field: "ResCore.FileSHA".to_string(),
                value: value.to_string(),
            });
        }
    };

    Ok(NetworkResourceCore {
        resource_type,
        id: int("ID", -1)?,
        derived_id: int("DerID", -1)?,
        loadable,
        file_size: if loadable { uint("FileSize", 0)? } else { u32::MAX },
        file_crc: if loadable { uint("FileCRC", 0)? } else { u32::MAX },
        chunk_size,
        contents_crc: uint("ContentsCRC", 0)?,
        file_sha,
        filename: string("Filename")?,
        author: string("Author")?,
    })
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

fn legacy_string_bytes(value: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(value.len());
    for character in value.chars() {
        let codepoint = u32::from(character);
        if codepoint <= u32::from(u8::MAX) {
            bytes.push(codepoint as u8);
        } else {
            let mut encoded = [0u8; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

#[cfg(windows)]
fn normalize_network_filename(value: &str) -> String {
    value.to_string()
}

#[cfg(not(windows))]
fn normalize_network_filename(value: &str) -> String {
    value.replace('\\', "/")
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
            Some('a') => result.push('\u{0007}'),
            Some('v') => result.push('\u{000b}'),
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
        assert!(!player.no_elimination_check());

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
    fn parses_remove_player_packet_fields_and_omitted_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=146\n\
    [Remove Player]\n\
      Plr=130\n\
      Disconnected=true\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=146\n\
    [Remove Player]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse RemovePlr controls"),
            vec![
                ControlPacket::RemovePlayer(RemovePlayerControlData {
                    player: 130,
                    disconnected: true,
                    by_client: 0,
                }),
                ControlPacket::RemovePlayer(RemovePlayerControlData::default()),
            ]
        );
    }

    #[test]
    fn parses_synchronize_and_sync_check_fields_and_omitted_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=134\n\
    [Synchronize]\n\
      SavePlrs=true\n\
      SyncClear=1\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=134\n\
    [Synchronize]\n\
  [IDPacket]\n\
    ID=133\n\
    [Sync Check]\n\
      Frame=11\n\
      ControlTick=12\n\
      Random3=-13\n\
      RandomCount=14\n\
      AllCrewPosX=-15\n\
      PXSCount=16\n\
      MassMoverIndex=17\n\
      ObjectCount=18\n\
      ObjectEnumerationIndex=19\n\
      SectShapeSum=-20\n\
      ByClient=3\n\
  [IDPacket]\n\
    ID=133\n\
    [Sync Check]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse synchronize controls"),
            vec![
                ControlPacket::Synchronize(SynchronizeControlData {
                    save_player_files: true,
                    sync_clearance: true,
                    by_client: 7,
                }),
                ControlPacket::Synchronize(SynchronizeControlData::default()),
                ControlPacket::SyncCheck(SyncCheckPacket {
                    frame: 11,
                    control_tick: 12,
                    random3: -13,
                    random_count: 14,
                    crew_positions_sum: -15,
                    pxs_count: 16,
                    mass_mover_index: 17,
                    object_count: 18,
                    object_enumeration_index: 19,
                    sector_shape_sum: -20,
                    by_client: 3,
                }),
                ControlPacket::SyncCheck(SyncCheckPacket {
                    frame: -1,
                    control_tick: 0,
                    random3: 0,
                    random_count: 0,
                    crew_positions_sum: 0,
                    pxs_count: 0,
                    mass_mover_index: 0,
                    object_count: 0,
                    object_enumeration_index: 0,
                    sector_shape_sum: 0,
                    by_client: -1,
                }),
            ]
        );
    }

    #[test]
    fn rejects_malformed_synchronize_boolean() {
        let input = "[Control]\n[IDPacket]\nID=134\n[Synchronize]\nSyncClear=tru\n";
        assert!(matches!(
            parse_control_ini(input),
            Err(ControlParseError::InvalidBooleanField { field, value })
                if field == "SyncClear" && value == "tru"
        ));
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
    fn parses_player_select_packet_array_and_omitted_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=160\n\
    [Player Select]\n\
      Player=7\n\
      ObjCnt=3\n\
      Objs=11,-2,0\n\
      ByClient=4\n\
  [IDPacket]\n\
    ID=160\n\
    [Player Select]\n\
  [IDPacket]\n\
    ID=160\n\
    [Player Select]\n\
      ObjCnt=3\n";

        assert_eq!(
            parse_control_ini(input).expect("parse player-select controls"),
            vec![
                ControlPacket::PlayerSelect(PlayerSelectControlData {
                    player: 7,
                    objects: vec![11, -2, 0],
                    by_client: 4,
                }),
                ControlPacket::PlayerSelect(PlayerSelectControlData {
                    player: -1,
                    objects: Vec::new(),
                    by_client: -1,
                }),
                ControlPacket::PlayerSelect(PlayerSelectControlData {
                    player: -1,
                    objects: vec![0, 0, 0],
                    by_client: -1,
                }),
            ]
        );
    }

    #[test]
    fn rejects_player_select_ini_counts_that_would_exhaust_memory() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=160\n\
    [Player Select]\n\
      ObjCnt=2147483647\n";

        assert!(matches!(
            parse_control_ini(input),
            Err(ControlParseError::InvalidPlayerSelectObjectCount { value: i32::MAX })
        ));
    }

    #[test]
    fn rejects_player_select_ini_arrays_longer_than_the_declared_count() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=160\n\
    [Player Select]\n\
      ObjCnt=1\n\
      Objs=11,12,13\n";

        assert!(matches!(
            parse_control_ini(input),
            Err(ControlParseError::PlayerSelectObjectCountMismatch {
                declared: 1,
                actual: 2,
            })
        ));
    }

    #[test]
    fn parses_script_packet_and_omitted_defaults() {
        // C4ControlScript::CompileFunc writes TargetObj/Strict/Script then
        // inherited ByClient. A second packet exercises every omitted INI
        // default from the same compiler.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=136\n\
    [Script]\n\
      TargetObj=-2\n\
      Strict=2\n\
      Script=\"SetGravity(17);\"\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=136\n\
    [Script]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse script controls"),
            vec![
                ControlPacket::Script(ScriptControlData {
                    target_object: SCRIPT_SCOPE_CONSOLE,
                    strictness: ScriptStrictness::Strict2,
                    script: LegacyCString::from_bytes(b"SetGravity(17);".to_vec())
                        .expect("fixture is NUL-free"),
                    by_client: 7,
                }),
                ControlPacket::Script(ScriptControlData::default()),
            ]
        );
    }

    #[test]
    fn parsed_legacy_strings_preserve_high_octal_bytes() {
        let input =
            "[Control]\n[IDPacket]\nID=136\n[Script]\nScript=\"\\200\\377\\a\\v\"\n";
        let packets = parse_control_ini(input).expect("parse legacy script bytes");
        let ControlPacket::Script(script) = &packets[0] else {
            panic!("expected Script, got {:?}", packets[0]);
        };
        assert_eq!(script.script.as_bytes(), &[0x80, 0xff, 0x07, 0x0b]);
    }

    #[test]
    fn rejects_script_strictness_outside_cpp_range() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=136\n\
    [Script]\n\
      Strict=4\n";

        assert!(matches!(
            parse_control_ini(input),
            Err(ControlParseError::InvalidScriptStrictness { value: 4 })
        ));
    }

    #[test]
    fn parses_message_board_answer_packet_and_omitted_defaults() {
        // C4ControlMessageBoardAnswer::CompileFunc writes Object/Answer,
        // inherited Plr, then inherited ByClient. StdCompilerINIWrite omits
        // their defaults 0, empty, -1, and -1 respectively.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=208\n\
    [Message Board Answer]\n\
      Object=130\n\
      Answer=\"typed answer\"\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=208\n\
    [Message Board Answer]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse message-board answers"),
            vec![
                ControlPacket::MessageBoardAnswer(MessageBoardAnswerControlData {
                    object: 130,
                    answer: LegacyCString::from_bytes(b"typed answer".to_vec())
                        .expect("fixture is NUL-free"),
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::MessageBoardAnswer(MessageBoardAnswerControlData::default()),
            ]
        );
    }

    #[test]
    fn parses_player_command_packet_and_omitted_defaults() {
        // C4ControlPlayerCommand::CompileFunc (C4Control.cpp:428-438) names
        // the eight body fields below and delegates inherited ByClient last.
        // StdCompilerINIWrite may omit zero/default fields, so Target2 and
        // Data deliberately exercise their C++ defaults here.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=162\n\
    [Player Command]\n\
      Player=3\n\
      Cmd=14\n\
      X=-25\n\
      Y=40\n\
      Target=91\n\
      AddMode=5\n\
      ByClient=7\n";

        assert_eq!(
            parse_control_ini(input).expect("parse player command"),
            vec![ControlPacket::PlayerCommand(PlayerCommandControlData {
                player: 3,
                command: 14,
                x: -25,
                y: 40,
                target: 91,
                target2: 0,
                data: 0,
                add_mode: 5,
                by_client: 7,
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
    fn parses_resource_backed_join_player_with_complete_core() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=145\n\
    [Join Player]\n\
      Filename=Original\\P.c4p\n\
      AtClient=2\n\
      InfoID=9\n\
      ByRes=true\n\
      [ResCore]\n\
        Type=Player\n\
        ID=17\n\
        DerID=5\n\
        Loadable=true\n\
        FileSize=1234\n\
        FileCRC=305419896\n\
        ChunkSize=1024\n\
        ContentsCRC=2596069104\n\
        FileSHA=00112233445566778899aabbccddeeff10203040\n\
        Filename=Players\\Tyler.c4p\n\
        Author=Host\\Tyler\n\
      ByClient=4\n";

        let packets = parse_control_ini(input).expect("parse resource join");
        let ControlPacket::JoinPlayer(join) = &packets[0] else {
            panic!("expected JoinPlayer, got {:?}", packets[0]);
        };
        assert_eq!(join.filename.to_str(), Ok("Original/P.c4p"));
        assert_eq!((join.at_client, join.info_id, join.by_client), (2, 9, 4));
        let JoinPlayerSource::Resource(core) = &join.source else {
            panic!("expected resource-backed join");
        };
        assert_eq!((core.resource_type, core.id, core.derived_id), (3, 17, 5));
        assert!(core.loadable);
        assert_eq!((core.file_size, core.file_crc), (1234, 305419896));
        assert_eq!((core.chunk_size, core.contents_crc), (1024, 2596069104));
        assert_eq!(
            core.file_sha,
            Some([
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99,
                0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff, 0x10, 0x20, 0x30, 0x40,
            ])
        );
        assert_eq!(core.filename.to_str(), Ok("Players/Tyler.c4p"));
        assert_eq!(core.author.to_str(), Ok("Host/Tyler"));
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
        Flags=NoScenarioInit|NoEliminationCheck\n\
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
                assert_eq!(info.flags, CLIENT_PLAYER_INFO_FLAG_INITIAL);
                assert_eq!(info.by_client, 0);
                assert_eq!(info.players.len(), 2);
                assert_eq!(info.players[0].id, 1);
                assert_eq!(info.players[0].name.to_str(), Ok("Tyler"));
                assert_eq!(info.players[0].color, 15997440);
                assert_eq!(info.players[0].team, 0);
                assert!(!info.players[0].is_script_player());
                assert!(info.players[0].no_scenario_init());
                assert!(info.players[0].no_elimination_check());
                assert_eq!(info.players[1].id, 2);
                assert_eq!(info.players[1].name.to_str(), Ok("Rival"));
                assert_eq!(info.players[1].team, 7);
                assert!(info.players[1].is_script_player());
            }
            other => panic!("expected PlayerInfo, got {other:?}"),
        }
    }

    #[test]
    fn player_info_parser_preserves_complete_compile_fields() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=144\n\
    [Player Info]\n\
      ID=9\n\
      Flags=AddPlayers|Updated|Initial\n\
      [Player]\n\
        Name=Visible\n\
        ForcedName=Forced\n\
        Filename=Bot.c4p\n\
        Flags=Joined|Removed|HasResource|64|Disconnected|Won|VotedOut|AttributesFixed|NoScenarioInit|NoEliminationCheck|Invisible\n\
        ID=17\n\
        Type=Script\n\
        Color=4294967295\n\
        OriginalColor=305419896\n\
        SavgamePlayer=-8\n\
        Team=4\n\
        AUID=auth\n\
        GameNumber=3\n\
        GameJoinFrame=123\n\
        GamePartFrame=456\n\
        ExtraData=ABCD\n\
        LeagueAccount=league\n\
        LeagueScore=-10\n\
        LeagueRank=11\n\
        LeagueRankSymbol=12\n\
        ProjectedGain=13\n\
        ClanTag=clan\n\
        LeaguePerformance=14\n\
        LeagueProgressData=progress\n\
        [ResCore]\n\
          Type=Player\n\
          ID=23\n\
          FileSize=1234\n\
          FileCRC=305419896\n\
          ChunkSize=1024\n\
          ContentsCRC=2596069104\n\
          Filename=Players/Bot.c4p\n\
          Author=Host/Bot\n\
      ByClient=5\n";

        let packets = parse_control_ini(input).expect("parse complete player info");
        let ControlPacket::PlayerInfo(info) = &packets[0] else {
            panic!("expected PlayerInfo, got {:?}", packets[0]);
        };
        assert_eq!(info.client_id, 9);
        assert_eq!(
            info.flags,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS
                | CLIENT_PLAYER_INFO_FLAG_UPDATED
                | CLIENT_PLAYER_INFO_FLAG_INITIAL
        );
        assert_eq!(info.by_client, 5);
        let player = &info.players[0];
        assert_eq!(player.name.to_str(), Ok("Visible"));
        assert_eq!(player.forced_name.to_str(), Ok("Forced"));
        assert_eq!(player.filename.to_str(), Ok("Bot.c4p"));
        assert_eq!(player.id, 17);
        assert_eq!(player.player_type, PLAYER_INFO_TYPE_SCRIPT);
        assert_eq!(player.color, u32::MAX);
        assert_eq!(player.original_color, 0x12345678);
        assert_eq!((player.savegame_player, player.team), (-8, 4));
        assert_eq!(player.auth_id.to_str(), Ok("auth"));
        assert_eq!(
            (
                player.game_number,
                player.game_join_frame,
                player.game_part_frame,
            ),
            (3, 123, 456)
        );
        assert_eq!(player.extra_data, *b"ABCD");
        assert_eq!(player.league_account.to_str(), Ok("league"));
        assert_eq!(
            (
                player.league_score,
                player.league_rank,
                player.league_rank_symbol,
                player.league_projected_gain,
                player.league_performance,
            ),
            (-10, 11, 12, 13, 14)
        );
        assert_eq!(player.clan_tag.to_str(), Ok("clan"));
        assert_eq!(player.league_progress_data.to_str(), Ok("progress"));
        assert_eq!(
            player.flags,
            PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_REMOVED
                | PLAYER_INFO_FLAG_HAS_RESOURCE
                | PLAYER_INFO_FLAG_IN_SCENARIO_FILE
                | PLAYER_INFO_FLAG_DISCONNECTED
                | PLAYER_INFO_FLAG_WON
                | PLAYER_INFO_FLAG_VOTED_OUT
                | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
                | PLAYER_INFO_FLAG_INVISIBLE
        );
        let resource = player.resource.as_ref().expect("resource core retained");
        assert_eq!((resource.resource_type, resource.id), (3, 23));
        assert_eq!((resource.file_size, resource.file_crc), (1234, 305419896));
        assert_eq!((resource.chunk_size, resource.contents_crc), (1024, 2596069104));
        assert_eq!(resource.filename.to_str(), Ok("Players/Bot.c4p"));
        assert_eq!(resource.author.to_str(), Ok("Host/Bot"));
    }

    #[test]
    fn player_info_parser_preserves_numeric_player_types() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=144\n\
    [Player Info]\n\
      [Player]\n\
        ID=1\n\
        Type=0\n\
      [Player]\n\
        ID=2\n\
        Type=2\n";
        let packets = parse_control_ini(input).expect("parse numeric player types");
        let ControlPacket::PlayerInfo(info) = &packets[0] else {
            panic!("expected PlayerInfo, got {:?}", packets[0]);
        };
        assert_eq!(info.players[0].player_type, PLAYER_INFO_TYPE_NONE);
        assert_eq!(info.players[1].player_type, PLAYER_INFO_TYPE_SCRIPT);
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
    fn preserves_untyped_and_lossy_commands_as_raw_bytes() {
        for command in [
            COM_CONTENTS,
            COM_WHEEL_UP,
            COM_WHEEL_DOWN,
            COM_LEFT | COM_SINGLE,
            COM_LEFT | COM_DOUBLE,
        ] {
            assert_eq!(
                interpret_player_control_command(i32::from(command)),
                Some(ControlEvent::RawPlayerControl { command, data: 0 })
            );
        }
    }

    #[test]
    fn every_in_com_byte_round_trips_through_the_interpreter() {
        fn button_code(button: ControlButton) -> u8 {
            match button {
                ControlButton::Left => COM_LEFT,
                ControlButton::Right => COM_RIGHT,
                ControlButton::Up => COM_UP,
                ControlButton::Down => COM_DOWN,
            }
        }

        fn command_code(command: ControlCommand) -> u8 {
            match command {
                ControlCommand::Throw => COM_THROW,
                ControlCommand::Dig => COM_DIG,
                ControlCommand::Special => COM_SPECIAL,
                ControlCommand::Special2 => COM_SPECIAL2,
                ControlCommand::CursorLeft => COM_CURSOR_LEFT,
                ControlCommand::CursorRight => COM_CURSOR_RIGHT,
                ControlCommand::CursorToggle => COM_CURSOR_TOGGLE,
                ControlCommand::PlayerMenu => COM_PLAYER_MENU,
                ControlCommand::MenuEnter => COM_MENU_ENTER,
                ControlCommand::MenuEnterAll => COM_MENU_ENTER_ALL,
                ControlCommand::MenuClose => COM_MENU_CLOSE,
                ControlCommand::MenuShowText => COM_MENU_SHOW_TEXT,
                ControlCommand::MenuLeft => COM_MENU_LEFT,
                ControlCommand::MenuRight => COM_MENU_RIGHT,
                ControlCommand::MenuUp => COM_MENU_UP,
                ControlCommand::MenuDown => COM_MENU_DOWN,
                ControlCommand::MenuSelect => COM_MENU_SELECT,
            }
        }

        fn encoded(event: ControlEvent) -> u8 {
            match event {
                ControlEvent::Press(button) => button_code(button),
                ControlEvent::Release(button) => button_code(button) + COM_RELEASE_OFFSET,
                ControlEvent::Command { command, kind } => {
                    let base = command_code(command);
                    match kind {
                        CommandKind::Press => base,
                        CommandKind::Release => base + COM_RELEASE_OFFSET,
                        CommandKind::Single => base | COM_SINGLE,
                        CommandKind::Double => base | COM_DOUBLE,
                    }
                }
                ControlEvent::RawPlayerControl { command, data } => {
                    assert_eq!(data, 0);
                    command
                }
                ControlEvent::ClearPressed => COM_CLEAR_PRESSED_COMS,
            }
        }

        for command in 1..=u8::MAX {
            let event = interpret_player_control_command(i32::from(command))
                .unwrap_or_else(|| panic!("command {command} was dropped"));
            assert_eq!(encoded(event), command, "command {command}");
        }
    }

    #[test]
    fn rejects_only_commands_outside_the_in_com_byte_domain() {
        assert!(interpret_player_control_command(999).is_none());
        assert!(interpret_player_control_command(-5).is_none());
        assert_eq!(
            interpret_player_control_command(41),
            Some(ControlEvent::RawPlayerControl {
                command: 41,
                data: 0
            })
        );
    }
}
