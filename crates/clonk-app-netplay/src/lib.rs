pub mod client_network_scenario;
pub mod client_start_barrier;
pub mod configured_client_players;
pub mod control_message;
pub mod host_game_resource_sources;
pub mod network;
pub mod network_host_preparation;
pub mod prepared_host_bootstrap;
pub mod resource_path_identity;

// Real-content suites for the host resource/bootstrap chain. They live in the
// library test harness instead of Cargo integration targets so the package
// keeps a single test binary.
#[cfg(test)]
mod host_game_resource_sources_tests;
#[cfg(test)]
mod prepared_host_bootstrap_tests;

pub use client_network_scenario::{
    compose_client_network_scenario, compose_client_network_scenario_with_maker_bytes,
    resolve_client_game_resources, resolve_client_scenario_resources, ClientNetworkScenarioError,
    ClientScenarioResources, ClientStartResourceRole, PendingClientStartResource,
    ResolvedClientStartResource,
};
pub use client_start_barrier::ClientStartBarrier;
pub use configured_client_players::{
    configured_native_boolean, configured_native_dynamic_value, configured_native_scalar,
    configured_native_value, load_configured_client_players, load_configured_mission_access,
    load_snapshotted_client_players, snapshot_configured_client_player_selection,
    update_configured_native_values, ConfiguredClientPlayerSelection, ConfiguredClientPlayers,
    ConfiguredClientPlayersError, NativeConfigValue,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use clonk_engine::{
    player_file::PlayerFile, ControlPlayerInfoEntry, ControlPlayerInfoRegistry, LegacyCString,
    NetworkResourceCore, PlayerInfoControlData, PlayerInfoUpdateRequest,
    CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS, CLIENT_PLAYER_INFO_FLAG_INITIAL,
    PLAYER_INFO_FLAG_HAS_RESOURCE,
};
use clonk_network::ClientPlayerResourceRequest;
use thiserror::Error;

/// One startup-selected local player before its client-owned network resource
/// has been published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedClientPlayer {
    source_path: PathBuf,
    module_filename: LegacyCString,
    resource_lookup_name: LegacyCString,
    resource_wire_name: LegacyCString,
    resource_opened_name: LegacyCString,
    player_name: LegacyCString,
    player_name_valid: bool,
    network_color: u32,
    alternate_color: u32,
    player_file: PlayerFile,
}

impl SelectedClientPlayer {
    pub fn new(
        source_path: impl Into<PathBuf>,
        wire_name: LegacyCString,
        player_file: PlayerFile,
    ) -> Self {
        let player_name =
            LegacyCString::from_bytes(clonk_script::c4_string_bytes(&player_file.name));
        let network_color = player_file.normalized_preferred_color();
        let alternate_color = player_file.normalized_alternate_color();
        Self {
            source_path: source_path.into(),
            module_filename: wire_name.clone(),
            resource_lookup_name: wire_name.clone(),
            resource_opened_name: wire_name.clone(),
            resource_wire_name: wire_name,
            player_name: player_name.clone().unwrap_or_default(),
            player_name_valid: player_name.is_some(),
            network_color,
            alternate_color,
            player_file,
        }
    }

    // Keep the classic loader's independently normalized path, resource-name,
    // player-name, and color fields explicit at this handoff. Combining them
    // would hide which C4PlayerInfoCore value each argument preserves.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_configured(
        source_path: PathBuf,
        module_filename: LegacyCString,
        resource_lookup_name: LegacyCString,
        resource_wire_name: LegacyCString,
        resource_opened_name: LegacyCString,
        player_name: LegacyCString,
        network_color: u32,
        alternate_color: u32,
        player_file: PlayerFile,
    ) -> Self {
        Self {
            source_path,
            module_filename,
            resource_lookup_name,
            resource_wire_name,
            resource_opened_name,
            player_name,
            player_name_valid: true,
            network_color,
            alternate_color,
            player_file,
        }
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn wire_name(&self) -> &LegacyCString {
        &self.resource_wire_name
    }

    pub fn module_filename(&self) -> &LegacyCString {
        &self.module_filename
    }

    pub fn resource_lookup_name(&self) -> &LegacyCString {
        &self.resource_lookup_name
    }

    pub fn resource_wire_name(&self) -> &LegacyCString {
        &self.resource_wire_name
    }

    pub fn resource_opened_name(&self) -> &LegacyCString {
        &self.resource_opened_name
    }

    pub fn player_name(&self) -> &LegacyCString {
        &self.player_name
    }

    pub fn network_color(&self) -> u32 {
        self.network_color
    }

    /// Host-local `C4PlayerInfo::dwAlternateColor`. This is intentionally
    /// absent from synchronized `ControlPlayerInfoEntry` serialization.
    pub fn alternate_color(&self) -> u32 {
        self.alternate_color
    }

    pub fn player_file(&self) -> &PlayerFile {
        &self.player_file
    }

    /// Builds the initial player-info request sent after the host assigns this
    /// client ID and the selected `.c4p` has a published resource core.
    pub fn initial_player_info_update(
        &self,
        client_id: i32,
        resource: NetworkResourceCore,
    ) -> Result<clonk_network::PlayerInfoUpdateRequest, SelectedClientPlayerError> {
        if !self.player_name_valid {
            return Err(SelectedClientPlayerError::PlayerNameContainsNul);
        }
        let color = self.network_color;
        Ok(clonk_network::PlayerInfoUpdateRequest::new(
            client_id,
            CLIENT_PLAYER_INFO_FLAG_INITIAL,
            vec![ControlPlayerInfoEntry {
                name: self.player_name.clone(),
                filename: self.module_filename.clone(),
                flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                color,
                original_color: color,
                resource: Some(resource),
                ..Default::default()
            }],
        ))
    }

    /// Builds the add-player request sent when an existing client selects a
    /// player file while the game is running.
    pub fn runtime_add_player_info_update(
        &self,
        client_id: i32,
        resource: NetworkResourceCore,
    ) -> Result<clonk_network::PlayerInfoUpdateRequest, SelectedClientPlayerError> {
        // JoinLocalPlayer(file, true) uses the same freshly loaded C4PlayerInfo
        // as initial joining, but C4ClientPlayerInfos selects CIF_AddPlayers
        // instead of CIF_Initial (src/C4PlayerInfo.cpp:357-395;
        // src/C4Network2Players.cpp:78-137).
        self.initial_player_info_update(client_id, resource)
            .map(|mut request| {
                request.flags = CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS;
                request
            })
    }

    /// Builds the local AddPlayers packet used by
    /// `C4PlayerList::CtrlJoinLocalNoNetwork`. Offline player infos retain
    /// their source filename and carry no network resource.
    pub fn offline_runtime_add_player_info_update(
        &self,
        client_id: i32,
    ) -> Result<PlayerInfoUpdateRequest, SelectedClientPlayerError> {
        if !self.player_name_valid {
            return Err(SelectedClientPlayerError::PlayerNameContainsNul);
        }
        let color = self.network_color;
        Ok(PlayerInfoUpdateRequest::new(
            client_id,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            vec![ControlPlayerInfoEntry {
                name: self.player_name.clone(),
                filename: self.module_filename.clone(),
                color,
                original_color: color,
                ..Default::default()
            }],
        ))
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SelectedClientPlayerError {
    #[error("selected player name contains an interior NUL")]
    PlayerNameContainsNul,
}

/// Builds the synchronized initial player-info packet for a teamless offline game.
pub fn build_teamless_offline_initial_player_info(
    configured: &ConfiguredClientPlayers,
    max_players: i32,
) -> PlayerInfoControlData {
    // Offline C4PlayerInfo::LoadFromLocalFile retains the configured module
    // filename without attaching a network resource. C4PlayerInfoList then
    // assigns IDs in packet order and prunes entries beyond the scenario
    // capacity (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-106,357-395,
    // 781-807,834-875,1273-1290).
    let players = configured
        .players()
        .iter()
        .map(|player| ControlPlayerInfoEntry {
            name: player.player_name().clone(),
            filename: player.module_filename().clone(),
            color: player.network_color,
            original_color: player.network_color,
            ..Default::default()
        })
        .collect();
    let mut registry = ControlPlayerInfoRegistry::default();
    registry
        .admit_request(
            PlayerInfoUpdateRequest::new(0, CLIENT_PLAYER_INFO_FLAG_INITIAL, players),
            usize::try_from(max_players).unwrap_or(0),
        )
        .expect("an initial player-info packet remains valid when empty")
}

/// Exact byte-preserving resource publication input for one configured player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfiguredClientPlayerResourceRequest {
    pub source_path: PathBuf,
    pub wire_name: LegacyCString,
    pub group_maker: LegacyCString,
}

/// Publishes configured participants using their raw C++ configuration names.
///
/// Like `C4Network2ResList::getRefRes` before `AddByFile`, duplicate source
/// paths reuse the first successfully published core.
pub fn publish_initial_configured_client_players<E>(
    client_id: i32,
    configured: &ConfiguredClientPlayers,
    mut publish: impl FnMut(ConfiguredClientPlayerResourceRequest) -> Result<NetworkResourceCore, E>,
) -> clonk_network::PlayerInfoUpdateRequest {
    let mut published_by_source = HashMap::<PathBuf, NetworkResourceCore>::new();
    let players = configured
        .players()
        .iter()
        .filter_map(|player| {
            let core = match published_by_source.get(&player.source_path) {
                Some(core) => core.clone(),
                None => {
                    let core = publish(ConfiguredClientPlayerResourceRequest {
                        source_path: player.source_path.clone(),
                        wire_name: player.resource_wire_name.clone(),
                        group_maker: configured.group_maker().clone(),
                    })
                    .ok()?;
                    published_by_source.insert(player.source_path.clone(), core.clone());
                    core
                }
            };
            player
                .initial_player_info_update(client_id, core)
                .ok()?
                .players
                .into_iter()
                .next()
        })
        .collect();
    clonk_network::PlayerInfoUpdateRequest::new(client_id, CLIENT_PLAYER_INFO_FLAG_INITIAL, players)
}

/// Publishes every configured local participant and combines the successful
/// entries into the one initial request used by the C++ join path.
pub fn publish_initial_client_players<E>(
    client_id: i32,
    selected: &[SelectedClientPlayer],
    group_maker: &str,
    mut publish: impl FnMut(ClientPlayerResourceRequest) -> Result<NetworkResourceCore, E>,
) -> clonk_network::PlayerInfoUpdateRequest {
    let mut published_by_source = HashMap::<PathBuf, NetworkResourceCore>::new();
    let players = selected
        .iter()
        .filter_map(|player| {
            let core = match published_by_source.get(&player.source_path) {
                Some(core) => core.clone(),
                None => {
                    let core = publish(ClientPlayerResourceRequest {
                        source_path: player.source_path.clone(),
                        wire_name: player.resource_wire_name.clone(),
                        group_maker: LegacyCString::from_bytes(group_maker.as_bytes().to_vec())
                            .unwrap_or_default(),
                    })
                    .ok()?;
                    published_by_source.insert(player.source_path.clone(), core.clone());
                    core
                }
            };
            player
                .initial_player_info_update(client_id, core)
                .ok()?
                .players
                .into_iter()
                .next()
        })
        .collect();
    clonk_network::PlayerInfoUpdateRequest::new(client_id, CLIENT_PLAYER_INFO_FLAG_INITIAL, players)
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use clonk_engine::{
        player_file::PlayerFile, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
        PlayerInfoUpdateRequest, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
        CLIENT_PLAYER_INFO_FLAG_INITIAL, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_TYPE_USER,
    };

    #[test]
    fn selected_client_player_builds_cpp_initial_player_info_request() {
        // C4ClientPlayerInfos retains the selected source filename, while
        // C4PlayerInfo::LoadFromLocalFile copies the player name and preferred
        // color, attaches the published NRT_Player core, and JoinLocalPlayer
        // wraps it in one CIF_Initial request after the host assigns a client ID
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104,357-395;
        // src/C4Network2Players.cpp:78-136).
        let source_path = PathBuf::from("/installed/Players/Alice.c4p");
        let wire_name = LegacyCString::from_bytes(b"Players/\x80lice.c4p".to_vec())
            .expect("fixture wire name is NUL-free");
        let player_file = PlayerFile {
            info_core: Default::default(),
            name: "Alice".to_string(),
            pref_color: 4,
            pref_color_dw: 0x12_34_56,
            pref_color2_dw: 0x00ab_cdef,
            ..Default::default()
        };
        let selected = super::SelectedClientPlayer::new(
            source_path.clone(),
            wire_name.clone(),
            player_file.clone(),
        );
        let resource = NetworkResourceCore {
            resource_type: 3,
            id: 7 << 16,
            loadable: true,
            filename: wire_name.clone(),
            ..Default::default()
        };

        assert_eq!(selected.source_path(), Path::new(&source_path));
        assert_eq!(selected.wire_name(), &wire_name);
        assert_eq!(selected.player_file(), &player_file);
        assert_eq!(selected.alternate_color(), 0x00ab_cdef);
        assert_eq!(
            selected
                .initial_player_info_update(7, resource.clone())
                .expect("valid player name builds an initial request"),
            clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![ControlPlayerInfoEntry {
                    name: LegacyCString::from_bytes(b"Alice".to_vec())
                        .expect("fixture player name is NUL-free"),
                    filename: wire_name,
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                    id: 0,
                    player_type: PLAYER_INFO_TYPE_USER,
                    color: 0x12_34_56,
                    original_color: 0x12_34_56,
                    resource: Some(resource),
                    ..Default::default()
                }],
            }
        );
    }

    #[test]
    fn selected_client_player_preserves_native_name_bytes_on_wire() {
        let selected = super::SelectedClientPlayer::new(
            "/installed/Players/Andre.c4p",
            LegacyCString::from_bytes(b"Andre.c4p".to_vec()).expect("valid wire name"),
            PlayerFile {
                name: clonk_script::c4_string_from_bytes(b"Andr\xe9"),
                ..PlayerFile::default()
            },
        );

        assert_eq!(selected.player_name().as_bytes(), b"Andr\xe9");
        assert_eq!(
            clonk_script::c4_string_bytes(&selected.player_file().name),
            b"Andr\xe9"
        );
    }

    #[test]
    fn selected_client_player_builds_cpp_runtime_add_request() {
        // JoinPlayer:<file> calls JoinLocalPlayer(file, true), whose
        // C4ClientPlayerInfos constructor sets CIF_AddPlayers before the
        // client sends PID_PlayerInfoUpdReq to the host
        // (pristine 9ffa0a5d src/C4MainMenu.cpp:760-771;
        // src/C4PlayerInfo.cpp:357-395;
        // src/C4Network2Players.cpp:78-137).
        let wire_name = LegacyCString::from_bytes(b"Players/Runtime.c4p".to_vec())
            .expect("fixture wire name is NUL-free");
        let selected = super::SelectedClientPlayer::new(
            "/installed/Players/Runtime.c4p",
            wire_name.clone(),
            PlayerFile {
                info_core: Default::default(),
                name: "Runtime".to_string(),
                pref_color: 4,
                pref_color_dw: 0x65_43_21,
                ..Default::default()
            },
        );
        let resource = NetworkResourceCore {
            resource_type: 3,
            id: 7 << 16,
            loadable: true,
            filename: wire_name.clone(),
            ..Default::default()
        };

        assert_eq!(
            selected
                .runtime_add_player_info_update(7, resource.clone())
                .expect("valid player name builds a runtime add request"),
            clonk_network::PlayerInfoUpdateRequest {
                client_id: 7,
                flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![ControlPlayerInfoEntry {
                    name: LegacyCString::from_bytes(b"Runtime".to_vec())
                        .expect("fixture player name is NUL-free"),
                    filename: wire_name,
                    flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                    id: 0,
                    player_type: PLAYER_INFO_TYPE_USER,
                    color: 0x65_43_21,
                    original_color: 0x65_43_21,
                    resource: Some(resource),
                    ..Default::default()
                }],
            }
        );
        assert_eq!(
            selected
                .offline_runtime_add_player_info_update(0)
                .expect("valid player name builds an offline runtime request"),
            PlayerInfoUpdateRequest {
                client_id: 0,
                flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![ControlPlayerInfoEntry {
                    name: LegacyCString::from_bytes(b"Runtime".to_vec()).unwrap(),
                    filename: selected.module_filename().clone(),
                    color: 0x65_43_21,
                    original_color: 0x65_43_21,
                    ..Default::default()
                }],
            }
        );
    }

    #[test]
    fn initial_request_publishes_all_loadable_participants_in_module_order() {
        // C4ClientPlayerInfos walks every semicolon-delimited participant in
        // module order, silently omits files LoadFromLocalFile cannot load,
        // and sends all remaining players in one CIF_Initial request
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:357-395;
        // src/C4Network2Players.cpp:78-136).
        let player = |name: &str| PlayerFile {
            info_core: Default::default(),
            name: name.to_string(),
            pref_color_dw: 0x11_22_33,
            ..Default::default()
        };
        let selected = [
            super::SelectedClientPlayer::new(
                "/players/Bravo.c4p",
                LegacyCString::from_bytes(b"Bravo.c4p".to_vec()).unwrap(),
                player("Bravo"),
            ),
            super::SelectedClientPlayer::new(
                "/players/Broken.c4p",
                LegacyCString::from_bytes(b"Broken.c4p".to_vec()).unwrap(),
                player("Broken"),
            ),
            super::SelectedClientPlayer::new(
                "/players/Alpha.c4p",
                LegacyCString::from_bytes(b"Alpha.c4p".to_vec()).unwrap(),
                player("Alpha"),
            ),
        ];
        let mut published = Vec::new();

        let request =
            super::publish_initial_client_players(7, &selected, "Network Player", |publication| {
                let wire_name = publication.wire_name.clone();
                published.push(publication);
                if wire_name.as_bytes() == b"Broken.c4p" {
                    return Err("broken player".to_string());
                }
                Ok(NetworkResourceCore {
                    resource_type: 3,
                    id: (7 << 16) + published.len() as i32 - 1,
                    loadable: true,
                    filename: wire_name,
                    ..Default::default()
                })
            });

        assert_eq!(
            published
                .iter()
                .map(|request| request.wire_name.as_bytes())
                .collect::<Vec<_>>(),
            vec![b"Bravo.c4p".as_slice(), b"Broken.c4p", b"Alpha.c4p"]
        );
        assert!(published
            .iter()
            .all(|request| request.group_maker.as_bytes() == b"Network Player"));
        assert_eq!(request.client_id, 7);
        assert_eq!(request.flags, CLIENT_PLAYER_INFO_FLAG_INITIAL);
        assert_eq!(
            request
                .players
                .iter()
                .map(|player| (player.name.as_bytes(), player.filename.as_bytes()))
                .collect::<Vec<_>>(),
            vec![
                (b"Bravo".as_slice(), b"Bravo.c4p".as_slice()),
                (b"Alpha".as_slice(), b"Alpha.c4p".as_slice()),
            ]
        );
    }

    #[test]
    fn configured_publication_caches_by_exact_source_path_and_preserves_raw_maker() {
        // LoadFromLocalFile looks up the Config.AtExeRelativePath source path,
        // and C4Network2ResList compares that file path with SEqual/strcmp.
        // Thus equal source paths reuse a core regardless of resource name,
        // while distinct source paths remain distinct even with equal names.
        // C4Group's global maker is the raw Config.General.Name byte string
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:96-101;
        // src/C4Network2Res.cpp:1397-1405;
        // src/C4Strings.cpp:104-108;
        // src/C4Application.cpp:118-121; src/C4Group.cpp:924-935).
        let player_file = |name: &str| PlayerFile {
            info_core: Default::default(),
            name: name.to_string(),
            pref_color_dw: 0x11_22_33,
            ..Default::default()
        };
        let raw = |bytes: &[u8]| LegacyCString::from_bytes(bytes.to_vec()).unwrap();
        let players = vec![
            super::SelectedClientPlayer::from_configured(
                PathBuf::from("/players/One.c4p"),
                raw(b"AliasOne.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Shared.c4p"),
                raw(b"One"),
                0x11_22_33,
                0,
                player_file("One"),
            ),
            super::SelectedClientPlayer::from_configured(
                PathBuf::from("/players/Two.c4p"),
                raw(b"AliasTwo.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Two"),
                0x11_22_33,
                0,
                player_file("Two"),
            ),
            super::SelectedClientPlayer::from_configured(
                PathBuf::from("/players/One.c4p"),
                raw(b"AliasThree.c4p"),
                raw(b"Renamed.c4p"),
                raw(b"Renamed.c4p"),
                raw(b"Renamed.c4p"),
                raw(b"Three"),
                0x11_22_33,
                0,
                player_file("Three"),
            ),
        ];
        let configured = super::ConfiguredClientPlayers::from_parts(players, raw(b"M\x80ker"));
        let mut publications = Vec::new();

        let request =
            super::publish_initial_configured_client_players(7, &configured, |publication| {
                let resource_id = (7 << 16) + publications.len() as i32;
                let filename = publication.wire_name.clone();
                publications.push(publication);
                Ok::<_, String>(NetworkResourceCore {
                    resource_type: 3,
                    id: resource_id,
                    loadable: true,
                    filename,
                    ..Default::default()
                })
            });

        assert_eq!(
            publications
                .iter()
                .map(|publication| (
                    publication.source_path.as_path(),
                    publication.wire_name.as_bytes(),
                    publication.group_maker.as_bytes(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    Path::new("/players/One.c4p"),
                    b"Shared.c4p".as_slice(),
                    b"M\x80ker".as_slice(),
                ),
                (
                    Path::new("/players/Two.c4p"),
                    b"Shared.c4p".as_slice(),
                    b"M\x80ker".as_slice(),
                ),
            ]
        );
        assert_eq!(
            request
                .players
                .iter()
                .map(|player| {
                    (
                        player.name.as_bytes(),
                        player.filename.as_bytes(),
                        player.resource.as_ref().map(|core| core.id),
                    )
                })
                .collect::<Vec<_>>(),
            vec![
                (b"One".as_slice(), b"AliasOne.c4p".as_slice(), Some(7 << 16)),
                (
                    b"Two".as_slice(),
                    b"AliasTwo.c4p".as_slice(),
                    Some((7 << 16) + 1),
                ),
                (
                    b"Three".as_slice(),
                    b"AliasThree.c4p".as_slice(),
                    Some(7 << 16),
                ),
            ]
        );
    }
}
