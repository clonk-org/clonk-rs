#[cfg(not(windows))]
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use lc_app::{
    build_teamless_offline_initial_player_info, ConfiguredClientPlayers, SelectedClientPlayer,
};

struct OfflineStartupPlayer {
    info_id: i32,
    selected: SelectedClientPlayer,
}

pub(super) struct OfflineStartupPlayers {
    pub(super) player_info: lc_engine::PlayerInfoControlData,
    players: Vec<OfflineStartupPlayer>,
}

impl OfflineStartupPlayers {
    pub(super) fn new(configured: ConfiguredClientPlayers, max_players: i32) -> Self {
        let player_info = build_teamless_offline_initial_player_info(&configured, max_players);
        let players = player_info
            .players
            .iter()
            .zip(configured.players())
            .map(|(info, selected)| OfflineStartupPlayer {
                info_id: info.id,
                selected: selected.clone(),
            })
            .collect();
        Self {
            player_info,
            players,
        }
    }

    pub(super) fn startup_player_count(&self) -> i32 {
        i32::try_from(self.players.len()).unwrap_or(i32::MAX)
    }

    pub(super) fn selected(&self, info_id: i32) -> Option<&SelectedClientPlayer> {
        self.players
            .iter()
            .find(|player| player.info_id == info_id)
            .map(|player| &player.selected)
    }
}

#[cfg(not(windows))]
pub(super) fn offline_player_real_path(path: &Path) -> io::Result<PathBuf> {
    // C++ RealPath delegates to POSIX realpath for an existing player file
    // (pristine 9ffa0a5d src/StdFile.cpp:114-145,696-707).
    fs::canonicalize(path)
}

#[cfg(windows)]
pub(super) fn offline_player_real_path(path: &Path) -> io::Result<PathBuf> {
    // C++ uses _fullpath on Windows, then compares without case
    // (pristine 9ffa0a5d src/StdFile.cpp:114-118,696-707).
    std::path::absolute(path)
}

#[cfg(not(windows))]
pub(super) fn offline_player_paths_identical(left: &Path, right: &Path) -> bool {
    left == right
}

#[cfg(windows)]
pub(super) fn offline_player_paths_identical(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}
