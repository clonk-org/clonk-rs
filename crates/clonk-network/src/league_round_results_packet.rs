use crate::legacy::{
    append_c_string, append_int32, append_raw_i32, append_raw_u32, LegacyControlError, Reader,
};
use clonk_engine::LegacyCString;

pub const PID_LEAGUE_ROUND_RESULTS: u8 = 0x17;
const MAX_ROUND_RESULT_PLAYERS: usize = 5_000;

/// `C4RoundResultsPlayer::LeagueStatus` as carried by
/// `C4PacketLeagueRoundResults`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeagueRoundPlayerStatus {
    Unknown,
    Lost,
    Won,
    Other(u8),
}

impl From<u8> for LeagueRoundPlayerStatus {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Unknown,
            1 => Self::Lost,
            2 => Self::Won,
            other => Self::Other(other),
        }
    }
}

impl From<LeagueRoundPlayerStatus> for u8 {
    fn from(value: LeagueRoundPlayerStatus) -> Self {
        match value {
            LeagueRoundPlayerStatus::Unknown => 0,
            LeagueRoundPlayerStatus::Lost => 1,
            LeagueRoundPlayerStatus::Won => 2,
            LeagueRoundPlayerStatus::Other(other) => other,
        }
    }
}

/// Exact binary fields serialized by `C4RoundResultsPlayer::CompileFunc`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueRoundResultsPlayer {
    pub player_info_id: i32,
    pub total_playing_time: u32,
    pub settlement_score_old: i32,
    pub settlement_score_new: i32,
    pub league_score_new: i32,
    pub league_score_gain: i32,
    pub league_rank_new: i32,
    pub league_rank_symbol_new: i32,
    pub league_progress_data: LegacyCString,
    pub status: LeagueRoundPlayerStatus,
}

/// Typed body of `PID_LeagueRoundResults` (`0x17`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeagueRoundResultsPacket {
    pub success: bool,
    pub result_string: LegacyCString,
    pub players: Vec<LeagueRoundResultsPlayer>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeagueRoundResultsDecodeError {
    #[error(transparent)]
    Legacy(#[from] LegacyControlError),
    #[error("league round-results player count {0} is outside the C++ range 0..=5000")]
    PlayerCountOutOfRange(i32),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum LeagueRoundResultsEncodeError {
    #[error("league round-results player count {0} exceeds the C++ limit of 5000")]
    PlayerCountOutOfRange(usize),
}

/// Decodes the body after `PID_LeagueRoundResults` in C++ `CompileFunc`
/// order. Like `CompileFromBuf`, this intentionally tolerates trailing bytes.
pub fn decode_league_round_results_payload(
    payload: &[u8],
) -> Result<LeagueRoundResultsPacket, LeagueRoundResultsDecodeError> {
    let mut reader = Reader::new(payload);
    let success = reader.read_u8()? != 0;
    let result_string = reader.read_c_string()?;
    let player_count = reader.read_int32()?;
    if !(0..=MAX_ROUND_RESULT_PLAYERS as i32).contains(&player_count) {
        return Err(LeagueRoundResultsDecodeError::PlayerCountOutOfRange(
            player_count,
        ));
    }

    let mut players = Vec::with_capacity(player_count as usize);
    for _ in 0..player_count {
        players.push(LeagueRoundResultsPlayer {
            player_info_id: reader.read_raw_i32()?,
            total_playing_time: reader.read_raw_u32()?,
            settlement_score_old: reader.read_raw_i32()?,
            settlement_score_new: reader.read_raw_i32()?,
            league_score_new: reader.read_raw_i32()?,
            league_score_gain: reader.read_raw_i32()?,
            league_rank_new: reader.read_raw_i32()?,
            league_rank_symbol_new: reader.read_raw_i32()?,
            league_progress_data: reader.read_c_string()?,
            status: reader.read_u8()?.into(),
        });
    }

    Ok(LeagueRoundResultsPacket {
        success,
        result_string,
        players,
    })
}

/// Encodes the exact body following `PID_LeagueRoundResults`.
pub fn encode_league_round_results_payload(
    packet: &LeagueRoundResultsPacket,
) -> Result<Vec<u8>, LeagueRoundResultsEncodeError> {
    if packet.players.len() > MAX_ROUND_RESULT_PLAYERS {
        return Err(LeagueRoundResultsEncodeError::PlayerCountOutOfRange(
            packet.players.len(),
        ));
    }

    let mut payload = Vec::new();
    payload.push(u8::from(packet.success));
    append_c_string(&mut payload, &packet.result_string);
    append_int32(&mut payload, packet.players.len() as i32);
    for player in &packet.players {
        append_raw_i32(&mut payload, player.player_info_id);
        append_raw_u32(&mut payload, player.total_playing_time);
        append_raw_i32(&mut payload, player.settlement_score_old);
        append_raw_i32(&mut payload, player.settlement_score_new);
        append_raw_i32(&mut payload, player.league_score_new);
        append_raw_i32(&mut payload, player.league_score_gain);
        append_raw_i32(&mut payload, player.league_rank_new);
        append_raw_i32(&mut payload, player.league_rank_symbol_new);
        append_c_string(&mut payload, &player.league_progress_data);
        payload.push(player.status.into());
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy(bytes: &[u8]) -> LegacyCString {
        LegacyCString::from_bytes(bytes.to_vec()).expect("fixture has no NUL")
    }

    #[test]
    fn cpp_empty_league_round_results_vector_round_trips() {
        let payload = [0x01, b'O', b'K', 0x00, 0x00];
        let packet = LeagueRoundResultsPacket {
            success: true,
            result_string: legacy(b"OK"),
            players: Vec::new(),
        };

        assert_eq!(
            decode_league_round_results_payload(&payload),
            Ok(packet.clone())
        );
        assert_eq!(
            encode_league_round_results_payload(&packet),
            Ok(payload.to_vec())
        );
    }

    #[test]
    fn cpp_one_player_league_round_results_vector_round_trips() {
        let payload = [
            0x01, b'O', b'K', 0x00, 0x01, 0x04, 0x03, 0x02, 0x01, 0x08, 0x07, 0x06, 0x05, 0xff,
            0xff, 0xff, 0xff, 0x64, 0x00, 0x00, 0x00, 0xc8, 0x00, 0x00, 0x00, 0xfb, 0xff, 0xff,
            0xff, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, b'P', 0x00, 0x02,
        ];
        let packet = LeagueRoundResultsPacket {
            success: true,
            result_string: legacy(b"OK"),
            players: vec![LeagueRoundResultsPlayer {
                player_info_id: 0x0102_0304,
                total_playing_time: 0x0506_0708,
                settlement_score_old: -1,
                settlement_score_new: 100,
                league_score_new: 200,
                league_score_gain: -5,
                league_rank_new: 3,
                league_rank_symbol_new: 4,
                league_progress_data: legacy(b"P"),
                status: LeagueRoundPlayerStatus::Won,
            }],
        };

        assert_eq!(
            decode_league_round_results_payload(&payload),
            Ok(packet.clone())
        );
        assert_eq!(
            encode_league_round_results_payload(&packet),
            Ok(payload.to_vec())
        );
    }

    #[test]
    fn league_round_results_count_uses_cpp_signed_pack_at_64() {
        let player = LeagueRoundResultsPlayer {
            player_info_id: 0,
            total_playing_time: 0,
            settlement_score_old: 0,
            settlement_score_new: 0,
            league_score_new: 0,
            league_score_gain: 0,
            league_rank_new: 0,
            league_rank_symbol_new: 0,
            league_progress_data: LegacyCString::default(),
            status: LeagueRoundPlayerStatus::Unknown,
        };
        let packet = LeagueRoundResultsPacket {
            success: false,
            result_string: LegacyCString::default(),
            players: vec![player; 64],
        };

        let payload = encode_league_round_results_payload(&packet).unwrap();
        assert_eq!(&payload[..4], &[0x00, 0x00, 0x40, 0x00]);
        assert_eq!(decode_league_round_results_payload(&payload), Ok(packet));
    }

    #[test]
    fn league_round_results_decode_matches_cpp_bounds_and_trailing_tolerance() {
        let mut trailing = vec![0x00, b'\x80', 0x00, 0x00, 0xaa];
        assert_eq!(
            decode_league_round_results_payload(&trailing)
                .expect("trailing bytes are ignored")
                .result_string,
            legacy(b"\x80")
        );

        trailing[3] = 0xff;
        assert!(matches!(
            decode_league_round_results_payload(&trailing),
            Err(LeagueRoundResultsDecodeError::PlayerCountOutOfRange(-1))
        ));
    }

    #[test]
    fn league_round_results_preserves_legacy_bytes_and_unknown_status() {
        let packet = LeagueRoundResultsPacket {
            success: false,
            result_string: legacy(b"\x80result"),
            players: vec![LeagueRoundResultsPlayer {
                player_info_id: 17,
                total_playing_time: 23,
                settlement_score_old: -1,
                settlement_score_new: 2,
                league_score_new: 3,
                league_score_gain: -4,
                league_rank_new: 5,
                league_rank_symbol_new: 6,
                league_progress_data: legacy(b"\xffprogress"),
                status: LeagueRoundPlayerStatus::Other(0x7f),
            }],
        };

        let payload = encode_league_round_results_payload(&packet).unwrap();
        assert_eq!(decode_league_round_results_payload(&payload), Ok(packet));
    }
}
