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
    pub(super) initial_game_data: InitialNetworkGameData,
    pub(super) runtime_players: Vec<RuntimeJoinPlayerSource>,
    pub(super) external_player_paths: HashMap<i32, PathBuf>,
}

pub(super) fn prepare_offline_savegame_startup(
    scenario_path: &Path,
    configured: ConfiguredClientPlayers,
    declared_max_players: i32,
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
    let initial_game_data = game_source
        .as_deref()
        .map(clonk_engine::parse_initial_network_game_data)
        .unwrap_or_default();
    initial_game_data
        .validate_runtime_application()
        .map_err(|error| format!("invalid offline savegame Game.txt: {error}"))?;

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
    let effective_max_players =
        declared_max_players.max(i32::try_from(restore_count).unwrap_or(i32::MAX));
    let mut startup = OfflineStartupPlayers::new_after_player_id(
        configured,
        effective_max_players,
        restore.last_player_id,
    );
    let (runtime_players, external_player_paths) =
        associate_offline_savegame_players(&mut startup, &restore);

    Ok((
        startup,
        OfflineSavegameStartup {
            initial_game_data,
            runtime_players,
            external_player_paths,
        },
    ))
}

/// Mirrors `InitLocal`, `CreateRestoreInfosForJoinedScriptPlayers`, and the
/// automatic non-network passes in `RestoreSavegameInfos`.
fn associate_offline_savegame_players(
    startup: &mut OfflineStartupPlayers,
    restore: &clonk_network::PlayerInfoListSnapshot,
) -> (Vec<RuntimeJoinPlayerSource>, HashMap<i32, PathBuf>) {
    let configured_paths = (0..startup.player_info.players.len())
        .filter_map(|row| {
            startup
                .selected_for_row(row)
                .map(|selected| selected.source_path().to_path_buf())
        })
        .collect::<Vec<_>>();
    let (player_info, runtime_players, external_player_paths) =
        associate_offline_savegame_player_info(
            startup.player_info.clone(),
            &configured_paths,
            restore,
        );
    startup.replace_player_info(player_info);
    (runtime_players, external_player_paths)
}

fn associate_offline_savegame_player_info(
    mut player_info: clonk_engine::PlayerInfoControlData,
    configured_paths: &[PathBuf],
    restore: &clonk_network::PlayerInfoListSnapshot,
) -> (
    clonk_engine::PlayerInfoControlData,
    Vec<RuntimeJoinPlayerSource>,
    HashMap<i32, PathBuf>,
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

    for matching_level in 0..=3 {
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
    (player_info, runtime_players, external_player_paths)
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

        let (player_info, sources, paths) = associate_offline_savegame_player_info(
            player_info,
            &[
                PathBuf::from("Players/Alice.c4p"),
                PathBuf::from("Players/New.c4p"),
            ],
            &restore,
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
