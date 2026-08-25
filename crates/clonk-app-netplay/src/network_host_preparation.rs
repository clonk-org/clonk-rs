//! Owned, thread-safe input for scenario-first network host preparation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clonk_resources::LanguagePacks;

use crate::prepared_host_bootstrap::{
    prepare_host_bootstrap_with_team_assignment_oracle, PrepareHostBootstrapError,
    PreparedHostBootstrap, PreparedHostBootstrapConfig, PreparedHostBootstrapSpec,
    PreparedHostPlayerSource, PreparedLeagueHostConfig, ProcessInitialHostTeamAssignmentOracle,
};

#[derive(Debug, Clone)]
pub struct NetworkHostPreparation {
    pub scenario_path: PathBuf,
    pub install_roots: Vec<PathBuf>,
    /// Exact effective module spellings selected while staging, before
    /// DefinitionPath expansion and folder-local discovery.
    pub effective_definition_modules: Vec<String>,
    /// Exact ordered `DefinitionFilenames` resources resolved while staging
    /// the host scenario, including their frozen network filename spellings.
    pub definition_resources: Vec<clonk_network::HostInitialResourceSource>,
    pub initial_definition_modules: Vec<String>,
    /// `Some(empty)` is distinct from an empty seed: fixed-empty suppresses
    /// a nonempty scenario preset just like C++ `FixedDefinitions`.
    pub fixed_definition_modules: Option<Vec<String>>,
    pub selector_definition_root: Option<PathBuf>,
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
    pub generated_team_name_template: clonk_engine::LegacyCString,
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
        let random_seed_unix_seconds = pinned_host_parameter_seed_seconds(unix_seconds_now());
        let mut team_assignment =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template);
        prepare_host_bootstrap_with_team_assignment_oracle(
            PreparedHostBootstrapSpec {
                scenario_path: &self.scenario_path,
                install_roots: &self.install_roots,
                effective_definition_modules: &self.effective_definition_modules,
                definition_resources: &self.definition_resources,
                initial_definition_modules: &self.initial_definition_modules,
                fixed_definition_modules: self.fixed_definition_modules.as_deref(),
                selector_definition_root: self.selector_definition_root.as_deref(),
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

/// The parameter seed a fresh host round freezes.
///
/// Native reads `time(nullptr)` (`C4GameParameters::Load` freezes it before
/// `FixRandom` and `Landscape::Init`), and this mirrors that. `LC_PIN_SEED`
/// overrides it for differential work exactly as it already does for an
/// offline round -- without it, two runs of the same mixed-engine scenario
/// generate different worlds and cannot be compared before/after a change
/// (clonk-org/clonk-rs#1050).
pub(crate) fn resolve_host_parameter_seed_seconds(now: i64, pin: Option<&str>) -> i64 {
    match pin {
        Some(pin) if !pin.trim().is_empty() => pin.trim().parse::<i64>().unwrap_or(now),
        _ => now,
    }
}

fn pinned_host_parameter_seed_seconds(now: i64) -> i64 {
    let pin = std::env::var_os("LC_PIN_SEED");
    let pin = pin.as_deref().map(|value| value.to_string_lossy());
    resolve_host_parameter_seed_seconds(now, pin.as_deref())
}

fn unix_seconds_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod host_parameter_seed_tests {
    use super::resolve_host_parameter_seed_seconds;

    #[test]
    fn a_pinned_seed_replaces_the_clock_for_a_fresh_host_round() {
        // Unpinned, the clock stands -- native reads time(nullptr).
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, None),
            1_700_000_000
        );
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some("")),
            1_700_000_000
        );
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some("   ")),
            1_700_000_000
        );

        // Pinned, the round is reproducible.
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some("1")),
            1
        );
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some(" 42 ")),
            42
        );
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some("-7")),
            -7
        );

        // A value that is not a number never silently becomes zero; the clock
        // stands, so a typo degrades to ordinary behaviour rather than pinning
        // every run to the same wrong world.
        assert_eq!(
            resolve_host_parameter_seed_seconds(1_700_000_000, Some("abc")),
            1_700_000_000
        );
    }
}
