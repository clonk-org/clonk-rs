use clonk_engine::LegacyCString;

use crate::legacy::{
    append_c_string, append_int32, append_raw_i32, append_raw_u32, LegacyControlError,
    LegacyEncodeError, Reader,
};

const C4_MAX_NAME: usize = 30;

/// Byte-preserving snapshot of one synchronized `C4Team`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinTeamSnapshot {
    pub id: i32,
    pub name: LegacyCString,
    pub player_start_index: i32,
    pub player_ids: Vec<i32>,
    pub color: u32,
    pub icon_spec: LegacyCString,
    pub max_players: i32,
}

/// Byte-preserving snapshot of the synchronized `C4TeamList` registry.
///
/// C++ binary-compiles each `bool` as one raw byte. Keeping those fields as
/// `u8` avoids silently canonicalizing their wire representation in the codec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinTeamListSnapshot {
    pub active: u8,
    pub custom: u8,
    pub allow_hostility_change: u8,
    pub allow_team_switch: u8,
    pub auto_generate_teams: u8,
    pub last_team_id: i32,
    pub team_distribution: u8,
    pub team_colors: u8,
    /// This is an explicit wire field (`C4Teams.cpp:575-578`). The omission in
    /// `C4TeamList::operator=` (`C4Teams.cpp:287-304`) belongs at the apply/copy
    /// boundary and must not make the JoinData codec discard it.
    pub max_script_players: i32,
    pub script_player_names: LegacyCString,
    pub random_team_count: i32,
    pub teams: Vec<JoinTeamSnapshot>,
}

pub(crate) fn decode_join_team_list(
    reader: &mut Reader<'_>,
) -> Result<JoinTeamListSnapshot, LegacyControlError> {
    // This order is C4TeamList::CompileFunc (src/C4Teams.cpp:556-603).
    let active = reader.read_u8()?;
    let custom = reader.read_u8()?;
    let allow_hostility_change = reader.read_u8()?;
    let allow_team_switch = reader.read_u8()?;
    let mut auto_generate_teams = reader.read_u8()?;
    let wire_last_team_id = reader.read_raw_i32()?;
    let team_distribution = reader.read_u8()?;
    let team_colors = reader.read_u8()?;
    let max_script_players = reader.read_raw_i32()?;
    let script_player_names = reader.read_c_string()?;
    let random_team_count = reader.read_raw_i32()?;

    // mkNamingCountAdapt is packed only for non-naming/binary compilers
    // (src/StdAdaptors.h:996-1014).
    let team_count = ensure_decode_count(reader.read_int32()?)?;
    let mut teams = Vec::new();
    for _ in 0..team_count {
        teams.push(decode_team(reader)?);
    }

    // C++ normalizes these after compiling rather than preserving the two
    // inconsistent runtime states (src/C4Teams.cpp:605-610).
    let largest_team_id = teams.iter().map(|team| team.id).fold(0, i32::max);
    let last_team_id = wire_last_team_id.max(largest_team_id);
    if teams.is_empty() {
        auto_generate_teams = 1;
    }

    Ok(JoinTeamListSnapshot {
        active,
        custom,
        allow_hostility_change,
        allow_team_switch,
        auto_generate_teams,
        last_team_id,
        team_distribution,
        team_colors,
        max_script_players,
        script_player_names,
        random_team_count,
        teams,
    })
}

pub(crate) fn encode_join_team_list(
    buffer: &mut Vec<u8>,
    list: &JoinTeamListSnapshot,
) -> Result<(), LegacyEncodeError> {
    // Validate first so an error never leaves a partial registry in the
    // caller's JoinData buffer.
    let team_count = ensure_encode_count(list.teams.len())?;
    for team in &list.teams {
        ensure_team_name(team)?;
        ensure_encode_count(team.player_ids.len())?;
    }

    buffer.push(list.active);
    buffer.push(list.custom);
    buffer.push(list.allow_hostility_change);
    buffer.push(list.allow_team_switch);
    buffer.push(list.auto_generate_teams);
    append_raw_i32(buffer, list.last_team_id);
    buffer.push(list.team_distribution);
    buffer.push(list.team_colors);
    append_raw_i32(buffer, list.max_script_players);
    append_c_string(buffer, &list.script_player_names);
    append_raw_i32(buffer, list.random_team_count);
    append_int32(buffer, team_count);
    for team in &list.teams {
        encode_team(buffer, team);
    }
    Ok(())
}

fn decode_team(reader: &mut Reader<'_>) -> Result<JoinTeamSnapshot, LegacyControlError> {
    // This order is C4Team::CompileFunc (src/C4Teams.cpp:138-150).
    let id = reader.read_raw_i32()?;
    let name = reader.read_c_string()?;
    if name.as_bytes().len() > C4_MAX_NAME {
        return Err(LegacyControlError::JoinDataTeamNameTooLong(
            name.as_bytes().len(),
        ));
    }
    let player_start_index = reader.read_raw_i32()?;
    let player_count = ensure_decode_count(reader.read_raw_i32()?)?;
    let mut player_ids = Vec::new();
    for _ in 0..player_count {
        player_ids.push(reader.read_raw_i32()?);
    }
    let color = reader.read_raw_u32()?;
    let icon_spec = reader.read_c_string()?;
    let max_players = reader.read_raw_i32()?;
    Ok(JoinTeamSnapshot {
        id,
        name,
        player_start_index,
        player_ids,
        color,
        icon_spec,
        max_players,
    })
}

fn encode_team(buffer: &mut Vec<u8>, team: &JoinTeamSnapshot) {
    append_raw_i32(buffer, team.id);
    append_c_string(buffer, &team.name);
    append_raw_i32(buffer, team.player_start_index);
    // Preflight in encode_join_team_list proved this conversion succeeds.
    append_raw_i32(buffer, team.player_ids.len() as i32);
    for player_id in &team.player_ids {
        append_raw_i32(buffer, *player_id);
    }
    append_raw_u32(buffer, team.color);
    append_c_string(buffer, &team.icon_spec);
    append_raw_i32(buffer, team.max_players);
}

fn ensure_team_name(team: &JoinTeamSnapshot) -> Result<(), LegacyEncodeError> {
    let name_len = team.name.as_bytes().len();
    if name_len > C4_MAX_NAME {
        Err(LegacyEncodeError::JoinDataTeamNameTooLong(name_len))
    } else {
        Ok(())
    }
}

fn ensure_decode_count(count: i32) -> Result<i32, LegacyControlError> {
    if count < 0 {
        Err(LegacyControlError::JoinDataCountOutOfRange(count))
    } else {
        Ok(count)
    }
}

fn ensure_encode_count(count: usize) -> Result<i32, LegacyEncodeError> {
    i32::try_from(count).map_err(|_| LegacyEncodeError::JoinDataCollectionTooLarge(count))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c_string(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).unwrap()
    }

    fn wire_snapshot() -> JoinTeamListSnapshot {
        JoinTeamListSnapshot {
            active: 1,
            custom: 0,
            allow_hostility_change: 1,
            allow_team_switch: 1,
            auto_generate_teams: 0,
            last_team_id: 9,
            team_distribution: 4,
            team_colors: 1,
            max_script_players: 2,
            script_player_names: c_string(b"BotA|BotB"),
            random_team_count: -3,
            teams: vec![
                JoinTeamSnapshot {
                    id: 9,
                    name: c_string(b"Red"),
                    player_start_index: 2,
                    player_ids: vec![0x0102_0304, -7],
                    color: 0x1122_3344,
                    icon_spec: c_string(b"Icon"),
                    max_players: 3,
                },
                JoinTeamSnapshot {
                    id: 4,
                    name: c_string(&[0xff, b'B']),
                    player_start_index: -1,
                    player_ids: Vec::new(),
                    color: 0xaabb_ccdd,
                    icon_spec: LegacyCString::default(),
                    max_players: 0,
                },
            ],
        }
    }

    #[test]
    fn team_list_matches_cpp_binary_field_order_exactly() {
        // C4TeamList::CompileFunc: src/C4Teams.cpp:556-603.
        // C4Team::CompileFunc: src/C4Teams.cpp:138-150.
        // The list count is the sole packed integer: src/StdAdaptors.h:996-1014.
        let expected = vec![
            1, 0, 1, 1, 0, // raw bool fields
            9, 0, 0, 0, // LastTeamID
            4, 1, // TeamDistribution, TeamColors
            2, 0, 0, 0, // MaxScriptPlayers
            b'B', b'o', b't', b'A', b'|', b'B', b'o', b't', b'B', 0, 0xfd, 0xff, 0xff,
            0xff, // RandomTeamCount
            2,    // packed team count
            9, 0, 0, 0, b'R', b'e', b'd', 0, // team 1 ID, Name
            2, 0, 0, 0, // PlrStartIndex
            2, 0, 0, 0, // raw PlayerCount
            4, 3, 2, 1, 0xf9, 0xff, 0xff, 0xff, // raw Players
            0x44, 0x33, 0x22, 0x11, // Color
            b'I', b'c', b'o', b'n', 0, // IconSpec
            3, 0, 0, 0, // MaxPlayer
            4, 0, 0, 0, 0xff, b'B', 0, // team 2 ID, Name
            0xff, 0xff, 0xff, 0xff, // PlrStartIndex
            0, 0, 0, 0, // raw PlayerCount
            0xdd, 0xcc, 0xbb, 0xaa, // Color
            0,    // empty IconSpec
            0, 0, 0, 0, // MaxPlayer
        ];

        let mut encoded = Vec::new();
        encode_join_team_list(&mut encoded, &wire_snapshot()).unwrap();
        assert_eq!(encoded, expected);

        let mut reader = Reader::new(&expected);
        assert_eq!(decode_join_team_list(&mut reader).unwrap(), wire_snapshot());
        assert_eq!(reader.remaining(), 0);
    }

    #[test]
    fn decode_normalizes_last_team_id_like_cpp() {
        // C4TeamList::CompileFunc takes max(GetLargestTeamID(), LastTeamID):
        // src/C4Teams.cpp:605-608; GetLargestTeamID starts at zero:
        // src/C4Teams.cpp:438-443.
        let mut wire = wire_snapshot();
        wire.last_team_id = -12;
        wire.max_script_players = 27;
        let mut encoded = Vec::new();
        encode_join_team_list(&mut encoded, &wire).unwrap();

        let decoded = decode_join_team_list(&mut Reader::new(&encoded)).unwrap();
        assert_eq!(decoded.last_team_id, 9);
        assert_eq!(decoded.max_script_players, 27);
        assert_eq!(decoded.teams, wire.teams);
    }

    #[test]
    fn decode_forces_auto_generation_for_an_empty_team_list() {
        // The C++ compiler forces this state after reading zero teams:
        // src/C4Teams.cpp:605-610.
        let mut wire = wire_snapshot();
        wire.auto_generate_teams = 0;
        wire.last_team_id = -12;
        wire.max_script_players = 27;
        wire.teams.clear();
        let mut encoded = Vec::new();
        encode_join_team_list(&mut encoded, &wire).unwrap();

        let decoded = decode_join_team_list(&mut Reader::new(&encoded)).unwrap();
        assert_eq!(decoded.auto_generate_teams, 1);
        assert_eq!(decoded.last_team_id, 0);
        assert_eq!(decoded.max_script_players, 27);
        assert!(decoded.teams.is_empty());
    }

    #[test]
    fn raw_boolean_bytes_survive_a_nonempty_registry_round_trip() {
        // StdCompilerBinRead/Write copies bool as a one-byte raw value:
        // src/StdCompiler.cpp:104-112,163-171,228-238.
        let mut wire = wire_snapshot();
        wire.active = 0x02;
        wire.custom = 0x03;
        wire.allow_hostility_change = 0x04;
        wire.allow_team_switch = 0x05;
        wire.auto_generate_teams = 0x06;
        wire.team_colors = 0x07;
        let mut encoded = Vec::new();
        encode_join_team_list(&mut encoded, &wire).unwrap();

        let decoded = decode_join_team_list(&mut Reader::new(&encoded)).unwrap();
        let mut reencoded = Vec::new();
        encode_join_team_list(&mut reencoded, &decoded).unwrap();
        assert_eq!(reencoded, encoded);
    }

    #[test]
    fn team_name_over_c4_max_name_is_rejected_without_partial_output() {
        // Name is char[C4MaxName + 1], where C4MaxName is 30:
        // src/C4Teams.h:55-58; src/C4Constants.h:25-26. The binary compiler
        // rejects byte 31 rather than truncating: src/StdCompiler.cpp:174-191.
        let mut wire = wire_snapshot();
        wire.teams[0].name = c_string(&[b'x'; C4_MAX_NAME + 1]);
        let mut output = vec![0xaa];
        assert_eq!(
            encode_join_team_list(&mut output, &wire),
            Err(LegacyEncodeError::JoinDataTeamNameTooLong(C4_MAX_NAME + 1))
        );
        assert_eq!(output, [0xaa]);

        let mut valid = wire_snapshot();
        valid.teams[0].name = c_string(&[b'x'; C4_MAX_NAME]);
        let mut encoded = Vec::new();
        encode_join_team_list(&mut encoded, &valid).unwrap();
        let name_start = 34;
        encoded.splice(
            name_start..name_start + C4_MAX_NAME + 1,
            [b'x'; C4_MAX_NAME + 1]
                .into_iter()
                .chain(std::iter::once(0)),
        );
        assert_eq!(
            decode_join_team_list(&mut Reader::new(&encoded)),
            Err(LegacyControlError::JoinDataTeamNameTooLong(C4_MAX_NAME + 1))
        );
    }
}
