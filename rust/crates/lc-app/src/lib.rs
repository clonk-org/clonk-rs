use std::path::{Path, PathBuf};

use lc_engine::{
    player_file::PlayerFile, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    CLIENT_PLAYER_INFO_FLAG_INITIAL, PLAYER_INFO_FLAG_HAS_RESOURCE,
};
use thiserror::Error;

/// One startup-selected local player before its client-owned network resource
/// has been published.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedClientPlayer {
    source_path: PathBuf,
    wire_name: LegacyCString,
    player_file: PlayerFile,
}

impl SelectedClientPlayer {
    pub fn new(
        source_path: impl Into<PathBuf>,
        wire_name: LegacyCString,
        player_file: PlayerFile,
    ) -> Self {
        Self {
            source_path: source_path.into(),
            wire_name,
            player_file,
        }
    }

    pub fn source_path(&self) -> &Path {
        &self.source_path
    }

    pub fn wire_name(&self) -> &LegacyCString {
        &self.wire_name
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
        let name = LegacyCString::from_bytes(self.player_file.name.as_bytes().to_vec())
            .ok_or(SelectedClientPlayerError::PlayerNameContainsNul)?;
        let color = self.player_file.normalized_preferred_color();
        Ok(lc_network::PlayerInfoUpdateRequest {
            client_id,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![ControlPlayerInfoEntry {
                name,
                filename: self.wire_name.clone(),
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
}
