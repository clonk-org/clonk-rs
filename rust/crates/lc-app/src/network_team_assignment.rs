use lc_engine::{
    ControlPlayerInfoEntry, ControlPlayerInfoRegistry, InitialHostTeamAssignmentOracle,
    InitialNetworkTeam, InitialNetworkTeamDistribution, InitialNetworkTeamMetadata, LegacyCString,
    PlayerInfoControlData, PlayerInfoUpdateRequest, TeamColorUpdateError,
};
use thiserror::Error;

use crate::prepared_host_bootstrap::ProcessInitialHostTeamAssignmentOracle;

/// Host-owned scenario team state after the initial PlayerInfo assignment.
///
/// C++ keeps this `C4TeamList` alive for later local and remote runtime
/// PlayerInfo requests (`src/C4Network2Players.cpp:160-205`).
#[derive(Debug, Clone)]
pub(crate) struct NetworkTeamAssignmentState {
    teams: InitialNetworkTeamMetadata,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum NetworkTeamControlError {
    #[error("team distribution {0} is outside C4TeamList's 0..=4 range")]
    InvalidDistribution(i32),
    #[error(
        "this team distribution would generate teams whose localized names and process-random colors are unavailable"
    )]
    GeneratedTeamsUnavailable,
    #[error(transparent)]
    TeamColors(#[from] TeamColorUpdateError),
}

#[derive(Default)]
struct GeneratedTeamDetectingOracle {
    generated: bool,
}

impl InitialHostTeamAssignmentOracle for GeneratedTeamDetectingOracle {
    fn safe_random(&mut self, _range: i32) -> i32 {
        0
    }

    fn generate_team(
        &mut self,
        id: i32,
        _existing_teams: &[InitialNetworkTeam],
    ) -> InitialNetworkTeam {
        self.generated = true;
        InitialNetworkTeam {
            id,
            name: LegacyCString::default(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        }
    }
}

impl NetworkTeamAssignmentState {
    pub(crate) fn from_prepared_host(teams: InitialNetworkTeamMetadata) -> Self {
        Self { teams }
    }

    pub(crate) fn teams_mut(&mut self) -> &mut InitialNetworkTeamMetadata {
        &mut self.teams
    }

    pub(crate) fn teams(&self) -> &InitialNetworkTeamMetadata {
        &self.teams
    }

    /// Execute the host half of `C4TeamList::SetTeamDistribution`. A preview
    /// with a side-effect-free oracle proves that the exact operation will not
    /// request an unmodelled generated team before the process RNG or live
    /// registries are touched.
    pub(crate) fn set_distribution(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        value: i32,
        has_or_will_have_lobby: bool,
    ) -> Result<Vec<PlayerInfoControlData>, NetworkTeamControlError> {
        let distribution = match value {
            0 => InitialNetworkTeamDistribution::Free,
            1 => InitialNetworkTeamDistribution::Host,
            2 => InitialNetworkTeamDistribution::None,
            3 => InitialNetworkTeamDistribution::Random,
            4 => InitialNetworkTeamDistribution::RandomInvisible,
            other => return Err(NetworkTeamControlError::InvalidDistribution(other)),
        };

        if !matches!(
            distribution,
            InitialNetworkTeamDistribution::None
                | InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        ) {
            self.teams.team_distribution = distribution;
            return Ok(Vec::new());
        }

        let mut preview_teams = self.teams.clone();
        preview_teams.team_distribution = distribution;
        let mut preview_infos = player_infos.clone();
        let mut detector = GeneratedTeamDetectingOracle::default();
        let _ = preview_infos.reassign_all_teams(
            &mut preview_teams,
            &mut detector,
            has_or_will_have_lobby,
        );
        if detector.generated {
            return Err(NetworkTeamControlError::GeneratedTeamsUnavailable);
        }

        self.teams.team_distribution = distribution;
        Ok(player_infos.reassign_all_teams(
            &mut self.teams,
            &mut ProcessInitialHostTeamAssignmentOracle,
            has_or_will_have_lobby,
        ))
    }

    /// Execute the host half of `C4TeamList::SetTeamColors`. Equal values are
    /// a true no-op. Attribute conflict paths that need host-local alternate
    /// colors fail in the registry before this synchronized flag changes.
    pub(crate) fn set_team_colors(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        enabled: bool,
        restore_players: &[ControlPlayerInfoEntry],
    ) -> Result<Vec<PlayerInfoControlData>, NetworkTeamControlError> {
        if self.teams.team_colors == enabled {
            return Ok(Vec::new());
        }
        let updates = player_infos.update_team_colors(&self.teams, enabled, restore_players)?;
        self.teams.team_colors = enabled;
        Ok(updates)
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
