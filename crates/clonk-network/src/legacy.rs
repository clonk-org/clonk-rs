use clonk_engine::{
    ActivateGameGoalMenuControlData, ActivateGameGoalRuleControlData, ClientCoreControlData,
    ClientJoinControlData, ClientRemoveControlData, ClientUpdateControlData,
    ControlPacket as EngineControlPacket, ControlPlayerInfoEntry, CustomCommandControlData,
    DebugRecordControlData, EliminatePlayerControlData, EmDrawToolControlData,
    EmDropDefControlData, EmMoveObjectControlData, InitScenarioPlayerControlData,
    JoinPlayerControlData, JoinPlayerSource, LegacyCString, MessageBoardAnswerControlData,
    MessageControlData, NetworkResourceCore, PlayerCommandControlData, PlayerControlData,
    PlayerInfoControlData, PlayerInfoUpdateRequest, PlayerSelectControlData,
    RemovePlayerControlData, ScriptControlData, ScriptStrictness, SetControlData,
    SetPlayerTeamControlData, SurrenderPlayerControlData, SyncCheckPacket, SynchronizeControlData,
    ToggleHostilityControlData, VoteControlData, CLIENT_UPDATE_ACTIVATE, EMMO_SCRIPT,
    MESSAGE_TYPE_PRIVATE, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
    PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_TYPE_SCRIPT,
};

use crate::join_client_registry::{
    decode_join_client_registry, encode_join_client_registry, JoinClientRegistrySnapshot,
};
use crate::join_player_registry::{
    decode_player_info_list, encode_player_info_list, PlayerInfoListSnapshot,
};
use crate::join_team_registry::{
    decode_join_team_list, encode_join_team_list, JoinTeamListSnapshot,
};
use crate::name_validation::{validate_name_allow_empty, validate_name_no_empty};
use crate::{ClientId, ControlPacket, NetworkStatus, ReadyBatch, Tick, BROADCAST_CLIENT_ID};

// C4PacketType::PID_None (src/C4PacketBase.h). Binary C4PacketList values
// terminate with a default C4IDPacket carrying this byte.
const PID_NONE: u8 = 0xff;
const CID_CLIENT_JOIN: u8 = 0x80;
const CID_CLIENT_UPDATE: u8 = 0x80 | 0x01;
const CID_CLIENT_REMOVE: u8 = 0x80 | 0x02;
const CID_VOTE: u8 = 0x80 | 0x03;
const CID_VOTE_END: u8 = 0x80 | 0x04;
const CID_PLR_INFO: u8 = 0x80 | 0x10;
const CID_JOIN_PLR: u8 = 0x80 | 0x11;
const CID_REMOVE_PLR: u8 = 0x80 | 0x12;
const CID_PLR_SELECT: u8 = 0x80 | 0x20;
const CID_PLR_CONTROL: u8 = 0x80 | 0x21;
const CID_PLR_COMMAND: u8 = 0x80 | 0x22;
const CID_MESSAGE: u8 = 0x80 | 0x23;
const CID_EM_MOVE_OBJECT: u8 = 0x80 | 0x30;
const CID_EM_DRAW_TOOL: u8 = 0x80 | 0x31;
const CID_EM_DROP_DEF: u8 = 0x80 | 0x32;
const CID_DEBUG_RECORD: u8 = 0x80 | 0x40;
const CID_INIT_SCENARIO_PLAYER: u8 = 0x80 | 0x52;
const CID_SURRENDER_PLAYER: u8 = 0x80 | 0x55;
const CID_SYNC_CHECK: u8 = 0x80 | 0x05;
const CID_SYNCHRONIZE: u8 = 0x80 | 0x06;
const CID_SET: u8 = 0x80 | 0x07;
const CID_SCRIPT: u8 = 0x80 | 0x08;
const CID_MESSAGE_BOARD_ANSWER: u8 = 0x80 | 0x50;
const CID_CUSTOM_COMMAND: u8 = 0x80 | 0x51;
const CID_ACTIVATE_GAME_GOAL_MENU: u8 = 0x80 | 0x53;
const CID_TOGGLE_HOSTILITY: u8 = 0x80 | 0x54;
const CID_ACTIVATE_GAME_GOAL_RULE: u8 = 0x80 | 0x56;
const CID_SET_PLAYER_TEAM: u8 = 0x80 | 0x57;
const CID_ELIMINATE_PLAYER: u8 = 0x80 | 0x58;
const MAX_VARINT_BYTES: usize = 5;
const MAX_PLAYER_INFO_COUNT: i32 = 5_000;
const PLAYER_INFO_SYNC_FLAGS: u16 = 0x7fcd;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyControlError {
    #[error("control payload is empty")]
    EmptyPayload,
    #[error("control packet is truncated")]
    UnexpectedEof,
    #[error("packed integer exceeds supported size")]
    VarintOverflow,
    #[error("control packet {0:#x} is not supported yet")]
    UnsupportedPacket(u8),
    #[error("script strictness {0} is outside the C++ range 0..=3")]
    InvalidScriptStrictness(u8),
    #[error("PlayerSelect object count {0} is negative")]
    PlayerSelectObjectCountOutOfRange(i32),
    #[error("EMMoveObject object count {0} is negative")]
    EmMoveObjectCountOutOfRange(i32),
    #[error("C4ID text exceeds its four-byte control field: {0} bytes")]
    ControlC4IdTooLong(usize),
    #[error("resource SHA contains an invalid hexadecimal byte")]
    InvalidResourceSha,
    #[error("JoinData C4ID is not exactly four uppercase letters, digits or underscores")]
    InvalidJoinDataC4Id,
    #[error("PlayerInfo count {0} is outside the C++ range")]
    PlayerInfoCountOutOfRange(i32),
    #[error("JoinData collection count {0} is outside the C++ range")]
    JoinDataCountOutOfRange(i32),
    #[error("PlayerInfo extra-data C4ID exceeds four bytes")]
    PlayerInfoExtraDataTooLong,
    #[error("loadable network resource has zero chunk size")]
    ZeroResourceChunkSize,
    #[error("JoinData team name is {0} bytes; C4MaxName is 30")]
    JoinDataTeamNameTooLong(usize),
    #[error("control payload contained negative client id {0}")]
    NegativeClientId(i32),
    #[error("control payload contained negative tick {0}")]
    NegativeTick(i32),
    #[error("control payload reported client {payload_id} but header contained {header_id}")]
    ClientIdMismatch {
        header_id: ClientId,
        payload_id: ClientId,
    },
    #[error("control payload reported tick {payload_tick} but header contained {header_tick}")]
    TickMismatch {
        header_tick: Tick,
        payload_tick: Tick,
    },
    #[error("control payload contained trailing data")]
    TrailingData,
    #[error("control payload does not end with a PID_NONE list terminator")]
    MissingListTerminator,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyEncodeError {
    #[error("control packet variant is not supported yet")]
    UnsupportedPacket,
    #[error("embedded JoinPlayer data length {0} exceeds uint32")]
    PlayerDataTooLarge(usize),
    #[error("PlayerInfo entry {0} has HasResource set without a resource core")]
    MissingPlayerInfoResource(i32),
    #[error("PlayerInfo count {0} is outside the C++ range")]
    PlayerInfoCountOutOfRange(usize),
    #[error("PlayerSelect object count {0} exceeds C++ int32")]
    PlayerSelectObjectCountTooLarge(usize),
    #[error("EMMoveObject object count {0} exceeds C++ int32")]
    EmMoveObjectCountTooLarge(usize),
    #[error("DebugRec payload length {0} exceeds C++ uint32")]
    DebugRecordTooLarge(usize),
    #[error("JoinData collection count {0} exceeds C++ int32")]
    JoinDataCollectionTooLarge(usize),
    #[error("JoinData client count {0} exceeds C++ uint32")]
    JoinDataClientCountTooLarge(usize),
    #[error("JoinData team name is {0} bytes; C4MaxName is 30")]
    JoinDataTeamNameTooLong(usize),
    #[error("loadable network resource has zero chunk size")]
    ZeroResourceChunkSize,
    #[error("client id {0} exceeds supported range")]
    ClientIdOutOfRange(ClientId),
    #[error("control tick {0} exceeds supported range")]
    TickOutOfRange(Tick),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LegacyAggregateError {
    #[error("control packet for client {client_id} could not be decoded: {source}")]
    Decode {
        client_id: ClientId,
        source: LegacyControlError,
    },
    #[error("control packet for client {client_id} has tick {packet_tick}, expected {tick}")]
    TickMismatch {
        client_id: ClientId,
        tick: Tick,
        packet_tick: Tick,
    },
    #[error("complete control packet could not be encoded: {0}")]
    Encode(#[from] LegacyEncodeError),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegacyControlFrame {
    pub client_id: ClientId,
    pub tick: Tick,
    pub timestamp_ms: u64,
    pub controls: Vec<EngineControlPacket>,
}

/// Full `C4PacketJoinData`, including its recursively compiled game parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinDataEnvelope {
    pub client_id: i32,
    pub start_control_tick: i32,
    pub status: NetworkStatus,
    pub dynamic: NetworkResourceCore,
    pub parameters: JoinGameParametersEnvelope,
}

/// Exact four-byte `C4IDAdapt` value used by JoinData ID lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JoinDataC4Id([u8; 4]);

impl JoinDataC4Id {
    pub fn from_bytes(bytes: [u8; 4]) -> Option<Self> {
        bytes
            .iter()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || *byte == b'_')
            .then_some(Self(bytes))
    }

    pub fn as_bytes(&self) -> &[u8; 4] {
        &self.0
    }
}

/// One `C4IDList` entry in the JoinData game parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinDataIdListEntry {
    pub id: JoinDataC4Id,
    pub count: i32,
}

/// The synchronized `C4GameParameters` payload carried by JoinData.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinGameParametersEnvelope {
    pub random_seed: i32,
    pub startup_player_count: i32,
    pub max_players: i32,
    pub use_fair_crew: bool,
    pub fair_crew_forced: bool,
    pub fair_crew_strength: i32,
    pub allow_debug: bool,
    pub is_network_game: bool,
    pub control_rate: i32,
    pub auto_frame_skip: bool,
    pub rules: Vec<JoinDataIdListEntry>,
    pub goals: Vec<JoinDataIdListEntry>,
    pub league: LegacyCString,
    pub league_address: LegacyCString,
    pub title: LegacyCString,
    pub scenario: NetworkResourceCore,
    pub game_resources: Vec<NetworkResourceCore>,
    pub player_infos: PlayerInfoListSnapshot,
    pub restore_player_infos: PlayerInfoListSnapshot,
    pub teams: JoinTeamListSnapshot,
    pub clients: JoinClientRegistrySnapshot,
}

/// Decodes the fixed `C4PacketJoinData` prefix
/// (`src/C4Network2IO.cpp:1683-1692`). `C4Network2Status` is compiled in
/// reference form, so its target tick is omitted and remains `-1`
/// (`src/C4Network2.cpp:54-55,108-123`).
pub fn decode_join_data_envelope(data: &[u8]) -> Result<JoinDataEnvelope, LegacyControlError> {
    let mut reader = Reader::new(data);
    let client_id = reader.read_int32()?;
    let start_control_tick = reader.read_int32()?;
    let status = NetworkStatus {
        state: reader.read_u8()?,
        control_mode: reader.read_int32()?,
        target_tick: -1,
    };
    let dynamic = reader.read_network_resource_core()?;
    let parameters_data = reader
        .data
        .get(reader.offset..)
        .ok_or(LegacyControlError::UnexpectedEof)?;
    let parameters = decode_join_game_parameters_envelope(parameters_data)?;
    Ok(JoinDataEnvelope {
        client_id,
        start_control_tick,
        status,
        dynamic,
        parameters,
    })
}

/// Re-encodes the complete typed JoinData packet body.
pub fn encode_join_data_envelope(
    envelope: &JoinDataEnvelope,
) -> Result<Vec<u8>, LegacyEncodeError> {
    validate_network_resource_core(&envelope.dynamic)?;
    let mut data = Vec::new();
    append_int32(&mut data, envelope.client_id);
    append_int32(&mut data, envelope.start_control_tick);
    data.push(envelope.status.state);
    append_int32(&mut data, envelope.status.control_mode);
    encode_network_resource_core(&mut data, &envelope.dynamic);
    data.extend(encode_join_game_parameters_envelope(&envelope.parameters)?);
    Ok(data)
}

/// Decodes the full C++ `C4GameParameters` JoinData payload
/// (`src/C4GameParameters.cpp:555-590`).
pub fn decode_join_game_parameters_envelope(
    data: &[u8],
) -> Result<JoinGameParametersEnvelope, LegacyControlError> {
    let mut reader = Reader::new(data);
    let random_seed = reader.read_raw_i32()?;
    let startup_player_count = reader.read_raw_i32()?;
    let max_players = reader.read_raw_i32()?;
    let use_fair_crew = reader.read_u8()? != 0;
    let fair_crew_forced = reader.read_u8()? != 0;
    let fair_crew_strength = reader.read_raw_i32()?;
    let allow_debug = reader.read_u8()? != 0;
    let is_network_game = reader.read_u8()? != 0;
    let control_rate = reader.read_raw_i32()?;
    let auto_frame_skip = reader.read_u8()? != 0;
    let rules = decode_join_data_id_list(&mut reader)?;
    let goals = decode_join_data_id_list(&mut reader)?;
    let league = reader.read_c_string()?;
    let league_address = reader.read_c_string()?;
    let title = reader.read_c_string()?;
    let scenario = reader.read_network_resource_core()?;
    let game_resource_count = reader.read_int32()?;
    if game_resource_count < 0 {
        return Err(LegacyControlError::JoinDataCountOutOfRange(
            game_resource_count,
        ));
    }
    let mut game_resources = Vec::new();
    for _ in 0..game_resource_count {
        game_resources.push(reader.read_network_resource_core()?);
    }
    let player_infos = decode_player_info_list(&mut reader)?;
    let restore_player_infos = decode_player_info_list(&mut reader)?;
    let teams = decode_join_team_list(&mut reader)?;
    let clients = decode_join_client_registry(&mut reader)?;
    Ok(JoinGameParametersEnvelope {
        random_seed,
        startup_player_count,
        max_players,
        use_fair_crew,
        fair_crew_forced,
        fair_crew_strength,
        allow_debug,
        is_network_game,
        control_rate,
        auto_frame_skip,
        rules,
        goals,
        league,
        league_address,
        title,
        scenario,
        game_resources,
        player_infos,
        restore_player_infos,
        teams,
        clients,
    })
}

/// Re-encodes the full typed C++ game-parameter payload.
pub fn encode_join_game_parameters_envelope(
    parameters: &JoinGameParametersEnvelope,
) -> Result<Vec<u8>, LegacyEncodeError> {
    validate_network_resource_core(&parameters.scenario)?;
    for resource in &parameters.game_resources {
        validate_network_resource_core(resource)?;
    }
    let mut data = Vec::new();
    append_raw_i32(&mut data, parameters.random_seed);
    append_raw_i32(&mut data, parameters.startup_player_count);
    append_raw_i32(&mut data, parameters.max_players);
    data.push(u8::from(parameters.use_fair_crew));
    data.push(u8::from(parameters.fair_crew_forced));
    append_raw_i32(&mut data, parameters.fair_crew_strength);
    data.push(u8::from(parameters.allow_debug));
    data.push(u8::from(parameters.is_network_game));
    append_raw_i32(&mut data, parameters.control_rate);
    data.push(u8::from(parameters.auto_frame_skip));
    encode_join_data_id_list(&mut data, &parameters.rules)?;
    encode_join_data_id_list(&mut data, &parameters.goals)?;
    append_c_string(&mut data, &parameters.league);
    append_c_string(&mut data, &parameters.league_address);
    append_c_string(&mut data, &parameters.title);
    encode_network_resource_core(&mut data, &parameters.scenario);
    let game_resource_count = i32::try_from(parameters.game_resources.len()).map_err(|_| {
        LegacyEncodeError::JoinDataCollectionTooLarge(parameters.game_resources.len())
    })?;
    append_int32(&mut data, game_resource_count);
    for resource in &parameters.game_resources {
        encode_network_resource_core(&mut data, resource);
    }
    encode_player_info_list(&mut data, &parameters.player_infos)?;
    encode_player_info_list(&mut data, &parameters.restore_player_infos)?;
    encode_join_team_list(&mut data, &parameters.teams)?;
    encode_join_client_registry(&mut data, &parameters.clients)?;
    Ok(data)
}

fn decode_join_data_id_list(
    reader: &mut Reader<'_>,
) -> Result<Vec<JoinDataIdListEntry>, LegacyControlError> {
    let count = reader.read_raw_i32()?;
    if count < 0 {
        return Err(LegacyControlError::JoinDataCountOutOfRange(count));
    }
    let mut entries = Vec::new();
    for _ in 0..count {
        let id = reader.read_c_string()?;
        let bytes: [u8; 4] = id
            .as_bytes()
            .try_into()
            .map_err(|_| LegacyControlError::InvalidJoinDataC4Id)?;
        entries.push(JoinDataIdListEntry {
            id: JoinDataC4Id::from_bytes(bytes).ok_or(LegacyControlError::InvalidJoinDataC4Id)?,
            count: reader.read_raw_i32()?,
        });
    }
    Ok(entries)
}

fn encode_join_data_id_list(
    data: &mut Vec<u8>,
    entries: &[JoinDataIdListEntry],
) -> Result<(), LegacyEncodeError> {
    let count = i32::try_from(entries.len())
        .map_err(|_| LegacyEncodeError::JoinDataCollectionTooLarge(entries.len()))?;
    append_raw_i32(data, count);
    for entry in entries {
        data.extend_from_slice(entry.id.as_bytes());
        data.push(0);
        append_raw_i32(data, entry.count);
    }
    Ok(())
}

/// Decoded `C4ControlSet` fields carried by `CID_Set` (0x87).
///
/// The engine owns the canonical typed packet; this compatibility wrapper
/// preserves the established network API used by the live app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LegacyControlSet {
    pub value_type: i32,
    pub data: i32,
    pub by_client: i32,
}

impl LegacyControlSet {
    pub fn into_control_packet(self) -> EngineControlPacket {
        EngineControlPacket::Set(self.into())
    }

    pub fn from_control_packet(control: &EngineControlPacket) -> Option<Self> {
        let EngineControlPacket::Set(data) = control else {
            return None;
        };
        Some((*data).into())
    }
}

impl From<LegacyControlSet> for SetControlData {
    fn from(value: LegacyControlSet) -> Self {
        Self {
            value_type: value.value_type,
            data: value.data,
            by_client: value.by_client,
        }
    }
}

impl From<SetControlData> for LegacyControlSet {
    fn from(value: SetControlData) -> Self {
        Self {
            value_type: value.value_type,
            data: value.data,
            by_client: value.by_client,
        }
    }
}

/// The validated outer fields and raw C4Control list body of one legacy
/// packet. The body excludes its final `PID_NONE`, so callers can concatenate
/// multiple source lists exactly like `C4GameControlPacket::Add`.
pub(crate) struct LegacyControlEnvelope<'a> {
    pub(crate) client_id: ClientId,
    pub(crate) tick: Tick,
    pub(crate) control_body: &'a [u8],
}

/// Validate and split off one terminated C4Control list. Decoding identifies
/// the first list terminator and deliberately ignores a trailing packet
/// suffix, matching the typed C++ packet compiler. Unknown control IDs remain
/// errors, as they are in `C4IDPacket::CompileFunc` (src/C4Packet2.cpp:193-217,
/// 298-335; src/C4GameControlNetwork.cpp:759-769).
pub(crate) fn validate_control_envelope(
    packet: &ControlPacket,
) -> Result<LegacyControlEnvelope<'_>, LegacyControlError> {
    let payload = packet.payload();
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }

    let (_, consumed) = packet.decoded_control_list()?;
    let control_body_len = consumed
        .checked_sub(1)
        .ok_or(LegacyControlError::MissingListTerminator)?;
    let control_body = payload
        .get(..control_body_len)
        .ok_or(LegacyControlError::MissingListTerminator)?;

    Ok(LegacyControlEnvelope {
        client_id: packet.client_id(),
        tick: packet.tick(),
        control_body,
    })
}

fn decode_client_id(client_id_raw: i32) -> Result<ClientId, LegacyControlError> {
    if client_id_raw == -1 {
        // C4ClientIDAll aliases C4ClientIDUnknown (-1) for a complete control
        // packet (C4GameControlNetwork.h:25-27).
        Ok(BROADCAST_CLIENT_ID)
    } else if client_id_raw < 0 {
        Err(LegacyControlError::NegativeClientId(client_id_raw))
    } else {
        Ok(client_id_raw as ClientId)
    }
}

pub fn decode_control_packet(
    packet: &ControlPacket,
) -> Result<LegacyControlFrame, LegacyControlError> {
    let (controls, _) = packet.decoded_control_list()?;
    Ok(LegacyControlFrame {
        client_id: packet.client_id(),
        tick: packet.tick(),
        timestamp_ms: packet.timestamp_ms(),
        controls: controls.to_vec(),
    })
}

/// Decode a complete serialized `C4GameControlPacket` body, including its
/// packed client ID and control tick. This is for C++ codec-oracle fixtures;
/// live [`ControlPacket::payload`] bytes contain only the terminated list.
pub fn decode_control_payload(payload: &[u8]) -> Result<LegacyControlFrame, LegacyControlError> {
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }
    let mut reader = Reader::new(payload);
    let client_id = decode_client_id(reader.read_int32()?)?;
    let tick_raw = reader.read_int32()?;
    if tick_raw < 0 {
        return Err(LegacyControlError::NegativeTick(tick_raw));
    }
    let tick = tick_raw as Tick;

    let controls = decode_control_list(&mut reader)?;

    Ok(LegacyControlFrame {
        client_id,
        tick,
        timestamp_ms: 0,
        controls,
    })
}

/// Decode the `C4IDPacket` body carried by `PID_ControlPkt` after its delivery
/// byte (src/C4Network2IO.cpp:1787-1793).
pub fn decode_control_entry_payload(
    payload: &[u8],
) -> Result<EngineControlPacket, LegacyControlError> {
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }
    let (control, _) = decode_control_entry_prefix(payload)?;
    Ok(control)
}

/// Decode one binary `C4IDPacket` from the beginning of `payload` and return
/// the number of bytes it consumed.
///
/// This reports the consumed prefix even though
/// [`decode_control_entry_payload`] also ignores a suffix, because
/// `CtrlRec.c4b` chunks have no explicit payload lengths and must advance by
/// the binary compiler's exact position (C4Record.cpp:503-538).
pub fn decode_control_entry_prefix(
    payload: &[u8],
) -> Result<(EngineControlPacket, usize), LegacyControlError> {
    let mut reader = Reader::new(payload);
    let control = decode_control(reader.read_u8()?, &mut reader)?;
    Ok((control, payload.len() - reader.remaining()))
}

/// Decode one binary `CID_InitScenarioPlayer` C4IDPacket body.
pub fn decode_init_scenario_player_control_entry_payload(
    payload: &[u8],
) -> Result<InitScenarioPlayerControlData, LegacyControlError> {
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }
    let mut reader = Reader::new(payload);
    let id = reader.read_u8()?;
    if id != CID_INIT_SCENARIO_PLAYER {
        return Err(LegacyControlError::UnsupportedPacket(id));
    }
    let control = InitScenarioPlayerControlData {
        team: reader.read_int32()?,
        player: reader.read_int32()?,
        by_client: reader.read_int32()?,
    };
    if reader.remaining() != 0 {
        return Err(LegacyControlError::TrailingData);
    }
    Ok(control)
}

/// Decode the `C4ClientPlayerInfos` body of `PID_PlayerInfoUpdReq`; unlike
/// `C4ControlPlayerInfo`, this packet has no `ByClient` field
/// (src/C4PlayerInfo.cpp:601-630,1800-1803).
pub fn decode_player_info_update_payload(
    payload: &[u8],
) -> Result<PlayerInfoUpdateRequest, LegacyControlError> {
    if payload.is_empty() {
        return Err(LegacyControlError::EmptyPayload);
    }
    let mut reader = Reader::new(payload);
    let (client_id, flags, players) = decode_player_info_contents(&mut reader)?;
    Ok(PlayerInfoUpdateRequest {
        client_id,
        flags,
        players,
    })
}

fn decode_control_list(
    reader: &mut Reader<'_>,
) -> Result<Vec<EngineControlPacket>, LegacyControlError> {
    let mut controls = Vec::new();
    loop {
        let id = reader.read_u8()?;
        if id == PID_NONE {
            break;
        }
        controls.push(decode_control(id, reader)?);
    }
    Ok(controls)
}

/// Decode one terminated binary `C4Control` list from the beginning of
/// `payload` and return the number of bytes it consumed, including the final
/// `PID_None` byte.
///
/// This prefix form is the boundary primitive for `RCT_Ctrl` chunks, whose
/// next two bytes are already the following record chunk header.
pub fn decode_control_list_prefix(
    payload: &[u8],
) -> Result<(Vec<EngineControlPacket>, usize), LegacyControlError> {
    #[cfg(test)]
    CONTROL_LIST_DECODE_PASSES.set(CONTROL_LIST_DECODE_PASSES.get() + 1);
    let mut reader = Reader::new(payload);
    let controls = decode_control_list(&mut reader)?;
    Ok((controls, payload.len() - reader.remaining()))
}

#[cfg(test)]
thread_local! {
    static CONTROL_LIST_DECODE_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_control_list_decode_passes() {
    CONTROL_LIST_DECODE_PASSES.set(0);
}

#[cfg(test)]
fn control_list_decode_passes() -> usize {
    CONTROL_LIST_DECODE_PASSES.get()
}

fn decode_control(
    id: u8,
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    match id {
        CID_CLIENT_JOIN => decode_client_join(reader),
        CID_CLIENT_UPDATE => decode_client_update(reader),
        CID_CLIENT_REMOVE => decode_client_remove(reader),
        CID_VOTE => decode_vote(reader),
        CID_VOTE_END => decode_vote_end(reader),
        CID_PLR_INFO => decode_player_info(reader),
        CID_JOIN_PLR => decode_join_player(reader),
        CID_REMOVE_PLR => decode_remove_player(reader),
        CID_PLR_SELECT => decode_player_select(reader),
        CID_PLR_CONTROL => decode_player_control(reader),
        CID_PLR_COMMAND => decode_player_command(reader),
        CID_MESSAGE => decode_message(reader),
        CID_EM_MOVE_OBJECT => decode_em_move_object(reader),
        CID_EM_DRAW_TOOL => decode_em_draw_tool(reader),
        CID_EM_DROP_DEF => decode_em_drop_def(reader),
        CID_DEBUG_RECORD => decode_debug_record(reader),
        CID_INIT_SCENARIO_PLAYER => decode_init_scenario_player(reader),
        CID_SURRENDER_PLAYER => decode_surrender_player(reader),
        CID_SYNC_CHECK => decode_sync_check(reader),
        CID_SYNCHRONIZE => decode_synchronize(reader),
        CID_SET => decode_control_set(reader),
        CID_SCRIPT => decode_script(reader),
        CID_MESSAGE_BOARD_ANSWER => decode_message_board_answer(reader),
        CID_CUSTOM_COMMAND => decode_custom_command(reader),
        CID_ACTIVATE_GAME_GOAL_MENU => decode_activate_game_goal_menu(reader),
        CID_TOGGLE_HOSTILITY => decode_toggle_hostility(reader),
        CID_ACTIVATE_GAME_GOAL_RULE => decode_activate_game_goal_rule(reader),
        CID_SET_PLAYER_TEAM => decode_set_player_team(reader),
        CID_ELIMINATE_PLAYER => decode_eliminate_player(reader),
        other => Err(LegacyControlError::UnsupportedPacket(other)),
    }
}

fn decode_client_join(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let core = ClientCoreControlData {
        client_id: reader.read_raw_i32()?,
        activated: reader.read_u8()? != 0,
        observer: reader.read_u8()? != 0,
        name: validate_name_no_empty(reader.read_c_string()?),
        nick: validate_name_no_empty(reader.read_c_string()?),
        lobby_ready: reader.read_u8()? != 0,
    };
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::ClientJoin(ClientJoinControlData {
        core,
        by_client,
    }))
}

fn decode_control_set(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(LegacyControlSet {
        // C4ControlSet::CompileFunc uses mkIntAdapt for Type, so the binary
        // compiler writes the enum's fixed-width int rather than IntPack.
        value_type: reader.read_raw_i32()?,
        data: reader.read_int32()?,
        by_client: reader.read_int32()?,
    }
    .into_control_packet())
}

fn decode_debug_record(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    // StdBuf casts its size to uint32_t before applying mkIntPackAdapt.
    let size = reader.read_uint32()? as usize;
    Ok(EngineControlPacket::DebugRecord(DebugRecordControlData {
        data: reader.read_bytes(size)?.to_vec(),
    }))
}

fn decode_remove_player(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::RemovePlayer(RemovePlayerControlData {
        player: reader.read_int32()?,
        disconnected: reader.read_u8()? != 0,
        by_client: reader.read_int32()?,
    }))
}

fn decode_script(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let target_object = reader.read_raw_i32()?;
    let strictness = reader.read_u8()?;
    // C4ControlScript::CheckStrictness runs immediately after compiling the
    // strictness byte, before the following script string is read.
    let strictness = ScriptStrictness::try_from(strictness)
        .map_err(LegacyControlError::InvalidScriptStrictness)?;
    let script = reader.read_c_string()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::Script(ScriptControlData {
        target_object,
        strictness,
        script,
        by_client,
    }))
}

fn decode_message_board_answer(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let object = reader.read_int32()?;
    let answer = reader.read_c_string()?;
    let player = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::MessageBoardAnswer(
        MessageBoardAnswerControlData {
            object,
            answer,
            player,
            by_client,
        },
    ))
}

fn decode_custom_command(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let command = reader.read_c_string()?;
    let argument = reader.read_c_string()?;
    let player = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::CustomCommand(
        CustomCommandControlData {
            command,
            argument,
            player,
            by_client,
        },
    ))
}

fn decode_activate_game_goal_menu(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::ActivateGameGoalMenu(
        ActivateGameGoalMenuControlData {
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_toggle_hostility(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::ToggleHostility(
        ToggleHostilityControlData {
            opponent: reader.read_int32()?,
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_activate_game_goal_rule(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::ActivateGameGoalRule(
        ActivateGameGoalRuleControlData {
            object: reader.read_int32()?,
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_set_player_team(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::SetPlayerTeam(
        SetPlayerTeamControlData {
            team: reader.read_int32()?,
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_eliminate_player(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::EliminatePlayer(
        EliminatePlayerControlData {
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_client_remove(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let client_id = reader.read_int32()?;
    let reason = reader.read_c_string()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::ClientRemove(ClientRemoveControlData {
        client_id,
        reason,
        by_client,
    }))
}

fn decode_vote(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::Vote(decode_vote_data(reader)?))
}

fn decode_vote_end(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::VoteEnd(decode_vote_data(reader)?))
}

fn decode_vote_data(reader: &mut Reader<'_>) -> Result<VoteControlData, LegacyControlError> {
    Ok(VoteControlData {
        vote_type: reader.read_u8()?,
        approve: reader.read_u8()? != 0,
        data: reader.read_raw_i32()?,
        by_client: reader.read_int32()?,
    })
}

fn decode_client_update(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let update_type = reader.read_u8()?;
    let client_id = reader.read_int32()?;
    let data = if update_type == CLIENT_UPDATE_ACTIVATE {
        reader.read_int32()?
    } else {
        0
    };
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::ClientUpdate(ClientUpdateControlData {
        update_type,
        client_id,
        data,
        by_client,
    }))
}

fn decode_player_info(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let (client_id, flags, players) = decode_player_info_contents(reader)?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::PlayerInfo(PlayerInfoControlData {
        client_id,
        flags,
        players,
        by_client,
    }))
}

fn decode_player_info_contents(
    reader: &mut Reader<'_>,
) -> Result<(i32, u32, Vec<ControlPlayerInfoEntry>), LegacyControlError> {
    let client_id = reader.read_raw_i32()?;
    let flags = reader.read_raw_u32()?;
    let player_count = reader.read_int32()?;
    if !(0..=MAX_PLAYER_INFO_COUNT).contains(&player_count) {
        return Err(LegacyControlError::PlayerInfoCountOutOfRange(player_count));
    }
    let players = (0..player_count)
        .map(|_| decode_player_info_entry(reader))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((client_id, flags, players))
}

fn decode_player_info_entry(
    reader: &mut Reader<'_>,
) -> Result<ControlPlayerInfoEntry, LegacyControlError> {
    let name = validate_name_no_empty(reader.read_c_string()?);
    let forced_name = validate_name_allow_empty(reader.read_c_string()?);
    let filename = reader.read_c_string()?;
    let mut flags = reader.read_raw_u16()?;
    let id = reader.read_raw_i32()?;
    let player_type = reader.read_u8()?;
    if player_type != PLAYER_INFO_TYPE_SCRIPT {
        flags &= !PLAYER_INFO_FLAG_INVISIBLE;
    }
    let color = reader.read_raw_u32()?;
    let original_color = reader.read_raw_u32()?;
    let savegame_player = reader.read_int32()?;
    let team = reader.read_int32()?;
    let auth_id = reader.read_c_string()?;
    let (game_number, game_join_frame) = if flags & PLAYER_INFO_FLAG_JOINED != 0 {
        (reader.read_raw_i32()?, reader.read_raw_i32()?)
    } else {
        (-1, -1)
    };
    let game_part_frame = if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
        reader.read_raw_i32()?
    } else {
        -1
    };
    let extra_data = reader.read_c4_id()?;
    let league_account = validate_name_allow_empty(reader.read_c_string()?);
    let league_score = reader.read_int32()?;
    let league_rank = reader.read_int32()?;
    let league_rank_symbol = reader.read_int32()?;
    let league_projected_gain = reader.read_int32()?;
    let clan_tag = validate_name_allow_empty(reader.read_c_string()?);
    let league_performance = reader.read_int32()?;
    let league_progress_data = reader.read_c_string()?;
    let resource = (flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0)
        .then(|| reader.read_network_resource_core())
        .transpose()?;

    Ok(ControlPlayerInfoEntry {
        name,
        forced_name,
        filename,
        flags,
        id,
        player_type,
        color,
        original_color,
        savegame_player,
        team,
        auth_id,
        game_number,
        game_join_frame,
        game_part_frame,
        extra_data,
        league_account,
        league_score,
        league_rank,
        league_rank_symbol,
        league_projected_gain,
        clan_tag,
        league_performance,
        // Binary C4PlayerInfo compilation materializes even an empty string.
        league_progress_data_is_null: false,
        league_progress_data,
        resource,
    })
}

fn decode_join_player(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let filename = reader.read_network_filename()?;
    let at_client = reader.read_int32()?;
    let info_id = reader.read_int32()?;
    let by_resource = reader.read_u8()? != 0;
    let source = if by_resource {
        JoinPlayerSource::Resource(reader.read_network_resource_core()?)
    } else {
        let player_data_len = reader.read_uint32()? as usize;
        JoinPlayerSource::Embedded(reader.read_bytes(player_data_len)?.to_vec())
    };
    let by_client = reader.read_int32()?;

    Ok(EngineControlPacket::JoinPlayer(JoinPlayerControlData {
        filename,
        at_client,
        info_id,
        source,
        by_client,
    }))
}

fn decode_player_control(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let player = reader.read_int32()?;
    let command = reader.read_int32()?;
    let data = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::PlayerControl(PlayerControlData {
        player,
        command,
        data,
        by_client,
    }))
}

fn decode_player_select(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let player = reader.read_raw_i32()?;
    let object_count = reader.read_raw_i32()?;
    if object_count < 0 {
        return Err(LegacyControlError::PlayerSelectObjectCountOutOfRange(
            object_count,
        ));
    }
    let mut objects = Vec::new();
    for _ in 0..object_count {
        objects.push(reader.read_raw_i32()?);
    }
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::PlayerSelect(PlayerSelectControlData {
        player,
        objects,
        by_client,
    }))
}

fn decode_player_command(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::PlayerCommand(
        PlayerCommandControlData {
            player: reader.read_int32()?,
            command: reader.read_int32()?,
            x: reader.read_raw_i32()?,
            y: reader.read_raw_i32()?,
            target: reader.read_raw_i32()?,
            target2: reader.read_raw_i32()?,
            data: reader.read_raw_i32()?,
            add_mode: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_message(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let message_type = reader.read_u8()?;
    let player = reader.read_int32()?;
    let to_player = if message_type == MESSAGE_TYPE_PRIVATE {
        reader.read_int32()?
    } else {
        -1
    };
    let message = reader.read_c_string()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::Message(MessageControlData {
        message_type,
        player,
        to_player,
        message,
        by_client,
    }))
}

fn decode_em_move_object(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    let action = reader.read_u8()?;
    let tx = reader.read_raw_i32()?;
    let ty = reader.read_raw_i32()?;
    let target_object = reader.read_raw_i32()?;
    let object_count = reader.read_int32()?;
    let strictness = reader.read_u8()?;
    // C4ControlEMMoveObject checks strictness before allocating or compiling
    // the object array, so retain that error precedence for truncated and
    // malformed controls.
    let strictness = ScriptStrictness::try_from(strictness)
        .map_err(LegacyControlError::InvalidScriptStrictness)?;
    if object_count < 0 {
        return Err(LegacyControlError::EmMoveObjectCountOutOfRange(
            object_count,
        ));
    }
    let mut objects = Vec::new();
    for _ in 0..object_count {
        objects.push(reader.read_raw_i32()?);
    }
    let script = if action == EMMO_SCRIPT {
        reader.read_c_string()?
    } else {
        LegacyCString::default()
    };
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::EmMoveObject(EmMoveObjectControlData {
        action,
        tx,
        ty,
        target_object,
        objects,
        strictness,
        script,
        by_client,
    }))
}

fn decode_em_draw_tool(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::EmDrawTool(EmDrawToolControlData {
        action: reader.read_u8()?,
        mode: reader.read_int32()?,
        x: reader.read_raw_i32()?,
        y: reader.read_raw_i32()?,
        x2: reader.read_raw_i32()?,
        y2: reader.read_raw_i32()?,
        grade: reader.read_int32()?,
        ift: reader.read_u8()? != 0,
        material: reader.read_c_string()?,
        texture: reader.read_c_string()?,
        by_client: reader.read_int32()?,
    }))
}

fn decode_em_drop_def(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let raw_id = reader.read_c_string()?;
    let id = match raw_id.as_bytes() {
        bytes if bytes.len() < 4 => *b"NONE",
        bytes if bytes.len() == 4 => normalize_c4_id_text(
            bytes
                .try_into()
                .map_err(|_| LegacyControlError::UnexpectedEof)?,
        ),
        bytes => return Err(LegacyControlError::ControlC4IdTooLong(bytes.len())),
    };
    Ok(EngineControlPacket::EmDropDef(EmDropDefControlData {
        id,
        x: reader.read_int32()?,
        y: reader.read_int32()?,
        by_client: reader.read_int32()?,
    }))
}

fn decode_init_scenario_player(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::InitScenarioPlayer(
        InitScenarioPlayerControlData {
            team: reader.read_int32()?,
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_surrender_player(
    reader: &mut Reader<'_>,
) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::SurrenderPlayer(
        SurrenderPlayerControlData {
            player: reader.read_int32()?,
            by_client: reader.read_int32()?,
        },
    ))
}

fn decode_sync_check(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    let frame = reader.read_int32()?;
    let control_tick = reader.read_int32()?;
    let random3 = reader.read_int32()?;
    let random_count = reader.read_raw_i32()?;
    let crew_positions_sum = reader.read_int32()?;
    let pxs_count = reader.read_int32()?;
    let mass_mover_index = reader.read_raw_i32()?;
    let object_count = reader.read_int32()?;
    let object_enumeration_index = reader.read_int32()?;
    let sector_shape_sum = reader.read_int32()?;
    let by_client = reader.read_int32()?;
    Ok(EngineControlPacket::SyncCheck(SyncCheckPacket {
        frame,
        control_tick,
        random3,
        random_count,
        crew_positions_sum,
        pxs_count,
        mass_mover_index,
        object_count,
        object_enumeration_index,
        sector_shape_sum,
        by_client,
    }))
}

fn decode_synchronize(reader: &mut Reader<'_>) -> Result<EngineControlPacket, LegacyControlError> {
    Ok(EngineControlPacket::Synchronize(SynchronizeControlData {
        save_player_files: reader.read_u8()? != 0,
        sync_clearance: reader.read_u8()? != 0,
        by_client: reader.read_int32()?,
    }))
}

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.offset)
    }

    pub(crate) fn read_u8(&mut self) -> Result<u8, LegacyControlError> {
        if self.offset >= self.data.len() {
            return Err(LegacyControlError::UnexpectedEof);
        }
        let byte = self.data[self.offset];
        self.offset += 1;
        Ok(byte)
    }

    pub(crate) fn read_bytes(&mut self, len: usize) -> Result<&'a [u8], LegacyControlError> {
        let end = self
            .offset
            .checked_add(len)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        self.offset = end;
        Ok(bytes)
    }

    pub(crate) fn read_c_string(&mut self) -> Result<LegacyCString, LegacyControlError> {
        let remaining = self
            .data
            .get(self.offset..)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let len = remaining
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self.read_bytes(len)?.to_vec();
        self.read_u8()?;
        LegacyCString::from_bytes(bytes).ok_or(LegacyControlError::UnexpectedEof)
    }

    #[cfg(windows)]
    fn read_network_filename(&mut self) -> Result<LegacyCString, LegacyControlError> {
        self.read_c_string()
    }

    #[cfg(not(windows))]
    fn read_network_filename(&mut self) -> Result<LegacyCString, LegacyControlError> {
        let filename = self.read_c_string()?;
        let native = filename
            .as_bytes()
            .iter()
            .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
            .collect();
        LegacyCString::from_bytes(native).ok_or(LegacyControlError::UnexpectedEof)
    }

    pub(crate) fn read_network_resource_core(
        &mut self,
    ) -> Result<NetworkResourceCore, LegacyControlError> {
        let resource_type = self.read_u8()?;
        let id = self.read_raw_i32()?;
        let derived_id = self.read_raw_i32()?;
        let loadable = self.read_u8()? != 0;
        let defaults = NetworkResourceCore::default();
        let (file_size, file_crc, chunk_size) = if loadable {
            (
                self.read_raw_u32()?,
                self.read_raw_u32()?,
                self.read_raw_u32()?,
            )
        } else {
            (defaults.file_size, defaults.file_crc, defaults.chunk_size)
        };
        if loadable && chunk_size == 0 {
            return Err(LegacyControlError::ZeroResourceChunkSize);
        }
        let contents_crc = self.read_raw_u32()?;
        let file_sha = (self.read_uint32()? != 0)
            .then(|| self.read_network_resource_sha())
            .transpose()?;
        let filename = self.read_network_filename()?;
        let author = self.read_network_filename()?;

        Ok(NetworkResourceCore {
            resource_type,
            id,
            derived_id,
            loadable,
            file_size,
            file_crc,
            chunk_size,
            contents_crc,
            file_sha,
            filename,
            author,
        })
    }

    fn read_network_resource_sha(&mut self) -> Result<[u8; 20], LegacyControlError> {
        // StdHexAdapt first compiles the raw digest and then, without returning,
        // compiles 20 NUL-terminated two-digit strings. On read, those strings
        // overwrite the raw bytes (src/StdAdaptors.h:1029-1050).
        self.read_bytes(20)?;
        let mut digest = [0; 20];
        for byte in &mut digest {
            let encoded = self.read_c_string()?;
            let [high, low] = encoded.as_bytes() else {
                return Err(LegacyControlError::InvalidResourceSha);
            };
            let high = decode_hex_nibble(*high).ok_or(LegacyControlError::InvalidResourceSha)?;
            let low = decode_hex_nibble(*low).ok_or(LegacyControlError::InvalidResourceSha)?;
            *byte = (high << 4) | low;
        }
        Ok(digest)
    }

    pub(crate) fn read_uint32(&mut self) -> Result<u32, LegacyControlError> {
        let mut value = 0u32;
        for shift in (0..32).step_by(7).take(MAX_VARINT_BYTES) {
            let byte = self.read_u8()?;
            value |= u32::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(LegacyControlError::VarintOverflow)
    }

    fn read_c4_id(&mut self) -> Result<[u8; 4], LegacyControlError> {
        let value = self.read_c_string()?;
        Ok(value
            .as_bytes()
            .try_into()
            .map(normalize_c4_id_text)
            .unwrap_or(*b"NONE"))
    }

    pub(crate) fn read_int32(&mut self) -> Result<i32, LegacyControlError> {
        let mut tmp = self.read_u8()? as i32;
        let mut bytes_read = 1;
        let mut val = clear_upper_i32(tmp);
        let mut data = val;
        let mut shift = 7;

        while (data as u8) != tmp as u8 {
            if bytes_read >= MAX_VARINT_BYTES {
                return Err(LegacyControlError::VarintOverflow);
            }
            tmp = self.read_u8()? as i32;
            bytes_read += 1;
            data = clear_upper_i32(tmp);
            let lower_mask = if shift >= 63 {
                -1i64
            } else {
                (1i64 << shift) - 1
            };
            let preserved = (val as i64) & lower_mask;
            let combined = ((data as i64) << shift) | preserved;
            val = combined as i32;
            shift += 7;
        }

        Ok(val)
    }

    pub(crate) fn read_raw_i32(&mut self) -> Result<i32, LegacyControlError> {
        let end = self
            .offset
            .checked_add(size_of::<i32>())
            .ok_or(LegacyControlError::UnexpectedEof)?;
        let bytes = self
            .data
            .get(self.offset..end)
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(LegacyControlError::UnexpectedEof)?;
        self.offset = end;
        Ok(i32::from_ne_bytes(bytes))
    }

    pub(crate) fn read_raw_u32(&mut self) -> Result<u32, LegacyControlError> {
        let bytes = self.read_bytes(size_of::<u32>())?;
        let bytes = bytes
            .try_into()
            .map_err(|_| LegacyControlError::UnexpectedEof)?;
        Ok(u32::from_ne_bytes(bytes))
    }

    pub(crate) fn read_raw_u16(&mut self) -> Result<u16, LegacyControlError> {
        let bytes = self.read_bytes(size_of::<u16>())?;
        let bytes = bytes
            .try_into()
            .map_err(|_| LegacyControlError::UnexpectedEof)?;
        Ok(u16::from_ne_bytes(bytes))
    }
}

fn clear_upper_i32(value: i32) -> i32 {
    (value << 25) >> 25
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn encode_int32(mut value: i32) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let chunk = clear_upper_i32(value);
        if chunk == value {
            bytes.push(chunk as u8);
            break;
        } else {
            bytes.push((chunk ^ 0x80) as u8);
            value >>= 7;
        }
    }
    bytes
}

pub(crate) fn append_int32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend(encode_int32(value));
}

pub(crate) fn append_uint32(buffer: &mut Vec<u8>, mut value: u32) {
    loop {
        let chunk = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            buffer.push(chunk);
            break;
        }
        buffer.push(chunk | 0x80);
    }
}

pub(crate) fn append_raw_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend(value.to_ne_bytes());
}

pub(crate) fn append_raw_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend(value.to_ne_bytes());
}

pub(crate) fn append_raw_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend(value.to_ne_bytes());
}

pub(crate) fn append_c_string(buffer: &mut Vec<u8>, value: &LegacyCString) {
    buffer.extend_from_slice(value.as_bytes());
    buffer.push(0);
}

/// `C4IDAdapt` decompiles the integer ID through `GetC4IdText` before the
/// binary compiler writes its NUL-terminated representation
/// (`src/C4Id.cpp:27-48`, `src/C4Id.h:127-147`).
pub(crate) fn append_c4_id(buffer: &mut Vec<u8>, value: &[u8; 4]) {
    let numeric = u32::from_ne_bytes(*value);
    if numeric == 0 || value == b"0000" {
        buffer.extend_from_slice(b"NONE");
    } else if numeric <= 9_999 {
        buffer.extend_from_slice(format!("{numeric:04}").as_bytes());
    } else {
        buffer.extend(value.iter().copied().take_while(|byte| *byte != 0));
    }
    buffer.push(0);
}

pub(crate) fn normalize_c4_id_text(value: [u8; 4]) -> [u8; 4] {
    if value == *b"0000" {
        *b"NONE"
    } else {
        value
    }
}

fn append_network_filename(buffer: &mut Vec<u8>, filename: &LegacyCString) {
    #[cfg(windows)]
    buffer.extend_from_slice(filename.as_bytes());
    #[cfg(not(windows))]
    buffer.extend(
        filename
            .as_bytes()
            .iter()
            .map(|byte| if *byte == b'/' { b'\\' } else { *byte }),
    );
    buffer.push(0);
}

fn encode_player_info(
    buffer: &mut Vec<u8>,
    data: &PlayerInfoControlData,
) -> Result<(), LegacyEncodeError> {
    buffer.push(CID_PLR_INFO);
    encode_player_info_contents(buffer, data.client_id, data.flags, &data.players)?;
    append_int32(buffer, data.by_client);
    Ok(())
}

fn encode_player_info_contents(
    buffer: &mut Vec<u8>,
    client_id: i32,
    flags: u32,
    players: &[ControlPlayerInfoEntry],
) -> Result<(), LegacyEncodeError> {
    let player_count = i32::try_from(players.len())
        .ok()
        .filter(|count| *count <= MAX_PLAYER_INFO_COUNT)
        .ok_or(LegacyEncodeError::PlayerInfoCountOutOfRange(players.len()))?;
    if let Some(player) = players.iter().find(|player| {
        player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 && player.resource.is_none()
    }) {
        return Err(LegacyEncodeError::MissingPlayerInfoResource(player.id));
    }
    for resource in players.iter().filter_map(|player| {
        (player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0)
            .then_some(player.resource.as_ref())
            .flatten()
    }) {
        validate_network_resource_core(resource)?;
    }

    append_raw_i32(buffer, client_id);
    append_raw_u32(buffer, flags);
    append_int32(buffer, player_count);
    for player in players {
        encode_player_info_entry(buffer, player)?;
    }
    Ok(())
}

fn encode_player_info_entry(
    buffer: &mut Vec<u8>,
    player: &ControlPlayerInfoEntry,
) -> Result<(), LegacyEncodeError> {
    let flags = player.flags & PLAYER_INFO_SYNC_FLAGS;
    append_c_string(buffer, &player.name);
    append_c_string(buffer, &player.forced_name);
    append_c_string(buffer, &player.filename);
    append_raw_u16(buffer, flags);
    append_raw_i32(buffer, player.id);
    buffer.push(player.player_type);
    append_raw_u32(buffer, player.color);
    append_raw_u32(buffer, player.original_color);
    append_int32(buffer, player.savegame_player);
    append_int32(buffer, player.team);
    append_c_string(buffer, &player.auth_id);
    if flags & PLAYER_INFO_FLAG_JOINED != 0 {
        append_raw_i32(buffer, player.game_number);
        append_raw_i32(buffer, player.game_join_frame);
    }
    if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
        append_raw_i32(buffer, player.game_part_frame);
    }
    append_c4_id(buffer, &player.extra_data);
    append_c_string(buffer, &player.league_account);
    append_int32(buffer, player.league_score);
    append_int32(buffer, player.league_rank);
    append_int32(buffer, player.league_rank_symbol);
    append_int32(buffer, player.league_projected_gain);
    append_c_string(buffer, &player.clan_tag);
    append_int32(buffer, player.league_performance);
    append_c_string(buffer, &player.league_progress_data);
    if flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
        let resource = player
            .resource
            .as_ref()
            .ok_or(LegacyEncodeError::MissingPlayerInfoResource(player.id))?;
        encode_network_resource_core(buffer, resource);
    }
    Ok(())
}

fn encode_join_player(
    buffer: &mut Vec<u8>,
    data: &JoinPlayerControlData,
) -> Result<(), LegacyEncodeError> {
    enum PreparedSource<'a> {
        Embedded(&'a [u8], u32),
        Resource(&'a NetworkResourceCore),
    }

    let source = match &data.source {
        JoinPlayerSource::Embedded(player_data) => PreparedSource::Embedded(
            player_data,
            u32::try_from(player_data.len())
                .map_err(|_| LegacyEncodeError::PlayerDataTooLarge(player_data.len()))?,
        ),
        JoinPlayerSource::Resource(resource) => {
            validate_network_resource_core(resource)?;
            PreparedSource::Resource(resource)
        }
    };

    buffer.push(CID_JOIN_PLR);
    append_network_filename(buffer, &data.filename);
    append_int32(buffer, data.at_client);
    append_int32(buffer, data.info_id);
    match source {
        PreparedSource::Embedded(player_data, player_data_len) => {
            buffer.push(0);
            append_uint32(buffer, player_data_len);
            buffer.extend_from_slice(player_data);
        }
        PreparedSource::Resource(resource) => {
            buffer.push(1);
            encode_network_resource_core(buffer, resource);
        }
    }
    append_int32(buffer, data.by_client);
    Ok(())
}

pub(crate) fn encode_network_resource_core(buffer: &mut Vec<u8>, resource: &NetworkResourceCore) {
    buffer.push(resource.resource_type);
    append_raw_i32(buffer, resource.id);
    append_raw_i32(buffer, resource.derived_id);
    buffer.push(u8::from(resource.loadable));
    if resource.loadable {
        append_raw_u32(buffer, resource.file_size);
        append_raw_u32(buffer, resource.file_crc);
        append_raw_u32(buffer, resource.chunk_size);
    }
    append_raw_u32(buffer, resource.contents_crc);
    if let Some(file_sha) = resource.file_sha {
        append_uint32(buffer, 1);
        buffer.extend_from_slice(&file_sha);
        for byte in file_sha {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            buffer.push(HEX[usize::from(byte >> 4)]);
            buffer.push(HEX[usize::from(byte & 0x0f)]);
            buffer.push(0);
        }
    } else {
        append_uint32(buffer, 0);
    }
    append_network_filename(buffer, &resource.filename);
    append_network_filename(buffer, &resource.author);
}

pub(crate) fn validate_network_resource_core(
    resource: &NetworkResourceCore,
) -> Result<(), LegacyEncodeError> {
    if resource.loadable && resource.chunk_size == 0 {
        Err(LegacyEncodeError::ZeroResourceChunkSize)
    } else {
        Ok(())
    }
}

fn encode_player_control(buffer: &mut Vec<u8>, data: &PlayerControlData) {
    buffer.push(CID_PLR_CONTROL);
    append_int32(buffer, data.player);
    append_int32(buffer, data.command);
    append_int32(buffer, data.data);
    append_int32(buffer, data.by_client);
}

fn encode_player_select(
    buffer: &mut Vec<u8>,
    data: &PlayerSelectControlData,
) -> Result<(), LegacyEncodeError> {
    let object_count = i32::try_from(data.objects.len())
        .map_err(|_| LegacyEncodeError::PlayerSelectObjectCountTooLarge(data.objects.len()))?;
    buffer.push(CID_PLR_SELECT);
    append_raw_i32(buffer, data.player);
    append_raw_i32(buffer, object_count);
    for object in &data.objects {
        append_raw_i32(buffer, *object);
    }
    append_int32(buffer, data.by_client);
    Ok(())
}

fn encode_player_command(buffer: &mut Vec<u8>, data: &PlayerCommandControlData) {
    buffer.push(CID_PLR_COMMAND);
    append_int32(buffer, data.player);
    append_int32(buffer, data.command);
    append_raw_i32(buffer, data.x);
    append_raw_i32(buffer, data.y);
    append_raw_i32(buffer, data.target);
    append_raw_i32(buffer, data.target2);
    append_raw_i32(buffer, data.data);
    append_int32(buffer, data.add_mode);
    append_int32(buffer, data.by_client);
}

fn encode_message(buffer: &mut Vec<u8>, data: &MessageControlData) {
    buffer.push(CID_MESSAGE);
    buffer.push(data.message_type);
    append_int32(buffer, data.player);
    if data.message_type == MESSAGE_TYPE_PRIVATE {
        append_int32(buffer, data.to_player);
    }
    append_c_string(buffer, &data.message);
    append_int32(buffer, data.by_client);
}

fn encode_em_move_object(
    buffer: &mut Vec<u8>,
    data: &EmMoveObjectControlData,
) -> Result<(), LegacyEncodeError> {
    let object_count = i32::try_from(data.objects.len())
        .map_err(|_| LegacyEncodeError::EmMoveObjectCountTooLarge(data.objects.len()))?;
    buffer.push(CID_EM_MOVE_OBJECT);
    buffer.push(data.action);
    append_raw_i32(buffer, data.tx);
    append_raw_i32(buffer, data.ty);
    append_raw_i32(buffer, data.target_object);
    append_int32(buffer, object_count);
    buffer.push(data.strictness.raw());
    for object in &data.objects {
        append_raw_i32(buffer, *object);
    }
    if data.action == EMMO_SCRIPT {
        append_c_string(buffer, &data.script);
    }
    append_int32(buffer, data.by_client);
    Ok(())
}

fn encode_em_draw_tool(buffer: &mut Vec<u8>, data: &EmDrawToolControlData) {
    buffer.push(CID_EM_DRAW_TOOL);
    buffer.push(data.action);
    append_int32(buffer, data.mode);
    append_raw_i32(buffer, data.x);
    append_raw_i32(buffer, data.y);
    append_raw_i32(buffer, data.x2);
    append_raw_i32(buffer, data.y2);
    append_int32(buffer, data.grade);
    buffer.push(u8::from(data.ift));
    append_c_string(buffer, &data.material);
    append_c_string(buffer, &data.texture);
    append_int32(buffer, data.by_client);
}

fn encode_em_drop_def(buffer: &mut Vec<u8>, data: &EmDropDefControlData) {
    buffer.push(CID_EM_DROP_DEF);
    append_c4_id(buffer, &data.id);
    append_int32(buffer, data.x);
    append_int32(buffer, data.y);
    append_int32(buffer, data.by_client);
}

fn encode_init_scenario_player(buffer: &mut Vec<u8>, data: &InitScenarioPlayerControlData) {
    buffer.push(CID_INIT_SCENARIO_PLAYER);
    append_int32(buffer, data.team);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_surrender_player(buffer: &mut Vec<u8>, data: &SurrenderPlayerControlData) {
    buffer.push(CID_SURRENDER_PLAYER);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_vote(buffer: &mut Vec<u8>, data: &VoteControlData) {
    encode_vote_data(buffer, CID_VOTE, data);
}

fn encode_vote_end(buffer: &mut Vec<u8>, data: &VoteControlData) {
    encode_vote_data(buffer, CID_VOTE_END, data);
}

fn encode_vote_data(buffer: &mut Vec<u8>, id: u8, data: &VoteControlData) {
    buffer.push(id);
    buffer.push(data.vote_type);
    buffer.push(u8::from(data.approve));
    append_raw_i32(buffer, data.data);
    append_int32(buffer, data.by_client);
}

fn encode_client_update(buffer: &mut Vec<u8>, data: &ClientUpdateControlData) {
    buffer.push(CID_CLIENT_UPDATE);
    buffer.push(data.update_type);
    append_int32(buffer, data.client_id);
    if data.update_type == CLIENT_UPDATE_ACTIVATE {
        append_int32(buffer, data.data);
    }
    append_int32(buffer, data.by_client);
}

fn encode_client_join(buffer: &mut Vec<u8>, data: &ClientJoinControlData) {
    buffer.push(CID_CLIENT_JOIN);
    append_raw_i32(buffer, data.core.client_id);
    buffer.push(u8::from(data.core.activated));
    buffer.push(u8::from(data.core.observer));
    append_c_string(buffer, &data.core.name);
    append_c_string(buffer, &data.core.nick);
    buffer.push(u8::from(data.core.lobby_ready));
    append_int32(buffer, data.by_client);
}

fn encode_client_remove(buffer: &mut Vec<u8>, data: &ClientRemoveControlData) {
    buffer.push(CID_CLIENT_REMOVE);
    append_int32(buffer, data.client_id);
    append_c_string(buffer, &data.reason);
    append_int32(buffer, data.by_client);
}

fn encode_sync_check(buffer: &mut Vec<u8>, data: &SyncCheckPacket) {
    buffer.push(CID_SYNC_CHECK);
    append_int32(buffer, data.frame);
    append_int32(buffer, data.control_tick);
    append_int32(buffer, data.random3);
    append_raw_i32(buffer, data.random_count);
    append_int32(buffer, data.crew_positions_sum);
    append_int32(buffer, data.pxs_count);
    append_raw_i32(buffer, data.mass_mover_index);
    append_int32(buffer, data.object_count);
    append_int32(buffer, data.object_enumeration_index);
    append_int32(buffer, data.sector_shape_sum);
    append_int32(buffer, data.by_client);
}

fn encode_synchronize(buffer: &mut Vec<u8>, data: &SynchronizeControlData) {
    buffer.push(CID_SYNCHRONIZE);
    buffer.push(u8::from(data.save_player_files));
    buffer.push(u8::from(data.sync_clearance));
    append_int32(buffer, data.by_client);
}

fn encode_control_set(buffer: &mut Vec<u8>, data: LegacyControlSet) {
    buffer.push(CID_SET);
    append_raw_i32(buffer, data.value_type);
    append_int32(buffer, data.data);
    append_int32(buffer, data.by_client);
}

fn encode_debug_record(
    buffer: &mut Vec<u8>,
    data: &DebugRecordControlData,
) -> Result<(), LegacyEncodeError> {
    let size = u32::try_from(data.data.len())
        .map_err(|_| LegacyEncodeError::DebugRecordTooLarge(data.data.len()))?;
    buffer.push(CID_DEBUG_RECORD);
    append_uint32(buffer, size);
    buffer.extend_from_slice(&data.data);
    Ok(())
}

fn encode_remove_player(buffer: &mut Vec<u8>, data: &RemovePlayerControlData) {
    buffer.push(CID_REMOVE_PLR);
    append_int32(buffer, data.player);
    buffer.push(u8::from(data.disconnected));
    append_int32(buffer, data.by_client);
}

fn encode_script(buffer: &mut Vec<u8>, data: &ScriptControlData) {
    buffer.push(CID_SCRIPT);
    append_raw_i32(buffer, data.target_object);
    buffer.push(data.strictness.raw());
    append_c_string(buffer, &data.script);
    append_int32(buffer, data.by_client);
}

fn encode_message_board_answer(buffer: &mut Vec<u8>, data: &MessageBoardAnswerControlData) {
    buffer.push(CID_MESSAGE_BOARD_ANSWER);
    append_int32(buffer, data.object);
    append_c_string(buffer, &data.answer);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_custom_command(buffer: &mut Vec<u8>, data: &CustomCommandControlData) {
    buffer.push(CID_CUSTOM_COMMAND);
    append_c_string(buffer, &data.command);
    append_c_string(buffer, &data.argument);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_activate_game_goal_menu(buffer: &mut Vec<u8>, data: &ActivateGameGoalMenuControlData) {
    buffer.push(CID_ACTIVATE_GAME_GOAL_MENU);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_toggle_hostility(buffer: &mut Vec<u8>, data: &ToggleHostilityControlData) {
    buffer.push(CID_TOGGLE_HOSTILITY);
    append_int32(buffer, data.opponent);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_activate_game_goal_rule(buffer: &mut Vec<u8>, data: &ActivateGameGoalRuleControlData) {
    buffer.push(CID_ACTIVATE_GAME_GOAL_RULE);
    append_int32(buffer, data.object);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_set_player_team(buffer: &mut Vec<u8>, data: &SetPlayerTeamControlData) {
    buffer.push(CID_SET_PLAYER_TEAM);
    append_int32(buffer, data.team);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_eliminate_player(buffer: &mut Vec<u8>, data: &EliminatePlayerControlData) {
    buffer.push(CID_ELIMINATE_PLAYER);
    append_int32(buffer, data.player);
    append_int32(buffer, data.by_client);
}

fn encode_controls(
    controls: &[EngineControlPacket],
    buffer: &mut Vec<u8>,
) -> Result<(), LegacyEncodeError> {
    for control in controls {
        encode_control(control, buffer)?;
    }
    Ok(())
}

/// Encode a binary `C4Control` list, including its final `PID_None` byte.
///
/// This is the exact payload written for an `RCT_Ctrl` record chunk. It does
/// not contain a `C4GameControlPacket` client ID or control tick.
pub fn encode_control_list_payload(
    controls: &[EngineControlPacket],
) -> Result<Vec<u8>, LegacyEncodeError> {
    let mut payload = Vec::new();
    encode_controls(controls, &mut payload)?;
    payload.push(PID_NONE);
    Ok(payload)
}

fn encode_control(
    control: &EngineControlPacket,
    buffer: &mut Vec<u8>,
) -> Result<(), LegacyEncodeError> {
    match control {
        EngineControlPacket::ClientJoin(data) => {
            encode_client_join(buffer, data);
            Ok(())
        }
        EngineControlPacket::ClientUpdate(data) => {
            encode_client_update(buffer, data);
            Ok(())
        }
        EngineControlPacket::ClientRemove(data) => {
            encode_client_remove(buffer, data);
            Ok(())
        }
        EngineControlPacket::Set(data) => {
            encode_control_set(buffer, (*data).into());
            Ok(())
        }
        EngineControlPacket::DebugRecord(data) => encode_debug_record(buffer, data),
        EngineControlPacket::PlayerInfo(data) => encode_player_info(buffer, data),
        EngineControlPacket::JoinPlayer(data) => encode_join_player(buffer, data),
        EngineControlPacket::RemovePlayer(data) => {
            encode_remove_player(buffer, data);
            Ok(())
        }
        EngineControlPacket::PlayerSelect(data) => encode_player_select(buffer, data),
        EngineControlPacket::PlayerControl(data) => {
            encode_player_control(buffer, data);
            Ok(())
        }
        EngineControlPacket::PlayerCommand(data) => {
            encode_player_command(buffer, data);
            Ok(())
        }
        EngineControlPacket::Message(data) => {
            encode_message(buffer, data);
            Ok(())
        }
        EngineControlPacket::EmMoveObject(data) => encode_em_move_object(buffer, data),
        EngineControlPacket::EmDrawTool(data) => {
            encode_em_draw_tool(buffer, data);
            Ok(())
        }
        EngineControlPacket::EmDropDef(data) => {
            encode_em_drop_def(buffer, data);
            Ok(())
        }
        EngineControlPacket::InitScenarioPlayer(data) => {
            encode_init_scenario_player(buffer, data);
            Ok(())
        }
        EngineControlPacket::SurrenderPlayer(data) => {
            encode_surrender_player(buffer, data);
            Ok(())
        }
        EngineControlPacket::Vote(data) => {
            encode_vote(buffer, data);
            Ok(())
        }
        EngineControlPacket::VoteEnd(data) => {
            encode_vote_end(buffer, data);
            Ok(())
        }
        EngineControlPacket::SyncCheck(data) => {
            encode_sync_check(buffer, data);
            Ok(())
        }
        EngineControlPacket::Synchronize(data) => {
            encode_synchronize(buffer, data);
            Ok(())
        }
        EngineControlPacket::Script(data) => {
            encode_script(buffer, data);
            Ok(())
        }
        EngineControlPacket::MessageBoardAnswer(data) => {
            encode_message_board_answer(buffer, data);
            Ok(())
        }
        EngineControlPacket::CustomCommand(data) => {
            encode_custom_command(buffer, data);
            Ok(())
        }
        EngineControlPacket::ActivateGameGoalMenu(data) => {
            encode_activate_game_goal_menu(buffer, data);
            Ok(())
        }
        EngineControlPacket::ToggleHostility(data) => {
            encode_toggle_hostility(buffer, data);
            Ok(())
        }
        EngineControlPacket::ActivateGameGoalRule(data) => {
            encode_activate_game_goal_rule(buffer, data);
            Ok(())
        }
        EngineControlPacket::SetPlayerTeam(data) => {
            encode_set_player_team(buffer, data);
            Ok(())
        }
        EngineControlPacket::EliminatePlayer(data) => {
            encode_eliminate_player(buffer, data);
            Ok(())
        }
        EngineControlPacket::Unknown { .. } => Err(LegacyEncodeError::UnsupportedPacket),
    }
}

/// Encode the `C4IDPacket` body carried by `PID_ControlPkt`; transport framing
/// writes the delivery byte separately (src/C4Network2IO.cpp:1787-1793).
pub fn encode_control_entry_payload(
    control: &EngineControlPacket,
) -> Result<Vec<u8>, LegacyEncodeError> {
    let mut payload = Vec::new();
    encode_control(control, &mut payload)?;
    Ok(payload)
}

/// Encode one binary `CID_InitScenarioPlayer` C4IDPacket body.
pub fn encode_init_scenario_player_control_entry_payload(
    control: &InitScenarioPlayerControlData,
) -> Vec<u8> {
    let mut payload = Vec::new();
    encode_init_scenario_player(&mut payload, control);
    payload
}

pub fn encode_player_info_update_payload(
    request: &PlayerInfoUpdateRequest,
) -> Result<Vec<u8>, LegacyEncodeError> {
    let mut payload = Vec::new();
    encode_player_info_contents(
        &mut payload,
        request.client_id,
        request.flags,
        &request.players,
    )?;
    Ok(payload)
}

/// Encode a complete `C4GameControlPacket` body for C++ codec-oracle fixtures.
/// Live transport uses [`encode_control_packet`], whose payload contains only
/// the terminated `C4Control` list.
pub fn encode_control_payload(frame: &LegacyControlFrame) -> Result<Vec<u8>, LegacyEncodeError> {
    let client_id = if frame.client_id == BROADCAST_CLIENT_ID {
        // C4GameControlNetwork::PackCompleteCtrl writes C4ClientIDAll (-1) for
        // the merged packet (src/C4GameControlNetwork.cpp:759-768).
        -1
    } else {
        i32::try_from(frame.client_id)
            .map_err(|_| LegacyEncodeError::ClientIdOutOfRange(frame.client_id))?
    };
    let tick =
        i32::try_from(frame.tick).map_err(|_| LegacyEncodeError::TickOutOfRange(frame.tick))?;
    let mut payload = Vec::new();
    append_int32(&mut payload, client_id);
    append_int32(&mut payload, tick);
    payload.extend(encode_control_list_payload(&frame.controls)?);
    Ok(payload)
}

pub fn encode_control_packet(
    frame: &LegacyControlFrame,
) -> Result<ControlPacket, LegacyEncodeError> {
    if frame.client_id != BROADCAST_CLIENT_ID {
        i32::try_from(frame.client_id)
            .map_err(|_| LegacyEncodeError::ClientIdOutOfRange(frame.client_id))?;
    }
    i32::try_from(frame.tick).map_err(|_| LegacyEncodeError::TickOutOfRange(frame.tick))?;
    let payload = encode_control_list_payload(&frame.controls)?;
    Ok(ControlPacket::builder(frame.client_id, frame.tick)
        .timestamp_ms(frame.timestamp_ms)
        .payload(payload))
}

/// Merge a coordinator-ready batch into the one complete control packet used
/// by the legacy lockstep protocol.
///
/// `C4GameControlNetwork::PackCompleteCtrl` waits for every client, marks the
/// result as `C4ClientIDAll`, and appends each client's control list in client
/// ID order (src/C4GameControlNetwork.cpp:741-777). Envelope validation strips
/// each decoded list terminator and any following packet extension bytes.
pub fn aggregate_ready_batch(batch: &ReadyBatch) -> Result<ControlPacket, LegacyAggregateError> {
    aggregate_control_packets_for_tick(batch.tick(), batch.packets())
}

pub(crate) fn aggregate_control_packets_for_tick(
    tick: Tick,
    packets: &[ControlPacket],
) -> Result<ControlPacket, LegacyAggregateError> {
    let mut packets = packets.iter().collect::<Vec<_>>();
    packets.sort_by_key(|packet| packet.client_id());

    let mut timestamp_ms = 0;
    let mut control_body = Vec::new();
    for packet in packets {
        if packet.tick() != tick {
            return Err(LegacyAggregateError::TickMismatch {
                client_id: packet.client_id(),
                tick,
                packet_tick: packet.tick(),
            });
        }
        let envelope =
            validate_control_envelope(packet).map_err(|source| LegacyAggregateError::Decode {
                client_id: packet.client_id(),
                source,
            })?;
        if envelope.tick != tick {
            return Err(LegacyAggregateError::TickMismatch {
                client_id: envelope.client_id,
                tick,
                packet_tick: envelope.tick,
            });
        }
        timestamp_ms = timestamp_ms.max(packet.timestamp_ms());
        control_body.extend_from_slice(envelope.control_body);
    }

    i32::try_from(tick)
        .map_err(|_| LegacyAggregateError::Encode(LegacyEncodeError::TickOutOfRange(tick)))?;
    let mut payload = control_body;
    payload.push(PID_NONE);
    Ok(ControlPacket::builder(BROADCAST_CLIENT_ID, tick)
        .timestamp_ms(timestamp_ms)
        .payload(payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_scenario_player_entry_matches_cpp_field_order_and_defaults() {
        // C4ControlInitScenarioPlayer writes Team before inherited Plr and
        // ByClient, with naming defaults 0, -1, and -1 respectively
        // (src/C4Control.cpp:1684-1688,1566-1570,53-57).
        assert_eq!(
            InitScenarioPlayerControlData::default(),
            InitScenarioPlayerControlData {
                team: 0,
                player: -1,
                by_client: -1,
            }
        );

        let control = InitScenarioPlayerControlData {
            team: 130,
            player: -4,
            by_client: 7,
        };
        let encoded = encode_init_scenario_player_control_entry_payload(&control);
        assert_eq!(encoded, [0xd2, 0x82, 0x01, 0xfc, 0x07]);
        assert_eq!(
            decode_init_scenario_player_control_entry_payload(&encoded),
            Ok(control)
        );
    }

    #[test]
    fn init_scenario_player_uses_general_control_codec() {
        // C4Player::DoTeamSelection queues CID_InitScenarioPlayer into the
        // ordinary synchronized C4Control list (src/C4Player.cpp:1775-1780).
        let expected = EngineControlPacket::InitScenarioPlayer(InitScenarioPlayerControlData {
            team: 130,
            player: -4,
            by_client: 7,
        });
        let encoded = [0xd2, 0x82, 0x01, 0xfc, 0x07];

        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected));
    }

    #[test]
    fn internal_player_script_entries_match_cpp_packed_field_order() {
        let cases = [
            (
                EngineControlPacket::ActivateGameGoalMenu(ActivateGameGoalMenuControlData {
                    player: -4,
                    by_client: 7,
                }),
                vec![0xd3, 0xfc, 0x07],
            ),
            (
                EngineControlPacket::ToggleHostility(ToggleHostilityControlData {
                    opponent: 130,
                    player: -4,
                    by_client: 7,
                }),
                vec![0xd4, 0x82, 0x01, 0xfc, 0x07],
            ),
            (
                EngineControlPacket::ActivateGameGoalRule(ActivateGameGoalRuleControlData {
                    object: 130,
                    player: -4,
                    by_client: 7,
                }),
                vec![0xd6, 0x82, 0x01, 0xfc, 0x07],
            ),
            (
                EngineControlPacket::SetPlayerTeam(SetPlayerTeamControlData {
                    team: 130,
                    player: -4,
                    by_client: 7,
                }),
                vec![0xd7, 0x82, 0x01, 0xfc, 0x07],
            ),
            (
                EngineControlPacket::EliminatePlayer(EliminatePlayerControlData {
                    player: -4,
                    by_client: 7,
                }),
                vec![0xd8, 0xfc, 0x07],
            ),
        ];

        for (control, encoded) in cases {
            assert_eq!(encode_control_entry_payload(&control), Ok(encoded.clone()));
            assert_eq!(decode_control_entry_payload(&encoded), Ok(control));
        }
    }

    #[test]
    fn internal_player_script_entries_reject_every_truncated_body() {
        for complete in [
            &[0xd3, 0xfc, 0x07][..],
            &[0xd4, 0x82, 0x01, 0xfc, 0x07][..],
            &[0xd6, 0x82, 0x01, 0xfc, 0x07][..],
            &[0xd7, 0x82, 0x01, 0xfc, 0x07][..],
            &[0xd8, 0xfc, 0x07][..],
        ] {
            for end in 1..complete.len() {
                assert_eq!(
                    decode_control_entry_payload(&complete[..end]),
                    Err(LegacyControlError::UnexpectedEof),
                    "unexpected result for {:02x?}",
                    &complete[..end]
                );
            }
        }
    }

    #[test]
    fn em_move_object_script_entry_matches_cpp_mixed_raw_and_packed_order() {
        // C4ControlEMMoveObject writes raw Action/tx/ty/TargetObj, packed
        // ObjectNum, raw Strict and object numbers, conditional Script, then
        // inherited packed ByClient (src/C4Control.cpp:972-992,53-57).
        let expected = EngineControlPacket::EmMoveObject(EmMoveObjectControlData {
            action: EMMO_SCRIPT,
            tx: 0x1122_3344,
            ty: -2,
            target_object: 0x0102_0304,
            objects: vec![130, -4],
            strictness: ScriptStrictness::Strict2,
            script: LegacyCString::from_bytes(b"SetX();\x80".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 130,
        });
        let encoded = [
            0xb0, 0x03, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x04, 0x03, 0x02, 0x01,
            0x02, 0x02, 0x82, 0x00, 0x00, 0x00, 0xfc, 0xff, 0xff, 0xff, b'S', b'e', b't', b'X',
            b'(', b')', b';', 0x80, 0x00, 0x82, 0x01,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn em_draw_tool_entry_matches_cpp_mixed_raw_packed_and_string_order() {
        // C4ControlEMDrawTool writes raw Action, packed Mode, four raw
        // coordinates, packed Grade, native bool IFT, two NUL-terminated
        // strings, then inherited packed ByClient (src/C4Control.cpp:
        // 1056-1069,53-57).
        let expected = EngineControlPacket::EmDrawTool(EmDrawToolControlData {
            action: clonk_engine::EMDT_RECT,
            mode: 130,
            x: 0x1122_3344,
            y: -2,
            x2: 0x0102_0304,
            y2: -4,
            grade: 130,
            ift: true,
            material: LegacyCString::from_bytes(b"Earth\x80".to_vec())
                .expect("fixture is NUL-free"),
            texture: LegacyCString::from_bytes(b"Rough".to_vec()).expect("fixture is NUL-free"),
            by_client: -4,
        });
        let encoded = [
            0xb1, 0x04, 0x82, 0x01, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x04, 0x03,
            0x02, 0x01, 0xfc, 0xff, 0xff, 0xff, 0x82, 0x01, 0x01, b'E', b'a', b'r', b't', b'h',
            0x80, 0x00, b'R', b'o', b'u', b'g', b'h', 0x00, 0xfc,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );

        let EngineControlPacket::EmDrawTool(mut unknown) = expected else {
            unreachable!("fixture variant is known")
        };
        unknown.action = u8::MAX;
        let unknown = EngineControlPacket::EmDrawTool(unknown);
        let encoded_unknown =
            encode_control_entry_payload(&unknown).expect("unknown action encodes");
        assert_eq!(encoded_unknown[1], u8::MAX);
        assert_eq!(decode_control_entry_payload(&encoded_unknown), Ok(unknown));
    }

    #[test]
    fn em_draw_tool_entry_rejects_every_truncated_body() {
        let complete = [
            0xb1, 0x04, 0x82, 0x01, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x04, 0x03,
            0x02, 0x01, 0xfc, 0xff, 0xff, 0xff, 0x82, 0x01, 0x01, b'E', b'a', b'r', b't', b'h',
            0x80, 0x00, b'R', b'o', b'u', b'g', b'h', 0x00, 0xfc,
        ];

        for end in 1..complete.len() {
            assert_eq!(
                decode_control_entry_payload(&complete[..end]),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {:02x?}",
                &complete[..end]
            );
        }
    }

    #[test]
    fn em_drop_def_entry_matches_cpp_c4id_and_packed_integer_order() {
        let expected = EngineControlPacket::EmDropDef(EmDropDefControlData {
            id: *b"HUT2",
            x: -130,
            y: 130,
            by_client: 7,
        });
        let encoded = [
            0xb2, b'H', b'U', b'T', b'2', 0x00, 0x7e, 0xfe, 0x82, 0x01, 0x07,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );

        let default = EngineControlPacket::EmDropDef(EmDropDefControlData::default());
        assert_eq!(
            encode_control_entry_payload(&default),
            Ok(vec![0xb2, b'N', b'O', b'N', b'E', 0x00, 0x00, 0x00, 0xff])
        );
        assert_eq!(
            decode_control_entry_payload(&[0xb2, b'A', 0x00, 0x00, 0x00, 0xff]),
            Ok(default)
        );
    }

    #[test]
    fn em_drop_def_entry_rejects_truncation_and_overlong_c4ids() {
        let complete = [
            0xb2, b'H', b'U', b'T', b'2', 0x00, 0x7e, 0xfe, 0x82, 0x01, 0x07,
        ];
        for end in 1..complete.len() {
            assert_eq!(
                decode_control_entry_payload(&complete[..end]),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {:02x?}",
                &complete[..end]
            );
        }

        assert_eq!(
            decode_control_entry_payload(&[
                0xb2, b'A', b'B', b'C', b'D', b'E', 0x00, 0x00, 0x00, 0xff,
            ]),
            Err(LegacyControlError::ControlC4IdTooLong(5))
        );
    }

    #[test]
    fn em_move_object_keeps_unknown_action_raw_and_omits_non_script_string() {
        let canonical = EngineControlPacket::EmMoveObject(EmMoveObjectControlData {
            action: u8::MAX,
            ..EmMoveObjectControlData::default()
        });
        let encoded = [
            0xb0, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xff, 0xff, 0x00, 0x03, 0xff,
        ];
        assert_eq!(
            decode_control_entry_payload(&encoded),
            Ok(canonical.clone())
        );
        assert_eq!(
            encode_control_entry_payload(&canonical),
            Ok(encoded.to_vec())
        );

        let with_unused_script = EngineControlPacket::EmMoveObject(EmMoveObjectControlData {
            action: u8::MAX,
            script: LegacyCString::from_bytes(b"ignored".to_vec()).expect("fixture is NUL-free"),
            ..EmMoveObjectControlData::default()
        });
        assert_eq!(
            encode_control_entry_payload(&with_unused_script),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn em_move_object_rejects_invalid_count_after_strictness_check() {
        let mut encoded = vec![CID_EM_MOVE_OBJECT, clonk_engine::EMMO_MOVE];
        encoded.extend(0_i32.to_ne_bytes());
        encoded.extend(0_i32.to_ne_bytes());
        encoded.extend((-1_i32).to_ne_bytes());
        encoded.push(0xff); // packed ObjectNum = -1
        encoded.push(4); // invalid Strict

        assert_eq!(
            decode_control_entry_payload(&encoded),
            Err(LegacyControlError::InvalidScriptStrictness(4))
        );
        *encoded.last_mut().expect("strictness byte") = 3;
        assert_eq!(
            decode_control_entry_payload(&encoded),
            Err(LegacyControlError::EmMoveObjectCountOutOfRange(-1))
        );
    }

    #[test]
    fn em_move_object_rejects_every_truncated_script_body() {
        let complete = [
            0xb0, 0x03, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x04, 0x03, 0x02, 0x01,
            0x02, 0x02, 0x82, 0x00, 0x00, 0x00, 0xfc, 0xff, 0xff, 0xff, b'S', 0x00, 0x82, 0x01,
        ];
        for end in 1..complete.len() {
            assert_eq!(
                decode_control_entry_payload(&complete[..end]),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {:02x?}",
                &complete[..end]
            );
        }
    }

    #[test]
    fn player_select_entry_matches_cpp_raw_array_and_packed_author_layout() {
        // C4ControlPlayerSelect writes raw native-endian Player/ObjCnt/Objs,
        // then the inherited signed IntPack ByClient
        // (src/C4Control.cpp:370-380,53-57).
        let expected = EngineControlPacket::PlayerSelect(PlayerSelectControlData {
            player: 7,
            objects: vec![0x0102_0304, -2],
            by_client: 3,
        });
        let encoded = [
            0xa0, 0x07, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x04, 0x03, 0x02, 0x01, 0xfe,
            0xff, 0xff, 0xff, 0x03,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );

        let empty = EngineControlPacket::PlayerSelect(PlayerSelectControlData {
            player: -1,
            objects: Vec::new(),
            by_client: -1,
        });
        let empty_bytes = [0xa0, 0xff, 0xff, 0xff, 0xff, 0, 0, 0, 0, 0xff];
        assert_eq!(
            decode_control_entry_payload(&empty_bytes),
            Ok(empty.clone())
        );
        assert_eq!(
            encode_control_entry_payload(&empty),
            Ok(empty_bytes.to_vec())
        );
    }

    #[test]
    fn player_select_rejects_a_negative_raw_object_count() {
        let mut encoded = vec![CID_PLR_SELECT];
        encoded.extend(7_i32.to_ne_bytes());
        encoded.extend((-1_i32).to_ne_bytes());
        assert_eq!(
            decode_control_entry_payload(&encoded),
            Err(LegacyControlError::PlayerSelectObjectCountOutOfRange(-1))
        );
    }

    #[test]
    fn player_command_entry_matches_cpp_mixed_packed_and_raw_layout() {
        // C4ControlPlayerCommand::CompileFunc uses packed Plr/Cmd/AddMode and
        // inherited ByClient, but native int32 for X/Y/Target/Target2/Data
        // (src/C4Control.cpp:428-438,53-57).
        let expected = EngineControlPacket::PlayerCommand(PlayerCommandControlData {
            player: -4,
            command: 130,
            x: 0x1122_3344,
            y: -2,
            target: 0x0102_0304,
            target2: -1,
            data: 0x5566_7788,
            add_mode: 4,
            by_client: 7,
        });
        let encoded = [
            0xa2, 0xfc, 0x82, 0x01, 0x44, 0x33, 0x22, 0x11, 0xfe, 0xff, 0xff, 0xff, 0x04, 0x03,
            0x02, 0x01, 0xff, 0xff, 0xff, 0xff, 0x88, 0x77, 0x66, 0x55, 0x04, 0x07,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn message_entries_match_cpp_conditional_packed_layout() {
        // C4ControlMessage writes raw uint8 Type, packed Player, and only for
        // Private writes packed ToPlayer, followed by Message and ByClient
        // (src/C4Control.cpp:1252-1260,53-57).
        let normal = EngineControlPacket::Message(MessageControlData {
            message_type: clonk_engine::MESSAGE_TYPE_NORMAL,
            player: -4,
            to_player: -1,
            message: LegacyCString::from_bytes(b"hi\x80".to_vec()).expect("fixture is NUL-free"),
            by_client: 130,
        });
        let normal_bytes = [0xa3, 0x00, 0xfc, b'h', b'i', 0x80, 0x00, 0x82, 0x01];
        assert_eq!(
            decode_control_entry_payload(&normal_bytes),
            Ok(normal.clone())
        );
        assert_eq!(
            encode_control_entry_payload(&normal),
            Ok(normal_bytes.to_vec())
        );

        let private = EngineControlPacket::Message(MessageControlData {
            message_type: MESSAGE_TYPE_PRIVATE,
            player: 130,
            to_player: -4,
            message: LegacyCString::from_bytes(b"secret".to_vec()).expect("fixture is NUL-free"),
            by_client: 7,
        });
        let private_bytes = [
            0xa3, 0x04, 0x82, 0x01, 0xfc, b's', b'e', b'c', b'r', b'e', b't', 0x00, 0x07,
        ];
        assert_eq!(
            decode_control_entry_payload(&private_bytes),
            Ok(private.clone())
        );
        assert_eq!(
            encode_control_entry_payload(&private),
            Ok(private_bytes.to_vec())
        );
    }

    #[test]
    fn message_unknown_raw_type_roundtrips_and_keeps_following_control_aligned() {
        // The binary enum is adapted through uint8_t without range
        // validation. Unknown types have the non-Private layout and survive
        // forwarding unchanged.
        let message = EngineControlPacket::Message(MessageControlData {
            message_type: 0x7f,
            player: 3,
            to_player: -1,
            message: LegacyCString::from_bytes(vec![0xfe, 0x81]).expect("fixture is NUL-free"),
            by_client: 4,
        });
        let message_bytes = [0xa3, 0x7f, 0x03, 0xfe, 0x81, 0x00, 0x04];
        assert_eq!(
            decode_control_entry_payload(&message_bytes),
            Ok(message.clone())
        );
        assert_eq!(
            encode_control_entry_payload(&message),
            Ok(message_bytes.to_vec())
        );

        let following = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 5,
            command: 6,
            data: 7,
            by_client: 4,
        });
        let frame = LegacyControlFrame {
            client_id: 4,
            tick: 9,
            timestamp_ms: 0,
            controls: vec![message, following],
        };
        let encoded = encode_control_payload(&frame).expect("mixed control list encodes");
        assert_eq!(
            decode_control_payload(&encoded)
                .expect("unknown message type leaves the following control aligned")
                .controls,
            frame.controls
        );
    }

    #[test]
    fn message_rejects_truncated_fields_and_accepts_trailing_data() {
        for encoded in [
            &[0xa3][..],
            &[0xa3, 0x00][..],
            &[0xa3, 0x04, 0x00][..],
            &[0xa3, 0x04, 0x00, 0x00][..],
            &[0xa3, 0x04, 0x00, 0x00, b'x'][..],
            &[0xa3, 0x04, 0x00, 0x00, 0x00][..],
        ] {
            assert_eq!(
                decode_control_entry_payload(encoded),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {encoded:02x?}"
            );
        }

        let with_trailing_byte = [0xa3, 0x00, 0x00, b'x', 0x00, 0x00, 0xaa];
        assert_eq!(
            decode_control_entry_payload(&with_trailing_byte),
            decode_control_entry_payload(&with_trailing_byte[..with_trailing_byte.len() - 1])
        );
    }

    #[test]
    fn script_entry_matches_cpp_raw_fields_and_packed_author() {
        // C4ControlScript writes native int32 TargetObj, raw uint8 Strict,
        // a NUL-terminated byte string, then inherited packed ByClient
        // (src/C4Control.cpp:315-326,53-57).
        let expected = EngineControlPacket::Script(ScriptControlData {
            target_object: -2,
            strictness: ScriptStrictness::Strict3,
            script: clonk_engine::LegacyCString::from_bytes(b"1+2\x80".to_vec())
                .expect("fixture is NUL-free"),
            by_client: 130,
        });
        let encoded = [
            0x88, 0xfe, 0xff, 0xff, 0xff, 0x03, b'1', b'+', b'2', 0x80, 0x00, 0x82, 0x01,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn script_entry_rejects_invalid_strictness_before_reading_script() {
        // CheckStrictness runs before C++ compiles Script, so an invalid byte
        // is reported even when the packet ends immediately afterwards.
        let encoded = [0x88, 0xff, 0xff, 0xff, 0xff, 0x04];
        assert_eq!(
            decode_control_entry_payload(&encoded),
            Err(LegacyControlError::InvalidScriptStrictness(4))
        );
    }

    #[test]
    fn script_entry_rejects_unterminated_script() {
        let encoded = [0x88, 0xff, 0xff, 0xff, 0xff, 0x03, b'1', b'+', b'2'];
        assert_eq!(
            decode_control_entry_payload(&encoded),
            Err(LegacyControlError::UnexpectedEof)
        );
    }

    #[test]
    fn message_board_answer_entry_matches_cpp_packed_field_order() {
        // C4ControlMessageBoardAnswer writes packed Object, a NUL-terminated
        // Answer, inherited packed Plr, then inherited packed ByClient.
        let expected = EngineControlPacket::MessageBoardAnswer(MessageBoardAnswerControlData {
            object: 130,
            answer: LegacyCString::from_bytes(b"a\"\\b".to_vec()).expect("fixture is NUL-free"),
            player: -4,
            by_client: 7,
        });
        let encoded = [0xd0, 0x82, 0x01, b'a', b'"', b'\\', b'b', 0x00, 0xfc, 0x07];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn message_board_answer_preserves_non_utf8_answer_bytes() {
        let expected = EngineControlPacket::MessageBoardAnswer(MessageBoardAnswerControlData {
            object: 0,
            answer: LegacyCString::from_bytes(vec![0x80, 0xff]).expect("fixture is NUL-free"),
            player: -1,
            by_client: 130,
        });
        let encoded = [0xd0, 0x00, 0x80, 0xff, 0x00, 0xff, 0x82, 0x01];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn message_board_answer_rejects_each_truncated_field() {
        for encoded in [
            &[0xd0][..],
            &[0xd0, 0x80][..],
            &[0xd0, 0x00, b'a'][..],
            &[0xd0, 0x00, 0x00][..],
            &[0xd0, 0x00, 0x00, 0x00][..],
        ] {
            assert_eq!(
                decode_control_entry_payload(encoded),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {encoded:02x?}"
            );
        }
    }

    #[test]
    fn custom_command_entry_matches_cpp_packed_field_order() {
        // C4ControlCustomCommand writes NUL-terminated Command and Argument,
        // inherited packed Plr, then inherited packed ByClient.
        let expected = EngineControlPacket::CustomCommand(CustomCommandControlData {
            command: LegacyCString::from_bytes(b"push".to_vec()).expect("fixture is NUL-free"),
            argument: LegacyCString::from_bytes(b"+130".to_vec()).expect("fixture is NUL-free"),
            player: -4,
            by_client: 7,
        });
        let encoded = [
            0xd1, b'p', b'u', b's', b'h', 0x00, b'+', b'1', b'3', b'0', 0x00, 0xfc, 0x07,
        ];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn custom_command_preserves_non_utf8_string_bytes() {
        let expected = EngineControlPacket::CustomCommand(CustomCommandControlData {
            command: LegacyCString::from_bytes(vec![0x80, 0xff]).expect("fixture is NUL-free"),
            argument: LegacyCString::from_bytes(vec![0xfe, 0x81]).expect("fixture is NUL-free"),
            player: -1,
            by_client: 130,
        });
        let encoded = [0xd1, 0x80, 0xff, 0x00, 0xfe, 0x81, 0x00, 0xff, 0x82, 0x01];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn custom_command_rejects_each_truncated_field() {
        for encoded in [
            &[0xd1][..],
            &[0xd1, b'p'][..],
            &[0xd1, b'p', 0x00][..],
            &[0xd1, b'p', 0x00, b'a'][..],
            &[0xd1, b'p', 0x00, b'a', 0x00][..],
            &[0xd1, b'p', 0x00, b'a', 0x00, 0x80][..],
            &[0xd1, b'p', 0x00, b'a', 0x00, 0x00][..],
        ] {
            assert_eq!(
                decode_control_entry_payload(encoded),
                Err(LegacyControlError::UnexpectedEof),
                "unexpected result for {encoded:02x?}"
            );
        }
    }

    #[test]
    fn remove_player_uses_cpp_control_codec() {
        // C4ControlRemovePlr writes signed IntPack Plr, one native bool byte,
        // then inherited signed IntPack ByClient (C4Control.cpp:1290-1305).
        let expected = EngineControlPacket::RemovePlayer(RemovePlayerControlData {
            player: 130,
            disconnected: true,
            by_client: 7,
        });
        let encoded = [0x92, 0x82, 0x01, 0x01, 0x07];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn surrender_player_uses_cpp_control_codec() {
        // C4MainMenu queues CID_SurrenderPlayer (0xd5) through CDT_Queue.
        // C4ControlSurrenderPlayer serializes inherited packed Plr followed
        // by inherited packed ByClient (pristine 9ffa0a5d
        // src/C4MainMenu.cpp:790-795; src/C4Control.cpp:1566-1570,53-57;
        // src/C4PacketBase.h:181).
        let expected =
            EngineControlPacket::SurrenderPlayer(clonk_engine::SurrenderPlayerControlData {
                player: -4,
                by_client: 7,
            });
        let encoded = [0xd5, 0xfc, 0x07];

        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected));
    }

    #[test]
    fn vote_entry_matches_cpp_field_order() {
        // CID_Vote (0x83) writes raw uint8 Type, raw bool Approve, native
        // int32 Data, then inherited packed ByClient (pristine 9ffa0a5d
        // src/C4PacketBase.h:151; src/C4Control.cpp:1446-1451,53-57).
        let expected = EngineControlPacket::Vote(clonk_engine::VoteControlData {
            vote_type: 1,
            approve: true,
            data: 7,
            by_client: 7,
        });
        let encoded = [0x83, 0x01, 0x01, 0x07, 0x00, 0x00, 0x00, 0x07];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn vote_end_entry_matches_cpp_field_order() {
        // CID_VoteEnd (0x84) delegates to the identical C4ControlVote body
        // compiler (pristine 9ffa0a5d src/C4PacketBase.h:152;
        // src/C4Control.cpp:1517-1520,1446-1451,53-57).
        let expected = EngineControlPacket::VoteEnd(clonk_engine::VoteControlData {
            vote_type: 1,
            approve: true,
            data: 7,
            by_client: 0,
        });
        let encoded = [0x84, 0x01, 0x01, 0x07, 0x00, 0x00, 0x00, 0x00];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    #[test]
    fn vote_entry_round_trips_unknown_raw_type() {
        // mkIntAdaptT<uint8_t> serializes the enum storage byte directly and
        // performs no range validation (pristine 9ffa0a5d
        // src/C4Control.cpp:1446-1451), so unknown values are wire-stable.
        let expected = EngineControlPacket::Vote(clonk_engine::VoteControlData {
            vote_type: 0xfe,
            approve: false,
            data: 7,
            by_client: 7,
        });
        let encoded = [0x83, 0xfe, 0x00, 0x07, 0x00, 0x00, 0x00, 0x07];

        assert_eq!(decode_control_entry_payload(&encoded), Ok(expected.clone()));
        assert_eq!(
            encode_control_entry_payload(&expected),
            Ok(encoded.to_vec())
        );
    }

    fn decode_test_hex(value: &str) -> Vec<u8> {
        value
            .split_ascii_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).expect("valid fixture byte"))
            .collect()
    }

    fn minimal_join_game_parameters() -> JoinGameParametersEnvelope {
        let empty_players = PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: Vec::new(),
        };
        JoinGameParametersEnvelope {
            random_seed: 0,
            startup_player_count: 0,
            max_players: 8,
            use_fair_crew: false,
            fair_crew_forced: false,
            fair_crew_strength: 0,
            allow_debug: true,
            is_network_game: true,
            control_rate: 1,
            auto_frame_skip: false,
            rules: Vec::new(),
            goals: Vec::new(),
            league: LegacyCString::default(),
            league_address: LegacyCString::default(),
            title: LegacyCString::from_bytes(b"No title".to_vec()).unwrap(),
            scenario: NetworkResourceCore::default(),
            game_resources: Vec::new(),
            player_infos: empty_players.clone(),
            restore_player_infos: empty_players,
            teams: JoinTeamListSnapshot {
                active: 1,
                custom: 0,
                allow_hostility_change: 1,
                allow_team_switch: 0,
                auto_generate_teams: 1,
                last_team_id: 0,
                team_distribution: 0,
                team_colors: 0,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: Vec::new(),
            },
            clients: JoinClientRegistrySnapshot {
                clients: Vec::new(),
                local_client_id: None,
            },
        }
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn complete_join_data_vector_matches_read_only_cpp_schema_audit() {
        // Fixed packet derived by walking the untouched C++ serializers. Raw
        // scalars make this vector little-endian-specific; packed fields and
        // NUL strings are platform-independent (see C4PacketJoinData,
        // C4GameParameters and their nested CompileFunc implementations).
        let packet = decode_test_hex(
            "15 03 11 02 01 02 17 00 00 00 ff ff ff ff 00 78 56 34 12 00 \
             44 79 6e 61 6d 69 63 2e 63 34 64 00 54 79 6c 65 72 00 04 03 \
             02 01 02 00 00 00 04 00 00 00 01 00 64 00 00 00 01 01 03 00 \
             00 00 00 01 00 00 00 43 4e 4d 54 00 07 00 00 00 01 00 00 00 \
             4d 45 4c 45 00 03 00 00 00 00 00 46 69 78 74 75 72 65 00 01 \
             07 00 00 00 ff ff ff ff 00 f0 de bc 9a 00 53 63 65 6e 61 72 69 \
             6f 2e 63 34 73 00 00 01 04 1f 00 00 00 ff ff ff ff 00 e0 ac 68 \
             24 00 48 6f 73 74 5c 44 65 66 69 6e 69 74 69 6f 6e 73 00 54 \
             79 6c 65 72 00 00 00 00 00 00 00 00 00 00 00 01 00 01 00 00 \
             00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00",
        );
        assert_eq!(packet.len(), 199);
        assert_eq!(packet[0], 0x15);

        let envelope = decode_join_data_envelope(&packet[1..]).expect("JoinData decodes");
        assert_eq!((envelope.client_id, envelope.start_control_tick), (3, 17));
        assert_eq!(envelope.dynamic.id, 23);
        assert_eq!(envelope.dynamic.filename.as_bytes(), b"Dynamic.c4d");
        let parameters = &envelope.parameters;
        assert_eq!(parameters.random_seed, 0x0102_0304);
        assert_eq!(
            (parameters.startup_player_count, parameters.max_players),
            (2, 4)
        );
        assert_eq!(parameters.rules[0].id.as_bytes(), b"CNMT");
        assert_eq!(parameters.goals[0].id.as_bytes(), b"MELE");
        assert_eq!(parameters.title.as_bytes(), b"Fixture");
        assert_eq!(parameters.scenario.id, 7);
        assert_eq!(parameters.game_resources[0].id, 31);
        assert_eq!(
            parameters.game_resources[0].filename.as_bytes(),
            b"Host/Definitions"
        );
        assert_eq!(parameters.teams.auto_generate_teams, 1);

        let mut reencoded_packet = vec![0x15];
        reencoded_packet.extend(encode_join_data_envelope(&envelope).unwrap());
        // The wire fixture deliberately starts with an empty team list whose
        // AutoGenerateTeams byte is false. The C++ compiler normalizes that
        // byte to true after reading (C4Teams.cpp:605-610), so its next write
        // differs at the team-list field exactly as Rust does.
        let mut cpp_normalized_packet = packet;
        cpp_normalized_packet[181] = 1;
        assert_eq!(reencoded_packet, cpp_normalized_packet);
    }

    #[test]
    fn join_data_prefix_matches_cpp_field_order_and_encodings() {
        // C4PacketJoinData serializes packed client/tick, reference-form
        // status, then C4Network2ResCore (src/C4Network2IO.cpp:1683-1692;
        // src/C4Network2.cpp:108-123; src/C4Network2Res.cpp:109-136).
        let mut bytes = vec![3, 17, 2, 1, 2];
        bytes.extend_from_slice(&23_i32.to_ne_bytes());
        bytes.extend_from_slice(&(-1_i32).to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0x1234_5678_u32.to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(b"Dynamic.c4s\0Host\0");
        let parameters = minimal_join_game_parameters();
        bytes.extend(encode_join_game_parameters_envelope(&parameters).unwrap());

        let envelope = decode_join_data_envelope(&bytes).expect("JoinData prefix decodes");

        assert_eq!((envelope.client_id, envelope.start_control_tick), (3, 17));
        assert_eq!(
            (
                envelope.status.state,
                envelope.status.control_mode,
                envelope.status.target_tick,
            ),
            (2, 1, -1)
        );
        assert_eq!(
            (envelope.dynamic.resource_type, envelope.dynamic.id),
            (2, 23)
        );
        assert_eq!(envelope.dynamic.contents_crc, 0x1234_5678);
        assert_eq!(envelope.dynamic.filename.as_bytes(), b"Dynamic.c4s");
        assert_eq!(envelope.dynamic.author.as_bytes(), b"Host");
        assert_eq!(envelope.parameters, parameters);
        assert_eq!(encode_join_data_envelope(&envelope).unwrap(), bytes);
    }

    #[test]
    fn join_game_parameters_prefix_matches_cpp_field_order_and_encodings() {
        // Exact C4GameParameters prefix through C4GameResList; the remaining
        // registries stay opaque (src/C4GameParameters.cpp:555-587;
        // src/C4IDList.cpp:39-51; src/C4Network2Res.cpp:109-136).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&12_345_i32.to_ne_bytes());
        bytes.extend_from_slice(&0_i32.to_ne_bytes());
        bytes.extend_from_slice(&12_i32.to_ne_bytes());
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&0_i32.to_ne_bytes());
        bytes.extend_from_slice(&[1, 1]);
        bytes.extend_from_slice(&2_i32.to_ne_bytes());
        bytes.push(1);
        bytes.extend_from_slice(&1_i32.to_ne_bytes());
        bytes.extend_from_slice(b"CNMT\0");
        bytes.extend_from_slice(&7_i32.to_ne_bytes());
        bytes.extend_from_slice(&1_i32.to_ne_bytes());
        bytes.extend_from_slice(b"MELE\0");
        bytes.extend_from_slice(&3_i32.to_ne_bytes());
        bytes.extend_from_slice(b"\0\0Envelope Test\0");
        bytes.push(1);
        bytes.extend_from_slice(&7_i32.to_ne_bytes());
        bytes.extend_from_slice(&(-1_i32).to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0x9abc_def0_u32.to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(b"Arena.c4s\0Host\0");
        bytes.push(1);
        bytes.push(4);
        bytes.extend_from_slice(&31_i32.to_ne_bytes());
        bytes.extend_from_slice(&(-1_i32).to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&0x2468_ace0_u32.to_ne_bytes());
        bytes.push(0);
        bytes.extend_from_slice(b"Definitions.c4d\0Host\\Definitions\0");
        // Empty PlayerInfos, RestorePlayerInfos, Teams and Clients in the
        // exact C4GameParameters::CompileFunc order (C4GameParameters.cpp:584-590).
        bytes.extend_from_slice(&[0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[
            1, 0, 1, 0, 1, // team-list bools
            0, 0, 0, 0, // LastTeamID
            2, 1, // TeamDistribution, TeamColors
            0, 0, 0, 0, // MaxScriptPlayers
            0, // ScriptPlayerNames
            0, 0, 0, 0, // RandomTeamCount
            0, // packed team count
            0, // packed client count
        ]);

        let parameters =
            decode_join_game_parameters_envelope(&bytes).expect("C4GameParameters prefix decodes");

        assert_eq!(parameters.random_seed, 12_345);
        assert_eq!(parameters.max_players, 12);
        assert_eq!(parameters.rules[0].id.as_bytes(), b"CNMT");
        assert_eq!(parameters.rules[0].count, 7);
        assert_eq!(parameters.goals[0].id.as_bytes(), b"MELE");
        assert_eq!(parameters.goals[0].count, 3);
        assert_eq!(parameters.title.as_bytes(), b"Envelope Test");
        assert_eq!(parameters.scenario.filename.as_bytes(), b"Arena.c4s");
        assert_eq!(parameters.game_resources[0].id, 31);
        assert_eq!(
            parameters.game_resources[0].author.as_bytes(),
            b"Host/Definitions"
        );
        assert_eq!(parameters.player_infos.last_player_id, 0);
        assert!(parameters.player_infos.clients.is_empty());
        assert_eq!(parameters.restore_player_infos.last_player_id, 0);
        assert!(parameters.restore_player_infos.clients.is_empty());
        assert_eq!(parameters.teams.team_distribution, 2);
        assert!(parameters.teams.teams.is_empty());
        assert!(parameters.clients.clients.is_empty());
        assert_eq!(
            encode_join_game_parameters_envelope(&parameters).unwrap(),
            bytes
        );

        let mut invalid = parameters.clone();
        invalid.scenario.loadable = true;
        invalid.scenario.chunk_size = 0;
        assert_eq!(
            encode_join_game_parameters_envelope(&invalid),
            Err(LegacyEncodeError::ZeroResourceChunkSize)
        );

        let mut invalid = parameters;
        invalid.game_resources[0].loadable = true;
        invalid.game_resources[0].chunk_size = 0;
        assert_eq!(
            encode_join_game_parameters_envelope(&invalid),
            Err(LegacyEncodeError::ZeroResourceChunkSize)
        );
    }

    fn build_payload(client: i32, tick: i32, controls: &[[i32; 4]]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend(super::encode_int32(client));
        payload.extend(super::encode_int32(tick));
        payload.extend(build_control_list(controls));
        payload
    }

    fn build_control_list(controls: &[[i32; 4]]) -> Vec<u8> {
        let mut payload = Vec::new();
        for control in controls {
            let data = PlayerControlData {
                player: control[0],
                command: control[1],
                data: control[2],
                by_client: control[3],
            };
            let mut encoded = Vec::new();
            super::encode_player_control(&mut encoded, &data);
            payload.extend(encoded);
        }
        payload.push(PID_NONE);
        payload
    }

    #[test]
    fn client_join_round_trip_matches_cpp_direct_control_body() {
        // C4ControlClientJoin compiles C4ClientCore followed by ByClient:
        // raw ID, bools, NUL strings, bool, signed packed author
        // (src/C4Client.cpp:75-83; src/C4Control.cpp:570-573).
        let mut payload = vec![0x80];
        payload.extend_from_slice(&3i32.to_ne_bytes());
        payload.extend_from_slice(&[
            0, 0, b'A', b'l', b'i', b'c', b'e', 0, b'A', b'l', b'i', 0, 0, 0,
        ]);

        let control = decode_control_entry_payload(&payload).expect("decode ClientJoin");
        assert_eq!(
            control,
            EngineControlPacket::ClientJoin(clonk_engine::ClientJoinControlData {
                core: clonk_engine::ClientCoreControlData {
                    client_id: 3,
                    activated: false,
                    observer: false,
                    name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                    nick: LegacyCString::from_bytes(b"Ali".to_vec()).unwrap(),
                    lobby_ready: false,
                },
                by_client: 0,
            })
        );
        assert_eq!(
            encode_control_entry_payload(&control).expect("encode ClientJoin"),
            payload
        );
    }

    #[test]
    fn client_join_decode_normalizes_cpp_validated_names_before_reencode() {
        // C4ClientCore compiles Name and Nick through
        // ValidatedStdStrBuf<VAL_NameNoEmpty>. An initially empty value first
        // becomes "empty"; a nonempty value cleaned down to nothing uses the
        // later "Unknown" fallback (src/C4Client.h:40-45;
        // src/C4InputValidation.cpp:39-55,97-118).
        let mut payload = vec![CID_CLIENT_JOIN];
        payload.extend_from_slice(&3_i32.to_ne_bytes());
        payload.extend_from_slice(&[0, 0, 0]);
        payload.extend_from_slice(b" {<i>  </i>{ \0");
        payload.extend_from_slice(&[0, 0]);

        let decoded = decode_control_entry_payload(&payload).expect("decode ClientJoin");
        let EngineControlPacket::ClientJoin(join) = &decoded else {
            panic!("decoded the wrong control variant");
        };
        assert_eq!(join.core.name.as_bytes(), b"empty");
        assert_eq!(join.core.nick.as_bytes(), b"Unknown");

        let mut canonical = vec![CID_CLIENT_JOIN];
        canonical.extend_from_slice(&3_i32.to_ne_bytes());
        canonical.extend_from_slice(&[0, 0]);
        canonical.extend_from_slice(b"empty\0Unknown\0");
        canonical.extend_from_slice(&[0, 0]);
        assert_eq!(
            encode_control_entry_payload(&decoded).expect("reencode normalized ClientJoin"),
            canonical
        );
    }

    #[test]
    fn player_info_decode_normalizes_all_cpp_validated_names_before_reencode() {
        fn c_string(bytes: &[u8]) -> LegacyCString {
            LegacyCString::from_bytes(bytes.to_vec()).expect("fixture is NUL-free")
        }

        // C4PlayerInfo validates these five fields after binary compilation.
        // Use a non-UTF-8 name to prove C4MaxName truncation counts native
        // bytes, while the other values cover brace removal, markup removal,
        // whitespace trimming, and allow-empty behavior
        // (src/C4PlayerInfo.h:85-104; src/C4InputValidation.cpp:97-118).
        let dirty_player = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(vec![0x80; 31]).unwrap(),
            forced_name: c_string(b" {<c ff00ff>A{lice</c>}} "),
            filename: c_string(b"Player.c4p"),
            flags: 0,
            id: 23,
            player_type: 0,
            color: 0x1122_3344,
            original_color: 0x5566_7788,
            savegame_player: 0,
            team: 0,
            auth_id: LegacyCString::default(),
            game_number: -1,
            game_join_frame: -1,
            game_part_frame: -1,
            extra_data: *b"NONE",
            league_account: c_string(b" { \t "),
            league_score: 0,
            league_rank: 0,
            league_rank_symbol: 0,
            league_projected_gain: 0,
            clan_tag: c_string(b"\t<i>Clan</i>\r"),
            league_performance: 0,
            league_progress_data_is_null: false,
            league_progress_data: LegacyCString::default(),
            resource: None,
        };
        let dirty_data = PlayerInfoControlData {
            client_id: 7,
            flags: 0,
            players: vec![dirty_player],
            by_client: 7,
        };
        let dirty = EngineControlPacket::PlayerInfo(dirty_data.clone());
        let wire = encode_control_entry_payload(&dirty).expect("encode dirty PlayerInfo fixture");

        let mut canonical_data = dirty_data;
        canonical_data.players[0].name = LegacyCString::from_bytes(vec![0x80; 30]).unwrap();
        canonical_data.players[0].forced_name = c_string(b"Alice");
        canonical_data.players[0].league_account = LegacyCString::default();
        canonical_data.players[0].clan_tag = c_string(b"Clan");
        let canonical = EngineControlPacket::PlayerInfo(canonical_data.clone());
        let decoded = decode_control_entry_payload(&wire).expect("decode PlayerInfo");

        assert_eq!(decoded, canonical);
        assert_eq!(
            encode_control_entry_payload(&decoded).expect("reencode normalized PlayerInfo"),
            encode_control_entry_payload(&canonical).expect("encode canonical PlayerInfo")
        );
        assert_ne!(
            encode_control_entry_payload(&decoded).unwrap(),
            wire,
            "C++ rewriting persists the post-compile validated values"
        );

        // PID_PlayerInfoUpdReq carries the same C4ClientPlayerInfos body and
        // therefore reaches the same C4PlayerInfo validation path.
        let dirty_update = PlayerInfoUpdateRequest {
            client_id: 7,
            flags: 0,
            players: match dirty {
                EngineControlPacket::PlayerInfo(data) => data.players,
                _ => unreachable!(),
            },
        };
        let update_wire =
            encode_player_info_update_payload(&dirty_update).expect("encode update fixture");
        let decoded_update =
            decode_player_info_update_payload(&update_wire).expect("decode PlayerInfo update");
        assert_eq!(decoded_update.players, canonical_data.players);
    }

    #[test]
    fn synchronize_round_trip_matches_cpp_control_body() {
        // CID_Synchronize writes its two raw bools before the packed base
        // ByClient field (pristine 9ffa0a5d src/C4PacketBase.h:145-156;
        // src/C4Control.cpp:537-550; src/StdCompiler.cpp:104-131).
        let payload = vec![0x86, 1, 1, 0];

        let control = decode_control_entry_payload(&payload).expect("decode Synchronize");

        assert_eq!(
            control,
            EngineControlPacket::Synchronize(clonk_engine::SynchronizeControlData {
                save_player_files: true,
                sync_clearance: true,
                by_client: 0,
            })
        );
        assert_eq!(
            encode_control_entry_payload(&control).expect("encode Synchronize"),
            payload
        );
    }

    #[test]
    fn decodes_single_player_control() {
        let payload = build_payload(2, 42, &[[1, 5, 0, 2]]);
        let frame = decode_control_payload(&payload).expect("decode succeeds");
        assert_eq!(frame.client_id, 2);
        assert_eq!(frame.tick, 42);
        assert_eq!(frame.controls.len(), 1);
        match &frame.controls[0] {
            EngineControlPacket::PlayerControl(data) => {
                assert_eq!(data.player, 1);
                assert_eq!(data.command, 5);
                assert_eq!(data.data, 0);
                assert_eq!(data.by_client, 2);
            }
            other => panic!("unexpected control packet: {other:?}"),
        }
    }

    #[test]
    fn control_set_codec_preserves_fixed_type_packed_data_and_author() {
        let set = LegacyControlSet {
            value_type: 5,
            data: -1,
            by_client: 0,
        };
        let control = set.into_control_packet();
        let encoded = encode_control_entry_payload(&control).expect("encode CID_Set");

        let mut expected = vec![CID_SET];
        expected.extend_from_slice(&5_i32.to_ne_bytes());
        expected.extend(encode_int32(-1));
        expected.extend(encode_int32(0));
        assert_eq!(encoded, expected);

        let decoded = decode_control_entry_payload(&encoded).expect("decode CID_Set");
        assert_eq!(LegacyControlSet::from_control_packet(&decoded), Some(set));
        assert_eq!(decoded, control);
    }

    #[test]
    fn debug_record_codec_preserves_opaque_bytes() {
        let control = EngineControlPacket::DebugRecord(DebugRecordControlData {
            data: vec![0x00, 0xff, 0x40, b'C', b'4'],
        });
        let encoded = encode_control_entry_payload(&control).expect("encode CID_DebugRec");

        let mut expected = vec![CID_DEBUG_RECORD];
        append_uint32(&mut expected, 5);
        expected.extend([0x00, 0xff, 0x40, b'C', b'4']);
        assert_eq!(encoded, expected);

        assert_eq!(
            decode_control_entry_payload(&encoded).expect("decode CID_DebugRec"),
            control
        );
    }

    #[test]
    fn decodes_cpp_embedded_join_player_bytes() {
        // C4ControlJoinPlayer writes Filename, packed AtClient/InfoID, the
        // raw ByRes bool, embedded StdBuf, and packed ByClient in that order;
        // the containing C4PacketList then writes PID_None (0xff)
        // (src/C4Control.cpp:852-863; src/StdBuf.cpp:86-100;
        // src/C4Packet2.cpp:193-220,298-335).
        let payload = vec![
            0x04, 0x40, 0x00, 0x91, b'P', 0x80, 0x00, 0xff, 0x40, 0x00, 0x00, 0x03, 0xaa, 0x00,
            0xcc, 0x04, 0xff,
        ];

        let frame = decode_control_payload(&payload).expect("C++ JoinPlayer bytes decode");

        assert_eq!(frame.client_id, 4);
        assert_eq!(frame.tick, 64);
        assert_eq!(frame.controls.len(), 1);
        match &frame.controls[0] {
            EngineControlPacket::JoinPlayer(join) => {
                assert_eq!(join.filename.as_bytes(), b"P\x80");
                assert_eq!(join.at_client, -1);
                assert_eq!(join.info_id, 64);
                assert_eq!(
                    join.source,
                    clonk_engine::JoinPlayerSource::Embedded(vec![0xaa, 0x00, 0xcc])
                );
                assert_eq!(join.by_client, 4);
            }
            other => panic!("expected JoinPlayer, got {other:?}"),
        }
    }

    #[test]
    fn cpp_control_lists_use_ff_as_pid_none() {
        // C4PacketType::PID_None is 0xff (src/C4PacketBase.h), and the
        // binary C4PacketList writer appends that default C4IDPacket as the
        // list terminator (src/C4Packet2.cpp).
        let frame = LegacyControlFrame {
            client_id: 2,
            tick: 42,
            timestamp_ms: 0,
            controls: Vec::new(),
        };
        let encoded = encode_control_payload(&frame).expect("empty control list encodes");
        assert_eq!(encoded.last(), Some(&0xff));
        assert!(decode_control_payload(&encoded).is_ok());

        let mut zero_terminated = encoded;
        *zero_terminated.last_mut().expect("payload has terminator") = 0x00;
        assert!(matches!(
            decode_control_payload(&zero_terminated),
            Err(LegacyControlError::UnsupportedPacket(0x00))
        ));
    }

    #[test]
    fn rejects_unsupported_packet() {
        let mut payload = build_payload(1, 1, &[]);
        let insert_at = payload
            .len()
            .checked_sub(1)
            .expect("payload includes terminator");
        // CID_Message (0xa3) is supported; 0xa4 is the next genuinely
        // unsupported legacy control entry.
        payload.insert(insert_at, CID_MESSAGE + 1);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::UnsupportedPacket(_)));
    }

    #[test]
    fn decode_uses_packet_metadata_and_list_payload() {
        let payload = build_control_list(&[[5, 64, 0, 5]]);
        let packet = ControlPacket::builder(5, 77)
            .timestamp_ms(1234)
            .payload(payload);
        let frame = decode_control_packet(&packet).expect("packet metadata and list decode");
        assert_eq!(frame.timestamp_ms, 1234);
        assert_eq!(frame.controls.len(), 1);
    }

    #[test]
    fn cloned_control_packet_reuses_one_list_decode_across_validation() {
        let packet = ControlPacket::builder(7, 12).payload(build_control_list(&[[1, 2, 3, 7]]));
        let cloned = packet.clone();
        reset_control_list_decode_passes();

        validate_control_envelope(&packet).unwrap();
        decode_control_packet(&cloned).unwrap();
        validate_control_envelope(&cloned).unwrap();

        assert_eq!(control_list_decode_passes(), 1);
    }

    #[test]
    fn packet_metadata_is_not_repeated_in_control_list() {
        let packet = ControlPacket::builder(4, 10).payload(build_control_list(&[]));
        let frame = decode_control_packet(&packet).expect("empty control list decodes");
        let envelope = validate_control_envelope(&packet).expect("empty envelope validates");

        assert_eq!(packet.payload(), [PID_NONE]);
        assert_eq!((frame.client_id, frame.tick), (4, 10));
        assert_eq!((envelope.client_id, envelope.tick), (4, 10));
        assert!(envelope.control_body.is_empty());
    }

    #[test]
    fn detects_truncated_payload() {
        let mut payload = build_payload(1, 2, &[[1, 2, 3, 4]]);
        payload.pop();
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::UnexpectedEof));
    }

    #[test]
    fn rejects_negative_client_or_tick() {
        let mut payload = build_payload(-2, 5, &[]);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::NegativeClientId(-2)));
        payload = build_payload(1, -2, &[]);
        let error = decode_control_payload(&payload).unwrap_err();
        assert!(matches!(error, LegacyControlError::NegativeTick(-2)));
    }

    #[test]
    fn encode_and_decode_roundtrip() {
        let frame = LegacyControlFrame {
            client_id: 3,
            tick: 12,
            timestamp_ms: 77,
            controls: vec![EngineControlPacket::PlayerControl(PlayerControlData {
                player: 1,
                command: 5,
                data: 0,
                by_client: 3,
            })],
        };
        let packet = encode_control_packet(&frame).expect("encoding succeeds");
        let decoded = decode_control_packet(&packet).expect("decoding succeeds");
        assert_eq!(decoded.client_id, frame.client_id);
        assert_eq!(decoded.tick, frame.tick);
        assert_eq!(decoded.timestamp_ms, frame.timestamp_ms);
        assert_eq!(decoded.controls, frame.controls);
    }

    #[test]
    fn encode_and_decode_sync_check() {
        let sync = SyncCheckPacket {
            frame: 120,
            control_tick: 118,
            random3: 33,
            random_count: 77,
            crew_positions_sum: 1024,
            pxs_count: 45,
            mass_mover_index: 12,
            object_count: 256,
            object_enumeration_index: 512,
            sector_shape_sum: 2048,
            by_client: 5,
        };
        let frame = LegacyControlFrame {
            client_id: 5,
            tick: 120,
            timestamp_ms: 55,
            controls: vec![EngineControlPacket::SyncCheck(sync.clone())],
        };
        let packet = encode_control_packet(&frame).expect("encode succeeds");
        let decoded = decode_control_packet(&packet).expect("decode succeeds");
        assert_eq!(decoded.client_id, frame.client_id);
        assert_eq!(decoded.tick, frame.tick);
        assert_eq!(decoded.timestamp_ms, frame.timestamp_ms);
        assert_eq!(decoded.controls.len(), 1);
        match &decoded.controls[0] {
            EngineControlPacket::SyncCheck(decoded_sync) => {
                assert_eq!(decoded_sync, &sync);
            }
            other => panic!("expected sync check control, got {other:?}"),
        }
    }

    #[test]
    fn sync_check_raw_dwords_match_cpp_binary_compiler() {
        // C++ oracle: C4ControlSyncCheck::CompileFunc packs most fields but
        // writes RandomCount and MassMoverIndex as plain int32 values
        // (src/C4Control.cpp:522-534). StdCompilerBinWrite::DWord writes those
        // four native bytes verbatim (src/StdCompiler.cpp:104-112,125-132).
        let sync = SyncCheckPacket {
            frame: 7,
            control_tick: 8,
            random3: 9,
            random_count: 0x0102_0304,
            crew_positions_sum: 10,
            pxs_count: 11,
            mass_mover_index: 0x1213_1415,
            object_count: 12,
            object_enumeration_index: 13,
            sector_shape_sum: 14,
            by_client: 5,
        };
        let frame = LegacyControlFrame {
            client_id: 5,
            tick: 6,
            timestamp_ms: 0,
            controls: vec![EngineControlPacket::SyncCheck(sync.clone())],
        };

        let mut expected = vec![5, 6, CID_SYNC_CHECK, 7, 8, 9];
        expected.extend(sync.random_count.to_ne_bytes());
        expected.extend([10, 11]);
        expected.extend(sync.mass_mover_index.to_ne_bytes());
        expected.extend([12, 13, 14, 5, PID_NONE]);

        assert_eq!(
            encode_control_payload(&frame).expect("sync check encodes"),
            expected,
            "raw DWord fields must not use packed-int encoding"
        );
        assert_eq!(
            decode_control_payload(&expected)
                .expect("C++ sync-check bytes decode")
                .controls,
            frame.controls
        );
    }

    #[test]
    fn aggregate_complete_control_is_ordered_and_has_one_list_terminator() {
        let host_control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 0,
            command: 2,
            data: 10,
            by_client: 0,
        });
        let client_control = EngineControlPacket::PlayerControl(PlayerControlData {
            player: 1,
            command: 5,
            data: 20,
            by_client: 1,
        });
        let packet = |client_id, timestamp_ms, control| {
            encode_control_packet(&LegacyControlFrame {
                client_id,
                tick: 7,
                timestamp_ms,
                controls: vec![control],
            })
            .expect("per-client control encodes")
        };

        // Deliberately supply client 1 before host 0. PackCompleteCtrl appends
        // controls in client-ID order (src/C4GameControlNetwork.cpp:760-769).
        let complete = aggregate_control_packets_for_tick(
            7,
            &[
                packet(1, 20, client_control.clone()),
                packet(0, 10, host_control.clone()),
            ],
        )
        .expect("ready controls aggregate");

        assert_eq!(complete.client_id(), BROADCAST_CLIENT_ID);
        assert_eq!(complete.tick(), 7);
        assert_eq!(complete.timestamp_ms(), 20);
        assert_eq!(complete.payload().last(), Some(&0xff));
        let decoded = decode_control_packet(&complete)
            .expect("one merged list decodes without trailing per-client data");
        assert_eq!(decoded.client_id, BROADCAST_CLIENT_ID);
        assert_eq!(decoded.controls, vec![host_control, client_control]);
    }

    #[test]
    fn aggregate_rejects_unknown_control_ids_like_cpp_typed_unpack() {
        let packet = ControlPacket::builder(0, 9).payload(vec![0x89, 0x31, PID_NONE]);
        assert!(matches!(
            aggregate_control_packets_for_tick(9, &[packet]),
            Err(LegacyAggregateError::Decode {
                client_id: 0,
                source: LegacyControlError::UnsupportedPacket(0x89),
            })
        ));
    }

    #[test]
    fn aggregate_strips_each_control_packet_trailing_suffix() {
        let control = |player, by_client| {
            EngineControlPacket::PlayerControl(PlayerControlData {
                player,
                command: 5,
                data: 0,
                by_client,
            })
        };
        let packet = |client_id: ClientId, control: EngineControlPacket| {
            let mut payload = encode_control_list_payload(std::slice::from_ref(&control))
                .expect("control list encodes");
            payload.extend_from_slice(&[0xaa, 0xbb]);
            ControlPacket::builder(client_id, 9).payload(payload)
        };
        let host = control(0, 0);
        let client = control(1, 1);

        let complete = aggregate_control_packets_for_tick(
            9,
            &[packet(1, client.clone()), packet(0, host.clone())],
        )
        .expect("trailing packet extensions are discarded before aggregation");

        assert_eq!(
            complete.payload(),
            encode_control_list_payload(&[host.clone(), client.clone()])
                .expect("expected merged list encodes")
        );
        assert_eq!(
            decode_control_packet(&complete)
                .expect("merged list decodes")
                .controls,
            vec![host, client]
        );
    }

    #[test]
    fn envelope_validator_rejects_a_missing_final_list_terminator() {
        let payload = vec![0x8a, 0x7f];
        let packet = ControlPacket::builder(3, 4).payload(payload);

        assert!(validate_control_envelope(&packet).is_err());
        assert!(matches!(
            aggregate_control_packets_for_tick(4, &[packet]),
            Err(LegacyAggregateError::Decode { client_id: 3, .. })
        ));
    }
}
