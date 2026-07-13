use lc_engine::{
    ControlPlayerInfoRegistry, InitialNetworkTeamMetadata, PlayerInfoControlData,
    PlayerInfoUpdateRequest,
};

use crate::prepared_host_bootstrap::ProcessInitialHostTeamAssignmentOracle;

/// Host-owned scenario team state after the initial PlayerInfo assignment.
///
/// C++ keeps this `C4TeamList` alive for later local and remote runtime
/// PlayerInfo requests (`src/C4Network2Players.cpp:160-205`).
#[derive(Debug, Clone)]
pub(crate) struct NetworkTeamAssignmentState {
    teams: InitialNetworkTeamMetadata,
}

impl NetworkTeamAssignmentState {
    pub(crate) fn from_prepared_host(teams: InitialNetworkTeamMetadata) -> Self {
        Self { teams }
    }

    pub(crate) fn teams_mut(&mut self) -> &mut InitialNetworkTeamMetadata {
        &mut self.teams
    }

    pub(crate) fn admit_request(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
    ) -> Option<PlayerInfoControlData> {
        player_infos.admit_remote_request_with_runtime_teams(
            request,
            max_players,
            &mut self.teams,
            &mut ProcessInitialHostTeamAssignmentOracle,
        )
    }
}
