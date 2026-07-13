mod client_network_scenario;
mod configured_client_players;

pub use configured_client_players::{
    load_configured_client_players, load_snapshotted_client_players,
    snapshot_configured_client_player_selection, ConfiguredClientPlayerSelection,
    ConfiguredClientPlayers, ConfiguredClientPlayersError,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lc_engine::{
    player_file::PlayerFile, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    CLIENT_PLAYER_INFO_FLAG_INITIAL, PLAYER_INFO_FLAG_HAS_RESOURCE,
};
use lc_network::ClientPlayerResourceRequest;
use thiserror::Error;

/// One startup-selected local player before its client-owned network resource
/// has been published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedClientPlayer {
    source_path: PathBuf,
    module_filename: LegacyCString,
    resource_wire_name: LegacyCString,
    player_name: LegacyCString,
    player_name_valid: bool,
    network_color: u32,
    player_file: PlayerFile,
}

impl SelectedClientPlayer {
    pub fn new(
        source_path: impl Into<PathBuf>,
        wire_name: LegacyCString,
        player_file: PlayerFile,
    ) -> Self {
        let player_name = LegacyCString::from_bytes(player_file.name.as_bytes().to_vec());
        let network_color = player_file.normalized_preferred_color();
        Self {
            source_path: source_path.into(),
            module_filename: wire_name.clone(),
            resource_wire_name: wire_name,
            player_name: player_name.clone().unwrap_or_default(),
            player_name_valid: player_name.is_some(),
            network_color,
            player_file,
        }
    }

    pub(crate) fn from_configured(
        source_path: PathBuf,
        module_filename: LegacyCString,
        resource_wire_name: LegacyCString,
        player_name: LegacyCString,
        network_color: u32,
        player_file: PlayerFile,
    ) -> Self {
        Self {
            source_path,
            module_filename,
            resource_wire_name,
            player_name,
            player_name_valid: true,
            network_color,
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

    pub fn resource_wire_name(&self) -> &LegacyCString {
        &self.resource_wire_name
    }

    pub fn player_name(&self) -> &LegacyCString {
        &self.player_name
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
    ) -> Result<lc_network::PlayerInfoUpdateRequest, SelectedClientPlayerError> {
        if !self.player_name_valid {
            return Err(SelectedClientPlayerError::PlayerNameContainsNul);
        }
        let color = self.network_color;
        Ok(lc_network::PlayerInfoUpdateRequest {
            client_id,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![ControlPlayerInfoEntry {
                name: self.player_name.clone(),
                filename: self.module_filename.clone(),
                flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                color,
                original_color: color,
                resource: Some(resource),
                ..Default::default()
            }],
        })
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SelectedClientPlayerError {
    #[error("selected player name contains an interior NUL")]
    PlayerNameContainsNul,
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
/// Like `C4Network2ResList::getRefRes` before `AddByFile`, duplicate resource
/// names reuse the first successfully published core.
pub fn publish_initial_configured_client_players<E>(
    client_id: i32,
    configured: &ConfiguredClientPlayers,
    mut publish: impl FnMut(ConfiguredClientPlayerResourceRequest) -> Result<NetworkResourceCore, E>,
) -> lc_network::PlayerInfoUpdateRequest {
    let mut published_by_name = HashMap::<Vec<u8>, NetworkResourceCore>::new();
    let players = configured
        .players()
        .iter()
        .filter_map(|player| {
            let resource_name = player.resource_wire_name.as_bytes().to_vec();
            let core = match published_by_name.get(&resource_name) {
                Some(core) => core.clone(),
                None => {
                    let core = publish(ConfiguredClientPlayerResourceRequest {
                        source_path: player.source_path.clone(),
                        wire_name: player.resource_wire_name.clone(),
                        group_maker: configured.group_maker().clone(),
                    })
                    .ok()?;
                    published_by_name.insert(resource_name, core.clone());
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
    lc_network::PlayerInfoUpdateRequest {
        client_id,
        flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
        players,
    }
}

/// Publishes every configured local participant and combines the successful
/// entries into the one initial request used by the C++ join path.
pub fn publish_initial_client_players<E>(
    client_id: i32,
    selected: &[SelectedClientPlayer],
    group_maker: &str,
    mut publish: impl FnMut(ClientPlayerResourceRequest) -> Result<NetworkResourceCore, E>,
) -> lc_network::PlayerInfoUpdateRequest {
    let mut published_by_name = HashMap::<Vec<u8>, NetworkResourceCore>::new();
    let players = selected
        .iter()
        .filter_map(|player| {
            let resource_name = player.resource_wire_name.as_bytes().to_vec();
            let core = match published_by_name.get(&resource_name) {
                Some(core) => core.clone(),
                None => {
                    let core = publish(ClientPlayerResourceRequest {
                        source_path: player.source_path.clone(),
                        wire_name: player.resource_wire_name.clone(),
                        group_maker: LegacyCString::from_bytes(group_maker.as_bytes().to_vec())
                            .unwrap_or_default(),
                    })
                    .ok()?;
                    published_by_name.insert(resource_name, core.clone());
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
    lc_network::PlayerInfoUpdateRequest {
        client_id,
        flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
        players,
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use lc_engine::{
        player_file::PlayerFile, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
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
            name: "Alice".to_string(),
            score: 0,
            total_playing_time: 0,
            pref_color: 4,
            pref_color_dw: 0x12_34_56,
            pref_position: 0,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
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
        assert_eq!(
            selected
                .initial_player_info_update(7, resource.clone())
                .expect("valid player name builds an initial request"),
            lc_network::PlayerInfoUpdateRequest {
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
    fn initial_request_publishes_all_loadable_participants_in_module_order() {
        // C4ClientPlayerInfos walks every semicolon-delimited participant in
        // module order, silently omits files LoadFromLocalFile cannot load,
        // and sends all remaining players in one CIF_Initial request
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:357-395;
        // src/C4Network2Players.cpp:78-136).
        let player = |name: &str| PlayerFile {
            name: name.to_string(),
            score: 0,
            total_playing_time: 0,
            pref_color: 0,
            pref_color_dw: 0x11_22_33,
            pref_position: 0,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
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
    fn configured_publication_reuses_duplicate_core_and_preserves_raw_maker() {
        // LoadFromLocalFile first reuses an existing local resource by its
        // Config.AtExeRelativePath name; only the first miss reaches AddByFile.
        // C4Group's global maker is the raw Config.General.Name byte string
        // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:70-104;
        // src/C4Application.cpp:118-121; src/C4Group.cpp:924-935).
        let player_file = |name: &str| PlayerFile {
            name: name.to_string(),
            score: 0,
            total_playing_time: 0,
            pref_color: 0,
            pref_color_dw: 0x11_22_33,
            pref_position: 0,
            pref_control_style: false,
            pref_auto_context_menu: false,
            crew: Vec::new(),
        };
        let raw = |bytes: &[u8]| LegacyCString::from_bytes(bytes.to_vec()).unwrap();
        let players = vec![
            super::SelectedClientPlayer::from_configured(
                PathBuf::from("/players/Shared.c4p"),
                raw(b"AliasOne.c4p"),
                raw(b"Shared.c4p"),
                raw(b"One"),
                0x11_22_33,
                player_file("One"),
            ),
            super::SelectedClientPlayer::from_configured(
                PathBuf::from("/players/Shared.c4p"),
                raw(b"AliasTwo.c4p"),
                raw(b"Shared.c4p"),
                raw(b"Two"),
                0x11_22_33,
                player_file("Two"),
            ),
        ];
        let configured = super::ConfiguredClientPlayers::from_parts(players, raw(b"M\x80ker"));
        let mut publications = Vec::new();

        let request =
            super::publish_initial_configured_client_players(7, &configured, |publication| {
                publications.push(publication);
                Ok::<_, String>(NetworkResourceCore {
                    resource_type: 3,
                    id: 7 << 16,
                    loadable: true,
                    filename: raw(b"Shared.c4p"),
                    ..Default::default()
                })
            });

        assert_eq!(publications.len(), 1);
        assert_eq!(publications[0].wire_name.as_bytes(), b"Shared.c4p");
        assert_eq!(publications[0].group_maker.as_bytes(), b"M\x80ker");
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
                (b"Two".as_slice(), b"AliasTwo.c4p".as_slice(), Some(7 << 16)),
            ]
        );
    }
}
