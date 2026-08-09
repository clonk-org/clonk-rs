use clonk_engine::{
    ControlPlayerInfoEntry, ControlPlayerInfoRegistry, InitialHostTeamAssignmentOracle,
    InitialNetworkTeamDistribution, InitialNetworkTeamMetadata, LegacyCString, PlayerInfoAdmission,
    PlayerInfoControlData, PlayerInfoUpdateRequest, TeamColorUpdateError,
};
use thiserror::Error;

use crate::prepared_host_bootstrap::ProcessInitialHostTeamAssignmentOracle;

/// Synchronized scenario team state after the initial PlayerInfo assignment.
///
/// Every peer keeps this `C4TeamList` alive while incoming PlayerInfo packets
/// generate/recheck teams; only the control host admits later update requests
/// (`src/C4Network2Players.cpp:160-205,245-258`).
#[derive(Debug, Clone)]
pub(crate) struct NetworkTeamAssignmentState {
    teams: InitialNetworkTeamMetadata,
    generated_team_name_template: LegacyCString,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum NetworkTeamControlError {
    #[error("team distribution {0} is outside C4TeamList's 0..=4 range")]
    InvalidDistribution(i32),
    #[error(transparent)]
    TeamColors(#[from] TeamColorUpdateError),
}

impl NetworkTeamAssignmentState {
    pub(crate) fn from_prepared_host(teams: InitialNetworkTeamMetadata) -> Self {
        Self::from_prepared_host_with_team_name_template(
            teams,
            LegacyCString::from_bytes(b"Team %d".to_vec())
                .expect("the shipped team-name resource contains no NUL"),
        )
    }

    pub(crate) fn from_prepared_host_with_team_name_template(
        teams: InitialNetworkTeamMetadata,
        generated_team_name_template: LegacyCString,
    ) -> Self {
        Self {
            teams,
            generated_team_name_template,
        }
    }

    pub(crate) fn teams_mut(&mut self) -> &mut InitialNetworkTeamMetadata {
        &mut self.teams
    }

    pub(crate) fn teams(&self) -> &InitialNetworkTeamMetadata {
        &self.teams
    }

    pub(crate) fn set_generated_team_name_template(&mut self, template: LegacyCString) {
        self.generated_team_name_template = template;
    }

    /// `C4TeamList::GetGenerateTeamByID`: while multiteams are active, create
    /// default teams through a missing positive ID. `TEAMID_New` (-1) asks the
    /// generator for the next ID, although the caller still retains -1 in its
    /// copied PlayerInfo exactly like the native restart hook.
    pub(crate) fn generate_team_for_id(&mut self, requested_id: i32) -> bool {
        if !self.teams.active {
            return false;
        }
        let target_id = if requested_id == -1 {
            self.teams
                .teams
                .iter()
                .map(|team| team.id)
                .max()
                .unwrap_or(0)
                .wrapping_add(1)
        } else {
            requested_id
        };
        if target_id <= self.teams.last_team_id {
            return false;
        }
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        while self.teams.last_team_id < target_id {
            let id = self.teams.last_team_id.wrapping_add(1);
            let generated = oracle.generate_team(id, &self.teams.teams);
            self.teams.last_team_id = id;
            self.teams.teams.push(generated);
        }
        true
    }

    /// Run the control host's post-PlayerInfo random-team reconciliation on
    /// the retained team state. Tie breaking shares C++'s process-global
    /// `SafeRandom` stream with initial and runtime team assignment.
    pub(crate) fn recheck_random_teams(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
    ) -> Vec<PlayerInfoControlData> {
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        player_infos.recheck_random_teams(&mut self.teams, &mut oracle)
    }

    /// Execute the host half of `C4TeamList::SetTeamDistribution`.
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

        self.teams.team_distribution = distribution;
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        Ok(player_infos.reassign_all_teams(&mut self.teams, &mut oracle, has_or_will_have_lobby))
    }

    /// Execute the network host's direct `C4TeamList::SetRandomTeamCount`.
    pub(crate) fn set_random_team_count(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        count: i32,
        has_or_will_have_lobby: bool,
    ) -> Vec<PlayerInfoControlData> {
        self.teams.random_team_count = count;
        if !matches!(
            self.teams.team_distribution,
            InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        ) {
            return Vec::new();
        }
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        player_infos.reassign_all_teams(&mut self.teams, &mut oracle, has_or_will_have_lobby)
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
        self.set_team_colors_with_alternate_colors(player_infos, enabled, restore_players, |_| None)
    }

    pub(crate) fn set_team_colors_with_alternate_colors(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        enabled: bool,
        restore_players: &[ControlPlayerInfoEntry],
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<Vec<PlayerInfoControlData>, NetworkTeamControlError> {
        if self.teams.team_colors == enabled {
            return Ok(Vec::new());
        }
        let mut updated_teams = self.teams.clone();
        updated_teams.team_colors = enabled;
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        let updates = player_infos.refresh_player_attributes_with_alternate_colors(
            Some(&updated_teams),
            restore_players,
            &mut oracle,
            alternate_color,
        )?;
        self.teams = updated_teams;
        Ok(updates)
    }

    pub(crate) fn admit_request(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        by_host: bool,
        has_or_will_have_lobby: bool,
        restore_players: &[ControlPlayerInfoEntry],
    ) -> Result<Option<PlayerInfoAdmission>, NetworkTeamControlError> {
        self.admit_request_with_alternate_colors(
            player_infos,
            request,
            max_players,
            by_host,
            has_or_will_have_lobby,
            restore_players,
            |_| None,
        )
    }

    pub(crate) fn admit_request_with_alternate_colors(
        &mut self,
        player_infos: &mut ControlPlayerInfoRegistry,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        by_host: bool,
        has_or_will_have_lobby: bool,
        restore_players: &[ControlPlayerInfoEntry],
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<Option<PlayerInfoAdmission>, NetworkTeamControlError> {
        let mut oracle =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template.clone());
        Ok(
            player_infos.admit_request_with_teams_and_attributes_and_alternate_colors(
                request,
                max_players,
                &mut self.teams,
                by_host,
                has_or_will_have_lobby,
                restore_players,
                &mut oracle,
                alternate_color,
            )?,
        )
    }
}

#[cfg(all(
    test,
    any(not(feature = "app-test-shard-mode"), feature = "app-test-shard-5",),
))]
mod tests {
    use super::*;

    #[test]
    fn random_recheck_regenerates_cpp_default_team_metadata() {
        let mut assignment = NetworkTeamAssignmentState::from_prepared_host_with_team_name_template(
            InitialNetworkTeamMetadata {
                active: true,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: false,
                auto_generate_teams: true,
                last_team_id: 7,
                team_distribution: InitialNetworkTeamDistribution::Random,
                team_colors: true,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 2,
                teams: vec![clonk_engine::InitialNetworkTeam {
                    id: 7,
                    name: LegacyCString::from_bytes(b"Old".to_vec()).unwrap(),
                    player_start_index: 0,
                    player_ids: Vec::new(),
                    color: 0,
                    icon_spec: LegacyCString::default(),
                    max_players: 0,
                }],
            },
            LegacyCString::from_bytes(b"Mannschaft %d".to_vec()).unwrap(),
        );
        let mut player_infos = ControlPlayerInfoRegistry::default();

        assert!(assignment
            .recheck_random_teams(&mut player_infos)
            .is_empty());
        assert_eq!(
            assignment
                .teams()
                .teams
                .iter()
                .map(|team| (team.id, team.name.as_bytes(), team.color))
                .collect::<Vec<_>>(),
            vec![
                (1, b"Mannschaft 1".as_slice(), 0x00f4_0000),
                (2, b"Mannschaft 2".as_slice(), 0x0000_c800),
            ],
        );
    }
}
