use clonk_engine::{
    ControlPlayerInfoEntry, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
    PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_TYPE_SCRIPT,
};

use crate::legacy::{
    append_c4_id, append_c_string, append_int32, append_raw_i32, append_raw_u16, append_raw_u32,
    encode_network_resource_core, normalize_c4_id_text, validate_network_resource_core,
    LegacyControlError, LegacyEncodeError, Reader,
};
use crate::name_validation::{validate_name_allow_empty, validate_name_no_empty};

const MAX_PLAYER_INFO_COUNT: i32 = 5_000;
const PLAYER_INFO_SYNC_FLAGS: u16 = 0x7fcd;

/// One complete `C4PlayerInfoList`, used for both `PlayerInfos` and
/// `RestorePlayerInfos` in `C4GameParameters`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayerInfoListSnapshot {
    pub last_player_id: i32,
    pub clients: Vec<ClientPlayerInfosSnapshot>,
}

/// The `C4ClientPlayerInfos` entry owned by one client in a player-info list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientPlayerInfosSnapshot {
    pub client_id: i32,
    pub flags: u32,
    pub players: Vec<ControlPlayerInfoEntry>,
}

pub(crate) fn decode_player_info_list(
    reader: &mut Reader<'_>,
) -> Result<PlayerInfoListSnapshot, LegacyControlError> {
    let last_player_id = reader.read_raw_i32()?;
    let client_count = reader.read_int32()?;
    ensure_decode_count(client_count)?;
    let clients = (0..client_count)
        .map(|_| decode_client_player_infos(reader))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(PlayerInfoListSnapshot {
        last_player_id,
        clients,
    })
}

pub(crate) fn encode_player_info_list(
    buffer: &mut Vec<u8>,
    snapshot: &PlayerInfoListSnapshot,
) -> Result<(), LegacyEncodeError> {
    validate_for_encode(snapshot)?;
    append_raw_i32(buffer, snapshot.last_player_id);
    append_int32(buffer, snapshot.clients.len() as i32);
    for client in &snapshot.clients {
        encode_client_player_infos(buffer, client);
    }
    Ok(())
}

fn decode_client_player_infos(
    reader: &mut Reader<'_>,
) -> Result<ClientPlayerInfosSnapshot, LegacyControlError> {
    let client_id = reader.read_raw_i32()?;
    let flags = reader.read_raw_u32()?;
    let player_count = reader.read_int32()?;
    ensure_decode_count(player_count)?;
    let players = (0..player_count)
        .map(|_| decode_player_info(reader))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ClientPlayerInfosSnapshot {
        client_id,
        flags,
        players,
    })
}

fn decode_player_info(
    reader: &mut Reader<'_>,
) -> Result<ControlPlayerInfoEntry, LegacyControlError> {
    let name = validate_name_no_empty(reader.read_c_string()?);
    let forced_name = validate_name_allow_empty(reader.read_c_string()?);
    let filename = reader.read_c_string()?;
    // Binary readers accept the raw u16. Normal C++ writers mask local-only
    // flags before transmission; preserve malformed/raw input just as C++ does.
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
    let extra_data = decode_player_extra_data(reader)?;
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

fn decode_player_extra_data(reader: &mut Reader<'_>) -> Result<[u8; 4], LegacyControlError> {
    let mut result = [0; 4];
    let mut length = 0;
    loop {
        let byte = reader.read_u8()?;
        if byte == 0 {
            return Ok(if length == result.len() {
                normalize_c4_id_text(result)
            } else {
                *b"NONE"
            });
        }
        if length == result.len() {
            return Err(LegacyControlError::PlayerInfoExtraDataTooLong);
        }
        result[length] = byte;
        length += 1;
    }
}

fn validate_for_encode(snapshot: &PlayerInfoListSnapshot) -> Result<(), LegacyEncodeError> {
    ensure_encode_count(snapshot.clients.len())?;
    for client in &snapshot.clients {
        ensure_encode_count(client.players.len())?;
        for player in &client.players {
            if player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0 {
                let resource = player
                    .resource
                    .as_ref()
                    .ok_or(LegacyEncodeError::MissingPlayerInfoResource(player.id))?;
                validate_network_resource_core(resource)?;
            }
        }
    }
    Ok(())
}

fn encode_client_player_infos(buffer: &mut Vec<u8>, client: &ClientPlayerInfosSnapshot) {
    append_raw_i32(buffer, client.client_id);
    append_raw_u32(buffer, client.flags);
    append_int32(buffer, client.players.len() as i32);
    for player in &client.players {
        encode_player_info(buffer, player);
    }
}

fn encode_player_info(buffer: &mut Vec<u8>, player: &ControlPlayerInfoEntry) {
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
        // Presence and chunk size were checked transactionally before writing.
        if let Some(resource) = &player.resource {
            encode_network_resource_core(buffer, resource);
        }
    }
}

fn ensure_decode_count(count: i32) -> Result<i32, LegacyControlError> {
    (0..=MAX_PLAYER_INFO_COUNT)
        .contains(&count)
        .then_some(count)
        .ok_or(LegacyControlError::PlayerInfoCountOutOfRange(count))
}

fn ensure_encode_count(count: usize) -> Result<i32, LegacyEncodeError> {
    i32::try_from(count)
        .ok()
        .filter(|count| *count <= MAX_PLAYER_INFO_COUNT)
        .ok_or(LegacyEncodeError::PlayerInfoCountOutOfRange(count))
}

#[cfg(test)]
mod tests {
    use clonk_engine::{
        LegacyCString, NetworkResourceCore, PLAYER_INFO_FLAG_HAS_RESOURCE,
        PLAYER_INFO_FLAG_INVISIBLE, PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
    };

    use super::*;

    fn cstring(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).unwrap()
    }

    fn push_c_string(buffer: &mut Vec<u8>, value: &[u8]) {
        buffer.extend_from_slice(value);
        buffer.push(0);
    }

    fn push_raw_i32(buffer: &mut Vec<u8>, value: i32) {
        buffer.extend_from_slice(&value.to_ne_bytes());
    }

    fn push_raw_u32(buffer: &mut Vec<u8>, value: u32) {
        buffer.extend_from_slice(&value.to_ne_bytes());
    }

    fn push_raw_u16(buffer: &mut Vec<u8>, value: u16) {
        buffer.extend_from_slice(&value.to_ne_bytes());
    }

    fn push_resource(buffer: &mut Vec<u8>, chunk_size: u32) {
        buffer.push(3);
        push_raw_i32(buffer, 9);
        push_raw_i32(buffer, -1);
        buffer.push(1);
        push_raw_u32(buffer, 1_234);
        push_raw_u32(buffer, 0xdead_beef);
        push_raw_u32(buffer, chunk_size);
        push_raw_u32(buffer, 0x0102_0304);
        buffer.push(0);
        push_c_string(buffer, b"Alice.ocp");
        push_c_string(buffer, b"Host");
    }

    fn nontrivial_player_info_list_wire(normalized: bool, chunk_size: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_raw_i32(&mut bytes, 42);
        bytes.push(1);
        push_raw_i32(&mut bytes, -7);
        push_raw_u32(&mut bytes, 0xa5a5_0007);
        bytes.push(1);

        push_c_string(
            &mut bytes,
            if normalized {
                b"Alice"
            } else {
                b" {<i>Alice</i>{ "
            },
        );
        push_c_string(&mut bytes, if normalized { b"Ace" } else { b"  Ace  " });
        push_c_string(&mut bytes, b"Player.ocp");
        push_raw_u16(&mut bytes, if normalized { 0x000d } else { 0x403d });
        push_raw_i32(&mut bytes, 101);
        bytes.push(PLAYER_INFO_TYPE_USER);
        push_raw_u32(&mut bytes, 0x0011_2233);
        push_raw_u32(&mut bytes, 0x0044_5566);
        bytes.extend_from_slice(&[0xac, 0x02]);
        bytes.push(0xfe);
        push_c_string(&mut bytes, b"auth");
        push_raw_i32(&mut bytes, 3);
        push_raw_i32(&mut bytes, 400);
        push_raw_i32(&mut bytes, 450);
        push_c_string(&mut bytes, b"AB_1");
        push_c_string(
            &mut bytes,
            if normalized {
                b"League"
            } else {
                b" <i>League</i> "
            },
        );
        bytes.push(0xf9);
        bytes.extend_from_slice(&[0x82, 0x01]);
        bytes.push(4);
        bytes.push(0xff);
        push_c_string(&mut bytes, if normalized { b"TAG}" } else { b" {TAG} " });
        bytes.extend_from_slice(&[0x84, 0x02]);
        push_c_string(&mut bytes, b"p=1");
        push_resource(&mut bytes, chunk_size);
        bytes
    }

    fn minimal_player_info_list_wire(flags: u16, player_type: u8, extra_data: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_raw_i32(&mut bytes, 0);
        bytes.push(1);
        push_raw_i32(&mut bytes, 0);
        push_raw_u32(&mut bytes, 0);
        bytes.push(1);
        push_c_string(&mut bytes, b"Bot");
        push_c_string(&mut bytes, b"");
        push_c_string(&mut bytes, b"");
        push_raw_u16(&mut bytes, flags);
        push_raw_i32(&mut bytes, 0);
        bytes.push(player_type);
        push_raw_u32(&mut bytes, 0);
        push_raw_u32(&mut bytes, 0);
        bytes.push(0);
        bytes.push(0);
        push_c_string(&mut bytes, b"");
        push_c_string(&mut bytes, extra_data);
        push_c_string(&mut bytes, b"");
        bytes.push(0);
        bytes.push(0);
        bytes.push(0);
        bytes.push(0xff);
        push_c_string(&mut bytes, b"");
        bytes.push(0);
        push_c_string(&mut bytes, b"");
        bytes
    }

    #[test]
    fn empty_player_info_list_uses_raw_last_id_and_packed_count() {
        // C4PlayerInfoList::CompileFunc serializes LastPlayerID as a raw i32,
        // followed by a packed client count (src/C4PlayerInfo.cpp:1733-1762).
        let mut bytes = 27_i32.to_ne_bytes().to_vec();
        bytes.push(0);
        let mut reader = Reader::new(&bytes);

        let snapshot = decode_player_info_list(&mut reader).unwrap();

        assert_eq!(
            snapshot,
            PlayerInfoListSnapshot {
                last_player_id: 27,
                clients: Vec::new(),
            }
        );
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn decodes_and_reencodes_nontrivial_cpp_player_registry_layout() {
        // C4PlayerInfoList, C4ClientPlayerInfos and C4PlayerInfo are nested in
        // this exact order (src/C4PlayerInfo.cpp:1733-1762,601-630,177-268).
        let bytes = nontrivial_player_info_list_wire(false, 102_400);
        let mut reader = Reader::new(&bytes);

        let snapshot = decode_player_info_list(&mut reader).unwrap();

        assert_eq!(reader.remaining(), 0);
        assert_eq!(snapshot.last_player_id, 42);
        let [client] = snapshot.clients.as_slice() else {
            panic!("expected one client registry");
        };
        assert_eq!((client.client_id, client.flags), (-7, 0xa5a5_0007));
        let [player] = client.players.as_slice() else {
            panic!("expected one player registry entry");
        };
        assert_eq!(player.name.as_bytes(), b"Alice");
        assert_eq!(player.forced_name.as_bytes(), b"Ace");
        assert_eq!(player.flags, 0x003d);
        assert_eq!((player.savegame_player, player.team), (300, -2));
        assert_eq!((player.game_number, player.game_join_frame), (3, 400));
        assert_eq!(player.game_part_frame, 450);
        assert_eq!(player.extra_data, *b"AB_1");
        assert_eq!(player.league_account.as_bytes(), b"League");
        assert_eq!(player.clan_tag.as_bytes(), b"TAG}");
        assert!(!player.league_progress_data_is_null);
        assert_eq!(player.league_progress_data.as_bytes(), b"p=1");
        assert_eq!(
            (
                player.league_score,
                player.league_rank,
                player.league_performance,
            ),
            (-7, 130, 260)
        );
        assert_eq!(
            player.resource,
            Some(NetworkResourceCore {
                resource_type: 3,
                id: 9,
                derived_id: -1,
                loadable: true,
                file_size: 1_234,
                file_crc: 0xdead_beef,
                chunk_size: 102_400,
                contents_crc: 0x0102_0304,
                file_sha: None,
                filename: cstring(b"Alice.ocp"),
                author: cstring(b"Host"),
            })
        );

        let mut encoded = Vec::new();
        encode_player_info_list(&mut encoded, &snapshot).unwrap();
        assert_eq!(encoded, nontrivial_player_info_list_wire(true, 102_400));
    }

    #[test]
    fn script_invisible_survives_but_regular_invisible_is_cleared() {
        // C4PlayerInfo::CompileFunc clears PIF_Invisible only for non-script
        // players after reading their type (src/C4PlayerInfo.cpp:207-224).
        let script_bytes = minimal_player_info_list_wire(
            PLAYER_INFO_FLAG_INVISIBLE,
            PLAYER_INFO_TYPE_SCRIPT,
            b"NONE",
        );
        let mut script_reader = Reader::new(&script_bytes);
        let script = decode_player_info_list(&mut script_reader).unwrap();
        assert_ne!(
            script.clients[0].players[0].flags & PLAYER_INFO_FLAG_INVISIBLE,
            0
        );

        let user_bytes = minimal_player_info_list_wire(
            PLAYER_INFO_FLAG_INVISIBLE,
            PLAYER_INFO_TYPE_USER,
            b"NONE",
        );
        let mut user_reader = Reader::new(&user_bytes);
        let user = decode_player_info_list(&mut user_reader).unwrap();
        assert_eq!(
            user.clients[0].players[0].flags & PLAYER_INFO_FLAG_INVISIBLE,
            0
        );
    }

    #[test]
    fn player_extra_data_matches_bounded_cpp_c4id_adaptor() {
        // C4IDAdapt reads a NUL-terminated string with maximum length four;
        // shorter values become NONE (src/C4Id.h:127-147;
        // src/StdCompiler.cpp:174-191).
        let mut exact = Reader::new(b"AB_1\0");
        assert_eq!(decode_player_extra_data(&mut exact).unwrap(), *b"AB_1");
        let mut zero = Reader::new(b"0000\0");
        assert_eq!(decode_player_extra_data(&mut zero).unwrap(), *b"NONE");
        let mut short = Reader::new(b"X\0");
        assert_eq!(decode_player_extra_data(&mut short).unwrap(), *b"NONE");
        let mut long = Reader::new(b"ABCDE\0");
        assert_eq!(
            decode_player_extra_data(&mut long),
            Err(LegacyControlError::PlayerInfoExtraDataTooLong)
        );

        let mut encoded = Vec::new();
        append_c4_id(&mut encoded, &[0; 4]);
        assert_eq!(encoded, b"NONE\0");
        encoded.clear();
        append_c4_id(&mut encoded, &[b'A', b'B', 0, b'C']);
        assert_eq!(encoded, b"AB\0");
    }

    #[test]
    fn rejects_cpp_player_and_client_counts_outside_zero_through_5000() {
        // Both nested counts are packed int32 values checked against
        // C4MaxClient/C4MaxPlayer (src/C4PlayerInfo.cpp:618-622,1743-1747;
        // src/C4Player.h:33-34).
        for count in [[0xff].as_slice(), [0x89, 0x27].as_slice()] {
            let mut outer = 0_i32.to_ne_bytes().to_vec();
            outer.extend_from_slice(count);
            let mut reader = Reader::new(&outer);
            assert!(matches!(
                decode_player_info_list(&mut reader),
                Err(LegacyControlError::PlayerInfoCountOutOfRange(_))
            ));

            let mut inner = 0_i32.to_ne_bytes().to_vec();
            inner.push(1);
            push_raw_i32(&mut inner, 0);
            push_raw_u32(&mut inner, 0);
            inner.extend_from_slice(count);
            let mut reader = Reader::new(&inner);
            assert!(matches!(
                decode_player_info_list(&mut reader),
                Err(LegacyControlError::PlayerInfoCountOutOfRange(_))
            ));
        }
        assert!(ensure_decode_count(5_000).is_ok());
        assert!(matches!(
            ensure_decode_count(5_001),
            Err(LegacyControlError::PlayerInfoCountOutOfRange(5_001))
        ));
    }

    #[test]
    fn rejects_zero_chunk_loadable_player_resource_on_decode_and_encode() {
        // C4Network2ResCore rejects a zero chunk size whenever Loadable is set
        // (src/C4Network2Res.cpp:126-136).
        let mut bytes = nontrivial_player_info_list_wire(false, 0);
        let mut reader = Reader::new(&bytes);
        assert_eq!(
            decode_player_info_list(&mut reader),
            Err(LegacyControlError::ZeroResourceChunkSize)
        );

        bytes = nontrivial_player_info_list_wire(false, 102_400);
        let mut reader = Reader::new(&bytes);
        let mut snapshot = decode_player_info_list(&mut reader).unwrap();
        snapshot.clients[0].players[0]
            .resource
            .as_mut()
            .unwrap()
            .chunk_size = 0;
        let mut encoded = Vec::new();
        assert_eq!(
            encode_player_info_list(&mut encoded, &snapshot),
            Err(LegacyEncodeError::ZeroResourceChunkSize)
        );
        assert!(encoded.is_empty());
    }

    #[test]
    fn encode_validates_counts_and_required_resources_before_writing() {
        // C4ClientPlayerInfos bounds the player count and conditionally
        // serializes ResCore under PIF_HasRes (src/C4PlayerInfo.cpp:618-630,
        // 260-267).
        let client = ClientPlayerInfosSnapshot {
            client_id: 0,
            flags: 0,
            players: Vec::new(),
        };
        let oversized = PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: vec![client; 5_001],
        };
        let mut encoded = Vec::new();
        assert_eq!(
            encode_player_info_list(&mut encoded, &oversized),
            Err(LegacyEncodeError::PlayerInfoCountOutOfRange(5_001))
        );
        assert!(encoded.is_empty());

        let player = ControlPlayerInfoEntry {
            id: 17,
            flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
            ..ControlPlayerInfoEntry::default()
        };
        let missing = PlayerInfoListSnapshot {
            last_player_id: 17,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![player],
            }],
        };
        assert_eq!(
            encode_player_info_list(&mut encoded, &missing),
            Err(LegacyEncodeError::MissingPlayerInfoResource(17))
        );
        assert!(encoded.is_empty());
    }
}
