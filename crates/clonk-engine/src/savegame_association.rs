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

/// One association a pass past `PML_PlrName` made, which C++ warns about
/// (`C4PlayerInfo.cpp:1384-1390`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WildSavegameTakeover {
    /// Index into the joining players.
    pub participant: usize,
    /// `C4PlayerInfo::GetID` of the savegame player it took over.
    pub savegame_player: i32,
}

/// The automatic association passes `RestoreSavegameInfos` runs
/// (`C4PlayerInfo.cpp:1373-1391`).
///
/// Four passes over every still-unassociated joining player, each taking the
/// first savegame player that pass accepts. The pass order is the whole
/// mechanism: an exact file+name match is claimed before anything can be taken
/// on colour alone, so running the levels in one combined pass would associate
/// different players.
///
/// The caller decides *whether* these run: C++ gates them on a non-network game
/// whose scenario sets `Head.SaveGame` (:1372).
pub fn associate_savegame_players(
    players: &mut [ControlPlayerInfoEntry],
    savegame_players: &[ControlPlayerInfoEntry],
) -> Vec<WildSavegameTakeover> {
    let mut wild = Vec::new();
    for matching_level in [
        MATCHING_LEVEL_PLAYER_FILE_NAME,
        MATCHING_LEVEL_PLAYER_NAME,
        MATCHING_LEVEL_PREFERRED_COLOR,
        MATCHING_LEVEL_ANY,
    ] {
        for participant in 0..players.len() {
            if players[participant].savegame_player != 0 {
                continue;
            }
            // Re-read on every candidate rather than once per pass: an
            // association made earlier in this same pass has to be visible.
            let taken = |candidate: &ControlPlayerInfoEntry| {
                players.iter().any(|player| {
                    player.savegame_player != 0 && player.savegame_player == candidate.id
                })
            };
            let current = &players[participant];
            let Some(saved) = savegame_players.iter().find(|saved| {
                !taken(saved) && savegame_players_match(current, saved, matching_level)
            }) else {
                continue;
            };
            players[participant].savegame_player = saved.id;
            if matching_level > MATCHING_LEVEL_PLAYER_NAME {
                wild.push(WildSavegameTakeover {
                    participant,
                    savegame_player: saved.id,
                });
            }
        }
    }
    wild
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
///
/// C++ splits on `DirectorySeparator || '/'`, and `DirectorySeparator` is
/// `'\\'` only on Windows (`StdFile.h:41-49`) — so a backslash begins a new
/// component there and is an ordinary filename byte everywhere else. The split
/// is host-conditional here for the same reason: a peer has to derive the same
/// basename as the C++ build it is playing against, and that build is running
/// this platform.
///
/// Deliberately *not* the cross-platform "split on either slash" rule. That is
/// stable across hosts but disagrees with C++ on unix, and both call sites feed
/// synchronised player state.
pub fn legacy_basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| *byte == b'/' || (cfg!(windows) && *byte == b'\\'))
        .map_or(path, |separator| &path[separator + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `StdFile.cpp:43-49` splits on `DirectorySeparator || '/'`, and
    /// `StdFile.h:41-49` makes `DirectorySeparator` `'\\'` on Windows and `'/'`
    /// elsewhere — so the backslash case is host-conditional by construction and
    /// the expectation has to be too, rather than pinning this host's answer.
    #[test]
    fn basename_splits_on_backslash_only_where_cpp_does() {
        assert_eq!(legacy_basename(b"Players/Alice.c4p"), b"Alice.c4p");
        assert_eq!(legacy_basename(b"Alice.c4p"), b"Alice.c4p");
        assert_eq!(legacy_basename(b""), b"");

        let windows_path: &[u8] = br"C:\Players\Alice.c4p";
        let expected: &[u8] = if cfg!(windows) {
            b"Alice.c4p"
        } else {
            windows_path
        };
        assert_eq!(legacy_basename(windows_path), expected);

        // A mixed path still ends at the last separator C++ recognises here.
        let mixed: &[u8] = br"Players\Sub/Alice.c4p";
        assert_eq!(legacy_basename(mixed), b"Alice.c4p");
    }
}
