//! Owned, thread-safe input for scenario-first network host preparation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use lc_network::HostInitialResourceSource;
use lc_resources::LanguagePacks;

use crate::prepared_host_bootstrap::{
    prepare_host_bootstrap, PrepareHostBootstrapError, PreparedHostBootstrap,
    PreparedHostBootstrapConfig, PreparedHostBootstrapSpec, PreparedLeagueHostConfig,
};

#[derive(Debug, Clone)]
pub struct NetworkHostPreparation {
    pub scenario_path: PathBuf,
    pub scenario_title: String,
    pub install_roots: Vec<PathBuf>,
    pub languages: Vec<String>,
    pub language_packs: LanguagePacks,
    pub network_work_path: String,
    pub network_directory: PathBuf,
    pub group_maker: String,
    pub host_name: String,
    pub host_nick: String,
    pub network_comment: String,
    pub netpuncher_address: String,
    pub player_sources: Vec<HostInitialResourceSource>,
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
        prepare_host_bootstrap(PreparedHostBootstrapSpec {
            scenario_path: &self.scenario_path,
            scenario_title: &self.scenario_title,
            install_roots: &self.install_roots,
            languages: &self.languages,
            language_packs: &self.language_packs,
            network_work_path: &self.network_work_path,
            network_directory: &self.network_directory,
            start_unix_seconds,
            random_seed_unix_seconds,
            group_maker: &self.group_maker,
            host_name: &self.host_name,
            host_nick: &self.host_nick,
            network_comment: &self.network_comment,
            netpuncher_address: &self.netpuncher_address,
            player_sources: &self.player_sources,
            config: self.config,
            league: self.league.as_ref(),
        })
    }
}

fn unix_seconds_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}
