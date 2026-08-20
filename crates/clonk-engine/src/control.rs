use std::collections::HashMap;

use thiserror::Error;

const MAX_PLAYER_SELECT_INI_OBJECTS: usize = 1_000_000;
const MAX_EM_MOVE_OBJECT_INI_OBJECTS: usize = 1_000_000;

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
    /// Synchronized game/control parameter mutation (`CID_Set`,
    /// C4Control.cpp:128-247).
    Set(SetControlData),
    /// Opaque debug-record payload (`CID_DebugRec`). Native execution is an
    /// intentional no-op; retaining the bytes keeps known records decodable.
    DebugRecord(DebugRecordControlData),
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
    /// Synchronized execution of a scenario-registered message-board command
    /// (`CID_CustomCommand`, C4Control.cpp:1596-1682).
    CustomCommand(CustomCommandControlData),
    /// Synchronized mouse/object selection (`CID_PlrSelect`,
    /// C4Control.cpp:329-380).
    PlayerSelect(PlayerSelectControlData),
    /// Player control command (`CID_PlrControl`).
    PlayerControl(PlayerControlData),
    /// Mouse/object command (`CID_PlrCommand`, C4Control.cpp:405-439).
    PlayerCommand(PlayerCommandControlData),
    /// Non-synchronized chat, sound, alert, or system message
    /// (`CID_Message`, C4Control.cpp:1071-1260).
    Message(MessageControlData),
    /// Synchronized editor object manipulation (`CID_EMMoveObj`,
    /// C4Control.cpp:865-992).
    EmMoveObject(EmMoveObjectControlData),
    /// Synchronized editor landscape drawing (`CID_EMDrawTool`,
    /// C4Control.cpp:994-1054).
    EmDrawTool(EmDrawToolControlData),
    /// Synchronized editor definition drop (`CID_EMDropDef`,
    /// C4Control.cpp:1524-1564).
    EmDropDef(EmDropDefControlData),
    /// Queued team choice that resumes a player waiting in
    /// `PS_TeamSelectionPending` (`CID_InitScenarioPlayer`).
    InitScenarioPlayer(InitScenarioPlayerControlData),
    /// Queued synchronized goal evaluation and local goal-menu activation
    /// (`CID_ActivateGameGoalMenu`).
    ActivateGameGoalMenu(ActivateGameGoalMenuControlData),
    /// Queued one-way hostility toggle (`CID_ToggleHostility`).
    ToggleHostility(ToggleHostilityControlData),
    /// Queued player surrender (`CID_SurrenderPlayer`). C++ authenticates the
    /// player through the inherited `ByClient` field before executing it.
    SurrenderPlayer(SurrenderPlayerControlData),
    /// Queued object-scoped goal or rule activation
    /// (`CID_ActivateGameGoalRule`).
    ActivateGameGoalRule(ActivateGameGoalRuleControlData),
    /// Queued runtime player-team switch (`CID_SetPlayerTeam`).
    SetPlayerTeam(SetPlayerTeamControlData),
    /// Host-only synchronized regular player elimination
    /// (`CID_EliminatePlayer`).
    EliminatePlayer(EliminatePlayerControlData),
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

/// Raw `C4CtrlValueType` values serialized by `C4ControlSet`.
///
/// The native compiler writes this enum through a fixed-width signed integer,
/// so unknown values remain representable and execute as release-build no-ops.
pub const SET_VALUE_NONE: i32 = -1;
pub const SET_VALUE_CONTROL_RATE: i32 = 0;
pub const SET_VALUE_DISABLE_DEBUG: i32 = 1;
pub const SET_VALUE_MAX_PLAYER: i32 = 2;
pub const SET_VALUE_TEAM_DISTRIBUTION: i32 = 3;
pub const SET_VALUE_TEAM_COLORS: i32 = 4;
pub const SET_VALUE_FAIR_CREW: i32 = 5;

/// Raw `C4ControlMessageType` values serialized by `C4ControlMessage`.
///
/// The C++ compiler casts the enum through `uint8_t` without validation, so
/// message types remain raw bytes and unknown values survive codec and replay
/// round trips (`src/C4Control.cpp:1252-1259`).
pub const MESSAGE_TYPE_NORMAL: u8 = 0;
pub const MESSAGE_TYPE_ME: u8 = 1;
pub const MESSAGE_TYPE_SAY: u8 = 2;
pub const MESSAGE_TYPE_TEAM: u8 = 3;
pub const MESSAGE_TYPE_PRIVATE: u8 = 4;
pub const MESSAGE_TYPE_SOUND: u8 = 5;
pub const MESSAGE_TYPE_ALERT: u8 = 6;
pub const MESSAGE_TYPE_SYSTEM: u8 = 10;

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

    /// Strictness representation used by `clonk-script`: non-strict has no
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

impl ClientUpdateControlData {
    pub const fn new(update_type: u8, client_id: i32, data: i32, by_client: i32) -> Self {
        Self {
            update_type,
            client_id,
            data,
            by_client,
        }
    }
}

/// Body of `C4ControlClientRemove` (`src/C4Control.cpp:682-687`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientRemoveControlData {
    pub client_id: i32,
    pub reason: LegacyCString,
    pub by_client: i32,
}

/// Body of `C4ControlSet` (`src/C4Control.cpp:249-254`).
///
/// `value_type` deliberately remains a signed integer: `C4CVT_None` is -1
/// and release builds tolerate otherwise unknown raw enum values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetControlData {
    pub value_type: i32,
    pub data: i32,
    pub by_client: i32,
}

impl Default for SetControlData {
    fn default() -> Self {
        Self {
            value_type: SET_VALUE_NONE,
            data: 0,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlDebugRec` (`src/C4Control.cpp:1307-1314`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebugRecordControlData {
    pub data: Vec<u8>,
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

/// Body of `C4ControlCustomCommand` (`CID_CustomCommand`).
///
/// Both strings remain NUL-free legacy bytes. C++ writes them before the
/// inherited packed `Plr` and `ByClient` fields
/// (`src/C4Control.cpp:1596-1600,1566-1570,53-57`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomCommandControlData {
    pub command: LegacyCString,
    pub argument: LegacyCString,
    pub player: i32,
    pub by_client: i32,
}

impl Default for CustomCommandControlData {
    fn default() -> Self {
        Self {
            command: LegacyCString::default(),
            argument: LegacyCString::default(),
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

impl PlayerControlData {
    pub const fn new(player: i32, command: i32, data: i32, by_client: i32) -> Self {
        Self {
            player,
            command,
            data,
            by_client,
        }
    }
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

/// Body of `C4ControlMessage` (`CID_Message`).
///
/// `ToPlayer` is present on the C++ wire only for raw message type
/// [`MESSAGE_TYPE_PRIVATE`]. Message bytes remain NUL-free legacy bytes so
/// controls round-trip without requiring UTF-8 (`src/C4Control.cpp:1252-1259`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageControlData {
    pub message_type: u8,
    pub player: i32,
    pub to_player: i32,
    pub message: LegacyCString,
    pub by_client: i32,
}

impl Default for MessageControlData {
    fn default() -> Self {
        Self {
            message_type: MESSAGE_TYPE_NORMAL,
            player: -1,
            to_player: -1,
            message: LegacyCString::default(),
            by_client: -1,
        }
    }
}

/// Raw `C4ControlEMObjectAction` values serialized by
/// `C4ControlEMMoveObject` (`src/C4Control.h:335-343`).
///
/// [`EmMoveObjectControlData::action`] deliberately remains a byte so unknown
/// values survive network and replay round trips like the C++ enum adaptor.
pub const EMMO_MOVE: u8 = 0;
pub const EMMO_ENTER: u8 = 1;
pub const EMMO_DUPLICATE: u8 = 2;
pub const EMMO_SCRIPT: u8 = 3;
pub const EMMO_REMOVE: u8 = 4;
pub const EMMO_EXIT: u8 = 5;

/// Body of `C4ControlEMMoveObject` (`CID_EMMoveObj`).
///
/// C++ writes the action byte, coordinates and target as fixed-width values,
/// the object count as a packed integer, strictness as a byte, each object as
/// a fixed-width value, an action-conditional script string, and inherited
/// packed `ByClient` last (`src/C4Control.cpp:972-992`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmMoveObjectControlData {
    pub action: u8,
    pub tx: i32,
    pub ty: i32,
    pub target_object: i32,
    pub objects: Vec<i32>,
    pub strictness: ScriptStrictness,
    pub script: LegacyCString,
    pub by_client: i32,
}

impl Default for EmMoveObjectControlData {
    fn default() -> Self {
        Self {
            action: EMMO_MOVE,
            tx: 0,
            ty: 0,
            target_object: -1,
            objects: Vec::new(),
            strictness: ScriptStrictness::Strict3,
            script: LegacyCString::default(),
            by_client: -1,
        }
    }
}

/// Raw `C4ControlEMDrawAction` values serialized by
/// `C4ControlEMDrawTool` (`src/C4Control.h:367-373`).
///
/// The C++ enum adaptor carries an unvalidated `uint8_t`, so unknown action
/// bytes must survive network and replay round trips.
pub const EMDT_SET_MODE: u8 = 0;
pub const EMDT_BRUSH: u8 = 1;
pub const EMDT_FILL: u8 = 2;
pub const EMDT_LINE: u8 = 3;
pub const EMDT_RECT: u8 = 4;

/// Body of `C4ControlEMDrawTool` (`CID_EMDrawTool`).
///
/// C++ writes the action byte, packed landscape mode, four fixed-width
/// coordinates, packed grade, native bool, two legacy strings, and inherited
/// packed `ByClient` in that order (`src/C4Control.cpp:1056-1068`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmDrawToolControlData {
    pub action: u8,
    pub mode: i32,
    pub x: i32,
    pub y: i32,
    pub x2: i32,
    pub y2: i32,
    pub grade: i32,
    pub ift: bool,
    pub material: LegacyCString,
    pub texture: LegacyCString,
    pub by_client: i32,
}

impl Default for EmDrawToolControlData {
    fn default() -> Self {
        Self {
            action: EMDT_SET_MODE,
            mode: 0,
            x: 0,
            y: 0,
            x2: 0,
            y2: 0,
            grade: 0,
            ift: false,
            material: LegacyCString::default(),
            texture: LegacyCString::default(),
            by_client: -1,
        }
    }
}

/// Body of `C4ControlEMDropDef` (`CID_EMDropDef`).
///
/// C++ writes the definition through `C4IDAdapt`, then packed X/Y and the
/// inherited packed `ByClient` (`src/C4Control.cpp:1524-1530,53-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmDropDefControlData {
    pub id: [u8; 4],
    pub x: i32,
    pub y: i32,
    pub by_client: i32,
}

impl Default for EmDropDefControlData {
    fn default() -> Self {
        Self {
            id: *b"NONE",
            x: 0,
            y: 0,
            by_client: -1,
        }
    }
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

/// Body of `C4ControlActivateGameGoalMenu` and its control bases.
///
/// The C++ compiler writes inherited packed `Plr` followed by packed
/// `ByClient` (`src/C4Control.cpp:1566-1570,53-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivateGameGoalMenuControlData {
    pub player: i32,
    pub by_client: i32,
}

impl Default for ActivateGameGoalMenuControlData {
    fn default() -> Self {
        Self {
            player: -1,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlToggleHostility` and its control bases.
///
/// The C++ compiler writes packed `Opponent`, inherited packed `Plr`, then
/// packed `ByClient` (`src/C4Control.cpp:1690-1694,1566-1570,53-57`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToggleHostilityControlData {
    pub opponent: i32,
    pub player: i32,
    pub by_client: i32,
}

impl Default for ToggleHostilityControlData {
    fn default() -> Self {
        Self {
            opponent: -1,
            player: -1,
            by_client: -1,
        }
    }
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

/// Body of `C4ControlActivateGameGoalRule` and its control bases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivateGameGoalRuleControlData {
    pub object: i32,
    pub player: i32,
    pub by_client: i32,
}

impl Default for ActivateGameGoalRuleControlData {
    fn default() -> Self {
        Self {
            object: 0,
            player: -1,
            by_client: -1,
        }
    }
}

/// Body of `C4ControlSetPlayerTeam` and its control bases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SetPlayerTeamControlData {
    pub team: i32,
    pub player: i32,
    pub by_client: i32,
}

impl Default for SetPlayerTeamControlData {
    fn default() -> Self {
        Self {
            team: 0,
            player: -1,
            by_client: -1,
        }
    }
}

/// Body of host-only `C4ControlEliminatePlayer` and its control bases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EliminatePlayerControlData {
    pub player: i32,
    pub by_client: i32,
}

impl Default for EliminatePlayerControlData {
    fn default() -> Self {
        Self {
            player: -1,
            by_client: -1,
        }
    }
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

impl PlayerInfoUpdateRequest {
    pub fn new(client_id: i32, flags: u32, players: Vec<ControlPlayerInfoEntry>) -> Self {
        Self {
            client_id,
            flags,
            players,
        }
    }
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

impl PlayerInfoControlData {
    pub fn new(
        client_id: i32,
        flags: u32,
        players: Vec<ControlPlayerInfoEntry>,
        by_client: i32,
    ) -> Self {
        Self {
            client_id,
            flags,
            players,
            by_client,
        }
    }
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
    /// Strict live-network comparison from `C4ControlSyncCheck::Execute`.
    pub fn matches(&self, other: &Self) -> bool {
        self.matches_with_replay_mode(other, false)
    }

    /// Replay comparison from `C4ControlSyncCheck::Execute`: playback pacing
    /// may shift `ControlTick`, while every simulation digest field remains
    /// strict.
    pub fn matches_replay(&self, other: &Self) -> bool {
        self.matches_with_replay_mode(other, true)
    }

    fn matches_with_replay_mode(&self, other: &Self, is_replay: bool) -> bool {
        self.frame == other.frame
            && (is_replay || self.control_tick == other.control_tick)
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
/// Mouse-region-only help control (C4Constants.h:237). C4MouseControl
/// consumes this locally; it must not enter the synchronized player queue.
pub const COM_HELP: u8 = 35;
pub const COM_PLAYER_MENU: u8 = 36;
/// Mouse-region-only external IRC chat control (C4Constants.h:239).
pub const COM_CHAT: u8 = 37;
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
        const CID_CLIENT_JOIN: u8 = 0x80;
        const CID_CLIENT_UPDATE: u8 = 0x81;
        const CID_CLIENT_REMOVE: u8 = 0x82;
        const CID_VOTE: u8 = 0x83;
        const CID_VOTE_END: u8 = 0x84;
        const CID_SYNC_CHECK: u8 = 0x85;
        const CID_SYNCHRONIZE: u8 = 0x86;
        const CID_SET: u8 = 0x87;
        const CID_SCRIPT: u8 = 0x88;
        const CID_MESSAGE_BOARD_ANSWER: u8 = 0xd0;
        const CID_CUSTOM_COMMAND: u8 = 0xd1;
        const CID_ACTIVATE_GAME_GOAL_MENU: u8 = 0xd3;
        const CID_TOGGLE_HOSTILITY: u8 = 0xd4;
        const CID_ACTIVATE_GAME_GOAL_RULE: u8 = 0xd6;
        const CID_SET_PLAYER_TEAM: u8 = 0xd7;
        const CID_ELIMINATE_PLAYER: u8 = 0xd8;
        const CID_PLR_SELECT: u8 = 0xA0;
        const CID_PLR_CONTROL: u8 = 0xA1;
        const CID_PLR_COMMAND: u8 = 0xA2;
        const CID_MESSAGE: u8 = 0xA3;
        const CID_EM_MOVE_OBJECT: u8 = 0xB0;
        const CID_EM_DRAW_TOOL: u8 = 0xB1;
        const CID_EM_DROP_DEF: u8 = 0xB2;
        const CID_DEBUG_RECORD: u8 = 0xC0;
        const CID_INIT_SCENARIO_PLAYER: u8 = 0xD2;
        const CID_SURRENDER_PLAYER: u8 = 0xD5;

        if id == PID_NONE {
            return Ok(None);
        }

        if id == CID_CLIENT_JOIN {
            // C4ControlClientJoin nests C4ClientCore and then appends the
            // inherited author. RawPacket's flattened field projection keeps
            // those uniquely named values available here.
            return Ok(Some(ControlPacket::ClientJoin(ClientJoinControlData {
                core: ClientCoreControlData {
                    client_id: parse_int_field_or(&self.fields, "ID", -1)?,
                    activated: parse_bool_field_or(&self.fields, "Activated", false)?,
                    observer: parse_bool_field_or(&self.fields, "Observer", false)?,
                    name: parse_legacy_string_field_or(&self.fields, "Name")?,
                    nick: parse_legacy_string_field_or(&self.fields, "Nick")?,
                    lobby_ready: parse_bool_field_or(&self.fields, "LobbyReady", false)?,
                },
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
        }

        if id == CID_CLIENT_UPDATE {
            let update_type = parse_u8_field_or(&self.fields, "Type", u8::MAX)?;
            let data = if update_type == CLIENT_UPDATE_ACTIVATE {
                parse_int_field_or(&self.fields, "Data", 0)?
            } else {
                0
            };
            return Ok(Some(ControlPacket::ClientUpdate(
                ClientUpdateControlData::new(
                    update_type,
                    parse_int_field_or(&self.fields, "ClientID", -1)?,
                    data,
                    parse_int_field_or(&self.fields, "ByClient", -1)?,
                ),
            )));
        }

        if id == CID_CLIENT_REMOVE {
            return Ok(Some(ControlPacket::ClientRemove(ClientRemoveControlData {
                client_id: parse_int_field_or(&self.fields, "ClientID", -1)?,
                reason: parse_legacy_string_field_or(&self.fields, "Reason")?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
        }

        if id == CID_VOTE || id == CID_VOTE_END {
            let vote = VoteControlData {
                vote_type: parse_u8_field_or(&self.fields, "Type", VOTE_TYPE_NONE)?,
                approve: parse_bool_field_or(&self.fields, "Approve", true)?,
                data: parse_int_field_or(&self.fields, "Data", 0)?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            };
            return Ok(Some(if id == CID_VOTE {
                ControlPacket::Vote(vote)
            } else {
                ControlPacket::VoteEnd(vote)
            }));
        }

        if id == CID_SET {
            return Ok(Some(ControlPacket::Set(SetControlData {
                value_type: parse_int_field_or(&self.fields, "Type", SET_VALUE_NONE)?,
                data: parse_int_field_or(&self.fields, "Data", 0)?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
        }

        if id == CID_DEBUG_RECORD {
            return Ok(Some(ControlPacket::DebugRecord(DebugRecordControlData {
                data: self
                    .fields
                    // StdBuf is not wrapped in a naming adaptor here, so the
                    // INI compiler writes it as the packet's own value.
                    .get("Debug Rec")
                    .or_else(|| self.fields.get("Data"))
                    .map(|value| parse_std_buf(value))
                    .unwrap_or_default(),
            })));
        }

        if id == CID_INIT_SCENARIO_PLAYER {
            return Ok(Some(ControlPacket::InitScenarioPlayer(
                InitScenarioPlayerControlData {
                    team: parse_int_field_or(&self.fields, "Team", 0)?,
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_SURRENDER_PLAYER {
            return Ok(Some(ControlPacket::SurrenderPlayer(
                SurrenderPlayerControlData {
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
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
            return Ok(Some(ControlPacket::Synchronize(SynchronizeControlData {
                save_player_files,
                sync_clearance,
                by_client,
            })));
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

        if id == CID_CUSTOM_COMMAND {
            // C4ControlCustomCommand writes Command/Argument, inherited Plr,
            // then inherited ByClient. The INI compiler may omit all four
            // default-valued fields.
            let command = self.fields.get("Command").cloned().unwrap_or_default();
            let command = LegacyCString::from_bytes(legacy_string_bytes(&command)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Command".to_string(),
                },
            )?;
            let argument = self.fields.get("Argument").cloned().unwrap_or_default();
            let argument = LegacyCString::from_bytes(legacy_string_bytes(&argument)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Argument".to_string(),
                },
            )?;
            let player = parse_int_field_or(&self.fields, "Plr", -1)?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::CustomCommand(
                CustomCommandControlData {
                    command,
                    argument,
                    player,
                    by_client,
                },
            )));
        }

        if id == CID_ACTIVATE_GAME_GOAL_MENU {
            return Ok(Some(ControlPacket::ActivateGameGoalMenu(
                ActivateGameGoalMenuControlData {
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_TOGGLE_HOSTILITY {
            return Ok(Some(ControlPacket::ToggleHostility(
                ToggleHostilityControlData {
                    opponent: parse_int_field_or(&self.fields, "Opponent", -1)?,
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_ACTIVATE_GAME_GOAL_RULE {
            return Ok(Some(ControlPacket::ActivateGameGoalRule(
                ActivateGameGoalRuleControlData {
                    object: parse_int_field_or(&self.fields, "Object", 0)?,
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_SET_PLAYER_TEAM {
            return Ok(Some(ControlPacket::SetPlayerTeam(
                SetPlayerTeamControlData {
                    team: parse_int_field_or(&self.fields, "Team", 0)?,
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_ELIMINATE_PLAYER {
            return Ok(Some(ControlPacket::EliminatePlayer(
                EliminatePlayerControlData {
                    player: parse_int_field_or(&self.fields, "Plr", -1)?,
                    by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
                },
            )));
        }

        if id == CID_EM_MOVE_OBJECT {
            // Action has no naming default in C++, while every subsequent
            // field does. The enum is adapted through uint8_t but remains raw
            // rather than being restricted to the six currently known
            // editor actions.
            let action_raw =
                self.fields
                    .get("Action")
                    .ok_or_else(|| ControlParseError::MissingField {
                        field: "Action".to_string(),
                    })?;
            let action =
                action_raw
                    .parse::<u8>()
                    .map_err(|_| ControlParseError::InvalidIntegerField {
                        field: "Action".to_string(),
                        value: action_raw.clone(),
                    })?;
            let tx = parse_int_field_or(&self.fields, "tx", 0)?;
            let ty = parse_int_field_or(&self.fields, "ty", 0)?;
            let target_object = parse_int_field_or(&self.fields, "TargetObj", -1)?;
            let object_count = parse_int_field_or(&self.fields, "ObjectNum", 0)?;
            let strictness_value = parse_int_field_or(
                &self.fields,
                "Strict",
                i32::from(ScriptStrictness::Strict3.raw()),
            )?;
            let strictness = ScriptStrictness::try_from(strictness_value)
                .map_err(|value| ControlParseError::InvalidScriptStrictness { value })?;
            let declared = usize::try_from(object_count).map_err(|_| {
                ControlParseError::InvalidEmMoveObjectCount {
                    value: object_count,
                }
            })?;
            if declared > MAX_EM_MOVE_OBJECT_INI_OBJECTS {
                return Err(ControlParseError::InvalidEmMoveObjectCount {
                    value: object_count,
                });
            }
            let mut objects = match self.fields.get("Objs") {
                None => vec![-1; declared],
                Some(raw) if raw.trim().is_empty() => vec![-1; declared],
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
            if objects.len() > declared {
                return Err(ControlParseError::EmMoveObjectCountMismatch {
                    declared,
                    actual: objects.len(),
                });
            }
            if objects.len() < declared {
                // The -1 default wraps the entire StdArrayAdapt. A missing
                // later list element throws NotFound, so C++ assigns -1 to
                // every slot rather than retaining a parsed prefix.
                objects = vec![-1; declared];
            }
            let script = if action == EMMO_SCRIPT {
                let script = self.fields.get("Script").cloned().unwrap_or_default();
                LegacyCString::from_bytes(legacy_string_bytes(&script)).ok_or(
                    ControlParseError::InteriorNulString {
                        field: "Script".to_string(),
                    },
                )?
            } else {
                LegacyCString::default()
            };
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::EmMoveObject(EmMoveObjectControlData {
                action,
                tx,
                ty,
                target_object,
                objects,
                strictness,
                script,
                by_client,
            })));
        }

        if id == CID_EM_DRAW_TOOL {
            // Action has no naming default. The enum travels through a raw
            // uint8 adaptor, while every remaining field uses the defaults
            // named by C4ControlEMDrawTool::CompileFunc.
            let action_raw =
                self.fields
                    .get("Action")
                    .ok_or_else(|| ControlParseError::MissingField {
                        field: "Action".to_string(),
                    })?;
            let action =
                action_raw
                    .parse::<u8>()
                    .map_err(|_| ControlParseError::InvalidIntegerField {
                        field: "Action".to_string(),
                        value: action_raw.clone(),
                    })?;
            let legacy_string = |field: &'static str| {
                let value = self.fields.get(field).cloned().unwrap_or_default();
                LegacyCString::from_bytes(legacy_string_bytes(&value)).ok_or(
                    ControlParseError::InteriorNulString {
                        field: field.to_string(),
                    },
                )
            };
            return Ok(Some(ControlPacket::EmDrawTool(EmDrawToolControlData {
                action,
                mode: parse_int_field_or(&self.fields, "Mode", 0)?,
                x: parse_int_field_or(&self.fields, "X", 0)?,
                y: parse_int_field_or(&self.fields, "Y", 0)?,
                x2: parse_int_field_or(&self.fields, "X2", 0)?,
                y2: parse_int_field_or(&self.fields, "Y2", 0)?,
                grade: parse_int_field_or(&self.fields, "Grade", 0)?,
                ift: parse_bool_field_or(&self.fields, "IFT", false)?,
                material: legacy_string("Material")?,
                texture: legacy_string("Texture")?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
        }

        if id == CID_EM_DROP_DEF {
            let raw_id = self
                .fields
                .get("ID")
                .map(|value| legacy_string_bytes(value))
                .unwrap_or_else(|| b"NONE".to_vec());
            // The INI identifier adaptor has a fixed four-byte buffer: long
            // values are truncated, while short values compile as C4ID_None.
            // RCT_ID stops at the first byte outside its identifier alphabet.
            let raw_id = raw_id
                .into_iter()
                .take_while(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'_' | b'-'))
                .take(4)
                .collect::<Vec<_>>();
            let mut id = *b"NONE";
            if raw_id.len() == id.len() {
                id.copy_from_slice(&raw_id);
            }
            if id == *b"0000" {
                id = *b"NONE";
            }
            return Ok(Some(ControlPacket::EmDropDef(EmDropDefControlData {
                id,
                x: parse_int_field_or(&self.fields, "X", 0)?,
                y: parse_int_field_or(&self.fields, "Y", 0)?,
                by_client: parse_int_field_or(&self.fields, "ByClient", -1)?,
            })));
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
                Some(value) => {
                    value
                        .parse::<i32>()
                        .map_err(|_| ControlParseError::InvalidIntegerField {
                            field: name.to_string(),
                            value: value.to_string(),
                        })
                }
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
                JoinPlayerSource::Embedded(field("PlrData").map(parse_std_buf).unwrap_or_default())
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
            return Ok(Some(ControlPacket::PlayerInfo(
                parse_player_info_client_data(
                    body,
                    &self.sections,
                    parse_int_field_or(&self.fields, "ByClient", -1)?,
                )?,
            )));
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
            return Ok(Some(ControlPacket::PlayerControl(PlayerControlData::new(
                player, command, data, by_client,
            ))));
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

        if id == CID_MESSAGE {
            // C4ControlMessage::CompileFunc writes a raw uint8 Type, packed
            // Player, a type-conditional packed ToPlayer, Message, then the
            // inherited packed ByClient. Unknown raw types are accepted by
            // C++ and, like every non-private type, omit ToPlayer.
            let message_type = match self.fields.get("Type") {
                None => MESSAGE_TYPE_NORMAL,
                Some(raw) => {
                    let parsed = raw
                        .strip_prefix("0x")
                        .or_else(|| raw.strip_prefix("0X"))
                        .map_or_else(
                            || raw.parse::<i128>(),
                            |hex| {
                                u128::from_str_radix(hex, 16)
                                    .map(|value| value.min(i128::MAX as u128) as i128)
                            },
                        );
                    parsed
                        .map(|value| {
                            if value < 0 {
                                u8::MAX
                            } else {
                                value.min(i128::from(u8::MAX)) as u8
                            }
                        })
                        .map_err(|_| ControlParseError::InvalidIntegerField {
                            field: "Type".to_string(),
                            value: raw.clone(),
                        })?
                }
            };
            let player = parse_int_field_or(&self.fields, "Player", -1)?;
            let to_player = if message_type == MESSAGE_TYPE_PRIVATE {
                parse_int_field_or(&self.fields, "ToPlayer", -1)?
            } else {
                -1
            };
            let message = self.fields.get("Message").cloned().unwrap_or_default();
            let message = LegacyCString::from_bytes(legacy_string_bytes(&message)).ok_or(
                ControlParseError::InteriorNulString {
                    field: "Message".to_string(),
                },
            )?;
            let by_client = parse_int_field_or(&self.fields, "ByClient", -1)?;
            return Ok(Some(ControlPacket::Message(MessageControlData {
                message_type,
                player,
                to_player,
                message,
                by_client,
            })));
        }

        Ok(Some(ControlPacket::Unknown {
            id: ControlPacketId::new(id),
            name: self.name,
            fields: self.fields,
        }))
    }
}

/// Compile one named `C4ClientPlayerInfos` body through the complete
/// `C4PlayerInfo` projection shared by replay `PlayerInfos.txt` and
/// `CID_PlrInfo` control packets.
fn parse_player_info_client_data(
    body: &[(String, String)],
    sections: &[(String, Vec<(String, String)>)],
    by_client: i32,
) -> Result<PlayerInfoControlData, ControlParseError> {
    let field = |fields: &[(String, String)], key: &str| -> Option<String> {
        fields
            .iter()
            .find(|(entry, _)| entry.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.clone())
    };
    let int = |fields: &[(String, String)], key: &str, default: i32| match field(fields, key) {
        None => Ok(default),
        Some(value) => value
            .parse::<i32>()
            .map_err(|_| ControlParseError::InvalidIntegerField {
                field: key.to_string(),
                value,
            }),
    };
    let uint = |fields: &[(String, String)], key: &str, default: u32| match field(fields, key) {
        None => Ok(default),
        Some(value) => value
            .parse::<u32>()
            .map_err(|_| ControlParseError::InvalidIntegerField {
                field: key.to_string(),
                value,
            }),
    };
    let string = |fields: &[(String, String)], key: &str| {
        LegacyCString::from_bytes(legacy_string_bytes(&field(fields, key).unwrap_or_default()))
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
            client_flags |=
                token
                    .parse::<u32>()
                    .map_err(|_| ControlParseError::InvalidIntegerField {
                        field: "Player Info.Flags".to_string(),
                        value: token.to_string(),
                    })?;
        } else if token.eq_ignore_ascii_case("AddPlayers") {
            client_flags |= CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS;
        } else if token.eq_ignore_ascii_case("Updated") {
            client_flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
        } else if token.eq_ignore_ascii_case("Initial") {
            client_flags |= CLIENT_PLAYER_INFO_FLAG_INITIAL;
        }
    }

    let players = sections
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
                        ("NoEliminationCheck", PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK),
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
                    Some(value) if value.eq_ignore_ascii_case("User") => PLAYER_INFO_TYPE_USER,
                    Some(value) if value.eq_ignore_ascii_case("Script") => PLAYER_INFO_TYPE_SCRIPT,
                    Some(value) => value
                        .parse::<u8>()
                        .map_err(|_| ControlParseError::InvalidPlayerType { value })?,
                };
                // C4PlayerInfo::CompileFunc strips the local-only impossible
                // combination after compiling the type.
                if player_type != PLAYER_INFO_TYPE_SCRIPT {
                    flags &= !PLAYER_INFO_FLAG_INVISIBLE;
                }
                let color = uint(fields, "Color", 0)?;
                let extra_data = field(fields, "ExtraData").unwrap_or_else(|| "NONE".to_string());
                let extra_data: [u8; 4] = extra_data.as_bytes().try_into().map_err(|_| {
                    ControlParseError::InvalidC4IdField {
                        field: "ExtraData".to_string(),
                        value: extra_data,
                    }
                })?;
                let resource = if flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
                    let resource_fields = sections
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
                    // Compilation materializes a C-string value; the legacy
                    // wire cannot preserve StdStrBuf's null/empty distinction.
                    league_progress_data_is_null: false,
                    league_progress_data: string(fields, "LeagueProgressData")?,
                    resource,
                })
            },
        )
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PlayerInfoControlData {
        client_id,
        flags: client_flags,
        players,
        by_client,
    })
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlParseError {
    #[error("control log did not start with [Control] section")]
    MissingControlSection,
    #[error("replay player-info data did not start with [PlayerInfoList] section")]
    MissingPlayerInfoListSection,
    #[error("unexpected section [{section}] inside control log")]
    UnexpectedSection { section: String },
    #[error("unexpected section [{section}] inside replay player-info data")]
    UnexpectedPlayerInfoSection { section: String },
    #[error("control log contained malformed line `{line}`")]
    MalformedLine { line: String },
    #[error("replay player-info data contained malformed line `{line}`")]
    MalformedPlayerInfoLine { line: String },
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
    #[error("replay player-info data contains {count} clients; C++ permits at most 5000")]
    TooManyPlayerInfoClients { count: usize },
    #[error(
        "replay player-info client {client_id} contains {count} players; C++ permits at most 5000"
    )]
    TooManyClientPlayerInfos { client_id: i32, count: usize },
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
    #[error("EMMoveObject object count {value} is outside the supported INI range")]
    InvalidEmMoveObjectCount { value: i32 },
    #[error(
        "EMMoveObject declared {declared} objects but its INI array contained {actual} entries"
    )]
    EmMoveObjectCountMismatch { declared: usize, actual: usize },
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
        let raw_value = value.trim();
        let mut value = unescape_value(raw_value);

        if let Some(packet) = current.as_mut() {
            // Unlike ordinary INI strings, StdCompiler's RCT_ID adaptor does
            // not strip quotes. Preserve that leading non-identifier byte so
            // CID_EMDropDef defaults a quoted ID to C4ID_None like C++.
            if packet.id == Some(0xb2)
                && key.eq_ignore_ascii_case("ID")
                && raw_value.starts_with('"')
            {
                value = raw_value.to_string();
            }
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

/// Placement of one control packet in the classic verbose INI compiler.
///
/// A control list (`RCT_Ctrl`) uses [`Self::IdPacketSection`], while a single
/// control packet (`RCT_CtrlPkt`) is compiled [`Self::Inline`] directly into
/// its parent record section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlIniPacketMode {
    IdPacketSection,
    Inline,
}

/// Failure while producing the classic `StdCompilerINIWrite` representation
/// of a typed control packet.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlIniEncodeError {
    #[error("control packet {id:#04x} ({name}) has no typed C++ INI serializer")]
    UnsupportedPacket { id: u8, name: String },
    #[error("loadable resource core has zero chunk size")]
    ZeroResourceChunkSize,
    #[error("player-info packet contains {count} players; C++ permits at most 5000")]
    TooManyPlayerInfos { count: usize },
    #[error("{field} contains {count} entries, which cannot fit the C++ signed count field")]
    CollectionTooLarge { field: &'static str, count: usize },
    #[error("player-info entry {index} has HasResource set but carries no resource core")]
    MissingPlayerInfoResource { index: usize },
}

/// Encode one packet as a top-level classic `[IDPacket]` INI section.
///
/// The returned bytes use CRLF, exact C++ field ordering/default omission,
/// byte-preserving escaped strings, and the indentation emitted by
/// `StdCompilerINIWrite`.
pub fn encode_control_packet_ini(packet: &ControlPacket) -> Result<Vec<u8>, ControlIniEncodeError> {
    let mut output = Vec::new();
    append_control_packet_ini(
        &mut output,
        packet,
        0,
        ControlIniPacketMode::IdPacketSection,
    )?;
    Ok(output)
}

/// Append one packet at an existing classic INI compiler depth.
///
/// `base_indent` is the indentation, in spaces, of the `[IDPacket]` section
/// and its `ID` field in section mode, or of the `ID` field in inline mode.
/// Thus record `RCT_Ctrl` chunks use `(2, IdPacketSection)` and `RCT_CtrlPkt`
/// chunks use `(0, Inline)`. Output is generated at the requested depth;
/// callers never need to re-indent escaped or `RCT_All` data after the fact.
pub fn append_control_packet_ini(
    output: &mut Vec<u8>,
    packet: &ControlPacket,
    base_indent: usize,
    mode: ControlIniPacketMode,
) -> Result<(), ControlIniEncodeError> {
    let (id, packet_name) = control_packet_ini_identity(packet)?;
    if mode == ControlIniPacketMode::IdPacketSection {
        append_ini_section_header(output, base_indent, "IDPacket");
    }
    append_ini_field_raw(output, base_indent, "ID", id.to_string().as_bytes());

    if let ControlPacket::DebugRecord(data) = packet {
        let mut value = data.data.len().to_string().into_bytes();
        value.extend_from_slice(b":");
        append_ini_escaped(&mut value, &data.data);
        append_ini_field_raw(output, base_indent, "Debug Rec", &value);
        return Ok(());
    }

    let body_indent = base_indent + 2;
    let body = encode_control_packet_ini_body(packet, body_indent)?;
    if !body.is_empty() {
        append_ini_section_header(output, body_indent, packet_name);
        output.extend_from_slice(&body.bytes);
    }
    Ok(())
}

fn control_packet_ini_identity(
    packet: &ControlPacket,
) -> Result<(u8, &'static str), ControlIniEncodeError> {
    Ok(match packet {
        ControlPacket::ClientJoin(_) => (0x80, "Client Join"),
        ControlPacket::ClientUpdate(_) => (0x81, "Client Update"),
        ControlPacket::ClientRemove(_) => (0x82, "Client Remove"),
        ControlPacket::Vote(_) => (0x83, "Voting"),
        ControlPacket::VoteEnd(_) => (0x84, "Voting End"),
        ControlPacket::SyncCheck(_) => (0x85, "Sync Check"),
        ControlPacket::Synchronize(_) => (0x86, "Synchronize"),
        ControlPacket::Set(_) => (0x87, "Set"),
        ControlPacket::Script(_) => (0x88, "Script"),
        ControlPacket::PlayerInfo(_) => (0x90, "Player Info"),
        ControlPacket::JoinPlayer(_) => (0x91, "Join Player"),
        ControlPacket::RemovePlayer(_) => (0x92, "Remove Player"),
        ControlPacket::PlayerSelect(_) => (0xa0, "Player Select"),
        ControlPacket::PlayerControl(_) => (0xa1, "Player Control"),
        ControlPacket::PlayerCommand(_) => (0xa2, "Player Command"),
        ControlPacket::Message(_) => (0xa3, "Message"),
        ControlPacket::EmMoveObject(_) => (0xb0, "EM Move Obj"),
        ControlPacket::EmDrawTool(_) => (0xb1, "EM Draw Tool"),
        ControlPacket::EmDropDef(_) => (0xb2, "EM Drop Def"),
        ControlPacket::DebugRecord(_) => (0xc0, "Debug Rec"),
        ControlPacket::MessageBoardAnswer(_) => (0xd0, "Message Board Answer"),
        ControlPacket::CustomCommand(_) => (0xd1, "Custom Command"),
        ControlPacket::InitScenarioPlayer(_) => (0xd2, "Init Scenario Player"),
        ControlPacket::ActivateGameGoalMenu(_) => (0xd3, "Activate Game Goal Menu"),
        ControlPacket::ToggleHostility(_) => (0xd4, "Toggle Hostility"),
        ControlPacket::SurrenderPlayer(_) => (0xd5, "Surrender Player"),
        ControlPacket::ActivateGameGoalRule(_) => (0xd6, "Activate Game Goal/Rule"),
        ControlPacket::SetPlayerTeam(_) => (0xd7, "Set Player Team"),
        ControlPacket::EliminatePlayer(_) => (0xd8, "Eliminate Player"),
        ControlPacket::Unknown { id, name, .. } => {
            return Err(ControlIniEncodeError::UnsupportedPacket {
                id: id.0,
                name: name
                    .clone()
                    .unwrap_or_else(|| "Unknown Packet Type".to_string()),
            });
        }
    })
}

#[derive(Default)]
struct ControlIniBody {
    indent: usize,
    bytes: Vec<u8>,
}

impl ControlIniBody {
    fn new(indent: usize) -> Self {
        Self {
            indent,
            bytes: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    fn raw(&mut self, name: &str, value: &[u8]) {
        append_ini_field_raw(&mut self.bytes, self.indent, name, value);
    }

    fn int<T: ToString + PartialEq>(&mut self, name: &str, value: T, default: T) {
        if value != default {
            self.raw(name, value.to_string().as_bytes());
        }
    }

    fn required_int<T: ToString>(&mut self, name: &str, value: T) {
        self.raw(name, value.to_string().as_bytes());
    }

    fn boolean(&mut self, name: &str, value: bool, default: bool) {
        if value != default {
            self.raw(name, if value { b"true" } else { b"false" });
        }
    }

    fn string(&mut self, name: &str, value: &[u8]) {
        if value.is_empty() {
            return;
        }
        let mut escaped = Vec::new();
        append_ini_escaped(&mut escaped, value);
        self.raw(name, &escaped);
    }

    fn std_buf(&mut self, name: &str, value: &[u8]) {
        let mut encoded = value.len().to_string().into_bytes();
        encoded.push(b':');
        append_ini_escaped(&mut encoded, value);
        self.raw(name, &encoded);
    }

    fn section(&mut self, name: &str, child: ControlIniBody) {
        if child.is_empty() {
            return;
        }
        // The enclosing packet section has already been emitted in the real
        // writer even when this is its first child, so every nested section
        // starts with the section separator CRLF.
        self.bytes.extend_from_slice(b"\r\n");
        self.bytes.resize(self.bytes.len() + self.indent + 2, b' ');
        self.bytes.push(b'[');
        self.bytes.extend_from_slice(name.as_bytes());
        self.bytes.extend_from_slice(b"]\r\n");
        self.bytes.extend_from_slice(&child.bytes);
    }
}

fn append_ini_section_header(output: &mut Vec<u8>, indent: usize, name: &str) {
    if !output.is_empty() {
        output.extend_from_slice(b"\r\n");
    }
    output.resize(output.len() + indent, b' ');
    output.push(b'[');
    output.extend_from_slice(name.as_bytes());
    output.extend_from_slice(b"]\r\n");
}

fn append_ini_field_raw(output: &mut Vec<u8>, indent: usize, name: &str, value: &[u8]) {
    output.resize(output.len() + indent, b' ');
    output.extend_from_slice(name.as_bytes());
    output.push(b'=');
    output.extend_from_slice(value);
    output.extend_from_slice(b"\r\n");
}

fn append_ini_escaped(output: &mut Vec<u8>, value: &[u8]) {
    output.push(b'"');
    let mut previous_was_numeric_escape = false;
    for &byte in value {
        let must_escape = !(0x20..=0x7e).contains(&byte)
            || matches!(byte, b'\\' | b'"')
            || (previous_was_numeric_escape && byte.is_ascii_digit());
        if !must_escape {
            output.push(byte);
            previous_was_numeric_escape = false;
            continue;
        }
        previous_was_numeric_escape = false;
        match byte {
            0x07 => output.extend_from_slice(b"\\a"),
            0x08 => output.extend_from_slice(b"\\b"),
            0x0c => output.extend_from_slice(b"\\f"),
            b'\n' => output.extend_from_slice(b"\\n"),
            b'\r' => output.extend_from_slice(b"\\r"),
            b'\t' => output.extend_from_slice(b"\\t"),
            0x0b => output.extend_from_slice(b"\\v"),
            b'"' => output.extend_from_slice(b"\\\""),
            b'\\' => output.extend_from_slice(b"\\\\"),
            other => {
                output.push(b'\\');
                output.extend_from_slice(format!("{other:o}").as_bytes());
                previous_was_numeric_escape = true;
            }
        }
    }
    output.push(b'"');
}

fn encode_control_packet_ini_body(
    packet: &ControlPacket,
    indent: usize,
) -> Result<ControlIniBody, ControlIniEncodeError> {
    let mut body = ControlIniBody::new(indent);
    match packet {
        ControlPacket::ClientJoin(data) => {
            let mut core = ControlIniBody::new(indent + 2);
            core.int("ID", data.core.client_id, -1);
            core.boolean("Activated", data.core.activated, false);
            core.boolean("Observer", data.core.observer, false);
            core.string("Name", data.core.name.as_bytes());
            core.string("Nick", data.core.nick.as_bytes());
            core.boolean("LobbyReady", data.core.lobby_ready, false);
            body.section("ClientCore", core);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::ClientUpdate(data) => {
            // Binary compilation casts CUT_None (-1) through uint8_t and
            // stores enum value 255. That no longer equals the signed enum
            // default, so C++ ReWriteText emits Type even for byte 255.
            body.required_int("Type", data.update_type);
            body.int("ClientID", data.client_id, -1);
            if data.update_type == CLIENT_UPDATE_ACTIVATE {
                body.int("Data", data.data, 0);
            }
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::ClientRemove(data) => {
            body.int("ClientID", data.client_id, -1);
            body.string("Reason", data.reason.as_bytes());
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::Set(data) => {
            body.int("Type", data.value_type, SET_VALUE_NONE);
            body.int("Data", data.data, 0);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::Vote(data) | ControlPacket::VoteEnd(data) => {
            encode_vote_ini_body(&mut body, data);
        }
        ControlPacket::Script(data) => {
            body.int("TargetObj", data.target_object, SCRIPT_SCOPE_GLOBAL);
            body.int(
                "Strict",
                data.strictness.raw(),
                ScriptStrictness::Strict3.raw(),
            );
            body.string("Script", data.script.as_bytes());
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::MessageBoardAnswer(data) => {
            body.int("Object", data.object, 0);
            body.string("Answer", data.answer.as_bytes());
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::CustomCommand(data) => {
            body.string("Command", data.command.as_bytes());
            body.string("Argument", data.argument.as_bytes());
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::PlayerSelect(data) => {
            body.int("Player", data.player, -1);
            let count = i32::try_from(data.objects.len()).map_err(|_| {
                ControlIniEncodeError::CollectionTooLarge {
                    field: "PlayerSelect.Objects",
                    count: data.objects.len(),
                }
            })?;
            body.int("ObjCnt", count, 0);
            if data.objects.iter().any(|&object| object != 0) {
                body.raw("Objs", &join_ini_i32(&data.objects));
            }
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::PlayerControl(data) => {
            body.int("Player", data.player, -1);
            body.int("Com", data.command, 0);
            body.int("Data", data.data, 0);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::PlayerCommand(data) => {
            body.int("Player", data.player, -1);
            body.int("Cmd", data.command, 0);
            body.int("X", data.x, 0);
            body.int("Y", data.y, 0);
            body.int("Target", data.target, 0);
            body.int("Target2", data.target2, 0);
            body.int("Data", data.data, 0);
            body.int("AddMode", data.add_mode, 0);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::Message(data) => {
            body.int("Type", data.message_type, MESSAGE_TYPE_NORMAL);
            body.int("Player", data.player, -1);
            if data.message_type == MESSAGE_TYPE_PRIVATE {
                body.int("ToPlayer", data.to_player, -1);
            }
            body.string("Message", data.message.as_bytes());
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::EmMoveObject(data) => {
            body.required_int("Action", data.action);
            body.int("tx", data.tx, 0);
            body.int("ty", data.ty, 0);
            body.int("TargetObj", data.target_object, -1);
            let count = i32::try_from(data.objects.len()).map_err(|_| {
                ControlIniEncodeError::CollectionTooLarge {
                    field: "EMMoveObject.Objects",
                    count: data.objects.len(),
                }
            })?;
            body.int("ObjectNum", count, 0);
            body.int(
                "Strict",
                data.strictness.raw(),
                ScriptStrictness::Strict3.raw(),
            );
            if data.objects.iter().any(|&object| object != -1) {
                body.raw("Objs", &join_ini_i32(&data.objects));
            }
            if data.action == EMMO_SCRIPT {
                body.string("Script", data.script.as_bytes());
            }
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::EmDrawTool(data) => {
            body.required_int("Action", data.action);
            body.int("Mode", data.mode, 0);
            body.int("X", data.x, 0);
            body.int("Y", data.y, 0);
            body.int("X2", data.x2, 0);
            body.int("Y2", data.y2, 0);
            body.int("Grade", data.grade, 0);
            body.boolean("IFT", data.ift, false);
            body.string("Material", data.material.as_bytes());
            body.string("Texture", data.texture.as_bytes());
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::EmDropDef(data) => {
            if c4_id_numeric(&data.id) != 0 {
                body.raw("ID", &c4_id_text(&data.id));
            }
            body.int("X", data.x, 0);
            body.int("Y", data.y, 0);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::InitScenarioPlayer(data) => {
            body.int("Team", data.team, 0);
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::ActivateGameGoalMenu(data) => {
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::ToggleHostility(data) => {
            body.int("Opponent", data.opponent, -1);
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::SurrenderPlayer(data) => {
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::ActivateGameGoalRule(data) => {
            body.int("Object", data.object, 0);
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::SetPlayerTeam(data) => {
            body.int("Team", data.team, 0);
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::EliminatePlayer(data) => {
            body.int("Plr", data.player, -1);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::SyncCheck(data) => {
            body.int("Frame", data.frame, -1);
            body.int("ControlTick", data.control_tick, 0);
            body.int("Random3", data.random3, 0);
            body.int("RandomCount", data.random_count, 0);
            body.int("AllCrewPosX", data.crew_positions_sum, 0);
            body.int("PXSCount", data.pxs_count, 0);
            body.int("MassMoverIndex", data.mass_mover_index, 0);
            body.int("ObjectCount", data.object_count, 0);
            body.int("ObjectEnumerationIndex", data.object_enumeration_index, 0);
            body.int("SectShapeSum", data.sector_shape_sum, 0);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::Synchronize(data) => {
            body.boolean("SavePlrs", data.save_player_files, false);
            body.boolean("SyncClear", data.sync_clearance, false);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::JoinPlayer(data) => {
            body.string(
                "Filename",
                &classic_network_filename(data.filename.as_bytes()),
            );
            body.int("AtClient", data.at_client, -1);
            body.int("InfoID", data.info_id, -1);
            match &data.source {
                JoinPlayerSource::Embedded(player_data) => {
                    body.std_buf("PlrData", player_data);
                }
                JoinPlayerSource::Resource(resource) => {
                    body.boolean("ByRes", true, false);
                    let resource = encode_network_resource_ini(resource, indent + 2)?;
                    body.section("ResCore", resource);
                }
            }
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::RemovePlayer(data) => {
            body.int("Plr", data.player, -1);
            body.boolean("Disconnected", data.disconnected, false);
            body.int("ByClient", data.by_client, -1);
        }
        ControlPacket::PlayerInfo(data) => {
            encode_player_info_ini_body(&mut body, data)?;
        }
        ControlPacket::DebugRecord(_) => unreachable!("handled before body encoding"),
        ControlPacket::Unknown { id, name, .. } => {
            return Err(ControlIniEncodeError::UnsupportedPacket {
                id: id.0,
                name: name
                    .clone()
                    .unwrap_or_else(|| "Unknown Packet Type".to_string()),
            });
        }
    }
    Ok(body)
}

fn encode_vote_ini_body(body: &mut ControlIniBody, data: &VoteControlData) {
    // Like ClientUpdate, VT_None (-1) becomes enum value 255 after the
    // uint8_t binary adaptor and is therefore not omitted by the INI writer.
    body.required_int("Type", data.vote_type);
    body.boolean("Approve", data.approve, true);
    body.int("Data", data.data, 0);
    body.int("ByClient", data.by_client, -1);
}

fn encode_player_info_ini_body(
    body: &mut ControlIniBody,
    data: &PlayerInfoControlData,
) -> Result<(), ControlIniEncodeError> {
    if data.players.len() > 5_000 {
        return Err(ControlIniEncodeError::TooManyPlayerInfos {
            count: data.players.len(),
        });
    }
    body.int("ID", data.client_id, -1);
    if data.flags != 0 {
        body.raw(
            "Flags",
            &encode_ini_bitfield(
                data.flags,
                &[
                    ("AddPlayers", CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS),
                    ("Updated", CLIENT_PLAYER_INFO_FLAG_UPDATED),
                    ("Initial", CLIENT_PLAYER_INFO_FLAG_INITIAL),
                ],
            ),
        );
    }
    for (index, player) in data.players.iter().enumerate() {
        let player_body = encode_player_info_entry_ini(player, body.indent + 2, index)?;
        body.section("Player", player_body);
    }
    body.int("ByClient", data.by_client, -1);
    Ok(())
}

fn encode_player_info_entry_ini(
    player: &ControlPlayerInfoEntry,
    indent: usize,
    index: usize,
) -> Result<ControlIniBody, ControlIniEncodeError> {
    const SYNC_FLAGS: u16 = PLAYER_INFO_FLAG_JOINED
        | PLAYER_INFO_FLAG_REMOVED
        | PLAYER_INFO_FLAG_HAS_RESOURCE
        | PLAYER_INFO_FLAG_IN_SCENARIO_FILE
        | PLAYER_INFO_FLAG_SAVEGAME_JOIN
        | PLAYER_INFO_FLAG_DISCONNECTED
        | PLAYER_INFO_FLAG_WON
        | PLAYER_INFO_FLAG_VOTED_OUT
        | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
        | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
        | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
        | PLAYER_INFO_FLAG_INVISIBLE;

    let mut body = ControlIniBody::new(indent);
    body.string("Name", player.name.as_bytes());
    body.string("ForcedName", player.forced_name.as_bytes());
    body.string("Filename", player.filename.as_bytes());

    let flags = player.flags & SYNC_FLAGS;
    if flags != 0 {
        body.raw(
            "Flags",
            &encode_ini_bitfield(
                flags,
                &[
                    ("Joined", PLAYER_INFO_FLAG_JOINED),
                    ("Removed", PLAYER_INFO_FLAG_REMOVED),
                    ("HasResource", PLAYER_INFO_FLAG_HAS_RESOURCE),
                    ("JoinIssued", PLAYER_INFO_FLAG_JOIN_ISSUED),
                    ("SavegameJoin", PLAYER_INFO_FLAG_SAVEGAME_JOIN),
                    ("Disconnected", PLAYER_INFO_FLAG_DISCONNECTED),
                    ("VotedOut", PLAYER_INFO_FLAG_VOTED_OUT),
                    ("Won", PLAYER_INFO_FLAG_WON),
                    ("AttributesFixed", PLAYER_INFO_FLAG_ATTRIBUTES_FIXED),
                    ("NoScenarioInit", PLAYER_INFO_FLAG_NO_SCENARIO_INIT),
                    ("NoEliminationCheck", PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK),
                    ("Invisible", PLAYER_INFO_FLAG_INVISIBLE),
                ],
            ),
        );
    }
    body.int("ID", player.id, 0);
    match player.player_type {
        PLAYER_INFO_TYPE_USER => {}
        PLAYER_INFO_TYPE_SCRIPT => body.raw("Type", b"Script"),
        other => body.raw("Type", other.to_string().as_bytes()),
    }
    body.int("Color", player.color, 0);
    body.int("OriginalColor", player.original_color, player.color);
    body.int("SavgamePlayer", player.savegame_player, 0);
    body.int("Team", player.team, 0);
    body.string("AUID", player.auth_id.as_bytes());
    if flags & PLAYER_INFO_FLAG_JOINED != 0 {
        body.int("GameNumber", player.game_number, -1);
        body.int("GameJoinFrame", player.game_join_frame, -1);
    }
    if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
        body.int("GamePartFrame", player.game_part_frame, -1);
    }
    if c4_id_numeric(&player.extra_data) != 0 {
        body.raw("ExtraData", &c4_id_text(&player.extra_data));
    }
    body.string("LeagueAccount", player.league_account.as_bytes());
    body.int("LeagueScore", player.league_score, 0);
    body.int("LeagueRank", player.league_rank, 0);
    body.int("LeagueRankSymbol", player.league_rank_symbol, 0);
    body.int("ProjectedGain", player.league_projected_gain, -1);
    if !player.clan_tag.is_empty() {
        body.raw("ClanTag", player.clan_tag.as_bytes());
    }
    body.int("LeaguePerformance", player.league_performance, 0);
    body.string("LeagueProgressData", player.league_progress_data.as_bytes());

    if flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
        let resource = player
            .resource
            .as_ref()
            .ok_or(ControlIniEncodeError::MissingPlayerInfoResource { index })?;
        let resource_body = encode_network_resource_ini(resource, indent + 2)?;
        body.section("ResCore", resource_body);
    }
    Ok(body)
}

fn encode_network_resource_ini(
    resource: &NetworkResourceCore,
    indent: usize,
) -> Result<ControlIniBody, ControlIniEncodeError> {
    if resource.loadable && resource.chunk_size == 0 {
        return Err(ControlIniEncodeError::ZeroResourceChunkSize);
    }
    let mut body = ControlIniBody::new(indent);
    match resource.resource_type {
        NETWORK_RESOURCE_TYPE_NULL => {}
        1 => body.raw("Type", b"Scenario"),
        2 => body.raw("Type", b"Dynamic"),
        3 => body.raw("Type", b"Player"),
        4 => body.raw("Type", b"Definitions"),
        5 => body.raw("Type", b"System"),
        6 => body.raw("Type", b"Material"),
        other => body.raw("Type", other.to_string().as_bytes()),
    }
    body.int("ID", resource.id, -1);
    body.int("DerID", resource.derived_id, -1);
    body.boolean("Loadable", resource.loadable, true);
    if resource.loadable {
        body.int("FileSize", resource.file_size, 0);
        body.int("FileCRC", resource.file_crc, 0);
        body.int(
            "ChunkSize",
            resource.chunk_size,
            NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE,
        );
    }
    body.int("ContentsCRC", resource.contents_crc, 0);
    if let Some(file_sha) = resource.file_sha {
        let mut hex = Vec::with_capacity(40);
        for byte in file_sha {
            hex.extend_from_slice(format!("{byte:02x}").as_bytes());
        }
        body.raw("FileSHA", &hex);
    }
    body.string(
        "Filename",
        &classic_network_filename(resource.filename.as_bytes()),
    );
    body.string(
        "Author",
        &classic_network_filename(resource.author.as_bytes()),
    );
    Ok(body)
}

fn encode_ini_bitfield<T>(mut value: T, entries: &[(&str, T)]) -> Vec<u8>
where
    T: Copy
        + PartialEq
        + std::ops::BitAnd<Output = T>
        + std::ops::BitAndAssign
        + std::ops::Not<Output = T>
        + ToString
        + From<u8>,
{
    let zero = T::from(0);
    let mut output = Vec::new();
    for &(name, bit) in entries {
        if bit & value == bit {
            if !output.is_empty() {
                output.push(b'|');
            }
            output.extend_from_slice(name.as_bytes());
            value &= !bit;
        }
    }
    if value != zero {
        if !output.is_empty() {
            output.push(b'|');
        }
        output.extend_from_slice(value.to_string().as_bytes());
    }
    output
}

fn join_ini_i32(values: &[i32]) -> Vec<u8> {
    let mut output = Vec::new();
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(b',');
        }
        output.extend_from_slice(value.to_string().as_bytes());
    }
    output
}

fn classic_network_filename(value: &[u8]) -> Vec<u8> {
    #[cfg(windows)]
    {
        value.to_vec()
    }
    #[cfg(not(windows))]
    {
        value
            .iter()
            .map(|&byte| if byte == b'/' { b'\\' } else { byte })
            .collect()
    }
}

fn c4_id_numeric(value: &[u8; 4]) -> u32 {
    if value == b"NONE" || value == b"0000" {
        return 0;
    }
    if value.iter().all(u8::is_ascii_digit) {
        return value
            .iter()
            .fold(0, |number, digit| number * 10 + u32::from(*digit - b'0'));
    }
    u32::from_ne_bytes(*value)
}

fn c4_id_text(value: &[u8; 4]) -> Vec<u8> {
    let numeric = c4_id_numeric(value);
    if numeric == 0 {
        b"NONE".to_vec()
    } else if numeric <= 9_999 {
        format!("{numeric:04}").into_bytes()
    } else {
        value
            .iter()
            .copied()
            .take_while(|&byte| byte != 0)
            .collect()
    }
}

/// Complete named `C4PlayerInfoList` stored in a replay's `PlayerInfos.txt`.
///
/// `last_player_id` is the exact persisted `C4PlayerInfoList::iLastPlayerID`;
/// it is deliberately not repaired from the player rows while decoding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayPlayerInfosDocument {
    pub last_player_id: i32,
    pub clients: Vec<PlayerInfoControlData>,
}

/// Parse the complete named `C4PlayerInfoList` stored in a replay's
/// `PlayerInfos.txt` into its exact counter and ordered per-client packets.
///
/// `PlayerInfos.txt` has no control-packet author field. The returned snapshot
/// packets use the authoritative host ID (`ByClient=0`) so callers can feed
/// them through the same replay projection as an initial `CID_PlrInfo` list.
/// All `C4ClientPlayerInfos`, `C4PlayerInfo`, and optional `C4Network2ResCore`
/// fields are decoded by the same helper used for live control packets. The
/// file is accepted as bytes because C++ INI strings use the legacy eight-bit
/// character domain rather than UTF-8.
pub fn parse_replay_player_infos_ini(
    input: &[u8],
) -> Result<ReplayPlayerInfosDocument, ControlParseError> {
    const C4_MAX_CLIENT: usize = 5_000;
    const C4_MAX_PLAYER: usize = 5_000;

    #[derive(Default)]
    struct RawClient {
        body: Vec<(String, String)>,
        sections: Vec<(String, Vec<(String, String)>)>,
    }

    let mut saw_root = false;
    let mut last_player_id = 0;
    let mut current_client = None::<RawClient>;
    let mut clients = Vec::<RawClient>::new();
    // Preserve the C++ compiler's byte-oriented string domain while reusing
    // the existing textual INI machinery: every source byte becomes the
    // same-valued Unicode scalar and `legacy_string_bytes` reverses this
    // mapping exactly. In particular, this never performs lossy UTF-8
    // decoding.
    let input = input.iter().copied().map(char::from).collect::<String>();

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
            if !saw_root {
                if !section.eq_ignore_ascii_case("PlayerInfoList") {
                    return Err(ControlParseError::MissingPlayerInfoListSection);
                }
                saw_root = true;
                continue;
            }

            if section.eq_ignore_ascii_case("Client") {
                if let Some(client) = current_client.take() {
                    clients.push(client);
                }
                if clients.len() >= C4_MAX_CLIENT {
                    return Err(ControlParseError::TooManyPlayerInfoClients {
                        count: clients.len() + 1,
                    });
                }
                current_client = Some(RawClient::default());
                continue;
            }

            let Some(client) = current_client.as_mut() else {
                return Err(ControlParseError::UnexpectedPlayerInfoSection {
                    section: section.to_string(),
                });
            };
            if section.eq_ignore_ascii_case("Player") {
                let count = client
                    .sections
                    .iter()
                    .filter(|(name, _)| name.eq_ignore_ascii_case("Player"))
                    .count();
                if count >= C4_MAX_PLAYER {
                    let client_id = client
                        .body
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case("ID"))
                        .and_then(|(_, value)| value.parse::<i32>().ok())
                        .unwrap_or(-1);
                    return Err(ControlParseError::TooManyClientPlayerInfos {
                        client_id,
                        count: count + 1,
                    });
                }
                client.sections.push((section.to_string(), Vec::new()));
                continue;
            }
            if section.eq_ignore_ascii_case("ResCore")
                && client
                    .sections
                    .last()
                    .is_some_and(|(name, _)| name.eq_ignore_ascii_case("Player"))
            {
                client.sections.push((section.to_string(), Vec::new()));
                continue;
            }
            return Err(ControlParseError::UnexpectedPlayerInfoSection {
                section: section.to_string(),
            });
        }

        let Some((key, raw_value)) = trimmed.split_once('=') else {
            return Err(ControlParseError::MalformedPlayerInfoLine {
                line: line.to_string(),
            });
        };
        if !saw_root {
            return Err(ControlParseError::MissingPlayerInfoListSection);
        }
        let key = key.trim().to_string();
        let value = unescape_value(raw_value.trim());
        if let Some(client) = current_client.as_mut() {
            if let Some((_, fields)) = client.sections.last_mut() {
                fields.push((key, value));
            } else {
                client.body.push((key, value));
            }
        } else if key.eq_ignore_ascii_case("LastPlayerID") {
            last_player_id =
                value
                    .parse::<i32>()
                    .map_err(|_| ControlParseError::InvalidIntegerField {
                        field: "LastPlayerID".to_string(),
                        value,
                    })?;
        }
    }

    if !saw_root {
        return Err(ControlParseError::MissingPlayerInfoListSection);
    }
    if let Some(client) = current_client {
        clients.push(client);
    }

    let clients = clients
        .iter()
        .map(|client| parse_player_info_client_data(&client.body, &client.sections, 0))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReplayPlayerInfosDocument {
        last_player_id,
        clients,
    })
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

fn parse_u8_field_or(
    fields: &HashMap<String, String>,
    name: &str,
    default: u8,
) -> Result<u8, ControlParseError> {
    match fields.get(name) {
        None => Ok(default),
        Some(raw) => parse_cpp_ini_u8(raw).ok_or_else(|| ControlParseError::InvalidIntegerField {
            field: name.to_string(),
            value: raw.clone(),
        }),
    }
}

/// `StdCompilerINIRead::Byte(uint8_t&)` reads through `strtoul` and clamps
/// into 0..=255. In particular, negative decimal input converts to a large
/// unsigned value and therefore becomes 255 instead of being rejected.
fn parse_cpp_ini_u8(raw: &str) -> Option<u8> {
    let raw = raw.trim();
    let (negative, unsigned) = match raw.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, raw.strip_prefix('+').unwrap_or(raw)),
    };
    let (radix, digits) = if !negative {
        unsigned
            .strip_prefix("0x")
            .or_else(|| unsigned.strip_prefix("0X"))
            .map_or((10, unsigned), |digits| (16, digits))
    } else {
        (10, unsigned)
    };
    if digits.is_empty()
        || !digits.bytes().all(|byte| {
            if radix == 16 {
                byte.is_ascii_hexdigit()
            } else {
                byte.is_ascii_digit()
            }
        })
    {
        return None;
    }
    let magnitude = u128::from_str_radix(digits, radix).unwrap_or(u128::MAX);
    if negative && magnitude != 0 {
        Some(u8::MAX)
    } else {
        Some(magnitude.min(u128::from(u8::MAX)) as u8)
    }
}

fn parse_legacy_string_field_or(
    fields: &HashMap<String, String>,
    name: &str,
) -> Result<LegacyCString, ControlParseError> {
    let value = fields.get(name).map(String::as_str).unwrap_or_default();
    LegacyCString::from_bytes(legacy_string_bytes(value)).ok_or_else(|| {
        ControlParseError::InteriorNulString {
            field: name.to_string(),
        }
    })
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
        Some(value) => value
            .parse::<i32>()
            .map_err(|_| ControlParseError::InvalidIntegerField {
                field: format!("ResCore.{name}"),
                value: value.to_string(),
            }),
    };
    let uint = |name: &str, default: u32| match field(name) {
        None => Ok(default),
        Some(value) => value
            .parse::<u32>()
            .map_err(|_| ControlParseError::InvalidIntegerField {
                field: format!("ResCore.{name}"),
                value: value.to_string(),
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
        Some(value) => value
            .parse::<u8>()
            .map_err(|_| ControlParseError::InvalidResourceType {
                value: value.to_string(),
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
                let pair = value
                    .as_bytes()
                    .get(offset..offset + 2)
                    .and_then(|pair| std::str::from_utf8(pair).ok())
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
        file_size: if loadable {
            uint("FileSize", 0)?
        } else {
            u32::MAX
        },
        file_crc: if loadable {
            uint("FileCRC", 0)?
        } else {
            u32::MAX
        },
        chunk_size,
        contents_crc: uint("ContentsCRC", 0)?,
        file_sha,
        filename: string("Filename")?,
        author: string("Author")?,
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
    fn parses_every_previously_unprojected_known_control_family() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=128\n\
    [Client Join]\n\
      [ClientCore]\n\
        ID=7\n\
        Activated=true\n\
        Observer=false\n\
        Name=Alice\n\
        Nick=Ally\n\
        LobbyReady=true\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=129\n\
    [Client Update]\n\
      Type=0\n\
      ClientID=7\n\
      Data=1\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=130\n\
    [Client Remove]\n\
      ClientID=8\n\
      Reason=gone\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=131\n\
    [Voting]\n\
      Type=1\n\
      Approve=false\n\
      Data=8\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=132\n\
    [Voting End]\n\
      Type=2\n\
      Data=1\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=135\n\
    [Set]\n\
      Type=5\n\
      Data=1234\n\
      ByClient=0\n\
  [IDPacket]\n\
    ID=192\n\
    Debug Rec=5:\"\\000\\377@C4\"\n\
  [IDPacket]\n\
    ID=210\n\
    [Init Scenario Player]\n\
      Team=4\n\
      Plr=2\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=213\n\
    [Surrender Player]\n\
      Plr=2\n\
      ByClient=7\n";

        assert_eq!(
            parse_control_ini(input).expect("all known controls parse"),
            vec![
                ControlPacket::ClientJoin(ClientJoinControlData {
                    core: ClientCoreControlData {
                        client_id: 7,
                        activated: true,
                        observer: false,
                        name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                        nick: LegacyCString::from_bytes(b"Ally".to_vec()).unwrap(),
                        lobby_ready: true,
                    },
                    by_client: 0,
                }),
                ControlPacket::ClientUpdate(ClientUpdateControlData {
                    update_type: CLIENT_UPDATE_ACTIVATE,
                    client_id: 7,
                    data: 1,
                    by_client: 0,
                }),
                ControlPacket::ClientRemove(ClientRemoveControlData {
                    client_id: 8,
                    reason: LegacyCString::from_bytes(b"gone".to_vec()).unwrap(),
                    by_client: 0,
                }),
                ControlPacket::Vote(VoteControlData {
                    vote_type: VOTE_TYPE_KICK,
                    approve: false,
                    data: 8,
                    by_client: 7,
                }),
                ControlPacket::VoteEnd(VoteControlData {
                    vote_type: VOTE_TYPE_PAUSE,
                    approve: true,
                    data: 1,
                    by_client: 0,
                }),
                ControlPacket::Set(SetControlData {
                    value_type: SET_VALUE_FAIR_CREW,
                    data: 1234,
                    by_client: 0,
                }),
                ControlPacket::DebugRecord(DebugRecordControlData {
                    data: vec![0x00, 0xff, b'@', b'C', b'4'],
                }),
                ControlPacket::InitScenarioPlayer(InitScenarioPlayerControlData {
                    team: 4,
                    player: 2,
                    by_client: 7,
                }),
                ControlPacket::SurrenderPlayer(SurrenderPlayerControlData {
                    player: 2,
                    by_client: 7,
                }),
            ]
        );
    }

    #[test]
    fn newly_projected_control_defaults_match_cpp_compile_defaults() {
        let input = "[Control]\n\
[IDPacket]\nID=129\n[Client Update]\n\
[IDPacket]\nID=131\n[Voting]\n\
[IDPacket]\nID=135\n[Set]\n\
[IDPacket]\nID=210\n[Init Scenario Player]\n\
[IDPacket]\nID=213\n[Surrender Player]\n";
        assert_eq!(
            parse_control_ini(input).expect("omitted defaults parse"),
            vec![
                ControlPacket::ClientUpdate(ClientUpdateControlData {
                    update_type: u8::MAX,
                    client_id: -1,
                    data: 0,
                    by_client: -1,
                }),
                ControlPacket::Vote(VoteControlData {
                    vote_type: VOTE_TYPE_NONE,
                    approve: true,
                    data: 0,
                    by_client: -1,
                }),
                ControlPacket::Set(SetControlData::default()),
                ControlPacket::InitScenarioPlayer(InitScenarioPlayerControlData::default()),
                ControlPacket::SurrenderPlayer(SurrenderPlayerControlData {
                    player: -1,
                    by_client: -1,
                }),
            ]
        );
    }

    #[test]
    fn unsigned_byte_control_fields_follow_cpp_clamping() {
        let packets = parse_control_ini(
            "[Control]\n\
             [IDPacket]\nID=129\n[Client Update]\nType=-1\n\
             [IDPacket]\nID=131\n[Voting]\nType=999\n\
             [IDPacket]\nID=132\n[Voting End]\nType=0x01\n",
        )
        .expect("C++ byte spellings parse");

        assert!(matches!(
            &packets[0],
            ControlPacket::ClientUpdate(ClientUpdateControlData {
                update_type: u8::MAX,
                ..
            })
        ));
        assert!(matches!(
            &packets[1],
            ControlPacket::Vote(VoteControlData {
                vote_type: u8::MAX,
                ..
            })
        ));
        assert!(matches!(
            &packets[2],
            ControlPacket::VoteEnd(VoteControlData {
                vote_type: VOTE_TYPE_KICK,
                ..
            })
        ));
    }

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
    fn sync_check_control_tick_mismatch_is_exempt_only_during_replay() {
        // C4ControlSyncCheck::Execute excludes ControlTick from the mismatch
        // condition only for replay control (src/C4Control.cpp:483-492).
        let local = SyncCheckPacket {
            frame: 100,
            control_tick: 41,
            random3: 2,
            random_count: 3,
            crew_positions_sum: 4,
            pxs_count: 5,
            mass_mover_index: 6,
            object_count: 7,
            object_enumeration_index: 8,
            sector_shape_sum: 9,
            by_client: 1,
        };
        let mut remote = local.clone();
        remote.control_tick = 42;

        assert!(!local.matches(&remote), "live control compares ticks");
        assert!(local.matches_replay(&remote), "replay ignores tick drift");

        remote.random_count += 1;
        assert!(
            !local.matches_replay(&remote),
            "replay still compares synchronized simulation fields"
        );
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
    fn parses_em_move_object_fields_conditional_script_and_cpp_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=176\n\
    [EM Move Obj]\n\
      Action=3\n\
      tx=-25\n\
      ty=40\n\
      TargetObj=91\n\
      ObjectNum=3\n\
      Strict=2\n\
      Objs=11,-2,0\n\
      Script=\"SetX(GetX()+1);\"\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=176\n\
    [EM Move Obj]\n\
      Action=0\n\
      ObjectNum=3\n\
      Script=\"not compiled for this action\"\n\
  [IDPacket]\n\
    ID=176\n\
    [EM Move Obj]\n\
      Action=1\n\
      ObjectNum=3\n\
      Objs=11\n\
  [IDPacket]\n\
    ID=176\n\
    [EM Move Obj]\n\
      Action=255\n";

        assert_eq!(
            parse_control_ini(input).expect("parse editor object controls"),
            vec![
                ControlPacket::EmMoveObject(EmMoveObjectControlData {
                    action: EMMO_SCRIPT,
                    tx: -25,
                    ty: 40,
                    target_object: 91,
                    objects: vec![11, -2, 0],
                    strictness: ScriptStrictness::Strict2,
                    script: LegacyCString::from_bytes(b"SetX(GetX()+1);".to_vec())
                        .expect("fixture is NUL-free"),
                    by_client: 7,
                }),
                ControlPacket::EmMoveObject(EmMoveObjectControlData {
                    action: EMMO_MOVE,
                    objects: vec![-1, -1, -1],
                    ..EmMoveObjectControlData::default()
                }),
                ControlPacket::EmMoveObject(EmMoveObjectControlData {
                    action: EMMO_ENTER,
                    objects: vec![-1, -1, -1],
                    ..EmMoveObjectControlData::default()
                }),
                ControlPacket::EmMoveObject(EmMoveObjectControlData {
                    action: u8::MAX,
                    ..EmMoveObjectControlData::default()
                }),
            ]
        );
    }

    #[test]
    fn em_move_object_ini_preserves_legacy_script_bytes() {
        let input =
            "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=3\nScript=\"\\200\\377\"\n";
        let packets = parse_control_ini(input).expect("parse editor script bytes");
        let ControlPacket::EmMoveObject(control) = &packets[0] else {
            panic!("expected EmMoveObject, got {:?}", packets[0]);
        };
        assert_eq!(control.script.as_bytes(), &[0x80, 0xff]);
    }

    #[test]
    fn rejects_invalid_em_move_object_ini_action_count_and_strictness() {
        for (input, expected) in [
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\n",
                ControlParseError::MissingField {
                    field: "Action".to_string(),
                },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=256\n",
                ControlParseError::InvalidIntegerField {
                    field: "Action".to_string(),
                    value: "256".to_string(),
                },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=0\nObjectNum=-1\nStrict=4\n",
                ControlParseError::InvalidScriptStrictness { value: 4 },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=0\nObjectNum=-1\n",
                ControlParseError::InvalidEmMoveObjectCount { value: -1 },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=0\nObjectNum=2147483647\n",
                ControlParseError::InvalidEmMoveObjectCount { value: i32::MAX },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=0\nObjectNum=1\nObjs=11,12\n",
                ControlParseError::EmMoveObjectCountMismatch {
                    declared: 1,
                    actual: 2,
                },
            ),
            (
                "[Control]\n[IDPacket]\nID=176\n[EM Move Obj]\nAction=0\nStrict=4\n",
                ControlParseError::InvalidScriptStrictness { value: 4 },
            ),
        ] {
            assert_eq!(parse_control_ini(input).unwrap_err(), expected);
        }
    }

    #[test]
    fn parses_em_draw_tool_fields_unknown_action_and_cpp_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=177\n\
    [EM Draw Tool]\n\
      Action=4\n\
      Mode=3\n\
      X=-25\n\
      Y=40\n\
      X2=91\n\
      Y2=-7\n\
      Grade=12\n\
      IFT=true\n\
      Material=Earth\n\
      Texture=Rough\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=177\n\
    [EM Draw Tool]\n\
      Action=255\n";

        assert_eq!(
            parse_control_ini(input).expect("parse editor landscape controls"),
            vec![
                ControlPacket::EmDrawTool(EmDrawToolControlData {
                    action: EMDT_RECT,
                    mode: 3,
                    x: -25,
                    y: 40,
                    x2: 91,
                    y2: -7,
                    grade: 12,
                    ift: true,
                    material: LegacyCString::from_bytes(b"Earth".to_vec()).unwrap(),
                    texture: LegacyCString::from_bytes(b"Rough".to_vec()).unwrap(),
                    by_client: 7,
                }),
                ControlPacket::EmDrawTool(EmDrawToolControlData {
                    action: u8::MAX,
                    ..EmDrawToolControlData::default()
                }),
            ]
        );
    }

    #[test]
    fn em_draw_tool_ini_requires_a_raw_byte_action() {
        for (input, expected) in [
            (
                "[Control]\n[IDPacket]\nID=177\n[EM Draw Tool]\n",
                ControlParseError::MissingField {
                    field: "Action".to_string(),
                },
            ),
            (
                "[Control]\n[IDPacket]\nID=177\n[EM Draw Tool]\nAction=256\n",
                ControlParseError::InvalidIntegerField {
                    field: "Action".to_string(),
                    value: "256".to_string(),
                },
            ),
        ] {
            assert_eq!(parse_control_ini(input).unwrap_err(), expected);
        }
    }

    #[test]
    fn parses_em_drop_def_fields_and_cpp_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=178\n\
    [EM Drop Def]\n\
      ID=HUT2\n\
      X=-130\n\
      Y=130\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=178\n\
    [EM Drop Def]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse editor definition drops"),
            vec![
                ControlPacket::EmDropDef(EmDropDefControlData {
                    id: *b"HUT2",
                    x: -130,
                    y: 130,
                    by_client: 7,
                }),
                ControlPacket::EmDropDef(EmDropDefControlData::default()),
            ]
        );
    }

    #[test]
    fn em_drop_def_ini_truncates_long_ids_and_maps_short_ids_to_none() {
        let input = "\
[Control]\n\
[IDPacket]\n\
ID=178\n\
[EM Drop Def]\n\
ID=TOO-LONG\n\
[IDPacket]\n\
ID=178\n\
[EM Drop Def]\n\
ID=ABC!ignored\n\
[IDPacket]\n\
ID=178\n\
[EM Drop Def]\n\
ID=\"HUT2\"\n";
        assert_eq!(
            parse_control_ini(input).expect("parse C4ID adaptor boundaries"),
            vec![
                ControlPacket::EmDropDef(EmDropDefControlData {
                    id: *b"TOO-",
                    ..EmDropDefControlData::default()
                }),
                ControlPacket::EmDropDef(EmDropDefControlData::default()),
                ControlPacket::EmDropDef(EmDropDefControlData::default()),
            ]
        );
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
        let input = "[Control]\n[IDPacket]\nID=136\n[Script]\nScript=\"\\200\\377\\a\\v\"\n";
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
    fn parses_custom_command_packet_and_omitted_defaults() {
        // C4ControlCustomCommand::CompileFunc writes Command/Argument,
        // inherited Plr, then inherited ByClient. StdCompilerINIWrite omits
        // their defaults empty, empty, -1, and -1 respectively.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=209\n\
    [Custom Command]\n\
      Command=push\n\
      Argument=\"+130tail\"\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=209\n\
    [Custom Command]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse custom commands"),
            vec![
                ControlPacket::CustomCommand(CustomCommandControlData {
                    command: LegacyCString::from_bytes(b"push".to_vec())
                        .expect("fixture is NUL-free"),
                    argument: LegacyCString::from_bytes(b"+130tail".to_vec())
                        .expect("fixture is NUL-free"),
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::CustomCommand(CustomCommandControlData::default()),
            ]
        );
    }

    #[test]
    fn custom_command_ini_preserves_non_utf8_legacy_bytes() {
        let input = "[Control]\n[IDPacket]\nID=209\n[Custom Command]\nCommand=\"\\200\"\nArgument=\"\\377\"\n";
        let packets = parse_control_ini(input).expect("parse custom command bytes");
        let ControlPacket::CustomCommand(command) = &packets[0] else {
            panic!("expected CustomCommand, got {:?}", packets[0]);
        };
        assert_eq!(command.command.as_bytes(), &[0x80]);
        assert_eq!(command.argument.as_bytes(), &[0xff]);
    }

    #[test]
    fn parses_internal_player_script_packets_and_cpp_defaults() {
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=211\n\
    [Activate Game Goal Menu]\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=212\n\
    [Toggle Hostility]\n\
      Opponent=130\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=214\n\
    [Activate Game Goal/Rule]\n\
      Object=130\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=215\n\
    [Set Player Team]\n\
      Team=130\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=216\n\
    [Eliminate Player]\n\
      Plr=-4\n\
      ByClient=7\n\
  [IDPacket]\n\
    ID=211\n\
    [Activate Game Goal Menu]\n\
  [IDPacket]\n\
    ID=212\n\
    [Toggle Hostility]\n\
  [IDPacket]\n\
    ID=214\n\
    [Activate Game Goal/Rule]\n\
  [IDPacket]\n\
    ID=215\n\
    [Set Player Team]\n\
  [IDPacket]\n\
    ID=216\n\
    [Eliminate Player]\n";

        assert_eq!(
            parse_control_ini(input).expect("parse internal player controls"),
            vec![
                ControlPacket::ActivateGameGoalMenu(ActivateGameGoalMenuControlData {
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::ToggleHostility(ToggleHostilityControlData {
                    opponent: 130,
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::ActivateGameGoalRule(ActivateGameGoalRuleControlData {
                    object: 130,
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::SetPlayerTeam(SetPlayerTeamControlData {
                    team: 130,
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::EliminatePlayer(EliminatePlayerControlData {
                    player: -4,
                    by_client: 7,
                }),
                ControlPacket::ActivateGameGoalMenu(ActivateGameGoalMenuControlData::default(),),
                ControlPacket::ToggleHostility(ToggleHostilityControlData::default()),
                ControlPacket::ActivateGameGoalRule(ActivateGameGoalRuleControlData::default(),),
                ControlPacket::SetPlayerTeam(SetPlayerTeamControlData::default()),
                ControlPacket::EliminatePlayer(EliminatePlayerControlData::default()),
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
    fn message_model_uses_cpp_raw_types_and_safe_defaults() {
        assert_eq!(
            [
                MESSAGE_TYPE_NORMAL,
                MESSAGE_TYPE_ME,
                MESSAGE_TYPE_SAY,
                MESSAGE_TYPE_TEAM,
                MESSAGE_TYPE_PRIVATE,
                MESSAGE_TYPE_SOUND,
                MESSAGE_TYPE_ALERT,
                MESSAGE_TYPE_SYSTEM,
            ],
            [0, 1, 2, 3, 4, 5, 6, 10]
        );
        assert_eq!(
            MessageControlData::default(),
            MessageControlData {
                message_type: MESSAGE_TYPE_NORMAL,
                player: -1,
                to_player: -1,
                message: LegacyCString::default(),
                by_client: -1,
            }
        );
    }

    #[test]
    fn parses_message_private_unknown_and_conditional_defaults() {
        // C4ControlMessage::CompileFunc includes ToPlayer only for raw type 4.
        // A non-private field is deliberately malformed: C++ never consumes
        // it, so it must not affect alignment or the safe -1 Rust default.
        let input = "\
[Control]\n\
  [IDPacket]\n\
    ID=163\n\
    [Message]\n\
      Type=0x04\n\
      Player=3\n\
      ToPlayer=9\n\
      Message=\"\\200\\377private\"\n\
      ByClient=2\n\
  [IDPacket]\n\
    ID=163\n\
    [Message]\n\
      ToPlayer=not-consumed\n\
  [IDPacket]\n\
    ID=163\n\
    [Message]\n\
      Type=9\n\
      Player=7\n\
      ToPlayer=also-not-consumed\n\
      Message=unknown\n\
      ByClient=8\n\
  [IDPacket]\n\
    ID=163\n\
    [Message]\n\
      Type=999\n\
      Message=clamped\n\
  [IDPacket]\n\
    ID=163\n\
    [Message]\n\
      Type=-1\n\
      Message=negative-clamped\n";

        assert_eq!(
            parse_control_ini(input).expect("parse message controls"),
            vec![
                ControlPacket::Message(MessageControlData {
                    message_type: MESSAGE_TYPE_PRIVATE,
                    player: 3,
                    to_player: 9,
                    message: LegacyCString::from_bytes(
                        [0x80, 0xff].into_iter().chain(*b"private").collect(),
                    )
                    .expect("fixture is NUL-free"),
                    by_client: 2,
                }),
                ControlPacket::Message(MessageControlData::default()),
                ControlPacket::Message(MessageControlData {
                    message_type: 9,
                    player: 7,
                    to_player: -1,
                    message: LegacyCString::from_bytes(b"unknown".to_vec())
                        .expect("fixture is NUL-free"),
                    by_client: 8,
                }),
                ControlPacket::Message(MessageControlData {
                    message_type: u8::MAX,
                    message: LegacyCString::from_bytes(b"clamped".to_vec())
                        .expect("fixture is NUL-free"),
                    ..Default::default()
                }),
                ControlPacket::Message(MessageControlData {
                    message_type: u8::MAX,
                    message: LegacyCString::from_bytes(b"negative-clamped".to_vec())
                        .expect("fixture is NUL-free"),
                    ..Default::default()
                }),
            ]
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
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd,
                0xee, 0xff, 0x10, 0x20, 0x30, 0x40,
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
        assert_eq!(
            (resource.chunk_size, resource.contents_crc),
            (1024, 2596069104)
        );
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
    fn replay_player_infos_parser_preserves_client_and_player_order_and_full_state() {
        let input = r#"[PlayerInfoList]
LastPlayerID=12

  [Client]
  ID=7
  Flags=Initial|Updated

    [Player]
    Name="Join\200ed"
    ForcedName="Forced\377"
    Flags=Joined|Invisible
    ID=4
    Type=User
    Color=1122867
    GameNumber=2
    GameJoinFrame=33

    [Player]
    Name=Removed
    Flags=Joined|Removed|Disconnected|VotedOut
    ID=5
    Type=Script
    Team=2
    GameNumber=3
    GameJoinFrame=34
    GamePartFrame=44

  [Client]
  ID=3
  Flags=AddPlayers|Initial

    [Player]
    Name="Unjoined\201"
    ID=6
    GameNumber=99
    GameJoinFrame=98
    GamePartFrame=97

    [Player]
    Name=Resource
    Flags=HasResource|Invisible
    ID=8
    Type=Script

      [ResCore]
      Type=Player
      ID=23
      Filename="Players/Plr\202.c4p"
      Author="Host\377"
"#;

        let document =
            parse_replay_player_infos_ini(input.as_bytes()).expect("parse PlayerInfos.txt");
        assert_eq!(document.last_player_id, 12);
        let clients = document.clients;
        assert_eq!(clients.len(), 2);
        assert_eq!(
            clients
                .iter()
                .map(|client| client.client_id)
                .collect::<Vec<_>>(),
            vec![7, 3]
        );
        assert!(clients.iter().all(|client| client.by_client == 0));
        assert_eq!(
            clients[0].flags,
            CLIENT_PLAYER_INFO_FLAG_INITIAL | CLIENT_PLAYER_INFO_FLAG_UPDATED
        );
        assert_eq!(
            clients[1].flags,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_INITIAL
        );

        let joined = &clients[0].players[0];
        assert_eq!(joined.name.as_bytes(), b"Join\x80ed");
        assert_eq!(joined.forced_name.as_bytes(), b"Forced\xff");
        assert_eq!(
            (joined.id, joined.game_number, joined.game_join_frame),
            (4, 2, 33)
        );
        assert_eq!(joined.game_part_frame, -1);
        assert_eq!(joined.original_color, joined.color);
        assert_eq!(joined.flags, PLAYER_INFO_FLAG_JOINED);

        let removed = &clients[0].players[1];
        assert_eq!(
            removed.flags,
            PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_REMOVED
                | PLAYER_INFO_FLAG_DISCONNECTED
                | PLAYER_INFO_FLAG_VOTED_OUT
        );
        assert_eq!(
            (
                removed.game_number,
                removed.game_join_frame,
                removed.game_part_frame
            ),
            (3, 34, 44)
        );

        let unjoined = &clients[1].players[0];
        assert_eq!(unjoined.name.as_bytes(), b"Unjoined\x81");
        assert_eq!(
            (
                unjoined.game_number,
                unjoined.game_join_frame,
                unjoined.game_part_frame
            ),
            (-1, -1, -1),
            "conditional in-game fields are reset when their flags are absent"
        );

        let resource_player = &clients[1].players[1];
        assert_eq!(
            resource_player.flags,
            PLAYER_INFO_FLAG_HAS_RESOURCE | PLAYER_INFO_FLAG_INVISIBLE
        );
        let resource = resource_player
            .resource
            .as_ref()
            .expect("HasResource compiles the following ResCore");
        assert_eq!(
            (resource.resource_type, resource.id, resource.derived_id),
            (3, 23, -1)
        );
        assert!(resource.loadable);
        assert_eq!((resource.file_size, resource.file_crc), (0, 0));
        assert_eq!(resource.chunk_size, NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE);
        assert_eq!(resource.contents_crc, 0);
        assert_eq!(resource.filename.as_bytes(), b"Players/Plr\x82.c4p");
        assert_eq!(resource.author.as_bytes(), b"Host\xff");
    }

    #[test]
    fn replay_player_infos_parser_applies_named_compile_defaults() {
        let document =
            parse_replay_player_infos_ini(b"[PlayerInfoList]\n  [Client]\n    [Player]\n")
                .expect("minimal named list compiles with C++ defaults");

        assert_eq!(document.last_player_id, 0);
        let clients = document.clients;
        assert_eq!(clients.len(), 1);
        let client = &clients[0];
        assert_eq!(
            (client.client_id, client.flags, client.by_client),
            (-1, 0, 0)
        );
        assert_eq!(client.players.len(), 1);
        let player = &client.players[0];
        assert!(player.name.is_empty());
        assert!(player.forced_name.is_empty());
        assert!(player.filename.is_empty());
        assert_eq!(player.flags, 0);
        assert_eq!(player.id, 0);
        assert_eq!(player.player_type, PLAYER_INFO_TYPE_USER);
        assert_eq!((player.color, player.original_color), (0, 0));
        assert_eq!((player.savegame_player, player.team), (0, 0));
        assert_eq!(
            (
                player.game_number,
                player.game_join_frame,
                player.game_part_frame
            ),
            (-1, -1, -1)
        );
        assert_eq!(player.extra_data, *b"NONE");
        assert_eq!(player.league_projected_gain, -1);
        assert!(!player.league_progress_data_is_null);
        assert!(player.league_progress_data.is_empty());
        assert!(player.resource.is_none());
    }

    #[test]
    fn replay_player_infos_parser_preserves_raw_legacy_bytes() {
        let mut input = b"[PlayerInfoList]\n[Client]\n[Player]\nName=\"Raw".to_vec();
        input.extend_from_slice(&[0x80, 0xff]);
        input.extend_from_slice(b"\\200\"\nFilename=\"Plr");
        input.push(0xfe);
        input.extend_from_slice(
            b".c4p\"\nFlags=HasResource\nType=Script\n[ResCore]\nType=Player\nFilename=\"Res",
        );
        input.push(0xfc);
        input.extend_from_slice(b".c4p\"\nAuthor=\"Host");
        input.push(0xfd);
        input.extend_from_slice(b"\"\n");

        let document = parse_replay_player_infos_ini(&input).expect("parse legacy byte strings");
        assert_eq!(document.last_player_id, 0);
        let clients = document.clients;
        let player = &clients[0].players[0];
        assert_eq!(player.name.as_bytes(), b"Raw\x80\xff\x80");
        assert_eq!(player.filename.as_bytes(), b"Plr\xfe.c4p");
        let resource = player.resource.as_ref().expect("resource core retained");
        assert_eq!(resource.filename.as_bytes(), b"Res\xfc.c4p");
        assert_eq!(resource.author.as_bytes(), b"Host\xfd");
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
    fn classic_control_ini_encoder_matches_cpp_layout_and_escaping() {
        let command = ControlPacket::PlayerCommand(PlayerCommandControlData {
            player: 3,
            command: 7,
            x: 0,
            y: -4,
            target: 12,
            target2: 0,
            data: 0,
            add_mode: 2,
            by_client: 9,
        });
        assert_eq!(
            encode_control_packet_ini(&command).unwrap(),
            concat!(
                "[IDPacket]\r\n",
                "ID=162\r\n",
                "\r\n",
                "  [Player Command]\r\n",
                "  Player=3\r\n",
                "  Cmd=7\r\n",
                "  Y=-4\r\n",
                "  Target=12\r\n",
                "  AddMode=2\r\n",
                "  ByClient=9\r\n",
            )
            .as_bytes()
        );

        let script = ControlPacket::Script(ScriptControlData {
            script: LegacyCString::from_bytes(vec![1, b'1', b'2', b'\n', b'"', b'\\', 0x80])
                .unwrap(),
            ..ScriptControlData::default()
        });
        let mut inline = b"[Rec]\r\nFrame=9\r\nType=1\r\n".to_vec();
        append_control_packet_ini(&mut inline, &script, 0, ControlIniPacketMode::Inline).unwrap();
        assert_eq!(
            inline,
            concat!(
                "[Rec]\r\n",
                "Frame=9\r\n",
                "Type=1\r\n",
                "ID=136\r\n",
                "\r\n",
                "  [Script]\r\n",
                "  Script=\"\\1\\61\\62\\n\\\"\\\\\\200\"\r\n",
            )
            .as_bytes()
        );

        let mut list = b"[Rec]\r\nFrame=10\r\nType=0\r\n".to_vec();
        append_control_packet_ini(
            &mut list,
            &ControlPacket::PlayerControl(PlayerControlData::new(1, 2, 0, -1)),
            2,
            ControlIniPacketMode::IdPacketSection,
        )
        .unwrap();
        assert_eq!(
            list,
            concat!(
                "[Rec]\r\n",
                "Frame=10\r\n",
                "Type=0\r\n",
                "\r\n",
                "  [IDPacket]\r\n",
                "  ID=161\r\n",
                "\r\n",
                "    [Player Control]\r\n",
                "    Player=1\r\n",
                "    Com=2\r\n",
            )
            .as_bytes()
        );

        assert_eq!(
            encode_control_packet_ini(&ControlPacket::DebugRecord(DebugRecordControlData {
                data: vec![0, b'7'],
            },))
            .unwrap(),
            concat!(
                "[IDPacket]\r\n",
                "ID=192\r\n",
                "Debug Rec=2:\"\\0\\67\"\r\n",
            )
            .as_bytes()
        );

        for packet in [
            ControlPacket::ClientUpdate(ClientUpdateControlData::new(u8::MAX, -1, 0, -1)),
            ControlPacket::Vote(VoteControlData {
                vote_type: VOTE_TYPE_NONE,
                approve: true,
                data: 0,
                by_client: -1,
            }),
        ] {
            let encoded = encode_control_packet_ini(&packet).unwrap();
            assert!(
                encoded
                    .windows(b"Type=255\r\n".len())
                    .any(|window| window == b"Type=255\r\n"),
                "binary byte 255 must not collapse back to the signed enum -1 default: {packet:?}"
            );
        }
    }

    #[test]
    fn classic_control_ini_encoder_matches_nested_cpp_fields() {
        let packet = ControlPacket::ClientJoin(ClientJoinControlData {
            core: ClientCoreControlData {
                client_id: -1,
                activated: false,
                observer: false,
                name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                nick: LegacyCString::default(),
                lobby_ready: false,
            },
            by_client: -1,
        });
        assert_eq!(
            encode_control_packet_ini(&packet).unwrap(),
            concat!(
                "[IDPacket]\r\n",
                "ID=128\r\n",
                "\r\n",
                "  [Client Join]\r\n",
                "\r\n",
                "    [ClientCore]\r\n",
                "    Name=\"Alice\"\r\n",
            )
            .as_bytes()
        );

        let resource = NetworkResourceCore {
            resource_type: 3,
            id: 4,
            derived_id: -1,
            loadable: true,
            file_size: 12,
            file_crc: 0,
            chunk_size: NETWORK_RESOURCE_DEFAULT_CHUNK_SIZE,
            contents_crc: 9,
            file_sha: Some([0xab; 20]),
            filename: LegacyCString::from_bytes(b"Players/A.c4p".to_vec()).unwrap(),
            author: LegacyCString::from_bytes(b"Host/A".to_vec()).unwrap(),
        };
        let mut player = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Bot".to_vec()).unwrap(),
            forced_name: LegacyCString::from_bytes(b"Forced".to_vec()).unwrap(),
            filename: LegacyCString::from_bytes(b"Bot.c4p".to_vec()).unwrap(),
            flags: PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_HAS_RESOURCE
                | PLAYER_INFO_FLAG_IN_SCENARIO_FILE
                | PLAYER_INFO_FLAG_JOIN_ISSUED,
            id: 7,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            color: 0x123456,
            original_color: 0x123456,
            team: 2,
            game_number: 8,
            game_join_frame: 20,
            extra_data: *b"0007",
            clan_tag: LegacyCString::from_bytes(b"A\nB".to_vec()).unwrap(),
            league_progress_data_is_null: false,
            resource: Some(resource),
            ..Default::default()
        };
        // C++ serializes only PIF_SyncFlags, so the local JoinIssued bit is
        // absent and the unnamed InScenarioFile bit is emitted numerically.
        player.flags |= PLAYER_INFO_FLAG_INVISIBLE;
        let encoded =
            encode_control_packet_ini(&ControlPacket::PlayerInfo(PlayerInfoControlData {
                client_id: 3,
                flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_INITIAL | 0x80,
                players: vec![player],
                by_client: 0,
            }))
            .unwrap();
        let text = std::str::from_utf8(&encoded).unwrap();
        assert!(text.contains("  Flags=AddPlayers|Initial|128\r\n"));
        assert!(text.contains("    Name=\"Bot\"\r\n"));
        assert!(text.contains("    ForcedName=\"Forced\"\r\n"));
        assert!(text.contains("    Flags=Joined|HasResource|Invisible|64\r\n"));
        assert!(!text.contains("JoinIssued"));
        assert!(text.contains("    ExtraData=0007\r\n"));
        assert!(text.contains("    ClanTag=A\nB\r\n"));
        assert!(text.contains("      [ResCore]\r\n"));
        assert!(text.contains("      Type=Player\r\n"));
        assert!(text.contains("      FileSHA=abababababababababababababababababababab\r\n"));
        assert!(text.contains("      Filename=\"Players\\\\A.c4p\"\r\n"));
    }

    #[test]
    fn classic_control_ini_encoder_covers_every_typed_packet() {
        let text = |value: &[u8]| LegacyCString::from_bytes(value.to_vec()).unwrap();
        let packets = vec![
            ControlPacket::ClientJoin(ClientJoinControlData {
                core: ClientCoreControlData::default(),
                by_client: -1,
            }),
            ControlPacket::ClientUpdate(ClientUpdateControlData {
                update_type: CLIENT_UPDATE_SET_OBSERVER,
                client_id: 1,
                data: 0,
                by_client: 0,
            }),
            ControlPacket::ClientRemove(ClientRemoveControlData {
                client_id: 1,
                reason: text(b"gone"),
                by_client: 0,
            }),
            ControlPacket::Set(SetControlData::default()),
            ControlPacket::DebugRecord(DebugRecordControlData {
                data: vec![0, 0xff],
            }),
            ControlPacket::Vote(VoteControlData {
                vote_type: VOTE_TYPE_KICK,
                approve: false,
                data: 2,
                by_client: 1,
            }),
            ControlPacket::VoteEnd(VoteControlData {
                vote_type: VOTE_TYPE_PAUSE,
                approve: true,
                data: 0,
                by_client: 0,
            }),
            ControlPacket::Script(ScriptControlData::default()),
            ControlPacket::MessageBoardAnswer(MessageBoardAnswerControlData::default()),
            ControlPacket::CustomCommand(CustomCommandControlData::default()),
            ControlPacket::PlayerSelect(PlayerSelectControlData {
                player: -1,
                objects: Vec::new(),
                by_client: -1,
            }),
            ControlPacket::PlayerControl(PlayerControlData {
                player: -1,
                command: 0,
                data: 0,
                by_client: -1,
            }),
            ControlPacket::PlayerCommand(PlayerCommandControlData {
                player: -1,
                command: 0,
                x: 0,
                y: 0,
                target: 0,
                target2: 0,
                data: 0,
                add_mode: 0,
                by_client: -1,
            }),
            ControlPacket::Message(MessageControlData::default()),
            ControlPacket::EmMoveObject(EmMoveObjectControlData::default()),
            ControlPacket::EmDrawTool(EmDrawToolControlData::default()),
            ControlPacket::EmDropDef(EmDropDefControlData::default()),
            ControlPacket::InitScenarioPlayer(InitScenarioPlayerControlData::default()),
            ControlPacket::ActivateGameGoalMenu(ActivateGameGoalMenuControlData::default()),
            ControlPacket::ToggleHostility(ToggleHostilityControlData::default()),
            ControlPacket::SurrenderPlayer(SurrenderPlayerControlData {
                player: -1,
                by_client: -1,
            }),
            ControlPacket::ActivateGameGoalRule(ActivateGameGoalRuleControlData::default()),
            ControlPacket::SetPlayerTeam(SetPlayerTeamControlData::default()),
            ControlPacket::EliminatePlayer(EliminatePlayerControlData::default()),
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
            ControlPacket::Synchronize(SynchronizeControlData::default()),
            ControlPacket::JoinPlayer(JoinPlayerControlData::default()),
            ControlPacket::RemovePlayer(RemovePlayerControlData::default()),
            ControlPacket::PlayerInfo(PlayerInfoControlData::default()),
        ];

        for packet in packets {
            let encoded = encode_control_packet_ini(&packet).unwrap();
            assert!(encoded.starts_with(b"[IDPacket]\r\nID="), "{packet:?}");
            assert!(encoded.ends_with(b"\r\n"), "{packet:?}");
        }
    }

    #[test]
    fn classic_control_ini_encoder_round_trips_valid_cpp_grammar() {
        let text = |value: &[u8]| LegacyCString::from_bytes(value.to_vec()).unwrap();
        let packets = [
            ControlPacket::ClientJoin(ClientJoinControlData {
                core: ClientCoreControlData {
                    client_id: 4,
                    activated: true,
                    observer: false,
                    name: text(b"Alice"),
                    nick: text(b"Ally"),
                    lobby_ready: true,
                },
                by_client: 0,
            }),
            ControlPacket::EmMoveObject(EmMoveObjectControlData {
                action: EMMO_SCRIPT,
                tx: 1,
                ty: 2,
                target_object: 3,
                objects: vec![4, 5],
                strictness: ScriptStrictness::Strict2,
                script: text(b"Do()"),
                by_client: 0,
            }),
            ControlPacket::Message(MessageControlData {
                message_type: MESSAGE_TYPE_PRIVATE,
                player: 1,
                to_player: 2,
                message: text(b"hello"),
                by_client: 4,
            }),
            ControlPacket::JoinPlayer(JoinPlayerControlData {
                filename: text(b"Players/A.c4p"),
                at_client: 4,
                info_id: 9,
                source: JoinPlayerSource::Embedded(vec![0, b'1', 0xff]),
                by_client: 0,
            }),
        ];
        let mut document = b"[Control]\r\n".to_vec();
        for packet in &packets {
            append_control_packet_ini(
                &mut document,
                packet,
                0,
                ControlIniPacketMode::IdPacketSection,
            )
            .unwrap();
        }
        let parsed = parse_control_ini(std::str::from_utf8(&document).unwrap()).unwrap();
        assert_eq!(parsed, packets);
    }

    #[test]
    fn classic_control_ini_encoder_reports_unrepresentable_typed_state() {
        assert_eq!(
            encode_control_packet_ini(&ControlPacket::Unknown {
                id: ControlPacketId(3),
                name: Some("Mystery".to_string()),
                fields: HashMap::new(),
            }),
            Err(ControlIniEncodeError::UnsupportedPacket {
                id: 3,
                name: "Mystery".to_string(),
            })
        );

        let mut resource = NetworkResourceCore::default();
        resource.loadable = true;
        resource.chunk_size = 0;
        assert_eq!(
            encode_control_packet_ini(&ControlPacket::JoinPlayer(JoinPlayerControlData {
                source: JoinPlayerSource::Resource(resource),
                ..JoinPlayerControlData::default()
            })),
            Err(ControlIniEncodeError::ZeroResourceChunkSize)
        );
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
