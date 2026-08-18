//! Automatic association of joining players with a savegame's stored players.
//!
//! `C4PlayerInfoList::RestoreSavegameInfos` runs four passes over the
//! unassociated players, each one a `MatchingLevel`
//! (`C4PlayerInfo.h:332`), and takes the first savegame player a pass accepts
//! (`C4PlayerInfo.cpp:1373-1391`). The per-level predicate is
//! `FindSavegameResumePlayerInfo`'s switch (`C4PlayerInfo.cpp:1102-1118`).
//!
//! The passes live here rather than beside the offline savegame UI because they
//! are engine semantics over [`ControlPlayerInfoEntry`], and because the
//! differential harness compares them against the C++ oracle.

use crate::control::ControlPlayerInfoEntry;

/// `MatchingLevel` (`C4PlayerInfo.h:332`), in C++'s order.
pub const MATCHING_LEVEL_PLAYER_FILE_NAME: u8 = 0;
pub const MATCHING_LEVEL_PLAYER_NAME: u8 = 1;
pub const MATCHING_LEVEL_PREFERRED_COLOR: u8 = 2;
pub const MATCHING_LEVEL_ANY: u8 = 3;

/// Whether one pass accepts this pairing.
///
/// Mirrors `FindSavegameResumePlayerInfo`'s switch
/// (`C4PlayerInfo.cpp:1102-1118`). Level 0 deliberately *falls through* into
/// level 1 in C++ (`// nobreak: Check player name as well`), so a file-name
/// match alone is not enough — the name has to match too.
pub fn savegame_players_match(
    current: &ControlPlayerInfoEntry,
    saved: &ControlPlayerInfoEntry,
    matching_level: u8,
) -> bool {
    match matching_level {
        MATCHING_LEVEL_PLAYER_FILE_NAME => {
            !current.filename.is_empty()
                && !saved.filename.is_empty()
                && legacy_bytes_equal_no_case(
                    legacy_basename(current.filename.as_bytes()),
                    legacy_basename(saved.filename.as_bytes()),
                )
                && legacy_bytes_equal_no_case(
                    effective_player_name(current),
                    effective_player_name(saved),
                )
        }
        MATCHING_LEVEL_PLAYER_NAME => {
            legacy_bytes_equal_no_case(effective_player_name(current), effective_player_name(saved))
        }
        MATCHING_LEVEL_PREFERRED_COLOR => current.original_color == saved.original_color,
        _ => true,
    }
}

/// The name C++ matches on: `C4PlayerInfo::GetName` prefers the league account,
/// then a forced name, then the player's own.
pub fn effective_player_name(player: &ControlPlayerInfoEntry) -> &[u8] {
    [&player.league_account, &player.forced_name, &player.name]
        .into_iter()
        .find(|name| !name.is_empty())
        .map_or(&[], |name| name.as_bytes())
}

/// `SEqualNoCase` over raw bytes, including the three Latin-1 umlauts C++'s
/// table folds.
fn legacy_bytes_equal_no_case(left: &[u8], right: &[u8]) -> bool {
    fn capital(byte: u8) -> u8 {
        match byte {
            b'a'..=b'z' => byte - 32,
            0xe4 => 0xc4,
            0xf6 => 0xd6,
            0xfc => 0xdc,
            _ => byte,
        }
    }

    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| capital(*left) == capital(*right))
}

/// `GetFilename` (`StdFile.cpp:43-49`): the component after the last separator.
///
/// Shared with the runtime-join filename derivation, which composes this same
/// helper's result (`C4PlayerInfo.cpp:1665`).
pub fn legacy_basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| matches!(*byte, b'/' | b'\\'))
        .map_or(path, |separator| &path[separator + 1..])
}
