//! Pure preparation of the supported C++ initial-network-host state.
//!
//! This stops before opening any listener or advertising the game. C++ keeps
//! `fAllowJoin` false throughout `C4Network2::InitHost`; the app may only open
//! admission after control and the initial local player packet are ready
//! (`src/C4Network2.cpp:222-278`; `src/C4Game.cpp:3847-3876`).

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::os::raw::c_int;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use clonk_engine::player_file::PlayerFile;
use clonk_engine::scenario::{
    LegacyDefinitionResolver, ScenarioGameParameterValues, ScenarioLoaderHead, ScenarioLobbyIdEntry,
};
use clonk_engine::{
    assign_initial_host_player_teams, parse_initial_network_game_data, ClientCoreControlData,
    ControlPlayerInfoEntry, InitialHostTeamAssignmentOracle, InitialNetworkGameApplyError,
    InitialNetworkGameData, InitialNetworkTeam, InitialNetworkTeamMetadata, LegacyCString,
    NetworkResourceCore, PlayerInfoControlData, PlayerInfoUpdateRequest, Scenario, ScenarioError,
    TeamColorUpdateError, CLIENT_PLAYER_INFO_FLAG_INITIAL, CLIENT_PLAYER_INFO_FLAG_UPDATED,
    PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE, PLAYER_INFO_FLAG_JOINED,
    PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_TYPE_SCRIPT,
};
use clonk_network::{
    compose_initial_network_dynamic, fill_scenario_derived_join_parameters,
    join_team_list_snapshot, publish_host_initial_resources, ClientPlayerInfosSnapshot, HostConfig,
    HostGameReference, HostGameReferenceError, HostGameReferenceMetadata,
    HostInitialResourcePublication, HostInitialResourcePublicationError,
    HostInitialResourcePublicationSpec, HostInitialResourceSource, HostResourceType,
    InitialNetworkDynamicError, InitialNetworkDynamicSpec, InitialNetworkMetadataError,
    InitialNetworkScenarioDefaults, JoinClientRegistrySnapshot, JoinDataC4Id, JoinDataIdListEntry,
    JoinGameParametersEnvelope, JoinTeamListSnapshot, LeagueHttpTransportConfig,
    LeagueStartResponse, NetworkAddress, NetworkGameReference, NetworkProtocol, NetworkStatus,
    PlayerInfoListSnapshot, ResourceFileOwnership, CURRENT_GAME_BUILD, CURRENT_GAME_VERSION,
    NETWORK_STATE_GO, NETWORK_STATE_INIT, NETWORK_STATE_LOBBY, NETWORK_STATE_NONE,
    NETWORK_STATE_PAUSE,
};
use clonk_resources::{decode_legacy_script_text, localize_script_source_with_components};
use clonk_resources::{Group, GroupError, LanguagePacks};
use parking_lot::Mutex;
use thiserror::Error;

use crate::host_game_resource_sources::{
    executable_relative_group_name, freeze_host_definition_resource_sources, open_group_path,
    opened_physical_group_name, resolve_host_game_resource_sources,
    validate_host_group_resource_source, HostGameResourceSourceError, HostGameResourceSourceKind,
};

/// Configuration values C++ reads while loading parameters and initializing
/// its network status. Values unrelated to this supported initial-host subset
/// remain fixed at their stock defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedHostBootstrapConfig {
    pub control_mode: i32,
    pub control_rate: i32,
    pub async_max_wait: i32,
    pub fair_crew: bool,
    pub fair_crew_strength: i32,
    pub auto_frame_skip: bool,
    pub max_load_file_size: u32,
    pub no_runtime_join: bool,
    pub enable_upnp: bool,
    pub network_tcp_port: u16,
    pub network_udp_port: u16,
}

/// League service inputs frozen before the host socket is opened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedLeagueHostConfig {
    pub endpoint: String,
    pub transport: LeagueHttpTransportConfig,
    pub update_period_secs: i64,
    pub league_server_signup: bool,
}

/// C++-normalized identity captured while loading one configured participant.
///
/// `C4PlayerInfoCore::Load` applies fixed-buffer truncation, exact-case INI
/// lookup, preferred-color defaults, and markup stripping before
/// `C4PlayerInfo` is constructed. Keep that result beside the resource source
/// so host preparation does not derive a different identity by reopening the
/// file through the generic player-file model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedHostPlayerIdentity {
    pub player_name: LegacyCString,
    pub network_color: u32,
    pub alternate_color: u32,
}

/// One selected host participant and, when it came from the classic config
/// loader, the already-normalized identity that belongs to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedHostPlayerSource {
    pub resource: HostInitialResourceSource,
    pub identity: Option<PreparedHostPlayerIdentity>,
}

impl From<HostInitialResourceSource> for PreparedHostPlayerSource {
    fn from(resource: HostInitialResourceSource) -> Self {
        Self {
            resource,
            identity: None,
        }
    }
}

/// Every process-global input used by the supported preparation path.
#[derive(Debug, Clone, Copy)]
pub struct PreparedHostBootstrapSpec<'a> {
    pub scenario_path: &'a Path,
    /// Ordered assembled install roots. Earlier roots shadow later roots.
    pub install_roots: &'a [PathBuf],
    /// Exact ordered external and folder-local definition resources already
    /// resolved by the OpenScenario-equivalent staging pass.
    pub definition_resources: &'a [HostInitialResourceSource],
    /// Effective selected module spellings before DefinitionPath expansion
    /// and folder-local discovery.
    pub effective_definition_modules: &'a [String],
    pub initial_definition_modules: &'a [String],
    pub fixed_definition_modules: Option<&'a [String]>,
    pub selector_definition_root: Option<&'a Path>,
    /// Native `Config.General.ExePath`, including its trailing separator.
    pub definition_executable_path: &'a str,
    /// Native `Config.General.DefinitionPath` (relative or absolute).
    pub definition_path: &'a str,
    /// Ordered legacy language fallbacks used while loading scenario-owned
    /// definitions and Teams.txt from `scenario_path`.
    pub languages: &'a [String],
    /// Process-global external language packs discovered from the app's one
    /// classic `Language.c4g` namespace.
    pub language_packs: &'a LanguagePacks,
    /// Logical `Config.Network.WorkPath` carried by resource core filenames.
    /// This must not be inferred from the host's physical cache directory.
    pub network_work_path: &'a str,
    pub network_directory: &'a Path,
    /// The earlier `time(nullptr)` read that identifies the game on this host.
    pub start_unix_seconds: i64,
    /// The later `time(nullptr)` read used as the no-Parameters random seed.
    /// Keeping this separate preserves the second-boundary case in C++.
    pub random_seed_unix_seconds: i64,
    /// `Config.General.Name`, used as the C4Group maker. This is intentionally
    /// independent of the two network client names.
    pub group_maker: &'a str,
    /// Final `C4ClientCore::Name` bytes after config and client validation.
    pub host_name: &'a str,
    /// Final nonempty `C4ClientCore::Nick` bytes after fallback and validation.
    pub host_nick: &'a str,
    /// Process-local `Game.Network` password selected before the host starts.
    pub network_password: &'a str,
    /// `Config.Network.Comment`, copied verbatim into the game reference.
    pub network_comment: &'a str,
    /// `Config.Network.PuncherAddress`, present even before a puncher ID exists.
    pub netpuncher_address: &'a str,
    /// Selected participant files in C++ module order, with their exact
    /// `Config.AtExeRelativePath` wire spellings.
    pub player_sources: &'a [PreparedHostPlayerSource],
    pub config: PreparedHostBootstrapConfig,
    pub league: Option<&'a PreparedLeagueHostConfig>,
}

/// Admission facts retained separately from the still-closed `HostConfig`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedHostAdmission {
    max_players: i32,
    no_runtime_join: bool,
}

impl PreparedHostAdmission {
    pub fn max_players(self) -> i32 {
        self.max_players
    }

    /// When leaving the lobby C++ applies `!Config.Network.NoRuntimeJoin`.
    pub fn runtime_join_allowed(self) -> bool {
        !self.no_runtime_join
    }
}

/// Capability produced only after the host's Initial PlayerInfo was applied
/// locally while admission was closed.
#[derive(Debug, PartialEq, Eq)]
#[must_use]
pub struct PreparedHostAdmissionReady {
    admission: PreparedHostAdmission,
}

impl PreparedHostAdmissionReady {
    /// `C4Game::InitNetworkHost` opens lobby joining after `Players.Init`.
    pub fn lobby_join_allowed(self) -> bool {
        true
    }

    pub fn runtime_join_allowed(self) -> bool {
        self.admission.runtime_join_allowed()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PreparedHostUseError {
    #[error("the prepared host resources were already claimed by a launch")]
    HostAlreadyLaunched,
    #[error("the prepared host scenario was already claimed by a launch")]
    ScenarioAlreadyClaimed,
    #[error("the initial host PlayerInfo was already installed")]
    InitialPlayerInfoAlreadyInstalled,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PreparedHostReferenceError {
    #[error("the prepared host has no initial JoinData snapshot")]
    MissingJoinSnapshot,
    #[error("network status state {0} has no C++ reference name")]
    UnsupportedStatus(u8),
    #[error(transparent)]
    Reference(#[from] HostGameReferenceError),
}

#[derive(Debug)]
struct PreparedHostLifetime {
    temporary_files: Vec<PathBuf>,
    scenario: Mutex<Option<Scenario>>,
    host_launched: AtomicBool,
    initial_player_info_installed: AtomicBool,
}

impl Drop for PreparedHostLifetime {
    fn drop(&mut self) {
        for path in &self.temporary_files {
            let _ = fs::remove_file(path);
        }
    }
}

struct PreparedTemporaryFiles {
    paths: Vec<PathBuf>,
    armed: bool,
}

impl PreparedTemporaryFiles {
    fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths, armed: true }
    }

    fn into_lifetime_paths(mut self) -> Vec<PathBuf> {
        self.armed = false;
        std::mem::take(&mut self.paths)
    }
}

impl Drop for PreparedTemporaryFiles {
    fn drop(&mut self) {
        if self.armed {
            for path in &self.paths {
                let _ = fs::remove_file(path);
            }
        }
    }
}

/// A host that is completely materialized but has not opened sockets or
/// admission. Fields stay private so an unprepared `HostConfig` cannot be
/// confused with this lifecycle state.
#[derive(Debug, Clone)]
pub struct PreparedHostBootstrap {
    host_config: HostConfig,
    initial_game: InitialNetworkGameData,
    /// Frozen `Game.C4S` defaults passed to every exact
    /// `Game.Parameters.Save(group, &Game.C4S)` decompile, including later
    /// runtime-join dynamics.
    scenario_defaults: InitialNetworkScenarioDefaults,
    has_initial_game: bool,
    admission: PreparedHostAdmission,
    start_time: i32,
    initial_host_player_info_control: PlayerInfoControlData,
    runtime_team_metadata: InitialNetworkTeamMetadata,
    scenario_wire_name: LegacyCString,
    scenario_origin: String,
    /// Unsuffixed `WorkPath + "Dyn" + scenario basename` passed to every
    /// C++ FindTempResFileName call, distinct from the allocated core name.
    dynamic_filename_seed: String,
    dynamic_wire_name: LegacyCString,
    /// Exact pre-publication `Game.DefinitionFilenames` vector. Resource-core
    /// type reuse may change which groups InitDefs opens, but never mutates
    /// this separately retained save/runtime-join identity.
    definition_modules: Vec<String>,
    /// Process-loaded path strings used by C4SDefinitions::SetModules. Keep
    /// the staged values beside DefinitionFilenames so later runtime dynamics
    /// cannot observe an unrelated on-disk config rewrite.
    definition_executable_path: String,
    definition_path: String,
    /// Final post-AddByFile NRT_Material rows used by both the host simulation
    /// and its process-local material renderer.
    material_resource_groups: Vec<Group>,
    reference_icon: i32,
    reference_comment: LegacyCString,
    netpuncher_address: LegacyCString,
    league: Option<PreparedLeagueHostConfig>,
    stream_address: LegacyCString,
    local_player_resources: Vec<(NetworkResourceCore, PathBuf)>,
    /// `C4PlayerInfo::dwAlternateColor` is deliberately absent from the
    /// synchronized compiler. Keep the host process's values beside their
    /// stable resource identities for every later attribute-resolution pass.
    local_player_alternate_colors_by_resource: HashMap<i32, u32>,
    pending_initial_league_players: Option<PendingInitialLeaguePlayers>,
    lifetime: Arc<PreparedHostLifetime>,
}

#[derive(Debug, Clone)]
struct PendingInitialLeaguePlayers {
    players: Vec<ControlPlayerInfoEntry>,
    alternate_colors_by_resource: HashMap<i32, u32>,
    restore_players: Vec<ControlPlayerInfoEntry>,
    restore_last_player_id: i32,
    team_metadata: InitialNetworkTeamMetadata,
}

#[derive(Debug, Clone)]
enum PreparedLocalPlayerIdentity {
    Configured(PreparedHostPlayerIdentity),
    Generic(PlayerFile),
}

impl PreparedLocalPlayerIdentity {
    fn player_name(&self) -> LegacyCString {
        match self {
            Self::Configured(identity) => identity.player_name.clone(),
            Self::Generic(player) => legacy_c4_string(&player.name),
        }
    }

    fn network_color(&self) -> u32 {
        match self {
            Self::Configured(identity) => identity.network_color,
            Self::Generic(player) => player.normalized_preferred_color(),
        }
    }

    fn alternate_color(&self) -> Option<u32> {
        match self {
            Self::Configured(identity) => Some(identity.alternate_color),
            Self::Generic(player) => Some(player.normalized_alternate_color()),
        }
    }
}

impl PreparedHostBootstrap {
    pub fn host_config(&self) -> &HostConfig {
        &self.host_config
    }

    /// Runtime state compiled from the already-opened `Game.txt`, or the
    /// InitSystem defaults retained when that component was absent. GO must
    /// consume this frozen value rather than reopening a mutable source group.
    pub fn initial_game_data(&self) -> &InitialNetworkGameData {
        &self.initial_game
    }

    pub fn scenario_defaults(&self) -> &InitialNetworkScenarioDefaults {
        &self.scenario_defaults
    }

    pub fn definition_modules(&self) -> &[String] {
        &self.definition_modules
    }

    pub fn definition_save_paths(&self) -> (&str, &str) {
        (&self.definition_executable_path, &self.definition_path)
    }

    pub fn material_resource_groups(&self) -> &[Group] {
        &self.material_resource_groups
    }

    /// Whether host preparation found a readable, nonempty `Game.txt` in the
    /// source scenario.
    pub fn has_initial_game_data(&self) -> bool {
        self.has_initial_game
    }

    /// Claims the resource-bearing configuration for exactly one live host.
    /// Clones retain metadata and temporary-file lifetime, but cannot start a
    /// second backend against the same C++ resource namespace.
    pub fn claim_host_config(&self) -> Result<HostConfig, PreparedHostUseError> {
        self.lifetime
            .host_launched
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PreparedHostUseError::HostAlreadyLaunched)?;
        Ok(self.host_config.clone())
    }

    /// Claims the `C4S` value loaded before host initialization. Every clone
    /// shares this one launch value, so entering the game cannot reopen a
    /// changed scenario source or start a second simulation from it.
    pub fn claim_scenario(&self) -> Result<Scenario, PreparedHostUseError> {
        self.lifetime
            .scenario
            .lock()
            .take()
            .ok_or(PreparedHostUseError::ScenarioAlreadyClaimed)
    }

    pub fn admission(&self) -> PreparedHostAdmission {
        self.admission
    }

    /// Updates the policy which C++ applies when the lobby transitions to
    /// gameplay. The Options sheet may change `NoRuntimeJoin` after the host
    /// bootstrap has already been prepared, so this retained admission value
    /// must remain mutable until the lobby ends.
    pub fn set_runtime_join_allowed(&mut self, allowed: bool) {
        self.admission.no_runtime_join = !allowed;
    }

    pub fn league_config(&self) -> Option<&PreparedLeagueHostConfig> {
        self.league.as_ref()
    }

    pub fn stream_address(&self) -> &LegacyCString {
        &self.stream_address
    }

    pub fn netpuncher_address(&self) -> &LegacyCString {
        &self.netpuncher_address
    }

    /// Applies the validated Start reply before either admission or the
    /// initial reference becomes visible. Native changes the synchronized
    /// parameters and forces only Async (2) to Central (1).
    pub fn apply_league_start_response(
        &mut self,
        response: &LeagueStartResponse,
    ) -> Result<(), PrepareHostBootstrapError> {
        let max_players = (response.max_players != 0)
            .then(|| usize::try_from(response.max_players))
            .transpose()
            .map_err(|_| PrepareHostBootstrapError::MaxPlayersOutOfRange(response.max_players))?;
        let parameters = &mut self
            .host_config
            .initial_join_snapshot
            .as_mut()
            .ok_or(PrepareHostBootstrapError::MissingJoinSnapshot)?
            .parameters;
        parameters.league = response.league.clone();
        if let Some(seed) = response.seed {
            parameters.random_seed = seed;
        }
        if response.league.is_empty() {
            parameters.league_address = LegacyCString::default();
            self.stream_address = LegacyCString::default();
        } else {
            self.stream_address = response.stream_to.clone();
            if self.host_config.initial_status.control_mode == 2 {
                self.host_config.initial_status.control_mode = 1;
            }
        }
        if let Some(max_players) = max_players {
            parameters.max_players = response.max_players;
            self.host_config.max_players = max_players;
            self.admission.max_players = response.max_players;
        }
        Ok(())
    }

    /// Mirrors `C4Network2::DeinitLeague`: clear the synchronized league
    /// identity while retaining Start's seed, capacity and stream address.
    pub fn clear_live_league_registration(&mut self) -> Result<(), PrepareHostBootstrapError> {
        let parameters = &mut self
            .host_config
            .initial_join_snapshot
            .as_mut()
            .ok_or(PrepareHostBootstrapError::MissingJoinSnapshot)?
            .parameters;
        parameters.league = LegacyCString::default();
        parameters.league_address = LegacyCString::default();
        Ok(())
    }

    pub fn start_time(&self) -> i32 {
        self.start_time
    }

    /// Builds `C4Network2Reference::InitLocal`'s initial Lobby snapshot after
    /// the acknowledged `AllowJoin(true)` transition.
    pub fn initial_host_game_reference(
        &self,
        join_allowed: bool,
        addresses: &[NetworkAddress],
    ) -> Result<HostGameReference, PreparedHostReferenceError> {
        let parameters = self
            .host_config
            .initial_join_snapshot
            .as_ref()
            .ok_or(PreparedHostReferenceError::MissingJoinSnapshot)?
            .parameters
            .clone();
        let state = match self.host_config.initial_status.state {
            NETWORK_STATE_NONE => "None",
            NETWORK_STATE_INIT => "Init",
            NETWORK_STATE_LOBBY => "Lobby",
            NETWORK_STATE_PAUSE => "Paused",
            NETWORK_STATE_GO => "Running",
            state => return Err(PreparedHostReferenceError::UnsupportedStatus(state)),
        };
        let player_names = parameters
            .player_infos
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .filter(|player| {
                player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                    && !(player.player_type == PLAYER_INFO_TYPE_SCRIPT
                        && player.flags & PLAYER_INFO_FLAG_INVISIBLE != 0)
            })
            .map(|player| {
                let name = if !player.league_account.as_bytes().is_empty() {
                    &player.league_account
                } else if !player.forced_name.as_bytes().is_empty() {
                    &player.forced_name
                } else {
                    &player.name
                };
                clonk_resources::decode_legacy_script_text(name.as_bytes())
            })
            .collect();
        let summary = NetworkGameReference {
            icon: self.reference_icon,
            title: clonk_resources::decode_legacy_script_text(parameters.title.as_bytes()),
            host_name: clonk_resources::decode_legacy_script_text(
                self.host_config.local_core.name.as_bytes(),
            ),
            host_nick: clonk_resources::decode_legacy_script_text(
                self.host_config.local_core.nick.as_bytes(),
            ),
            state: state.to_string(),
            control_mode: self.host_config.initial_status.control_mode,
            time: self.initial_game.time,
            start_time: i64::from(self.start_time),
            comment: clonk_resources::decode_legacy_script_text(self.reference_comment.as_bytes()),
            join_allowed,
            password_needed: !self.host_config.password.is_empty(),
            official_server: false,
            use_fair_crew: parameters.use_fair_crew,
            goals: parameters
                .goals
                .iter()
                .map(|goal| goal.id.as_bytes().iter().copied().map(char::from).collect())
                .collect(),
            league: clonk_resources::decode_legacy_script_text(parameters.league.as_bytes()),
            league_address: clonk_resources::decode_legacy_script_text(
                parameters.league_address.as_bytes(),
            ),
            max_players: parameters.max_players,
            player_names,
            game: "LegacyClonk".to_string(),
            version: CURRENT_GAME_VERSION,
            build: CURRENT_GAME_BUILD,
            addresses: addresses.to_vec(),
            source_address: std::net::SocketAddr::V6(std::net::SocketAddrV6::new(
                std::net::Ipv6Addr::UNSPECIFIED,
                0,
                0,
                0,
            )),
            netpuncher_ipv4: 0,
            netpuncher_ipv6: 0,
            netpuncher_address: clonk_resources::decode_legacy_script_text(
                self.netpuncher_address.as_bytes(),
            ),
            tcp_addresses: addresses
                .iter()
                .filter(|address| address.protocol == NetworkProtocol::Tcp)
                .map(|address| address.endpoint)
                .collect(),
        };
        let metadata = HostGameReferenceMetadata {
            icon: self.reference_icon,
            time: self.initial_game.time,
            frame: self.initial_game.frame,
            league_performance: 0,
            comment: self.reference_comment.clone(),
            addresses: addresses.to_vec(),
            netpuncher_ipv4: 0,
            netpuncher_ipv6: 0,
            netpuncher_address: self.netpuncher_address.clone(),
        };
        HostGameReference::new(summary, metadata, parameters).map_err(Into::into)
    }

    /// The host-authored `CID_PlrInfo`/`CDT_Direct` value executed by
    /// `C4Network2Players::Init` before joining is opened.
    pub fn initial_host_player_info_control(&self) -> &PlayerInfoControlData {
        &self.initial_host_player_info_control
    }

    pub fn local_player_alternate_colors_by_resource(&self) -> &HashMap<i32, u32> {
        &self.local_player_alternate_colors_by_resource
    }

    pub fn pending_initial_league_players(&self) -> Option<&[ControlPlayerInfoEntry]> {
        self.pending_initial_league_players
            .as_ref()
            .map(|pending| pending.players.as_slice())
    }

    /// Runs the post-Auth half of the host's initial local-player admission.
    /// The callback is the lobby-only `Action=Join` check and therefore sees
    /// host-assigned IDs, teams, colors and names. Restore script rows are
    /// appended only after this callback, matching `Players.Init`.
    pub fn finalize_initial_league_players(
        &mut self,
        authenticated_players: Vec<ControlPlayerInfoEntry>,
        team_assignment_oracle: &mut impl InitialHostTeamAssignmentOracle,
        check: impl FnMut(&mut ControlPlayerInfoEntry) -> bool,
    ) -> Result<bool, PrepareHostBootstrapError> {
        let Some(pending) = self.pending_initial_league_players.take() else {
            return Ok(false);
        };
        let max_players = usize::try_from(self.admission.max_players).map_err(|_| {
            PrepareHostBootstrapError::MaxPlayersOutOfRange(self.admission.max_players)
        })?;
        let (control, team_metadata, last_player_id) = finalize_initial_host_player_info(
            authenticated_players,
            &pending.alternate_colors_by_resource,
            pending.restore_last_player_id,
            max_players,
            &pending.restore_players,
            pending.team_metadata,
            team_assignment_oracle,
            check,
        )?;
        self.initial_host_player_info_control = control.clone();
        self.runtime_team_metadata = team_metadata.clone();
        if let Some(snapshot) = self.host_config.initial_join_snapshot.as_mut() {
            snapshot.parameters.player_infos = PlayerInfoListSnapshot {
                last_player_id,
                clients: vec![ClientPlayerInfosSnapshot {
                    client_id: control.client_id,
                    flags: control.flags & !CLIENT_PLAYER_INFO_FLAG_UPDATED,
                    players: control.players,
                }],
            };
            snapshot.parameters.teams = join_team_list_snapshot(team_metadata);
        }
        Ok(true)
    }

    /// The live team list after initial host PlayerInfo assignment. C++ keeps
    /// this state for subsequent runtime PlayerInfo requests.
    pub fn runtime_team_metadata(&self) -> &InitialNetworkTeamMetadata {
        &self.runtime_team_metadata
    }

    /// Executes the local half of the host's direct Initial PlayerInfo and
    /// returns the only capability which can open lobby admission.
    pub fn install_initial_host_player_state(
        &self,
        registry: &mut clonk_engine::ControlPlayerInfoRegistry,
        mut install_resource: impl FnMut(&NetworkResourceCore, &Path),
    ) -> Result<PreparedHostAdmissionReady, PreparedHostUseError> {
        self.lifetime
            .initial_player_info_installed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PreparedHostUseError::InitialPlayerInfoAlreadyInstalled)?;
        for (core, path) in &self.local_player_resources {
            install_resource(core, path);
        }
        // Parameters::Load transfers RestorePlayerInfos' raw allocation
        // counter into PlayerInfos before any local admission. Preserve that
        // counter even when it is greater than every retained row ID.
        let last_player_id = self
            .host_config
            .initial_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.player_infos.last_player_id)
            .unwrap_or_else(|| {
                self.initial_host_player_info_control
                    .players
                    .iter()
                    .map(|player| player.id)
                    .max()
                    .unwrap_or(0)
            });
        registry.replace_snapshot(
            last_player_id,
            [self.initial_host_player_info_control.clone()],
        );
        Ok(PreparedHostAdmissionReady {
            admission: self.admission,
        })
    }

    pub fn scenario_wire_name(&self) -> &LegacyCString {
        &self.scenario_wire_name
    }

    pub fn scenario_origin(&self) -> &str {
        &self.scenario_origin
    }

    pub fn dynamic_wire_name(&self) -> &LegacyCString {
        &self.dynamic_wire_name
    }

    pub fn dynamic_filename_seed(&self) -> &str {
        &self.dynamic_filename_seed
    }
}

#[cfg(test)]
impl PreparedHostBootstrap {
    #[allow(dead_code)]
    pub(crate) fn transport_test_fixture(
        configured_tcp_port: u16,
        configured_udp_port: u16,
        league: Option<PreparedLeagueHostConfig>,
    ) -> Self {
        let host_config = HostConfig {
            configured_tcp_port: Some(configured_tcp_port),
            configured_udp_port: Some(configured_udp_port),
            ..HostConfig::default()
        };
        Self {
            host_config,
            initial_game: InitialNetworkGameData::default(),
            scenario_defaults: InitialNetworkScenarioDefaults::default(),
            has_initial_game: false,
            admission: PreparedHostAdmission {
                max_players: 8,
                no_runtime_join: false,
            },
            start_time: 1,
            initial_host_player_info_control: PlayerInfoControlData::default(),
            runtime_team_metadata: InitialNetworkTeamMetadata {
                active: false,
                custom: false,
                allow_hostility_change: true,
                allow_team_switch: false,
                auto_generate_teams: false,
                last_team_id: 0,
                team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
                team_colors: false,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: Vec::new(),
            },
            scenario_wire_name: LegacyCString::default(),
            scenario_origin: String::new(),
            dynamic_filename_seed: String::new(),
            dynamic_wire_name: LegacyCString::default(),
            definition_modules: Vec::new(),
            definition_executable_path: String::new(),
            definition_path: String::new(),
            material_resource_groups: Vec::new(),
            reference_icon: 0,
            reference_comment: LegacyCString::default(),
            netpuncher_address: LegacyCString::default(),
            league,
            stream_address: LegacyCString::default(),
            local_player_resources: Vec::new(),
            local_player_alternate_colors_by_resource: HashMap::new(),
            pending_initial_league_players: None,
            lifetime: Arc::new(PreparedHostLifetime {
                temporary_files: Vec::new(),
                scenario: Mutex::new(None),
                host_launched: AtomicBool::new(false),
                initial_player_info_installed: AtomicBool::new(false),
            }),
        }
    }
}

#[derive(Debug, Error)]
pub enum PrepareHostBootstrapError {
    #[error("the selected local player could not be admitted into the scenario player slots")]
    LocalPlayerAdmissionRejected,
    #[error("initial local player attributes could not be resolved: {0}")]
    LocalPlayerAttributeConflict(#[source] TeamColorUpdateError),
    #[error("replay network hosting is rejected by C++")]
    ReplayUnsupported,
    #[error("a scenario already marked NetworkGame cannot be direct-started as a host")]
    NetworkGameScenarioUnsupported,
    #[error("old-save Game.txt DefinitionFiles overrides are not yet supported for network hosts")]
    SavegameDefinitionOverrideUnsupported,
    #[error(
        "staged definition resources changed before host preparation: staged {staged:?}, prepared {prepared:?}"
    )]
    StagedDefinitionResourcesChanged {
        staged: Vec<PathBuf>,
        prepared: Vec<PathBuf>,
    },
    #[error(
        "staged definition selection changed before host preparation: staged {staged:?}, prepared {prepared:?}"
    )]
    StagedDefinitionSelectionChanged {
        staged: Vec<String>,
        prepared: Vec<String>,
    },
    #[error(
        "staged definition publication names changed before host preparation: staged {staged:?}, prepared {prepared:?}"
    )]
    StagedDefinitionPublicationChanged {
        staged: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
        prepared: Vec<(Vec<u8>, Vec<u8>, Vec<u8>)>,
    },
    #[error("published game resource {resource_id} has no retained local file")]
    PublishedResourceFileMissing { resource_id: i32 },
    #[error(
        "published game resource {resource_id} could not be reopened at {}: {source}",
        path.display()
    )]
    PublishedResourceGroup {
        resource_id: i32,
        path: PathBuf,
        #[source]
        source: GroupError,
    },
    #[error("scenario flag `{key}` has unsupported value `{value}`")]
    InvalidScenarioFlag { key: &'static str, value: String },
    #[error("scenario group could not be opened at {}: {source}", path.display())]
    ScenarioGroup {
        path: PathBuf,
        #[source]
        source: GroupError,
    },
    #[error("scenario group entry `{entry}` could not be read: {source}")]
    ScenarioEntry {
        entry: &'static str,
        #[source]
        source: GroupError,
    },
    #[error("scenario has no Scenario.txt core")]
    ScenarioCoreMissing,
    #[error("scenario Scenario.txt core is not UTF-8")]
    ScenarioCoreEncoding,
    #[error("scenario path is not contained by an explicit install root: {0}")]
    ScenarioOutsideInstallRoots(PathBuf),
    #[error("scenario root-relative path is not representable on the legacy wire: {0}")]
    InvalidScenarioWirePath(PathBuf),
    #[error("scenario path has no C4S basename: {0}")]
    InvalidScenarioBasename(PathBuf),
    #[error("network work path is not a supported relative legacy path: {0}")]
    InvalidNetworkWorkPath(String),
    #[error("{field} is outside the exact ASCII input subset")]
    UnsupportedText { field: &'static str },
    #[error("{field} Unix time {value} does not fit the C++ signed 32-bit field")]
    UnixSecondsOutOfRange { field: &'static str, value: i64 },
    #[error("scenario max-player value {0} cannot be represented by HostConfig")]
    MaxPlayersOutOfRange(i32),
    #[error("saved control tick {0} cannot be represented by HostConfig")]
    ControlTickOutOfRange(i32),
    #[error("saved Game.txt runtime state cannot be applied: {0}")]
    InvalidGameRuntime(#[from] InitialNetworkGameApplyError),
    #[error("the prepared host has no initial JoinData snapshot")]
    MissingJoinSnapshot,
    #[error("scenario metadata could not be prepared: {0}")]
    Scenario(#[from] ScenarioError),
    #[error("scenario metadata could not be adapted: {0}")]
    Metadata(#[from] InitialNetworkMetadataError),
    #[error("host game resources could not be resolved: {0}")]
    Resources(#[from] HostGameResourceSourceError),
    #[error("initial network dynamic could not be composed: {0}")]
    Dynamic(#[from] InitialNetworkDynamicError),
    #[error("initial host resources could not be published: {0}")]
    Publication(#[from] HostInitialResourcePublicationError),
}

extern "C" {
    #[link_name = "rand"]
    fn c_rand() -> c_int;
}

/// Serializes every main-thread-style transaction over C's process-global
/// `rand()` stream. Loader, audio, and team assignment all share this owner.
pub static CLASSIC_SAFE_RANDOM_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[allow(dead_code)] // The lightweight clonk-app library test harness omits network.rs.
pub(crate) fn league_checksum_start() -> u32 {
    let _guard = CLASSIC_SAFE_RANDOM_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // C4LeagueClient uses `rand() | rand() << 16` before each request.
    let low = unsafe { c_rand() } as u32;
    let high = unsafe { c_rand() } as u32;
    low | high.wrapping_shl(16)
}

fn format_generated_team_name(template: &LegacyCString, id: i32) -> LegacyCString {
    let id = id.to_string();
    let mut formatted = Vec::with_capacity(template.as_bytes().len().saturating_add(id.len()));
    let mut source = template.as_bytes().iter().copied();
    while let Some(byte) = source.next() {
        if byte != b'%' {
            formatted.push(byte);
            continue;
        }
        match source.next() {
            Some(b'd') => formatted.extend_from_slice(id.as_bytes()),
            Some(b'%') => formatted.push(b'%'),
            Some(other) => formatted.extend_from_slice(&[b'%', other]),
            None => formatted.push(b'%'),
        }
    }
    formatted.truncate(30);
    LegacyCString::from_bytes(formatted)
        .expect("a validated resource string and decimal team ID contain no NUL")
}

/// The same process-global C runtime stream used by C++ `SafeRandom`.
///
/// This deliberately does not derive from `Parameters.RandomSeed`: C++ seeds
/// that separate deterministic simulation stream only after the lobby.
pub struct ProcessInitialHostTeamAssignmentOracle {
    team_name_template: LegacyCString,
    _guard: Option<std::sync::MutexGuard<'static, ()>>,
}

impl ProcessInitialHostTeamAssignmentOracle {
    pub fn new(team_name_template: LegacyCString) -> Self {
        Self {
            team_name_template,
            _guard: None,
        }
    }

    fn ensure_random_locked(&mut self) {
        if self._guard.is_none() {
            self._guard = Some(
                CLASSIC_SAFE_RANDOM_LOCK
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()),
            );
        }
    }

    fn with_shipped_team_name() -> Self {
        Self::new(
            LegacyCString::from_bytes(b"Team %d".to_vec())
                .expect("the shipped team-name resource contains no NUL"),
        )
    }
}

impl InitialHostTeamAssignmentOracle for ProcessInitialHostTeamAssignmentOracle {
    fn safe_random(&mut self, range: i32) -> i32 {
        if range == 0 {
            return 0;
        }
        self.ensure_random_locked();
        // SAFETY: `rand` has no pointer preconditions and C++ calls this exact
        // process-global function from `SafeRandom` (src/C4Random.h:71-75).
        unsafe { c_rand() % range }
    }

    fn generate_team(
        &mut self,
        id: i32,
        existing_teams: &[InitialNetworkTeam],
    ) -> InitialNetworkTeam {
        let name = format_generated_team_name(&self.team_name_template, id);
        clonk_engine::generate_default_initial_team(id, name, existing_teams, self)
    }
}

/// `C4PlayerInfoList::CreateRestoreInfosForJoinedScriptPlayers`: append a
/// copy of each still-unclaimed restore script player to the first (host)
/// packet in restore-list storage order.
fn append_unclaimed_script_restore_infos(
    host_players: &mut Vec<ControlPlayerInfoEntry>,
    restore_players: &[ControlPlayerInfoEntry],
) {
    for restore in restore_players {
        if !restore.is_script_player()
            || host_players
                .iter()
                .any(|player| player.savegame_player == restore.id)
        {
            continue;
        }
        let mut rejoin = restore.clone();
        rejoin.savegame_player = restore.id;
        host_players.push(rejoin);
    }
}

fn swap_remove_rejected_players(
    players: &mut Vec<ControlPlayerInfoEntry>,
    mut retain: impl FnMut(&mut ControlPlayerInfoEntry) -> bool,
) {
    let mut index = 0;
    while index < players.len() {
        if retain(&mut players[index]) {
            index += 1;
        } else {
            players.swap_remove(index);
        }
    }
}

fn finalize_initial_host_player_info(
    initial_players: Vec<ControlPlayerInfoEntry>,
    alternate_colors_by_resource: &HashMap<i32, u32>,
    restore_last_player_id: i32,
    max_players: usize,
    restore_players: &[ControlPlayerInfoEntry],
    mut team_metadata: InitialNetworkTeamMetadata,
    team_assignment_oracle: &mut impl InitialHostTeamAssignmentOracle,
    mut check: impl FnMut(&mut ControlPlayerInfoEntry) -> bool,
) -> Result<(PlayerInfoControlData, InitialNetworkTeamMetadata, i32), PrepareHostBootstrapError> {
    let mut player_allocator = clonk_engine::ControlPlayerInfoRegistry::default();
    player_allocator.replace_snapshot(
        restore_last_player_id,
        std::iter::empty::<PlayerInfoControlData>(),
    );
    let mut control = player_allocator
        .admit_request(
            PlayerInfoUpdateRequest {
                client_id: 0,
                flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: initial_players,
            },
            max_players,
        )
        .ok_or(PrepareHostBootstrapError::LocalPlayerAdmissionRejected)?;
    assign_initial_host_player_teams(
        &mut team_metadata,
        &mut control.players,
        team_assignment_oracle,
    );
    let admission = player_allocator
        .resolve_admitted_player_attributes_with_alternate_colors(
            control,
            Some(&team_metadata),
            restore_players,
            team_assignment_oracle,
            |player| {
                player
                    .resource
                    .as_ref()
                    .and_then(|resource| alternate_colors_by_resource.get(&resource.id).copied())
            },
        )
        .map_err(PrepareHostBootstrapError::LocalPlayerAttributeConflict)?;
    assert!(
        admission.updated_existing.is_empty(),
        "a fresh initial-host registry cannot produce retained PlayerInfo updates"
    );
    control = admission.admitted;
    for player in &mut control.players {
        player.league_projected_gain = -1;
    }
    swap_remove_rejected_players(&mut control.players, &mut check);
    append_unclaimed_script_restore_infos(&mut control.players, restore_players);
    player_allocator.apply(control.clone());
    player_allocator.recheck_team_players(&mut team_metadata);
    let (last_player_id, _) = player_allocator.retained_rows_snapshot();
    Ok((control, team_metadata, last_player_id))
}
fn join_data_id_list(
    entries: &[ScenarioLobbyIdEntry],
) -> Result<Vec<JoinDataIdListEntry>, InitialNetworkMetadataError> {
    entries
        .iter()
        .map(|entry| {
            let bytes: [u8; 4] = entry.id().as_bytes().try_into().map_err(|_| {
                InitialNetworkMetadataError::InvalidScenarioId(entry.id().to_string())
            })?;
            let id = JoinDataC4Id::from_bytes(bytes).ok_or_else(|| {
                InitialNetworkMetadataError::InvalidScenarioId(entry.id().to_string())
            })?;
            Ok(JoinDataIdListEntry {
                id,
                count: entry.count(),
            })
        })
        .collect()
}

/// Applies the `mkParAdapt(*this, pScenario)` compiler result used when an
/// embedded Parameters.txt exists. Client rows are deliberately omitted:
/// `C4Game::InitNetworkHost` clears that saved list and installs the fresh
/// local host before creating the initial dynamic.
fn apply_embedded_game_parameters(
    parameters: &mut JoinGameParametersEnvelope,
    embedded: &ScenarioGameParameterValues,
) -> Result<(), InitialNetworkMetadataError> {
    parameters.random_seed = embedded.random_seed();
    parameters.startup_player_count = embedded.startup_player_count();
    parameters.max_players = embedded.max_players();
    parameters.use_fair_crew = embedded.use_fair_crew();
    parameters.fair_crew_forced = embedded.fair_crew_forced();
    parameters.fair_crew_strength = embedded.fair_crew_strength();
    parameters.allow_debug = embedded.allow_debug();
    parameters.is_network_game = embedded.is_network_game();
    parameters.control_rate = embedded.control_rate();
    parameters.auto_frame_skip = embedded.auto_frame_skip();
    parameters.rules = join_data_id_list(embedded.rules())?;
    parameters.goals = join_data_id_list(embedded.goals())?;
    parameters.league = legacy_c4_string(embedded.league());
    Ok(())
}

fn enforce_league_rules(
    parameters: &mut JoinGameParametersEnvelope,
    teams: &mut InitialNetworkTeamMetadata,
    max_players_league: i32,
) {
    parameters.allow_debug = false;
    teams.allow_team_switch = false;
    if !parameters.fair_crew_forced {
        parameters.use_fair_crew = true;
        parameters.fair_crew_forced = true;
        parameters.fair_crew_strength = 20_000;
    }
    parameters.max_players = max_players_league;
}

/// Builds the exact currently-supported initial host state without opening a
/// socket, registering with a masterserver, or making the game joinable.
pub fn prepare_host_bootstrap(
    spec: PreparedHostBootstrapSpec<'_>,
) -> Result<PreparedHostBootstrap, PrepareHostBootstrapError> {
    let mut oracle = ProcessInitialHostTeamAssignmentOracle::with_shipped_team_name();
    prepare_host_bootstrap_with_team_assignment_oracle(spec, &mut oracle)
}

/// Injection seam for the process-local services used by C++ team assignment.
#[doc(hidden)]
pub fn prepare_host_bootstrap_with_team_assignment_oracle(
    spec: PreparedHostBootstrapSpec<'_>,
    team_assignment_oracle: &mut impl InitialHostTeamAssignmentOracle,
) -> Result<PreparedHostBootstrap, PrepareHostBootstrapError> {
    validate_inputs(&spec)?;
    let scenario_group = open_group_path(spec.scenario_path).map_err(|source| {
        PrepareHostBootstrapError::ScenarioGroup {
            path: spec.scenario_path.to_path_buf(),
            source,
        }
    })?;
    // C4Game resolves the presentation title from the native Title component,
    // but copies the component bytes verbatim into synchronized parameters.
    // Resolve it again here because the app-facing selector model deliberately
    // retains only Unicode presentation text.
    let loader_head = ScenarioLoaderHead::load_from_group_with_languages_and_packs(
        &scenario_group,
        spec.languages,
        spec.language_packs,
    )?;
    let scenario_title_native =
        LegacyCString::from_bytes(loader_head.scenario_title_bytes().to_vec())
            .expect("a resolved scenario title contains no interior NUL");
    let scenario_title_c4 = clonk_script::c4_string_from_bytes(scenario_title_native.as_bytes());
    if !matches!(
        loader_head.savegame_definition_override(),
        clonk_engine::scenario::ScenarioSavegameDefinitionOverride::None
    ) {
        return Err(PrepareHostBootstrapError::SavegameDefinitionOverrideUnsupported);
    }
    let original_game_text = validate_scenario_group(&scenario_group)?;
    let has_embedded_parameters = read_direct_entry(&scenario_group, "Parameters.txt")?
        .is_some_and(|source| !source.is_empty());
    let definition_resolver = InstallRootDefinitionResolver::new(
        spec.install_roots,
        spec.language_packs,
        spec.effective_definition_modules,
        spec.definition_resources,
        spec.selector_definition_root.is_some(),
    );
    let mut scenario =
        Scenario::load_from_group_with_languages_and_definition_selection_and_prefix(
            &scenario_group,
            &definition_resolver,
            spec.languages,
            spec.initial_definition_modules,
            spec.fixed_definition_modules,
            spec.selector_definition_root,
        )?;
    let lobby_metadata = scenario
        .lobby_metadata()
        .ok_or(ScenarioError::InitialNetworkScenarioUnsupported)?;
    let prepared_definition_modules = lobby_metadata
        .definitions()
        .effective_modules()
        .ok_or(PrepareHostBootstrapError::SavegameDefinitionOverrideUnsupported)?;
    if prepared_definition_modules != spec.effective_definition_modules {
        return Err(
            PrepareHostBootstrapError::StagedDefinitionSelectionChanged {
                staged: spec.effective_definition_modules.to_vec(),
                prepared: prepared_definition_modules.to_vec(),
            },
        );
    }
    let effective_definition_resource_paths = lobby_metadata
        .definitions()
        .resolved_load_resources()
        .ok_or(PrepareHostBootstrapError::SavegameDefinitionOverrideUnsupported)?;
    let staged_definition_resource_paths = spec
        .definition_resources
        .iter()
        .map(|resource| resource.path.clone())
        .collect::<Vec<_>>();
    if effective_definition_resource_paths != staged_definition_resource_paths {
        return Err(
            PrepareHostBootstrapError::StagedDefinitionResourcesChanged {
                staged: staged_definition_resource_paths,
                prepared: effective_definition_resource_paths.to_vec(),
            },
        );
    }
    let prepared_definition_spellings = lobby_metadata.definitions().requested_module_spellings();
    let definition_executable_root = path_from_legacy_text(spec.definition_executable_path);
    let prepared_definition_resources = freeze_host_definition_resource_sources(
        effective_definition_resource_paths,
        spec.scenario_path,
        prepared_definition_spellings,
        lobby_metadata.definitions().definition_root_applied(),
        &definition_executable_root,
        spec.definition_path,
    )?;
    let publication_names = |resources: &[HostInitialResourceSource]| {
        resources
            .iter()
            .map(|resource| {
                (
                    resource.lookup_name.as_bytes().to_vec(),
                    resource.opened_name.as_bytes().to_vec(),
                    resource.wire_name.as_bytes().to_vec(),
                )
            })
            .collect::<Vec<_>>()
    };
    let staged_publication_names = publication_names(spec.definition_resources);
    let prepared_publication_names = publication_names(&prepared_definition_resources);
    if staged_publication_names != prepared_publication_names {
        return Err(
            PrepareHostBootstrapError::StagedDefinitionPublicationChanged {
                staged: staged_publication_names,
                prepared: prepared_publication_names,
            },
        );
    }
    let resource_sources = resolve_host_game_resource_sources(
        spec.scenario_path,
        spec.install_roots,
        spec.definition_resources,
        &definition_executable_root,
    )?;
    let embedded_parameters = has_embedded_parameters
        .then(|| lobby_metadata.embedded_game_parameter_values())
        .flatten();
    let is_save_game = lobby_metadata.head().is_save_game();
    let max_players_league = lobby_metadata.head().max_players_league();
    let restore_player_infos = load_restore_player_infos(
        &scenario_group,
        &spec,
        is_save_game
            .then_some(original_game_text.as_deref())
            .flatten(),
    )?;
    let restore_players = restore_player_infos
        .clients
        .iter()
        .flat_map(|client| client.players.iter().cloned())
        .collect::<Vec<_>>();
    let (scenario_origin, _, dynamic_group_filename, dynamic_wire_name) = network_names(
        spec.scenario_path,
        spec.install_roots,
        spec.network_work_path,
    )?;
    let scenario_wire_name = LegacyCString::from_bytes(executable_relative_group_name(
        scenario_group.root(),
        &definition_executable_root,
    ))
    .expect("an OS path cannot contain an interior NUL");
    let scenario_opened_name = LegacyCString::from_bytes(opened_physical_group_name(
        scenario_group.root(),
        &definition_executable_root,
    ))
    .expect("an OS path cannot contain an interior NUL");
    let scenario_metadata = scenario.initial_network_scenario_metadata()?;

    let mut team_metadata = scenario.initial_network_team_metadata()?;
    let local_players = spec
        .player_sources
        .iter()
        .filter_map(|selected| {
            let identity = match selected.identity.as_ref() {
                Some(identity) => PreparedLocalPlayerIdentity::Configured(identity.clone()),
                None => match open_group_path(&selected.resource.path)
                    .map_err(|error| error.to_string())
                    .and_then(|group| PlayerFile::load(&group).map_err(|error| error.to_string()))
                {
                    Ok(player) => PreparedLocalPlayerIdentity::Generic(player),
                    Err(error) => {
                        // Generic callers have not already loaded a classic
                        // player core. Match C4ClientPlayerInfos by dropping
                        // only this failed module and continuing the ordered
                        // participant list.
                        tracing::warn!(
                            path = %selected.resource.path.display(),
                            %error,
                            "skipping unreadable initial host player"
                        );
                        return None;
                    }
                },
            };
            let source = match validate_host_group_resource_source(
                HostGameResourceSourceKind::Player,
                selected.resource.clone(),
            ) {
                Ok(source) => source,
                Err(error) => {
                    tracing::warn!(
                        path = %selected.resource.path.display(),
                        %error,
                        "skipping unpublishable initial host player"
                    );
                    return None;
                }
            };
            Some((source, identity))
        })
        .map(|(source, identity)| {
            match &identity {
                PreparedLocalPlayerIdentity::Configured(identity) => validate_network_name_bytes(
                    "local player name",
                    identity.player_name.as_bytes(),
                    false,
                )?,
                PreparedLocalPlayerIdentity::Generic(player) => {
                    validate_c4_network_name("local player name", &player.name, false)?;
                }
            }
            Ok::<_, PrepareHostBootstrapError>((source, identity))
        })
        .collect::<Result<Vec<_>, _>>()?;
    // SaveCore writes Game.DefinitionFilenames, not the unmodified scenario
    // module list. OpenScenario appends every folder-local definitions group
    // before this save (src/C4Game.cpp:179-213; C4GameSave.cpp:89-92).
    let definition_modules = resource_sources
        .definitions
        .iter()
        .map(|source| clonk_script::c4_string_from_bytes(source.lookup_name.as_bytes()))
        .collect::<Vec<_>>();
    let group_maker = legacy_string(spec.group_maker);
    let host_name = legacy_string(spec.host_name);
    let host_nick = legacy_string(spec.host_nick);
    let local_core = ClientCoreControlData {
        client_id: 0,
        activated: true,
        observer: false,
        name: host_name,
        nick: host_nick,
        lobby_ready: false,
    };
    let dynamic_host_player_info_control = PlayerInfoControlData {
        client_id: 0,
        flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
        players: Vec::new(),
        by_client: 0,
    };
    let initial_host_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: vec![ClientPlayerInfosSnapshot {
            client_id: dynamic_host_player_info_control.client_id,
            flags: dynamic_host_player_info_control.flags,
            players: dynamic_host_player_info_control.players.clone(),
        }],
    };
    let mut parameters = JoinGameParametersEnvelope {
        random_seed: spec.random_seed_unix_seconds as i32,
        startup_player_count: 0,
        max_players: 0,
        use_fair_crew: spec.config.fair_crew,
        fair_crew_forced: false,
        fair_crew_strength: spec.config.fair_crew_strength,
        allow_debug: true,
        is_network_game: true,
        control_rate: spec.config.control_rate,
        auto_frame_skip: spec.config.auto_frame_skip,
        rules: Vec::new(),
        goals: Vec::new(),
        league: LegacyCString::default(),
        // Parameters.txt cannot compile LeagueAddress while a Scenario is
        // supplied. Configured league signup installs it only after the
        // initial dynamic has been created.
        league_address: LegacyCString::default(),
        title: scenario_title_native,
        scenario: NetworkResourceCore::default(),
        game_resources: Vec::new(),
        player_infos: initial_host_players,
        restore_player_infos: restore_player_infos.clone(),
        teams: empty_team_snapshot(),
        clients: JoinClientRegistrySnapshot {
            clients: vec![local_core.clone()],
            local_client_id: Some(0),
        },
    };
    let scenario_defaults = fill_scenario_derived_join_parameters(
        &mut parameters,
        &scenario_metadata,
        team_metadata.clone(),
    )?;
    if let Some(embedded) = embedded_parameters.as_ref() {
        apply_embedded_game_parameters(&mut parameters, embedded)?;
        // `League` is only the synchronized display name. Native's
        // `isLeague()` tests `LeagueAddress`, which is not compiled while a
        // scenario is supplied, so an embedded display name does not enforce
        // league restrictions. InitLeague clears it after CreateDynamic;
        // only configured league signup below installs an address and calls
        // EnforceLeagueRules (C4GameParameters.h:173;
        // C4GameParameters.cpp:362-471,575; C4Network2.cpp:2224-2246).
    }
    if is_save_game {
        let restore_count = i32::try_from(restore_players.len()).unwrap_or(i32::MAX);
        parameters.max_players = parameters.max_players.max(restore_count);
    }

    // Every stored nonempty Game.txt is compiled, including an ordinary
    // scenario whose component contains only the legacy [Player...] tail.
    // That compile applies named defaults (not InitSystem's live defaults)
    // before the initial save canonicalizes and re-appends the player tail.
    let game = original_game_text
        .as_deref()
        .map(parse_initial_network_game_data)
        .unwrap_or_default();
    game.validate_runtime_application()?;
    scenario.validate_initial_network_game_data(&game)?;
    // InitHost snapshots the already-compiled runtime ControlTick into both
    // its lobby status and CreateDynamic. ControlRate is still the stock one
    // at this point, so getNextControlTick() is exactly ControlTick
    // (src/C4Network2.cpp:222-230,1945-1971;
    // src/C4GameControl.cpp:363-366).
    let dynamic_tick = game.control_tick;
    let host_start_tick = u32::try_from(dynamic_tick)
        .map_err(|_| PrepareHostBootstrapError::ControlTickOutOfRange(dynamic_tick))?;
    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: &dynamic_group_filename,
        maker: group_maker.as_bytes(),
        scenario: &scenario,
        scenario_title: &scenario_title_c4,
        definition_modules: &definition_modules,
        definition_executable_path: spec.definition_executable_path,
        definition_path: spec.definition_path,
        scenario_origin: &scenario_origin,
        game: &game,
        original_game_text: original_game_text.as_deref(),
        parameters: &parameters,
        scenario_defaults: &scenario_defaults,
    })?;

    // InitLeague runs after CreateDynamic. It clears any embedded display
    // league, then configured league signup supplies the authoritative
    // address and enforces the scenario's league restrictions.
    parameters.league = LegacyCString::default();
    parameters.league_address = LegacyCString::default();
    if let Some(league) = spec.league.filter(|league| league.league_server_signup) {
        parameters.league_address = legacy_string(&league.endpoint);
        enforce_league_rules(&mut parameters, &mut team_metadata, max_players_league);
        parameters.teams = join_team_list_snapshot(team_metadata.clone());
    }
    let max_players = usize::try_from(parameters.max_players)
        .map_err(|_| PrepareHostBootstrapError::MaxPlayersOutOfRange(parameters.max_players))?;

    let scenario_resource = validate_host_group_resource_source(
        HostGameResourceSourceKind::Scenario,
        HostInitialResourceSource {
            path: spec.scenario_path.to_path_buf(),
            lookup_name: scenario_opened_name.clone(),
            opened_name: scenario_opened_name,
            wire_name: scenario_wire_name.clone(),
            virtual_group_bytes: None,
        },
    )?;
    let mut publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: spec.network_directory.to_path_buf(),
        group_maker: group_maker.clone(),
        max_load_file_size: spec.config.max_load_file_size,
        scenario: scenario_resource,
        definitions: resource_sources.definitions,
        system: resource_sources.system,
        materials: resource_sources.materials,
        players: local_players
            .iter()
            .map(|(source, _)| source.clone())
            .collect(),
        dynamic,
        dynamic_wire_name: dynamic_wire_name.clone(),
        parameters,
        dynamic_tick,
    })?;
    // Publication has transferred ownership of generated standalones. Arm
    // their cleanup before any post-publication reopen/reload can fail.
    let temporary_files = publication
        .resource_files
        .iter()
        .filter(|resource| resource.ownership == ResourceFileOwnership::Temporary)
        .map(|resource| resource.path.clone())
        .collect();
    let temporary_files = PreparedTemporaryFiles::new(temporary_files);
    // C++'s pre-publication OpenScenario only establishes metadata and probes
    // the definition groups. InitDefs/InitMaterialTexture run after every
    // AddByFile/SetNetRes row is final and after Parameters.RandomSeed is
    // frozen. Rebuild unconditionally from those exact rows, even when no
    // cross-type reuse occurred, so the host and clients consume identical
    // bytes and random landscape seed.
    let definition_groups =
        published_game_resource_groups(&publication, HostResourceType::Definitions)?;
    let material_resource_groups =
        published_game_resource_groups(&publication, HostResourceType::Material)?;
    let graphics_groups = definition_resolver
        .resolve_graphics_groups_with_definition_roots(&scenario_group, &definition_groups)?;
    let random_seed = u64::from(publication.join_snapshot.parameters.random_seed as u32);
    scenario = Scenario::load_network_from_group_with_languages_and_seed_and_packs(
        &scenario_group,
        &definition_groups,
        &material_resource_groups,
        &graphics_groups,
        spec.languages,
        random_seed,
        spec.language_packs,
    )?;
    let mut published_index = 0;
    let published_local_players = local_players
        .iter()
        .filter_map(|(source, player)| {
            let (published_path, core) =
                publication.player_resource_sources.get(published_index)?;
            if published_path != &source.path {
                return None;
            }
            published_index += 1;
            Some((source, player, core))
        })
        .collect::<Vec<_>>();
    debug_assert_eq!(published_index, publication.player_resource_sources.len());
    let initial_players = published_local_players
        .iter()
        .map(|(_, identity, core)| {
            let color = identity.network_color();
            ControlPlayerInfoEntry {
                name: identity.player_name(),
                filename: core.filename.clone(),
                flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                color,
                original_color: color,
                resource: Some((**core).clone()),
                ..ControlPlayerInfoEntry::default()
            }
        })
        .collect();
    // AlternateColorDw is deliberately absent from C4PlayerInfo's wire
    // compiler. Resource IDs survive league authentication, swap-removal and
    // capacity pruning, so they provide the stable process-local identity
    // needed when the host later resolves the admitted packet.
    let alternate_colors_by_resource = published_local_players
        .iter()
        .filter_map(|(_, identity, core)| {
            identity
                .alternate_color()
                .map(|alternate| (core.id, alternate))
        })
        .collect::<HashMap<_, _>>();
    // Both master-reference and league signup run their Start transaction
    // before Network.Players.Init. Its returned MaxPlayers must therefore be
    // installed before local IDs, team assignment and slot pruning.
    let defer_league_players = spec.league.is_some();
    let (
        initial_host_player_info_control,
        runtime_team_metadata,
        last_player_id,
        pending_initial_league_players,
    ) = if defer_league_players {
        (
            PlayerInfoControlData {
                client_id: 0,
                flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: Vec::new(),
                by_client: 0,
            },
            team_metadata.clone(),
            restore_player_infos.last_player_id,
            Some(PendingInitialLeaguePlayers {
                players: initial_players,
                alternate_colors_by_resource: alternate_colors_by_resource.clone(),
                restore_players: restore_players.clone(),
                restore_last_player_id: restore_player_infos.last_player_id,
                team_metadata,
            }),
        )
    } else {
        let (control, team_metadata, last_player_id) = finalize_initial_host_player_info(
            initial_players,
            &alternate_colors_by_resource,
            restore_player_infos.last_player_id,
            max_players,
            &restore_players,
            team_metadata,
            team_assignment_oracle,
            |_| true,
        )?;
        (control, team_metadata, last_player_id, None)
    };
    publication.join_snapshot.parameters.player_infos = PlayerInfoListSnapshot {
        last_player_id,
        clients: vec![ClientPlayerInfosSnapshot {
            client_id: initial_host_player_info_control.client_id,
            flags: initial_host_player_info_control.flags & !CLIENT_PLAYER_INFO_FLAG_UPDATED,
            players: initial_host_player_info_control.players.clone(),
        }],
    };
    publication.join_snapshot.parameters.teams =
        join_team_list_snapshot(runtime_team_metadata.clone());
    let local_player_resources = published_local_players
        .iter()
        .map(|(source, _, core)| {
            let path = if source.path.exists() {
                source.path.clone()
            } else {
                publication
                    .resource_files
                    .iter()
                    .find(|resource| resource.core.id == core.id)
                    .map(|resource| resource.path.clone())
                    .unwrap_or_else(|| source.path.clone())
            };
            ((**core).clone(), path)
        })
        .collect();
    let resolved_dynamic_wire_name = publication.join_snapshot.dynamic.filename.clone();
    let mut host_config = HostConfig {
        max_players,
        start_tick: host_start_tick,
        async_max_wait_frames: spec.config.async_max_wait,
        local_core,
        group_maker,
        initial_status: NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: spec.config.control_mode,
            target_tick: game.control_tick,
        },
        password: legacy_string(spec.network_password),
        allow_join: false,
        enable_upnp: spec.config.enable_upnp,
        configured_tcp_port: Some(spec.config.network_tcp_port),
        configured_udp_port: Some(spec.config.network_udp_port),
        local_resource_roots: spec.install_roots.to_vec(),
        ..HostConfig::default()
    };
    publication.apply_to(&mut host_config);
    let temporary_files = temporary_files.into_lifetime_paths();

    Ok(PreparedHostBootstrap {
        host_config,
        initial_game: game,
        scenario_defaults,
        has_initial_game: original_game_text.is_some(),
        admission: PreparedHostAdmission {
            max_players: i32::try_from(max_players)
                .expect("validated nonnegative C++ max-player value round-trips"),
            no_runtime_join: spec.config.no_runtime_join,
        },
        start_time: spec.start_unix_seconds as i32,
        initial_host_player_info_control,
        runtime_team_metadata,
        scenario_wire_name,
        scenario_origin,
        dynamic_filename_seed: dynamic_group_filename,
        dynamic_wire_name: resolved_dynamic_wire_name,
        definition_modules,
        definition_executable_path: spec.definition_executable_path.to_owned(),
        definition_path: spec.definition_path.to_owned(),
        material_resource_groups,
        reference_icon: scenario_metadata.icon,
        reference_comment: legacy_string(spec.network_comment),
        netpuncher_address: legacy_string(spec.netpuncher_address),
        league: spec.league.cloned(),
        stream_address: LegacyCString::default(),
        local_player_resources,
        local_player_alternate_colors_by_resource: alternate_colors_by_resource,
        pending_initial_league_players,
        lifetime: Arc::new(PreparedHostLifetime {
            temporary_files,
            scenario: Mutex::new(Some(scenario)),
            host_launched: AtomicBool::new(false),
            initial_player_info_installed: AtomicBool::new(false),
        }),
    })
}

fn path_from_legacy_text(value: &str) -> PathBuf {
    clonk_resources::path_from_legacy_bytes(&clonk_script::c4_string_bytes(value))
}

fn published_game_resource_groups(
    publication: &HostInitialResourcePublication,
    resource_type: HostResourceType,
) -> Result<Vec<Group>, PrepareHostBootstrapError> {
    publication
        .join_snapshot
        .parameters
        .game_resources
        .iter()
        .filter(|core| core.resource_type == resource_type as u8)
        .map(|core| {
            let resource = publication
                .resource_files
                .iter()
                .find(|resource| resource.core.id == core.id)
                .ok_or(PrepareHostBootstrapError::PublishedResourceFileMissing {
                    resource_id: core.id,
                })?;
            open_group_path(&resource.path).map_err(|source| {
                PrepareHostBootstrapError::PublishedResourceGroup {
                    resource_id: core.id,
                    path: resource.path.clone(),
                    source,
                }
            })
        })
        .collect()
}

struct InstallRootDefinitionResolver<'a> {
    roots: &'a [PathBuf],
    language_packs: &'a LanguagePacks,
    staged_definitions: Mutex<HashMap<String, VecDeque<PathBuf>>>,
}

impl<'a> InstallRootDefinitionResolver<'a> {
    fn new(
        roots: &'a [PathBuf],
        language_packs: &'a LanguagePacks,
        effective_modules: &[String],
        definition_resources: &[HostInitialResourceSource],
        definition_root_applied: bool,
    ) -> Self {
        let original_start =
            usize::from(definition_root_applied).saturating_mul(effective_modules.len());
        let mut staged_definitions: HashMap<String, VecDeque<PathBuf>> = HashMap::new();
        for (module, resource) in effective_modules.iter().zip(
            definition_resources
                .iter()
                .skip(original_start)
                .take(effective_modules.len()),
        ) {
            let normalized = module.replace('\\', "/");
            if Path::new(&normalized).is_absolute() {
                continue;
            }
            staged_definitions
                .entry(normalized.to_ascii_lowercase())
                .or_default()
                .push_back(resource.path.clone());
        }
        Self {
            roots,
            language_packs,
            staged_definitions: Mutex::new(staged_definitions),
        }
    }
}

impl LegacyDefinitionResolver for InstallRootDefinitionResolver<'_> {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let normalized = identifier.replace('\\', "/");
        let staged_path = self
            .staged_definitions
            .lock()
            .get_mut(&normalized.to_ascii_lowercase())
            .and_then(VecDeque::pop_front);
        if let Some(path) = staged_path {
            return open_group_path(&path)
                .map(|group| vec![group])
                .map_err(ScenarioError::Resources);
        }
        let relative = Path::new(&normalized);
        for root in self.roots {
            let candidate = root.join(relative);
            match open_group_path(&candidate) {
                Ok(group) => return Ok(vec![group]),
                Err(
                    GroupError::Missing(_)
                    | GroupError::NotDirectory(_)
                    | GroupError::EntryNotFound(_),
                ) => {}
                Err(GroupError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(ScenarioError::Resources(source)),
            }
        }
        Err(ScenarioError::LegacyDefinitionNotFound {
            path: identifier.to_owned(),
        })
    }

    fn resolve_graphics_groups_with_definition_roots(
        &self,
        scenario: &Group,
        definition_roots: &[Group],
    ) -> Result<Vec<Group>, ScenarioError> {
        let mut groups = Vec::new();
        for definition_root in definition_roots {
            // C4GroupSet::RegisterGroups silently skips a definition root
            // whose direct Graphics.c4g child cannot be opened.
            if let Ok(graphics) = definition_root.open_child(Path::new("Graphics.c4g")) {
                groups.push(graphics);
            }
        }
        groups.extend(self.resolve_graphics_groups(scenario)?);
        Ok(groups)
    }

    fn resolve_language_packs(&self, _scenario: &Group) -> Result<LanguagePacks, ScenarioError> {
        Ok(self.language_packs.clone())
    }
}

fn validate_inputs(spec: &PreparedHostBootstrapSpec<'_>) -> Result<(), PrepareHostBootstrapError> {
    for source in spec.player_sources {
        let wire_name = source.resource.wire_name.as_bytes();
        validate_native_text("local player filename", wire_name, false)?;
        if wire_name.contains(&b';') {
            return Err(PrepareHostBootstrapError::UnsupportedText {
                field: "local player filename",
            });
        }
    }
    validate_c4_text("C4Group maker", spec.group_maker, true)?;
    validate_final_client_name("host network name", spec.host_name)?;
    validate_final_client_name("host network nick", spec.host_nick)?;
    validate_c4_text("network password", spec.network_password, true)?;
    validate_c4_text("network comment", spec.network_comment, true)?;
    validate_ascii_text("netpuncher address", spec.netpuncher_address, true)?;
    if let Some(league) = spec.league {
        validate_ascii_text("league server address", &league.endpoint, false)?;
    }
    for (field, value) in [
        ("game start", spec.start_unix_seconds),
        ("parameter seed", spec.random_seed_unix_seconds),
    ] {
        i32::try_from(value)
            .map(|_| ())
            .map_err(|_| PrepareHostBootstrapError::UnixSecondsOutOfRange { field, value })?;
    }
    Ok(())
}

fn load_restore_player_infos(
    group: &Group,
    spec: &PreparedHostBootstrapSpec<'_>,
    old_style_game_text: Option<&[u8]>,
) -> Result<PlayerInfoListSnapshot, PrepareHostBootstrapError> {
    if let Some(restore_infos) =
        load_save_player_infos_entry(group, spec.languages, spec.language_packs)?
    {
        return Ok(restore_infos);
    }

    Ok(old_style_game_text
        .map(|source| load_old_style_restore_player_infos(group, spec.scenario_path, source))
        .unwrap_or_else(empty_player_info_list))
}

/// Loads the savegame restore list used by ordinary offline scenario opens.
///
/// `C4GameParameters::Load` first tests for the physical
/// `SavePlayerInfos.txt` entry. Any present entry owns the result, including
/// an empty, unreadable or malformed one. Only a genuinely absent entry in a
/// savegame permits the historical `Game.txt` `[PlayerFiles]` fallback.
pub fn load_offline_savegame_restore_player_infos(
    group: &Group,
    scenario_path: &Path,
    languages: &[String],
    language_packs: &LanguagePacks,
    old_style_game_text: Option<&[u8]>,
) -> PlayerInfoListSnapshot {
    match load_save_player_infos_entry(group, languages, language_packs) {
        Ok(Some(restore_infos)) => restore_infos,
        Ok(None) => old_style_game_text
            .map(|source| load_old_style_restore_player_infos(group, scenario_path, source))
            .unwrap_or_else(empty_player_info_list),
        Err(error) => {
            tracing::warn!(%error, "ignoring unreadable SavePlayerInfos.txt");
            empty_player_info_list()
        }
    }
}

/// Loads the restore list that `C4Game::InitPlayers` keeps local while a
/// runtime network client recreates players from its combined scenario.
///
/// Unlike `C4GameParameters::Load`, this path has no old-style `Game.txt`
/// fallback: `C4GameSaveNetwork(false)` always writes `SavePlayerInfos.txt`,
/// and a missing/empty/malformed component leaves the temporary list empty.
pub fn load_runtime_join_restore_player_infos(
    group: &Group,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> PlayerInfoListSnapshot {
    match load_save_player_infos_entry(group, languages, language_packs) {
        Ok(Some(restore_infos)) => restore_infos,
        Ok(None) => empty_player_info_list(),
        Err(error) => {
            // InitPlayers intentionally ignores LocalRestorePlayerInfos.Load's
            // return value. A failed component read therefore continues with
            // the list that Load cleared before attempting the read.
            tracing::warn!(%error, "ignoring unreadable runtime SavePlayerInfos.txt");
            empty_player_info_list()
        }
    }
}

fn load_save_player_infos_entry(
    group: &Group,
    languages: &[String],
    language_packs: &LanguagePacks,
) -> Result<Option<PlayerInfoListSnapshot>, PrepareHostBootstrapError> {
    let Some(source) = read_direct_entry(group, "SavePlayerInfos.txt")? else {
        return Ok(None);
    };
    // C4PlayerInfoList::Load treats an unreadable/empty group entry as an
    // absent list. The direct-entry read above has already distinguished a
    // genuine I/O error from this zero-byte legacy case.
    if source.is_empty() {
        return Ok(Some(empty_player_info_list()));
    }

    let loader_head = ScenarioLoaderHead::load_from_group_for_resource_registration(group)?;
    let components = language_packs.component_groups(group, Some(group), loader_head.origin());
    // SavePlayerInfos is native-byte compiler data, not UTF-8 text. Route it
    // through C4Script's lossless private-use representation so undefined
    // Windows-1252 bytes survive localization unchanged.
    let source = clonk_script::c4_string_from_bytes(&source);
    let localized = localize_script_source_with_components(&components, &source, languages)
        .map_err(|source| PrepareHostBootstrapError::ScenarioEntry {
            entry: "SavePlayerInfos.txt",
            source,
        })?;
    let localized = clonk_script::c4_string_bytes(&localized);
    Ok(Some(
        match clonk_network::decode_player_info_list_ini(&localized) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                // Parameters::Load deliberately ignores C4PlayerInfoList::Load's
                // false return after CompileFromBuf_LogWarn. The list was cleared
                // before compilation, so hosting continues with no restore rows.
                tracing::warn!(%error, "ignoring malformed SavePlayerInfos.txt");
                empty_player_info_list()
            }
        },
    ))
}

fn empty_player_info_list() -> PlayerInfoListSnapshot {
    PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
    }
}

/// `C4PlayerInfoList::LoadFromGameText` compatibility for savegames created
/// before `SavePlayerInfos.txt`. The legacy `[PlayerFiles]` list names player
/// groups embedded directly in the scenario; successfully loaded rows receive
/// ascending IDs and joined game numbers before Parameters.txt is compiled.
fn load_old_style_restore_player_infos(
    group: &Group,
    scenario_path: &Path,
    game_text: &[u8],
) -> PlayerInfoListSnapshot {
    let effective = &game_text[..game_text
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(game_text.len())];
    let Some(marker) = effective
        .windows(b"[PlayerFiles]".len())
        .position(|window| window == b"[PlayerFiles]")
    else {
        return PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: Vec::new(),
        };
    };

    let mut position = marker + b"[PlayerFiles]".len();
    let mut players = Vec::new();
    loop {
        while effective
            .get(position)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            position += 1;
        }
        if position >= effective.len() {
            break;
        }
        let line_end = effective[position..]
            .iter()
            .position(|byte| *byte == b'\r')
            .map(|offset| position + offset)
            .unwrap_or(effective.len());
        let line = &effective[position..line_end];
        position = line_end;
        let Some(equals) = line.iter().position(|byte| *byte == b'=') else {
            break;
        };
        let key = &line[..equals];
        if !key.starts_with(b"Player") {
            break;
        }
        let filename = &line[equals + 1..];
        let relative = legacy_group_path(filename);
        let player = group
            .open_child(&relative)
            .ok()
            .and_then(|player_group| PlayerFile::load(&player_group).ok());
        let Some(player) = player else {
            tracing::warn!(
                filename = %String::from_utf8_lossy(filename),
                "skipping unreadable old-style savegame player"
            );
            continue;
        };

        let section = [b"[".as_slice(), key, b"]".as_slice()].concat();
        let game_number = effective
            .windows(section.len())
            .position(|window| window == section)
            .and_then(|section_position| {
                let tail = &effective[section_position + section.len()..];
                tail.windows(b"Index=".len())
                    .position(|window| window == b"Index=")
                    .and_then(|index_position| {
                        parse_leading_legacy_i32(&tail[index_position + b"Index=".len()..])
                    })
            })
            .unwrap_or(-1);
        let color = player.normalized_preferred_color();
        let full_path = scenario_path.join(&relative);
        let id = i32::try_from(players.len())
            .unwrap_or(i32::MAX)
            .saturating_add(1);
        players.push(ControlPlayerInfoEntry {
            name: legacy_c4_string(&player.name),
            filename: LegacyCString::from_bytes(clonk_resources::path_to_legacy_bytes(&full_path))
                .unwrap_or_default(),
            flags: PLAYER_INFO_FLAG_JOINED,
            id,
            color,
            original_color: color,
            game_number,
            game_join_frame: 0,
            ..ControlPlayerInfoEntry::default()
        });
    }

    let last_player_id = i32::try_from(players.len()).unwrap_or(i32::MAX);
    PlayerInfoListSnapshot {
        last_player_id,
        clients: (!players.is_empty())
            .then_some(ClientPlayerInfosSnapshot {
                client_id: -1,
                flags: 0,
                players,
            })
            .into_iter()
            .collect(),
    }
}

#[cfg(unix)]
fn legacy_group_path(bytes: &[u8]) -> PathBuf {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    PathBuf::from(OsString::from_vec(
        bytes
            .iter()
            .map(|byte| if *byte == b'\\' { b'/' } else { *byte })
            .collect(),
    ))
}

#[cfg(not(unix))]
fn legacy_group_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(bytes).replace('\\', "/"))
}

fn parse_leading_legacy_i32(bytes: &[u8]) -> Option<i32> {
    let bytes = bytes.trim_ascii_start();
    let length = bytes
        .iter()
        .enumerate()
        .take_while(|(index, byte)| {
            byte.is_ascii_digit() || (*index == 0 && matches!(**byte, b'+' | b'-'))
        })
        .count();
    (length != 0)
        .then(|| std::str::from_utf8(&bytes[..length]).ok()?.parse().ok())
        .flatten()
}

fn validate_ascii_text(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    if (!allow_empty && value.is_empty()) || !value.is_ascii() || value.as_bytes().contains(&0) {
        return Err(PrepareHostBootstrapError::UnsupportedText { field });
    }
    Ok(())
}

fn validate_native_text(
    field: &'static str,
    value: &[u8],
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    if (!allow_empty && value.is_empty()) || value.contains(&0) {
        return Err(PrepareHostBootstrapError::UnsupportedText { field });
    }
    Ok(())
}

fn validate_c4_text(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    let value = clonk_resources::encode_legacy_script_text(value)
        .ok_or(PrepareHostBootstrapError::UnsupportedText { field })?;
    validate_native_text(field, &value, allow_empty)
}

fn validate_final_client_name(
    field: &'static str,
    value: &str,
) -> Result<(), PrepareHostBootstrapError> {
    let value = clonk_resources::encode_legacy_script_text(value)
        .ok_or(PrepareHostBootstrapError::UnsupportedText { field })?;
    validate_native_text(field, &value, false)?;
    if value.len() > 30 {
        return Err(PrepareHostBootstrapError::UnsupportedText { field });
    }
    Ok(())
}

fn validate_c4_network_name(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    validate_network_name_bytes(field, &clonk_script::c4_string_bytes(value), allow_empty)
}

fn validate_network_name_bytes(
    field: &'static str,
    value: &[u8],
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    if value.is_empty() && allow_empty {
        return Ok(());
    }
    if value.is_empty()
        || value.contains(&0)
        || value.first().is_some_and(u8::is_ascii_whitespace)
        || value.last().is_some_and(u8::is_ascii_whitespace)
        || value.len() > 30
        || value.contains(&b'{')
        || value.contains(&b'<')
        || value.windows(2).any(|pair| pair == b"}}")
    {
        return Err(PrepareHostBootstrapError::UnsupportedText { field });
    }
    Ok(())
}

fn validate_scenario_group(group: &Group) -> Result<Option<Vec<u8>>, PrepareHostBootstrapError> {
    let scenario_core = read_direct_entry(group, "Scenario.txt")?
        .ok_or(PrepareHostBootstrapError::ScenarioCoreMissing)?;
    let scenario_core = decode_legacy_script_text(&scenario_core);
    scenario_head_flag(&scenario_core, "SaveGame")?;
    if scenario_head_flag(&scenario_core, "Replay")? {
        return Err(PrepareHostBootstrapError::ReplayUnsupported);
    }
    if scenario_head_flag(&scenario_core, "NetworkGame")? {
        return Err(PrepareHostBootstrapError::NetworkGameScenarioUnsupported);
    }

    let Some(game) = read_direct_entry(group, "Game.txt")? else {
        return Ok(None);
    };
    // C4Group::LoadEntryString rejects a zero-byte component. Any stored
    // nonzero byte count succeeds even when its C string is empty/whitespace.
    Ok((!game.is_empty()).then_some(game))
}

fn read_direct_entry(
    group: &Group,
    expected: &'static str,
) -> Result<Option<Vec<u8>>, PrepareHostBootstrapError> {
    let Some(path) = direct_entry_path(group, expected)? else {
        return Ok(None);
    };
    group
        .read_file(path)
        .map(Some)
        .map_err(|source| PrepareHostBootstrapError::ScenarioEntry {
            entry: expected,
            source,
        })
}

fn direct_entry_path(
    group: &Group,
    expected: &'static str,
) -> Result<Option<PathBuf>, PrepareHostBootstrapError> {
    group
        .entries()
        .map_err(|source| PrepareHostBootstrapError::ScenarioEntry {
            entry: expected,
            source,
        })
        .map(|entries| {
            entries
                .into_iter()
                .find(|entry| {
                    !entry.is_directory
                        && entry.relative_path.components().count() == 1
                        && entry.relative_path.file_name().is_some_and(|name| {
                            clonk_resources::path_to_legacy_bytes(Path::new(name))
                                .eq_ignore_ascii_case(expected.as_bytes())
                        })
                })
                .map(|entry| entry.relative_path)
        })
}

fn scenario_head_flag(
    scenario_core: &str,
    key: &'static str,
) -> Result<bool, PrepareHostBootstrapError> {
    let mut in_head = false;
    for raw_line in scenario_core.lines() {
        let line = raw_line.trim().trim_start_matches('\u{feff}');
        if line.starts_with('[') && line.ends_with(']') {
            in_head = line[1..line.len() - 1].trim().eq_ignore_ascii_case("Head");
            continue;
        }
        if !in_head {
            continue;
        }
        let Some((candidate, value)) = line.split_once('=') else {
            continue;
        };
        if !candidate.trim().eq_ignore_ascii_case(key) {
            continue;
        }
        let value = value.trim();
        if value.eq_ignore_ascii_case("true") {
            return Ok(true);
        }
        if value.eq_ignore_ascii_case("false") {
            return Ok(false);
        }
        return value.parse::<i32>().map(|value| value != 0).map_err(|_| {
            PrepareHostBootstrapError::InvalidScenarioFlag {
                key,
                value: value.to_owned(),
            }
        });
    }
    Ok(false)
}

fn network_names(
    scenario_path: &Path,
    install_roots: &[PathBuf],
    network_work_path: &str,
) -> Result<(String, LegacyCString, String, LegacyCString), PrepareHostBootstrapError> {
    let relative = install_roots
        .iter()
        .find_map(|root| scenario_path.strip_prefix(root).ok())
        .ok_or_else(|| {
            PrepareHostBootstrapError::ScenarioOutsideInstallRoots(scenario_path.to_path_buf())
        })?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PrepareHostBootstrapError::InvalidScenarioWirePath(
            relative.to_path_buf(),
        ));
    }
    let origin = relative
        .to_str()
        .filter(|value| value.is_ascii() && !value.as_bytes().contains(&0))
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| {
            PrepareHostBootstrapError::InvalidScenarioWirePath(relative.to_path_buf())
        })?;
    let scenario_wire_name =
        LegacyCString::from_bytes(origin.as_bytes().to_vec()).ok_or_else(|| {
            PrepareHostBootstrapError::InvalidScenarioWirePath(relative.to_path_buf())
        })?;
    let basename = scenario_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| {
            name.is_ascii()
                && !name.as_bytes().contains(&0)
                && name.to_ascii_lowercase().ends_with(".c4s")
        })
        .ok_or_else(|| {
            PrepareHostBootstrapError::InvalidScenarioBasename(scenario_path.to_path_buf())
        })?;
    let dynamic_group_filename = format!("Dyn{basename}");
    let network_work_path = normalize_network_work_path(network_work_path)?;
    let dynamic_group_filename = format!("{network_work_path}{dynamic_group_filename}");
    let dynamic_wire_name = LegacyCString::from_bytes(dynamic_group_filename.as_bytes().to_vec())
        .expect("validated ASCII legacy paths are NUL-free");
    Ok((
        origin,
        scenario_wire_name,
        dynamic_group_filename,
        dynamic_wire_name,
    ))
}

fn normalize_network_work_path(value: &str) -> Result<String, PrepareHostBootstrapError> {
    if value.is_empty() || !value.is_ascii() || value.as_bytes().contains(&0) {
        return Err(PrepareHostBootstrapError::InvalidNetworkWorkPath(
            value.to_owned(),
        ));
    }
    let mut work_path = value.to_owned();
    if !work_path.ends_with(std::path::MAIN_SEPARATOR) {
        work_path.push(std::path::MAIN_SEPARATOR);
    }
    Ok(work_path)
}

fn legacy_string(value: &str) -> LegacyCString {
    LegacyCString::from_bytes(
        clonk_resources::encode_legacy_script_text(value)
            .expect("validated text is representable as Windows-1252"),
    )
    .expect("validated supported text is NUL-free")
}

fn legacy_c4_string(value: &str) -> LegacyCString {
    LegacyCString::from_bytes(clonk_script::c4_string_bytes(value))
        .expect("validated C4 text is NUL-free")
}

fn empty_team_snapshot() -> JoinTeamListSnapshot {
    JoinTeamListSnapshot {
        active: 0,
        custom: 0,
        allow_hostility_change: 0,
        allow_team_switch: 0,
        auto_generate_teams: 0,
        last_team_id: 0,
        team_distribution: 0,
        team_colors: 0,
        max_script_players: 0,
        script_player_names: LegacyCString::default(),
        random_team_count: 0,
        teams: Vec::new(),
    }
}

#[cfg(test)]
mod definition_root_graphics_tests {
    use super::*;
    use tempfile::tempdir;

    fn league_prepared_host(control_mode: i32) -> PreparedHostBootstrap {
        let mut host_config = HostConfig::default();
        host_config.initial_status.control_mode = control_mode;
        let parameters = &mut host_config
            .initial_join_snapshot
            .as_mut()
            .expect("default host JoinData")
            .parameters;
        parameters.random_seed = 77;
        parameters.max_players = 8;
        parameters.league_address = legacy_string("https://league.example/");
        PreparedHostBootstrap {
            host_config,
            initial_game: InitialNetworkGameData::default(),
            scenario_defaults: InitialNetworkScenarioDefaults::default(),
            has_initial_game: false,
            admission: PreparedHostAdmission {
                max_players: 8,
                no_runtime_join: false,
            },
            start_time: 1,
            initial_host_player_info_control: PlayerInfoControlData::default(),
            runtime_team_metadata: InitialNetworkTeamMetadata {
                active: false,
                custom: false,
                allow_hostility_change: true,
                allow_team_switch: false,
                auto_generate_teams: false,
                last_team_id: 0,
                team_distribution: clonk_engine::InitialNetworkTeamDistribution::Free,
                team_colors: false,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: Vec::new(),
            },
            scenario_wire_name: LegacyCString::default(),
            scenario_origin: String::new(),
            dynamic_filename_seed: String::new(),
            dynamic_wire_name: LegacyCString::default(),
            definition_modules: Vec::new(),
            definition_executable_path: String::new(),
            definition_path: String::new(),
            material_resource_groups: Vec::new(),
            reference_icon: 0,
            reference_comment: LegacyCString::default(),
            netpuncher_address: LegacyCString::default(),
            league: Some(PreparedLeagueHostConfig {
                endpoint: "https://league.example/".to_string(),
                transport: LeagueHttpTransportConfig::default(),
                update_period_secs: 120,
                league_server_signup: true,
            }),
            stream_address: legacy_string("old-stream"),
            local_player_resources: Vec::new(),
            local_player_alternate_colors_by_resource: HashMap::new(),
            pending_initial_league_players: None,
            lifetime: Arc::new(PreparedHostLifetime {
                temporary_files: Vec::new(),
                scenario: Mutex::new(None),
                host_launched: AtomicBool::new(false),
                initial_player_info_installed: AtomicBool::new(false),
            }),
        }
    }

    #[test]
    fn l085_lobby_runtime_join_choice_updates_retained_admission_policy() {
        let mut prepared = league_prepared_host(1);
        assert!(prepared.admission().runtime_join_allowed());

        prepared.set_runtime_join_allowed(false);
        assert!(!prepared.admission().runtime_join_allowed());

        prepared.set_runtime_join_allowed(true);
        assert!(prepared.admission().runtime_join_allowed());
    }

    #[test]

    fn league_start_applies_nonempty_overrides_and_forces_only_async_central() {
        let mut prepared = league_prepared_host(2);
        prepared
            .apply_league_start_response(&LeagueStartResponse {
                league: legacy_string("Gold League"),
                stream_to: legacy_string("https://stream.example/upload?"),
                seed: None,
                max_players: 0,
                ..LeagueStartResponse::default()
            })
            .expect("apply Start response");
        let parameters = &prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters;
        assert_eq!(parameters.league.as_bytes(), b"Gold League");
        assert_eq!(
            parameters.random_seed, 77,
            "absent Seed retains the old value"
        );
        assert_eq!(parameters.max_players, 8, "zero MaxPlayers is no override");
        assert_eq!(prepared.admission().max_players(), 8);
        assert_eq!(prepared.host_config().initial_status.control_mode, 1);
        assert_eq!(
            prepared.stream_address().as_bytes(),
            b"https://stream.example/upload?"
        );
    }

    #[test]
    fn l082_live_league_deinit_clears_identity_but_retains_start_overrides() {
        let mut prepared = league_prepared_host(1);
        prepared
            .apply_league_start_response(&LeagueStartResponse {
                league: legacy_string("Gold League"),
                stream_to: legacy_string("https://stream.example/upload?"),
                seed: Some(123),
                max_players: 4,
                ..LeagueStartResponse::default()
            })
            .expect("apply Start response");
        prepared
            .clear_live_league_registration()
            .expect("clear live league registration");

        let parameters = &prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters;
        assert!(parameters.league.is_empty());
        assert!(parameters.league_address.is_empty());
        assert_eq!(parameters.random_seed, 123);
        assert_eq!(parameters.max_players, 4);
        assert_eq!(
            prepared.stream_address().as_bytes(),
            b"https://stream.example/upload?"
        );
    }

    #[test]
    fn league_host_finalization_assigns_survivors_before_join_check_and_appends_scripts() {
        let mut prepared = league_prepared_host(1);
        let player = |name: &str, color| ControlPlayerInfoEntry {
            name: legacy_string(name),
            color,
            original_color: color,
            ..ControlPlayerInfoEntry::default()
        };
        let script = ControlPlayerInfoEntry {
            name: legacy_string("Script"),
            id: 40,
            player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
            color: 0x000a_0b0c,
            original_color: 0x000a_0b0c,
            ..ControlPlayerInfoEntry::default()
        };
        let a = player("A", 0x0001_0203);
        let b = player("B", 0x0004_0506);
        let c = player("C", 0x0007_0809);
        prepared.pending_initial_league_players = Some(PendingInitialLeaguePlayers {
            players: vec![a, b.clone(), c.clone()],
            alternate_colors_by_resource: HashMap::new(),
            restore_players: vec![script],
            restore_last_player_id: 40,
            team_metadata: prepared.runtime_team_metadata.clone(),
        });
        let mut checked = Vec::new();
        let mut oracle = ProcessInitialHostTeamAssignmentOracle::with_shipped_team_name();

        assert!(prepared
            .finalize_initial_league_players(vec![c, b], &mut oracle, |player| {
                checked.push((player.name.clone(), player.id));
                true
            })
            .expect("finalize authenticated initial players"));

        assert_eq!(
            prepared
                .initial_host_player_info_control()
                .players
                .iter()
                .map(|player| (player.name.as_bytes(), player.id, player.savegame_player))
                .collect::<Vec<_>>(),
            vec![
                (b"C".as_slice(), 41, 0),
                (b"B".as_slice(), 42, 0),
                (b"Script".as_slice(), 40, 40),
            ]
        );
        assert_eq!(
            checked
                .iter()
                .map(|(name, id)| (name.as_bytes(), *id))
                .collect::<Vec<_>>(),
            vec![(b"C".as_slice(), 41), (b"B".as_slice(), 42)]
        );

        let snapshot = prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .unwrap();
        assert_eq!(snapshot.parameters.player_infos.last_player_id, 42);
        assert_eq!(snapshot.parameters.player_infos.clients[0].players.len(), 3);
        assert!(prepared.pending_initial_league_players().is_none());
    }

    #[test]
    fn league_reordering_keeps_host_local_alternate_color_by_resource_identity() {
        let mut prepared = league_prepared_host(1);
        let player = |name: &str, resource_id: i32, color: u32, original_color: u32| {
            ControlPlayerInfoEntry {
                name: legacy_string(name),
                color,
                original_color,
                resource: Some(NetworkResourceCore {
                    id: resource_id,
                    ..NetworkResourceCore::default()
                }),
                ..ControlPlayerInfoEntry::default()
            }
        };
        let blocker = player("Blocker", 11, 0x00f4_0000, 0x00f4_0000);
        let candidate = player("Candidate", 22, 0x0000_c800, 0x00f4_0000);
        prepared.pending_initial_league_players = Some(PendingInitialLeaguePlayers {
            players: vec![blocker.clone(), candidate.clone()],
            alternate_colors_by_resource: HashMap::from([(11, 0), (22, 0x0000_00e8)]),
            restore_players: Vec::new(),
            restore_last_player_id: 0,
            team_metadata: prepared.runtime_team_metadata.clone(),
        });
        let mut oracle = ProcessInitialHostTeamAssignmentOracle::with_shipped_team_name();

        prepared
            .finalize_initial_league_players(vec![candidate, blocker], &mut oracle, |_| true)
            .expect("reordered authenticated players finalize");

        let players = &prepared.initial_host_player_info_control().players;
        assert_eq!(players[0].name.as_bytes(), b"Candidate");
        assert_eq!(
            players[0].resource.as_ref().map(|resource| resource.id),
            Some(22)
        );
        assert_eq!(players[0].color, 0x0000_00e8);
        assert_eq!(players[1].name.as_bytes(), b"Blocker");
        assert_eq!(players[1].color, 0x00f4_0000);
    }

    #[test]
    fn league_start_capacity_is_applied_before_initial_player_ids_are_assigned() {
        let mut prepared = league_prepared_host(1);
        let player = |name: &str, color| ControlPlayerInfoEntry {
            name: legacy_string(name),
            color,
            original_color: color,
            ..ControlPlayerInfoEntry::default()
        };
        let b = player("B", 0x0001_0203);
        let c = player("C", 0x0004_0506);
        prepared.pending_initial_league_players = Some(PendingInitialLeaguePlayers {
            players: vec![c.clone(), b.clone()],
            alternate_colors_by_resource: HashMap::new(),
            restore_players: Vec::new(),
            restore_last_player_id: 0,
            team_metadata: prepared.runtime_team_metadata.clone(),
        });
        prepared
            .apply_league_start_response(&LeagueStartResponse {
                league: legacy_string("Gold League"),
                max_players: 1,
                ..LeagueStartResponse::default()
            })
            .expect("apply Start capacity");
        let mut checked = Vec::new();
        let mut oracle = ProcessInitialHostTeamAssignmentOracle::with_shipped_team_name();
        prepared
            .finalize_initial_league_players(vec![c, b], &mut oracle, |player| {
                checked.push(player.name.clone());
                true
            })
            .expect("finalize capped initial players");

        let players = &prepared.initial_host_player_info_control().players;
        assert_eq!(players.len(), 1);
        assert_eq!(players[0].name.as_bytes(), b"C");
        assert_eq!(players[0].id, 1);
        assert_eq!(
            checked
                .iter()
                .map(LegacyCString::as_bytes)
                .collect::<Vec<_>>(),
            vec![b"C".as_slice()]
        );
        assert_eq!(
            prepared
                .host_config()
                .initial_join_snapshot
                .as_ref()
                .unwrap()
                .parameters
                .player_infos
                .last_player_id,
            1
        );
    }

    #[test]
    fn empty_start_league_clears_addresses_and_applies_zero_seed_and_capacity() {
        let mut prepared = league_prepared_host(2);
        prepared
            .apply_league_start_response(&LeagueStartResponse {
                league: LegacyCString::default(),
                stream_to: legacy_string("ignored-stream"),
                seed: Some(0),
                max_players: 4,
                ..LeagueStartResponse::default()
            })
            .expect("apply Start response");

        let parameters = &prepared
            .host_config()
            .initial_join_snapshot
            .as_ref()
            .expect("prepared JoinData")
            .parameters;
        assert!(parameters.league_address.is_empty());
        assert!(prepared.stream_address().is_empty());
        assert_eq!(parameters.random_seed, 0);
        assert_eq!(parameters.max_players, 4);
        assert_eq!(prepared.host_config().max_players, 4);
        assert_eq!(prepared.admission().max_players(), 4);
        assert_eq!(prepared.host_config().initial_status.control_mode, 2);
    }

    #[test]
    fn generated_team_name_formats_resource_percent_and_c4_name_limit() {
        let template =
            LegacyCString::from_bytes(b"Very long %% localized team %d suffix".to_vec()).unwrap();
        let formatted = format_generated_team_name(&template, 12);
        assert_eq!(formatted.as_bytes().len(), 30);
        assert_eq!(formatted.as_bytes(), b"Very long % localized team 12 ");
    }

    #[test]
    fn definition_pack_graphics_precede_prepared_host_base_graphics() {
        let dir = tempdir().expect("prepared host graphics fixture");
        let scenario = dir.path().join("Scenario.c4s");
        let definition = dir.path().join("Objects.c4d");
        let definition_graphics = definition.join("Graphics.c4g");
        let base_graphics = dir.path().join("Graphics.c4g");
        fs::create_dir_all(&scenario).expect("scenario group");
        fs::create_dir_all(&definition_graphics).expect("definition graphics");
        fs::create_dir_all(&base_graphics).expect("base graphics");

        let roots = [dir.path().to_path_buf()];
        let language_packs = LanguagePacks::default();
        let resolver = InstallRootDefinitionResolver::new(&roots, &language_packs, &[], &[], false);
        let graphics = resolver
            .resolve_graphics_groups_with_definition_roots(
                &Group::open(&scenario).expect("scenario root"),
                &[Group::open(&definition).expect("definition root")],
            )
            .expect("prepared host graphics chain");

        assert_eq!(
            graphics
                .iter()
                .map(|group| group.root().to_path_buf())
                .collect::<Vec<_>>(),
            [definition_graphics, base_graphics]
        );
    }
}
