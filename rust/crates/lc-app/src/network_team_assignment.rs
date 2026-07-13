use lc_engine::InitialNetworkTeamMetadata;

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
}
