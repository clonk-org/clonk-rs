use crate::{player_file::PlayerFile, JoinPlayerConfig, JoinPlayerControlData};
use crate::{ControlPlayerInfoEntry, PlayerInfoControlData, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS};

/// `C4PlayerInfoList`'s synchronized per-client player-info registry.
#[derive(Debug, Default)]
pub struct ControlPlayerInfoRegistry {
    clients: Vec<ClientPlayerInfos>,
}

#[derive(Debug)]
struct ClientPlayerInfos {
    client_id: i32,
    players: Vec<ControlPlayerInfoEntry>,
}

impl ControlPlayerInfoRegistry {
    pub fn apply(&mut self, info: PlayerInfoControlData) {
        let PlayerInfoControlData {
            client_id,
            flags,
            players,
            ..
        } = info;
        if let Some(existing) = self
            .clients
            .iter_mut()
            .find(|client| client.client_id == client_id)
        {
            if flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0 {
                existing.players.extend(players);
            } else {
                existing.players = players;
            }
        } else {
            self.clients.push(ClientPlayerInfos { client_id, players });
        }
    }

    pub fn get(&self, info_id: i32) -> Option<&ControlPlayerInfoEntry> {
        self.clients
            .iter()
            .flat_map(|client| &client.players)
            .find(|player| player.id == info_id)
    }

    pub fn player_count(&self) -> usize {
        self.clients.iter().map(|client| client.players.len()).sum()
    }
}

pub struct JoinPlayerPreparation<'a> {
    pub join: &'a JoinPlayerControlData,
    pub info: &'a ControlPlayerInfoEntry,
    pub player_file: Option<&'a PlayerFile>,
    pub startup_player_count: i32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrepareJoinPlayerError {
    #[error("join references player info {control_id}, but entry has id {info_id}")]
    PlayerInfoIdMismatch { control_id: i32, info_id: i32 },
    #[error("user player {info_id} has no player file data")]
    MissingPlayerData { info_id: i32 },
    #[error("script player {info_id} is not supported yet")]
    UnsupportedScriptPlayer { info_id: i32 },
    #[error("NoScenarioInit player {info_id} is not supported yet")]
    UnsupportedNoScenarioInit { info_id: i32 },
}

pub fn prepare_join_player_config(
    input: JoinPlayerPreparation<'_>,
) -> Result<JoinPlayerConfig, PrepareJoinPlayerError> {
    if input.join.info_id != input.info.id {
        return Err(PrepareJoinPlayerError::PlayerInfoIdMismatch {
            control_id: input.join.info_id,
            info_id: input.info.id,
        });
    }
    if input.info.is_script_player() {
        return Err(PrepareJoinPlayerError::UnsupportedScriptPlayer {
            info_id: input.info.id,
        });
    }
    if input.info.no_scenario_init() {
        return Err(PrepareJoinPlayerError::UnsupportedNoScenarioInit {
            info_id: input.info.id,
        });
    }
    let file = input
        .player_file
        .ok_or(PrepareJoinPlayerError::MissingPlayerData {
            info_id: input.info.id,
        })?;
    let name = [
        &input.info.league_account,
        &input.info.forced_name,
        &input.info.name,
    ]
    .into_iter()
    .find(|name| !name.is_empty())
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default();

    Ok(JoinPlayerConfig {
        name,
        player_info_id: input.info.id,
        score: file.score,
        total_playing_time: file.total_playing_time,
        team: (input.info.team != 0).then_some(input.info.team),
        color_dw: input.info.color & 0x00ff_ffff,
        pref_color: file.pref_color,
        pref_position: file.pref_position,
        crew: file.crew.clone(),
        control_style: file.pref_control_style,
        auto_context_menu: file.pref_auto_context_menu,
        startup_player_count: input.startup_player_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: i32) -> ControlPlayerInfoEntry {
        ControlPlayerInfoEntry {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn non_add_packet_replaces_the_clients_player_list() {
        // C4PlayerInfoList::AddInfo replaces an existing client's entire
        // C4ClientPlayerInfos unless CIF_AddPlayers is set
        // (src/C4PlayerInfo.cpp:834-880).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(8)],
            ..Default::default()
        });

        assert!(registry.get(7).is_none());
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 1);
    }

    #[test]
    fn add_packet_appends_to_the_clients_player_list() {
        // CIF_AddPlayers makes C4PlayerInfoList::AddInfo call
        // C4ClientPlayerInfos::GrabMergeFrom, which appends in packet order
        // (src/C4PlayerInfo.cpp:458-482,834-880).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![player(8)],
            ..Default::default()
        });

        assert_eq!(registry.get(7).map(|entry| entry.id), Some(7));
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 2);
    }

    #[test]
    fn user_join_combines_player_info_with_player_file_core() {
        // C4Player::Init takes ID/team/name/color from C4PlayerInfo, while
        // C4Player::Load supplies score, preferences and crew from the .c4p
        // (src/C4Player.cpp:246-284,1089-1106).
        let info = ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(b"Network Tyler".to_vec())
                .expect("valid legacy name"),
            id: 7,
            team: 2,
            color: 0x0011_2233,
            ..Default::default()
        };
        let crew = vec![crate::player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: "Ada".to_string(),
            rank: 3,
            experience: 50,
            participation: 1,
            in_action: false,
            has_died: false,
        }];
        let file = PlayerFile {
            name: "File Tyler".to_string(),
            score: 250,
            total_playing_time: 1_234,
            pref_color: 4,
            pref_color_dw: 0x00aa_bbcc,
            pref_position: 2,
            pref_control_style: true,
            pref_auto_context_menu: false,
            crew: crew.clone(),
        };
        let join = JoinPlayerControlData {
            info_id: 7,
            ..Default::default()
        };

        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: Some(&file),
            startup_player_count: 2,
        })
        .expect("user join prepares");

        assert_eq!(
            config,
            JoinPlayerConfig {
                name: "Network Tyler".to_string(),
                player_info_id: 7,
                score: 250,
                total_playing_time: 1_234,
                team: Some(2),
                color_dw: 0x0011_2233,
                pref_color: 4,
                pref_position: 2,
                crew,
                control_style: true,
                auto_context_menu: false,
                startup_player_count: 2,
            }
        );
    }
}
