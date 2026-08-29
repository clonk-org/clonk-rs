//! Owned, thread-safe input for scenario-first network host preparation.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use clonk_resources::LanguagePacks;

use crate::prepared_host_bootstrap::{
    prepare_host_bootstrap_with_staged_scenario_and_team_assignment_oracle,
    PrepareHostBootstrapError, PreparedHostBootstrap, PreparedHostBootstrapConfig,
    PreparedHostBootstrapSpec, PreparedHostPlayerSource, PreparedLeagueHostConfig,
    ProcessInitialHostTeamAssignmentOracle,
};

#[derive(Debug)]
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
    /// Scenario already loaded by the OpenScenario-equivalent selector pass.
    /// The ordinary app path supplies it so InitNetworkHost does not reopen
    /// every definition before the lobby can be shown.
    pub staged_scenario: Option<clonk_engine::Scenario>,
}

impl NetworkHostPreparation {
    pub fn with_staged_scenario(mut self, scenario: clonk_engine::Scenario) -> Self {
        self.staged_scenario = Some(scenario);
        self
    }

    /// Builds the resource-empty lobby round used while exact standalones are
    /// materialized. It carries the final identity and transport policy, but
    /// admission stays closed and no JoinData exists until `prepare` succeeds.
    pub fn preparing_host_config(&self) -> clonk_network::HostConfig {
        let host_name = clonk_engine::LegacyCString::from_bytes(self.host_name.as_bytes().to_vec())
            .unwrap_or_default();
        let host_nick = clonk_engine::LegacyCString::from_bytes(self.host_nick.as_bytes().to_vec())
            .unwrap_or_else(|| host_name.clone());
        let group_maker =
            clonk_engine::LegacyCString::from_bytes(self.group_maker.as_bytes().to_vec())
                .unwrap_or_default();
        let password =
            clonk_engine::LegacyCString::from_bytes(self.network_password.as_bytes().to_vec())
                .unwrap_or_default();
        let max_players = self
            .staged_scenario
            .as_ref()
            .and_then(clonk_engine::Scenario::lobby_metadata)
            .map_or(8, |metadata| metadata.head().max_players())
            .max(0) as usize;
        clonk_network::HostConfig {
            max_players,
            local_core: clonk_engine::ClientCoreControlData {
                client_id: 0,
                activated: true,
                observer: false,
                name: host_name,
                nick: host_nick,
                lobby_ready: false,
            },
            group_maker,
            initial_status: clonk_network::NetworkStatus::new(
                clonk_network::NETWORK_STATE_LOBBY,
                self.config.control_mode,
                -1,
            ),
            password,
            allow_join: false,
            initial_join_snapshot: None,
            enable_upnp: self.config.enable_upnp,
            configured_tcp_port: Some(self.config.network_tcp_port),
            configured_udp_port: Some(self.config.network_udp_port),
            local_resource_roots: self.install_roots.clone(),
            ..clonk_network::HostConfig::default()
        }
    }

    /// Materializes every exact scenario/resource input after the preliminary
    /// closed-admission listener exists and before the final host is bound.
    /// The two clock reads remain separate because C++ reads `time(nullptr)`
    /// once for game identity and later for the parameter seed.
    pub fn prepare(self) -> Result<PreparedHostBootstrap, PrepareHostBootstrapError> {
        let start_unix_seconds = unix_seconds_now();
        let random_seed_unix_seconds = pinned_host_parameter_seed_seconds(unix_seconds_now());
        let mut team_assignment =
            ProcessInitialHostTeamAssignmentOracle::new(self.generated_team_name_template);
        prepare_host_bootstrap_with_staged_scenario_and_team_assignment_oracle(
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
            self.staged_scenario,
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
    use super::{resolve_host_parameter_seed_seconds, NetworkHostPreparation};

    #[test]
    fn preparing_host_has_no_join_data_and_keeps_admission_closed() {
        // InitHost precedes Players.Init and AllowJoin in native startup
        // (oracle src/C4Game.cpp:421-438,3847-3876).
        let preparation = NetworkHostPreparation {
            scenario_path: "Scenario.c4s".into(),
            install_roots: vec!["planet".into()],
            effective_definition_modules: Vec::new(),
            definition_resources: Vec::new(),
            initial_definition_modules: Vec::new(),
            fixed_definition_modules: None,
            selector_definition_root: None,
            definition_executable_path: String::new(),
            definition_path: String::new(),
            languages: Vec::new(),
            language_packs: clonk_resources::LanguagePacks::default(),
            network_work_path: "Network".to_string(),
            network_directory: "Network".into(),
            group_maker: "Host".to_string(),
            host_name: "Host".to_string(),
            host_nick: "Host".to_string(),
            network_password: String::new(),
            network_comment: String::new(),
            netpuncher_address: String::new(),
            generated_team_name_template: clonk_engine::LegacyCString::default(),
            player_sources: Vec::new(),
            config: super::PreparedHostBootstrapConfig {
                control_mode: 0,
                control_rate: 2,
                async_max_wait: 2,
                fair_crew: false,
                fair_crew_strength: 1_000,
                auto_frame_skip: true,
                max_load_file_size: 100 * 1024 * 1024,
                no_runtime_join: false,
                enable_upnp: false,
                network_tcp_port: 11_112,
                network_udp_port: 11_113,
            },
            league: None,
            staged_scenario: None,
        };

        let config = preparation.preparing_host_config();
        assert!(!config.allow_join);
        assert!(config.initial_join_snapshot.is_none());
        assert!(config.resource_files.is_empty());
        assert_eq!(config.configured_tcp_port, Some(11_112));
        assert_eq!(config.configured_udp_port, Some(11_113));
    }

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
