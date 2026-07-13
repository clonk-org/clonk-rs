use crate::{
    player_file::PlayerFile, JoinPlayerConfig, JoinPlayerControlData, JoinPlayerSource,
    PlayerInfoUpdateRequest, ScenarioError, PLAYER_INFO_FLAG_REMOVED,
};
use crate::{
    ControlPlayerInfoEntry, PlayerInfoControlData, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
    CLIENT_PLAYER_INFO_FLAG_INITIAL,
};

/// `C4PlayerInfoList`'s synchronized per-client player-info registry.
#[derive(Debug, Default)]
pub struct ControlPlayerInfoRegistry {
    clients: Vec<ClientPlayerInfos>,
    last_player_id: i32,
}

#[derive(Debug)]
struct ClientPlayerInfos {
    client_id: i32,
    players: Vec<ControlPlayerInfoEntry>,
}

impl ControlPlayerInfoRegistry {
    /// Apply the ID-allocation and slot-pruning portion of the host's
    /// `HandlePlayerInfoUpdRequest` path. Nonzero IDs remain untouched exactly
    /// like `C4PlayerInfoList::AssignPlayerIDs`
    /// (src/C4PlayerInfo.cpp:781-807,1765-1775).
    pub fn admit_request(
        &mut self,
        mut request: PlayerInfoUpdateRequest,
        max_players: usize,
    ) -> Option<PlayerInfoControlData> {
        if request.players.is_empty() && request.flags & CLIENT_PLAYER_INFO_FLAG_INITIAL == 0 {
            return None;
        }
        let startup_count = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .filter(|player| player.flags & PLAYER_INFO_FLAG_REMOVED == 0)
            .count();
        let free_slots = max_players.saturating_sub(startup_count);
        let mut joins_granted = 0usize;
        request.players.retain_mut(|player| {
            if player.id != 0 {
                return true;
            }
            if joins_granted >= free_slots {
                return false;
            }
            self.last_player_id = self.last_player_id.wrapping_add(1);
            player.id = self.last_player_id;
            joins_granted += 1;
            true
        });
        if request.players.is_empty()
            && request.flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0
        {
            return None;
        }
        Some(PlayerInfoControlData {
            client_id: request.client_id,
            flags: request.flags,
            players: request.players,
            by_client: 0,
        })
    }

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

#[derive(Debug)]
pub enum RemoteEmbeddedPlayerData {
    PlayerFile(PlayerFile),
    ScriptWithoutFile,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveRemoteEmbeddedPlayerDataError {
    #[error("player {info_id} join is resource-backed, not embedded")]
    ResourceBacked { info_id: i32 },
    #[error("user player {info_id} has no embedded player file data")]
    MissingPlayerData { info_id: i32 },
    #[error("embedded player data for player {info_id} is not a gzip archive")]
    UnsupportedArchiveMagic { info_id: i32 },
    #[error("failed to load embedded player data for player {info_id}: {source}")]
    PlayerDataLoad {
        info_id: i32,
        #[source]
        source: ScenarioError,
    },
}

pub fn resolve_remote_embedded_player_data(
    join: &JoinPlayerControlData,
    info: &ControlPlayerInfoEntry,
) -> Result<RemoteEmbeddedPlayerData, ResolveRemoteEmbeddedPlayerDataError> {
    let JoinPlayerSource::Embedded(data) = &join.source else {
        return Err(ResolveRemoteEmbeddedPlayerDataError::ResourceBacked { info_id: info.id });
    };
    if data.is_empty() {
        if info.is_script_player() {
            return Ok(RemoteEmbeddedPlayerData::ScriptWithoutFile);
        }
        return Err(ResolveRemoteEmbeddedPlayerDataError::MissingPlayerData { info_id: info.id });
    }
    if !matches!(data.as_slice(), [0x1e, 0x8c, ..] | [0x1f, 0x8b, ..]) {
        return Err(
            ResolveRemoteEmbeddedPlayerDataError::UnsupportedArchiveMagic { info_id: info.id },
        );
    }
    let label = std::path::PathBuf::from(join.filename.to_string_lossy().into_owned());
    PlayerFile::load_from_bytes(label, data.clone())
        .map(RemoteEmbeddedPlayerData::PlayerFile)
        .map_err(
            |source| ResolveRemoteEmbeddedPlayerDataError::PlayerDataLoad {
                info_id: info.id,
                source,
            },
        )
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
    if input.info.no_scenario_init() {
        return Err(PrepareJoinPlayerError::UnsupportedNoScenarioInit {
            info_id: input.info.id,
        });
    }
    let script_defaults =
        (input.player_file.is_none() && input.info.is_script_player()).then(|| PlayerFile {
            name: "Neuling".to_string(),
            score: 0,
            total_playing_time: 0,
            pref_color: 0,
            pref_color_dw: 0xff,
            pref_position: 0,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
        });
    let file = input.player_file.or(script_defaults.as_ref()).ok_or(
        PrepareJoinPlayerError::MissingPlayerData {
            info_id: input.info.id,
        },
    )?;
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
    fn host_admission_assigns_the_next_id_and_preserves_the_claimed_client() {
        // AssignPlayerIDs changes only zero IDs to ++iLastPlayerID, then the
        // host constructs C4ControlPlayerInfo without rebinding the packet's
        // client ID (src/C4PlayerInfo.cpp:781-807;
        // src/C4Network2Players.cpp:160-205,232-239).
        let mut registry = ControlPlayerInfoRegistry::default();
        let existing = registry
            .admit_request(
                crate::PlayerInfoUpdateRequest {
                    client_id: 1,
                    flags: 0,
                    players: vec![player(0); 7],
                },
                8,
            )
            .expect("seven free player slots accept the first request");
        registry.apply(existing);
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![player(0)],
        };

        let admitted = registry
            .admit_request(request, 8)
            .expect("one free player slot accepts the request");

        assert_eq!((admitted.client_id, admitted.by_client), (3, 0));
        assert_eq!(admitted.flags, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
        let [admitted_player] = admitted.players.as_slice() else {
            panic!("expected one admitted player");
        };
        assert_eq!(admitted_player.id, 8);
    }

    #[test]
    fn host_admission_rejects_an_empty_non_initial_request() {
        // HandlePlayerInfoUpdRequest drops an empty packet unless it carries
        // CIF_Initial, before ID assignment or direct PlayerInfo emission
        // (src/C4Network2Players.cpp:167-190).
        let mut registry = ControlPlayerInfoRegistry::default();
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: 0,
            players: Vec::new(),
        };

        assert_eq!(registry.admit_request(request, 8), None);
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

    #[test]
    fn script_player_without_file_prepares_cpp_core_defaults() {
        // C4Player::Init permits a missing core file only for script players;
        // C4PlayerInfoCore defaults remain in force before PlayerInfo supplies
        // name/team/color (src/C4Player.cpp:256-284;
        // src/C4InfoCore.cpp:66-85).
        let info = ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(b"Script Tyler".to_vec())
                .expect("valid legacy name"),
            id: 9,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            color: 0x0044_5566,
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: 9,
            ..Default::default()
        };

        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 1,
        })
        .expect("script player prepares without a file");

        assert_eq!(config.name, "Script Tyler");
        assert_eq!(config.player_info_id, 9);
        assert_eq!((config.score, config.total_playing_time), (0, 0));
        assert_eq!(config.color_dw, 0x0044_5566);
        assert_eq!((config.pref_color, config.pref_position), (0, 0));
        assert!(config.crew.is_empty());
        assert!(!config.control_style);
        assert!(!config.auto_context_menu);
    }

    #[test]
    fn remote_embedded_join_uses_player_data_not_the_transmitted_path() {
        // Remote non-resource joins save PlrData and load that temporary .c4p;
        // the transmitted Filename is not opened (src/C4Control.cpp:731-744).
        let join = JoinPlayerControlData {
            filename: crate::LegacyCString::from_bytes(
                b"/definitely/missing/RemotePlayer.c4p".to_vec(),
            )
            .expect("valid legacy filename"),
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(
                include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec(),
            ),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let RemoteEmbeddedPlayerData::PlayerFile(file) =
            resolve_remote_embedded_player_data(&join, &info)
                .expect("embedded player data resolves")
        else {
            panic!("user player data must resolve to a player file");
        };

        assert_eq!((file.name.as_str(), file.score), ("Embedded Tyler", 42));
    }

    #[test]
    fn remote_embedded_join_rejects_non_gzip_player_data() {
        // CStdFile recognizes packed C4Groups only by the custom 1e8c or
        // standard 1f8b gzip magic (src/CStdFile.cpp:92-107;
        // src/StdGzCompressedFile.cpp:62-114).
        let mut data = include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec();
        data[..2].copy_from_slice(&[0, 0]);
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(data),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let error = resolve_remote_embedded_player_data(&join, &info)
            .expect_err("raw or non-gzip player data must be rejected");

        assert!(matches!(
            error,
            ResolveRemoteEmbeddedPlayerDataError::UnsupportedArchiveMagic { info_id: 7 }
        ));
    }

    #[test]
    fn remote_user_join_requires_embedded_player_data() {
        // A remote non-resource user join with empty PlrData is rejected as a
        // ghost player (src/C4Control.cpp:750-755).
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(Vec::new()),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let error = resolve_remote_embedded_player_data(&join, &info)
            .expect_err("empty user player data must be rejected");

        assert!(matches!(
            error,
            ResolveRemoteEmbeddedPlayerDataError::MissingPlayerData { info_id: 7 }
        ));
    }

    #[test]
    fn remote_script_join_accepts_empty_player_data() {
        // Remote script players join without a player filename when PlrData is
        // empty (src/C4Control.cpp:745-749).
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(Vec::new()),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };

        let resolved = resolve_remote_embedded_player_data(&join, &info)
            .expect("empty script player data is valid");

        assert!(matches!(
            resolved,
            RemoteEmbeddedPlayerData::ScriptWithoutFile
        ));
    }
}
