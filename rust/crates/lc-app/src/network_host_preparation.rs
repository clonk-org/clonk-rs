//! Owned, thread-safe input for scenario-first network host preparation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use lc_resources::LanguagePacks;

use crate::prepared_host_bootstrap::{
    prepare_host_bootstrap_with_team_assignment_oracle, PrepareHostBootstrapError,
    PreparedHostBootstrap, PreparedHostBootstrapConfig, PreparedHostBootstrapSpec,
    PreparedHostPlayerSource, PreparedLeagueHostConfig, ProcessInitialHostTeamAssignmentOracle,
};

#[derive(Debug, Clone)]
pub struct NetworkHostPreparation {
    pub scenario_path: PathBuf,
    pub install_roots: Vec<PathBuf>,
    pub definition_executable_path: String,
    pub definition_path: String,
    pub languages: Vec<String>,
    pub language_packs: LanguagePacks,
    pub network_work_path: String,
    pub network_directory: PathBuf,
    pub group_maker: String,
    pub host_name: String,
    pub host_nick: String,
    pub network_password: String,
    pub network_comment: String,
    pub netpuncher_address: String,
    pub generated_team_name_template: lc_engine::LegacyCString,
    pub player_sources: Vec<PreparedHostPlayerSource>,
    pub config: PreparedHostBootstrapConfig,
    pub league: Option<PreparedLeagueHostConfig>,
}

impl NetworkHostPreparation {
    /// Materializes every scenario/resource input before the listener exists.
    /// The two clock reads remain separate because C++ reads `time(nullptr)`
    /// once for game identity and later for the parameter seed.
    pub fn prepare(self) -> Result<PreparedHostBootstrap, PrepareHostBootstrapError> {
        let start_unix_seconds = unix_seconds_now();
        let random_seed_unix_seconds = unix_seconds_now();
        let mut team_assignment =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template);
        prepare_host_bootstrap_with_team_assignment_oracle(
            PreparedHostBootstrapSpec {
                scenario_path: &self.scenario_path,
                install_roots: &self.install_roots,
                definition_executable_path: &self.definition_executable_path,
                definition_path: &self.definition_path,
                languages: &self.languages,
                language_packs: &self.language_packs,
                network_work_path: &self.network_work_path,
                network_directory: &self.network_directory,
                start_unix_seconds,
                random_seed_unix_seconds,
                group_maker: &self.group_maker,
                host_name: &self.host_name,
                host_nick: &self.host_nick,
                network_password: &self.network_password,
                network_comment: &self.network_comment,
                netpuncher_address: &self.netpuncher_address,
                player_sources: &self.player_sources,
                config: self.config,
                league: self.league.as_ref(),
            },
            &mut team_assignment,
        )
    }
}

fn unix_seconds_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}
