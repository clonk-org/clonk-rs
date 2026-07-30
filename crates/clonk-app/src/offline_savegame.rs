use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use clonk_app_netplay::ConfiguredClientPlayers;
use clonk_engine::{
    ControlPlayerInfoEntry, ControlPlayerInfoRegistry, InitialNetworkGameData,
    RuntimeJoinPlayerSource,
};
use clonk_resources::LanguagePacks;

use crate::offline_startup::OfflineStartupPlayers;

/// State frozen during `OpenScenario` and consumed at the ordinary offline
/// `InitPlayers` boundary. User profiles stay external; script profiles keep
/// the filenames embedded by `C4GameSaveSavegame`.
pub(super) struct OfflineSavegameStartup {
    /// `None` when the scenario ships no `Game.txt`: `CompileRuntimeData` over
    /// a null `GameText` leaves every live value alone (C4Game.cpp:224,267).
    pub(super) initial_game_data: Option<InitialNetworkGameData>,
    pub(super) runtime_players: Vec<RuntimeJoinPlayerSource>,
    pub(super) external_player_paths: HashMap<i32, PathBuf>,
    /// `C4S.Head.SaveGame`: only a real savegame insists on a runtime section
    /// per restored player (C4Player.cpp:359-371).
    pub(super) save_game: bool,
    /// One entry per assignment made beyond `PML_PlrName`. C++ logs each and,
    /// in graphical non-replay mode, shows a hideable modal
    /// (C4PlayerInfo.cpp:1384-1390).
    pub(super) wild_takeovers: Vec<OfflineWildTakeover>,
}

/// A savegame player taken over by a participant the weaker passes matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OfflineWildTakeover {
    /// `pInfo->GetName()`, the joining participant.
    pub(super) participant: Vec<u8>,
    /// `pSavegameInfo->GetName()`, the savegame player being continued.
    pub(super) savegame_player: Vec<u8>,
}

pub(super) fn prepare_offline_savegame_startup(
    scenario_path: &Path,
    configured: ConfiguredClientPlayers,
    declared_max_players: i32,
    save_game: bool,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<(OfflineStartupPlayers, OfflineSavegameStartup), String> {
    let group = super::open_group_path_for_folder_map(scenario_path).map_err(|error| {
        format!(
            "failed to open offline savegame {}: {error}",
            scenario_path.display()
        )
    })?;
    let game_source =
        super::read_optional_initial_network_game_source(&group).map_err(|error| {
            format!(
                "offline savegame {} has an unreadable Game.txt: {error}",
                scenario_path.display()
            )
        })?;
    let initial_game_data = offline_savegame_initial_game_data(game_source.as_deref())?;

    // A present SavePlayerInfos owns the result even when malformed. Only an
    // absent component permits C4GameParameters::Load's old Game.txt
    // [PlayerFiles] fallback.
    let restore = crate::prepared_host_bootstrap::load_offline_savegame_restore_player_infos(
        &group,
        scenario_path,
        languages,
        language_packs,
        game_source.as_deref(),
    );
    let restore_count = restore
        .clients
        .iter()
        .map(|client| client.players.len())
        .sum::<usize>();
    let mut startup = OfflineStartupPlayers::new_after_player_id(
        configured,
        effective_offline_max_players(declared_max_players, restore_count, save_game),
        restore.last_player_id,
    );
    let (runtime_players, external_player_paths, wild_takeovers) =
        associate_offline_savegame_players(&mut startup, &restore, save_game);

    Ok((
        startup,
        OfflineSavegameStartup {
            initial_game_data,
            runtime_players,
            external_player_paths,
            save_game,
            wild_takeovers,
        },
    ))
}

/// `C4Game::OpenScenario` compiles runtime data only from a `Game.txt` it
/// actually found; over a null `GameText` every `CompileRuntimeData` caller is
/// a no-op and the live scenario state stands (C4Game.cpp:224,267;
/// C4Player.cpp:1652-1656).
fn offline_savegame_initial_game_data(
    game_source: Option<&[u8]>,
) -> Result<Option<InitialNetworkGameData>, String> {
    let Some(source) = game_source else {
        return Ok(None);
    };
    let data = clonk_engine::parse_initial_network_game_data(source);
    data.validate_runtime_application()
        .map(|()| Some(data))
        .map_err(|error| format!("invalid offline savegame Game.txt: {error}"))
}

/// `C4Game::Init` raises the frozen player capacity to the restore-row count
/// behind `if (C4S.Head.SaveGame)`, so a regular scenario shipping restore
/// infos keeps its declared C4S/Parameters capacity (C4Game.cpp:242-250).
fn effective_offline_max_players(declared: i32, restore_count: usize, save_game: bool) -> i32 {
    if save_game {
        declared.max(i32::try_from(restore_count).unwrap_or(i32::MAX))
    } else {
        declared
    }
}

/// Mirrors `InitLocal`, `CreateRestoreInfosForJoinedScriptPlayers`, and the
/// automatic non-network passes in `RestoreSavegameInfos`.
fn associate_offline_savegame_players(
    startup: &mut OfflineStartupPlayers,
    restore: &clonk_network::PlayerInfoListSnapshot,
    save_game: bool,
) -> (
    Vec<RuntimeJoinPlayerSource>,
    HashMap<i32, PathBuf>,
    Vec<OfflineWildTakeover>,
) {
    let configured_paths = (0..startup.player_info.players.len())
        .filter_map(|row| {
            startup
                .selected_for_row(row)
                .map(|selected| selected.source_path().to_path_buf())
        })
        .collect::<Vec<_>>();
    let (player_info, runtime_players, external_player_paths, wild_takeovers) =
        associate_offline_savegame_player_info(
            startup.player_info.clone(),
            &configured_paths,
            restore,
            save_game,
        );
    startup.replace_player_info(player_info);
    (runtime_players, external_player_paths, wild_takeovers)
}

fn associate_offline_savegame_player_info(
    mut player_info: clonk_engine::PlayerInfoControlData,
    configured_paths: &[PathBuf],
    restore: &clonk_network::PlayerInfoListSnapshot,
    save_game: bool,
) -> (
    clonk_engine::PlayerInfoControlData,
    Vec<RuntimeJoinPlayerSource>,
    HashMap<i32, PathBuf>,
    Vec<OfflineWildTakeover>,
) {
    let restore_players = restore
        .clients
        .iter()
        .flat_map(|client| client.players.iter().cloned())
        .collect::<Vec<_>>();
    let configured_count = configured_paths.len();

    // Script players are copied into the first local packet and pre-associated
    // before automatic matching scans user players.
    for restore_player in &restore_players {
        if !restore_player.is_script_player()
            || player_info
                .players
                .iter()
                .any(|player| player.savegame_player == restore_player.id)
        {
            continue;
        }
        let mut script = restore_player.clone();
        script.savegame_player = restore_player.id;
        player_info.players.push(script);
    }

    // The automatic passes run for non-network savegames only; a regular
    // scenario shipping restore infos keeps its participants unassociated
    // (C4PlayerInfo.cpp:1372).
    let mut wild_takeovers = Vec::new();
    let matching_levels: &[u8] = if save_game { &[0, 1, 2, 3] } else { &[] };
    for &matching_level in matching_levels {
        for player_index in 0..player_info.players.len() {
            if player_info.players[player_index].savegame_player != 0 {
                continue;
            }
            let assigned = player_info
                .players
                .iter()
                .filter_map(|player| {
                    (player.savegame_player != 0).then_some(player.savegame_player)
                })
                .collect::<HashSet<_>>();
            let current = &player_info.players[player_index];
            let Some(saved) = restore_players.iter().find(|saved| {
                !assigned.contains(&saved.id)
                    && savegame_players_match(current, saved, matching_level)
            }) else {
                continue;
            };
            player_info.players[player_index].savegame_player = saved.id;
            // Levels past PML_PlrName are "wild": C++ warns about each one
            // (C4PlayerInfo.cpp:1384-1390).
            if matching_level > 1 {
                wild_takeovers.push(OfflineWildTakeover {
                    participant: effective_player_name(&player_info.players[player_index]).to_vec(),
                    savegame_player: effective_player_name(saved).to_vec(),
                });
            }
        }
    }

    let mut external_player_paths = HashMap::new();
    for row in 0..configured_count {
        let Some(saved_id) = player_info
            .players
            .get(row)
            .map(|player| player.savegame_player)
            .filter(|saved_id| *saved_id != 0)
        else {
            continue;
        };
        if let Some(path) = configured_paths.get(row) {
            external_player_paths.insert(saved_id, path.clone());
        }
    }

    // SetSavegameResume copies only the native takeover fields; in
    // particular the selected user filename/profile identity remains live.
    let original_by_client = player_info.by_client;
    let mut registry = ControlPlayerInfoRegistry::default();
    registry.replace_snapshot(restore.last_player_id, [player_info]);
    for restore_player in &restore_players {
        registry.resume_savegame_player_from_info(restore_player);
    }
    let (_, mut packets) = registry.retained_rows_snapshot();
    let player_info = packets
        .pop()
        .map(
            |(client_id, flags, players)| clonk_engine::PlayerInfoControlData {
                client_id,
                flags,
                players,
                by_client: original_by_client,
            },
        )
        .unwrap_or_default();

    let runtime_players = registry
        .recreation_players()
        .into_iter()
        .filter_map(|(client_id, info_id)| {
            registry
                .get(info_id)
                .cloned()
                .map(|info| RuntimeJoinPlayerSource {
                    client_id,
                    info,
                    load_unnamed_portraits: true,
                })
        })
        .collect();
    (
        player_info,
        runtime_players,
        external_player_paths,
        wild_takeovers,
    )
}

fn savegame_players_match(
    current: &ControlPlayerInfoEntry,
    saved: &ControlPlayerInfoEntry,
    matching_level: u8,
) -> bool {
    match matching_level {
        0 => {
            !current.filename.is_empty()
                && !saved.filename.is_empty()
                && legacy_basename(current.filename.as_bytes())
                    .eq_ignore_ascii_case(legacy_basename(saved.filename.as_bytes()))
                && effective_player_name(current).eq_ignore_ascii_case(effective_player_name(saved))
        }
        1 => effective_player_name(current).eq_ignore_ascii_case(effective_player_name(saved)),
        2 => current.original_color == saved.original_color,
        _ => true,
    }
}

fn effective_player_name(player: &ControlPlayerInfoEntry) -> &[u8] {
    [&player.league_account, &player.forced_name, &player.name]
        .into_iter()
        .find(|name| !name.is_empty())
        .map_or(&[], |name| name.as_bytes())
}

fn legacy_basename(path: &[u8]) -> &[u8] {
    path.iter()
        .rposition(|byte| matches!(*byte, b'/' | b'\\'))
        .map_or(path, |separator| &path[separator + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_engine::{
        LegacyCString, PlayerInfoControlData, PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_TYPE_SCRIPT,
    };
    use clonk_network::{ClientPlayerInfosSnapshot, PlayerInfoListSnapshot};

    fn c4(value: &str) -> LegacyCString {
        LegacyCString::from_bytes(value.as_bytes().to_vec()).unwrap()
    }

    /// `RestoreSavegameInfos` runs four passes; any assignment made past
    /// `PML_PlrName` is a "wild" match that C++ logs and warns about
    /// (C4PlayerInfo.cpp:1373-1391). Exact filename and player-name matches
    /// stay silent.
    #[test]
    fn offline_wild_takeover_reports_only_matches_past_player_name() {
        let player_info = PlayerInfoControlData {
            client_id: 0,
            players: vec![
                // Exact file + name: PML_PlrFileName, silent.
                ControlPlayerInfoEntry {
                    name: c4("Alice"),
                    filename: c4("Players/Alice.c4p"),
                    id: 41,
                    original_color: 0x11,
                    ..Default::default()
                },
                // Name-only: PML_PlrName, still silent.
                ControlPlayerInfoEntry {
                    name: c4("Bob"),
                    filename: c4("Players/Renamed.c4p"),
                    id: 42,
                    original_color: 0x22,
                    ..Default::default()
                },
                // Colour-only: PML_PrefColor, a wild match.
                ControlPlayerInfoEntry {
                    name: c4("Carol"),
                    filename: c4("Players/Carol.c4p"),
                    id: 43,
                    original_color: 0x33,
                    ..Default::default()
                },
                // Nothing in common: PML_Any, also wild.
                ControlPlayerInfoEntry {
                    name: c4("Dave"),
                    filename: c4("Players/Dave.c4p"),
                    id: 44,
                    original_color: 0x99,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let saved = |name: &str, id: i32, color: u32| ControlPlayerInfoEntry {
            name: c4(name),
            flags: PLAYER_INFO_FLAG_JOINED,
            id,
            original_color: color,
            ..Default::default()
        };
        let restore = PlayerInfoListSnapshot {
            last_player_id: 40,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![
                    ControlPlayerInfoEntry {
                        filename: c4("Players/Alice.c4p"),
                        ..saved("Alice", 1, 0x11)
                    },
                    saved("Bob", 2, 0x77),
                    saved("Ghost", 3, 0x33),
                    saved("Stranger", 4, 0x88),
                ],
            }],
        };

        let (player_info, _, _, wild) = associate_offline_savegame_player_info(
            player_info,
            &[
                PathBuf::from("Players/Alice.c4p"),
                PathBuf::from("Players/Renamed.c4p"),
                PathBuf::from("Players/Carol.c4p"),
                PathBuf::from("Players/Dave.c4p"),
            ],
            &restore,
            true,
        );

        // Every participant is still assigned; the warning changes nothing.
        assert_eq!(
            player_info
                .players
                .iter()
                .map(|player| player.savegame_player)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        // Only the two weaker passes are reported, in assignment order.
        assert_eq!(
            wild,
            vec![
                OfflineWildTakeover {
                    participant: b"Carol".to_vec(),
                    savegame_player: b"Ghost".to_vec(),
                },
                OfflineWildTakeover {
                    participant: b"Dave".to_vec(),
                    savegame_player: b"Stranger".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn offline_savegame_matches_users_and_appends_embedded_scripts() {
        let player_info = PlayerInfoControlData {
            client_id: 0,
            players: vec![
                ControlPlayerInfoEntry {
                    name: c4("Alice"),
                    filename: c4("Players/Alice.c4p"),
                    id: 41,
                    original_color: 0x11,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    name: c4("New"),
                    filename: c4("Players/New.c4p"),
                    id: 42,
                    original_color: 0x22,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let alice = ControlPlayerInfoEntry {
            name: c4("alice"),
            flags: PLAYER_INFO_FLAG_JOINED,
            id: 7,
            color: 0x44,
            original_color: 0x11,
            team: 3,
            ..Default::default()
        };
        let script = ControlPlayerInfoEntry {
            name: c4("Script"),
            filename: c4("ScriptPlr-9.c4p"),
            flags: PLAYER_INFO_FLAG_JOINED,
            id: 9,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };
        let restore = PlayerInfoListSnapshot {
            last_player_id: 40,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![alice, script],
            }],
        };

        let (player_info, sources, paths, _) = associate_offline_savegame_player_info(
            player_info,
            &[
                PathBuf::from("Players/Alice.c4p"),
                PathBuf::from("Players/New.c4p"),
            ],
            &restore,
            true,
        );

        assert_eq!(player_info.players.len(), 3);
        assert_eq!(player_info.players[0].id, 7);
        assert_eq!(player_info.players[0].team, 3);
        assert_eq!(player_info.players[1].id, 42);
        assert_eq!(player_info.players[2].id, 9);
        assert_eq!(paths.get(&7), Some(&PathBuf::from("Players/Alice.c4p")));
        assert_eq!(
            sources
                .iter()
                .map(|source| source.info.id)
                .collect::<Vec<_>>(),
            vec![7, 9]
        );
    }

    /// `RestoreSavegameInfos` runs its automatic association passes only for
    /// non-network *savegames* (C4PlayerInfo.cpp:1372). A regular scenario
    /// shipping restore infos leaves its participants unassociated; only the
    /// unconditional script copy made by
    /// `CreateRestoreInfosForJoinedScriptPlayers` survives
    /// (C4PlayerInfo.cpp:1288,1326-1358).
    #[test]
    fn regular_scenario_restore_infos_skip_automatic_savegame_matching() {
        let player_info = PlayerInfoControlData {
            client_id: 0,
            players: vec![ControlPlayerInfoEntry {
                name: c4("Alice"),
                filename: c4("Players/Alice.c4p"),
                id: 41,
                original_color: 0x11,
                ..Default::default()
            }],
            ..Default::default()
        };
        // An exact filename+name match: a savegame would grab it on the first
        // pass, so only the SaveGame gate can keep it unassociated.
        let alice = ControlPlayerInfoEntry {
            name: c4("Alice"),
            filename: c4("Players/Alice.c4p"),
            flags: PLAYER_INFO_FLAG_JOINED,
            id: 1,
            original_color: 0x11,
            ..Default::default()
        };
        let script = ControlPlayerInfoEntry {
            name: c4("$TeamEnemy$"),
            filename: c4("ScriptPlr-1.c4p"),
            flags: PLAYER_INFO_FLAG_JOINED,
            id: 2,
            player_type: PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };
        let restore = PlayerInfoListSnapshot {
            last_player_id: 2,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![alice, script],
            }],
        };

        let (player_info, sources, paths, wild) = associate_offline_savegame_player_info(
            player_info,
            &[PathBuf::from("Players/Alice.c4p")],
            &restore,
            false,
        );

        assert_eq!(
            player_info.players[0].savegame_player, 0,
            "a non-savegame never takes over a saved user player",
        );
        assert!(wild.is_empty());
        assert!(paths.is_empty());
        assert_eq!(
            sources
                .iter()
                .map(|source| source.info.id)
                .collect::<Vec<_>>(),
            vec![2],
            "only the copied script player is recreated",
        );
    }

    /// `C4Game::OpenScenario` only reads `Game.txt` into `GameText`, and
    /// `CompileRuntimeData` is a no-op over a null buffer (C4Game.cpp:224,267;
    /// C4Player.cpp:1652-1656). A regular scenario shipping restore infos has
    /// no `Game.txt`, so the live C4S-derived rules, music and section state
    /// must survive untouched instead of being overwritten by a default.
    #[test]
    fn a_restore_scenario_without_game_txt_has_no_runtime_data() {
        assert!(offline_savegame_initial_game_data(None)
            .expect("an absent Game.txt is not an error")
            .is_none());
        assert!(offline_savegame_initial_game_data(Some(b"[Game]\r\n"))
            .expect("an empty runtime section compiles")
            .is_some());
    }

    /// `C4Game::Init` raises the frozen capacity to the restore-row count
    /// behind `if (C4S.Head.SaveGame)` (C4Game.cpp:242-250).
    #[test]
    fn restore_rows_raise_the_player_capacity_only_for_savegames() {
        assert_eq!(effective_offline_max_players(2, 4, true), 4);
        assert_eq!(effective_offline_max_players(2, 4, false), 2);
        assert_eq!(effective_offline_max_players(6, 4, true), 6);
    }

    #[test]
    fn matching_falls_back_from_name_to_original_color_then_storage_order() {
        let current = ControlPlayerInfoEntry {
            name: c4("Different"),
            original_color: 0x1234,
            ..Default::default()
        };
        let same_color = ControlPlayerInfoEntry {
            name: c4("Saved"),
            original_color: 0x1234,
            ..Default::default()
        };
        assert!(!savegame_players_match(&current, &same_color, 1));
        assert!(savegame_players_match(&current, &same_color, 2));
        assert!(savegame_players_match(&current, &same_color, 3));
    }
}
