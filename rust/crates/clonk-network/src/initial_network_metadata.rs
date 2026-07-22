//! Adapters from neutral engine scenario metadata to synchronized network data.

use clonk_engine::{InitialNetworkScenarioMetadata, InitialNetworkTeamMetadata, ScenarioIdListEntry};
use thiserror::Error;

use crate::{
    InitialNetworkScenarioDefaults, JoinDataC4Id, JoinDataIdListEntry, JoinGameParametersEnvelope,
    JoinTeamListSnapshot, JoinTeamSnapshot,
};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InitialNetworkMetadataError {
    #[error("scenario ID is not an exact four-byte legacy C4ID: {0:?}")]
    InvalidScenarioId(String),
}

/// Adapts the scenario values used as `C4GameParameters::CompileFunc`
/// defaults, including ordered C4IDList entries.
pub fn initial_network_scenario_defaults(
    metadata: &InitialNetworkScenarioMetadata,
) -> Result<InitialNetworkScenarioDefaults, InitialNetworkMetadataError> {
    Ok(InitialNetworkScenarioDefaults {
        random_seed: metadata.random_seed,
        max_players: metadata.max_players,
        use_fair_crew: metadata.use_fair_crew,
        fair_crew_forced: metadata.fair_crew_forced,
        fair_crew_strength: metadata.fair_crew_strength,
        rules: join_data_id_list(&metadata.rules)?,
        goals: join_data_id_list(&metadata.goals)?,
    })
}

/// Moves the post-load engine team registry into its byte-preserving JoinData
/// representation.
pub fn join_team_list_snapshot(metadata: InitialNetworkTeamMetadata) -> JoinTeamListSnapshot {
    JoinTeamListSnapshot {
        active: u8::from(metadata.active),
        custom: u8::from(metadata.custom),
        allow_hostility_change: u8::from(metadata.allow_hostility_change),
        allow_team_switch: u8::from(metadata.allow_team_switch),
        auto_generate_teams: u8::from(metadata.auto_generate_teams),
        last_team_id: metadata.last_team_id,
        team_distribution: metadata.team_distribution as u8,
        team_colors: u8::from(metadata.team_colors),
        max_script_players: metadata.max_script_players,
        script_player_names: metadata.script_player_names,
        random_team_count: metadata.random_team_count,
        teams: metadata
            .teams
            .into_iter()
            .map(|team| JoinTeamSnapshot {
                id: team.id,
                name: team.name,
                player_start_index: team.player_start_index,
                player_ids: team.player_ids,
                color: team.color,
                icon_spec: team.icon_spec,
                max_players: team.max_players,
            })
            .collect(),
    }
}

/// Fills only scenario-owned JoinData fields while retaining runtime values
/// supplied by the caller. The returned defaults are the same values needed
/// when serializing the initial `Parameters.txt`.
pub fn fill_scenario_derived_join_parameters(
    parameters: &mut JoinGameParametersEnvelope,
    scenario: &InitialNetworkScenarioMetadata,
    teams: InitialNetworkTeamMetadata,
) -> Result<InitialNetworkScenarioDefaults, InitialNetworkMetadataError> {
    let defaults = initial_network_scenario_defaults(scenario)?;
    let caller_fair_crew = parameters.use_fair_crew;
    let caller_fair_crew_strength = parameters.fair_crew_strength;

    parameters.max_players = defaults.max_players;
    parameters.fair_crew_forced = defaults.fair_crew_forced;
    parameters.use_fair_crew = if defaults.fair_crew_forced {
        defaults.use_fair_crew
    } else {
        caller_fair_crew
    };
    parameters.fair_crew_strength = if defaults.fair_crew_strength != 0 {
        defaults.fair_crew_strength
    } else if parameters.use_fair_crew {
        caller_fair_crew_strength
    } else {
        0
    };
    parameters.rules = defaults.rules.clone();
    parameters.goals = defaults.goals.clone();
    parameters.teams = join_team_list_snapshot(teams);

    Ok(defaults)
}

fn join_data_id_list(
    entries: &[ScenarioIdListEntry],
) -> Result<Vec<JoinDataIdListEntry>, InitialNetworkMetadataError> {
    entries
        .iter()
        .map(|entry| {
            let bytes: [u8; 4] =
                entry.id.as_bytes().try_into().map_err(|_| {
                    InitialNetworkMetadataError::InvalidScenarioId(entry.id.clone())
                })?;
            let id = JoinDataC4Id::from_bytes(bytes)
                .ok_or_else(|| InitialNetworkMetadataError::InvalidScenarioId(entry.id.clone()))?;
            Ok(JoinDataIdListEntry {
                id,
                count: entry.count,
            })
        })
        .collect()
}
