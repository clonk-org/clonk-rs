//! Pure preparation of the supported C++ initial-network-host state.
//!
//! This stops before opening any listener or advertising the game. C++ keeps
//! `fAllowJoin` false throughout `C4Network2::InitHost`; the app may only open
//! admission after control and the initial local player packet are ready
//! (`src/C4Network2.cpp:222-278`; `src/C4Game.cpp:3847-3876`).

use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use lc_engine::player_file::PlayerFile;
use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, InitialNetworkGameData, LegacyCString,
    NetworkResourceCore, PlayerInfoControlData, PlayerInfoUpdateRequest, Scenario, ScenarioError,
    CLIENT_PLAYER_INFO_FLAG_INITIAL, PLAYER_INFO_FLAG_HAS_RESOURCE,
};
use lc_network::{
    compose_initial_network_dynamic, fill_scenario_derived_join_parameters,
    publish_host_initial_resources, ClientPlayerInfosSnapshot, HostConfig, HostGameReference,
    HostGameReferenceError, HostGameReferenceMetadata, HostInitialResourcePublicationError,
    HostInitialResourcePublicationSpec, HostInitialResourceSource, InitialNetworkDynamicError,
    InitialNetworkDynamicSpec, InitialNetworkMetadataError, JoinClientRegistrySnapshot,
    JoinGameParametersEnvelope, JoinTeamListSnapshot, NetworkAddress, NetworkGameReference,
    NetworkProtocol, NetworkStatus, PlayerInfoListSnapshot, ResourceFileOwnership,
    CURRENT_GAME_BUILD, CURRENT_GAME_VERSION, NETWORK_STATE_GO, NETWORK_STATE_INIT,
    NETWORK_STATE_LOBBY, NETWORK_STATE_NONE, NETWORK_STATE_PAUSE,
};
use lc_resources::{Group, GroupError};
use thiserror::Error;

use crate::host_game_resource_sources::{
    resolve_host_game_resource_sources, HostGameResourceSourceError,
};

/// Configuration values C++ reads while loading parameters and initializing
/// its network status. Values unrelated to this supported initial-host subset
/// remain fixed at their stock defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PreparedHostBootstrapConfig {
    pub control_mode: i32,
    pub control_rate: i32,
    pub fair_crew: bool,
    pub fair_crew_strength: i32,
    pub auto_frame_skip: bool,
    pub max_load_file_size: u32,
    pub no_runtime_join: bool,
}

/// Every process-global input used by the supported preparation path.
#[derive(Debug, Clone, Copy)]
pub struct PreparedHostBootstrapSpec<'a> {
    pub scenario_path: &'a Path,
    pub scenario_title: &'a str,
    /// Ordered assembled install roots. Earlier roots shadow later roots.
    pub install_roots: &'a [PathBuf],
    /// Ordered legacy language fallbacks used while loading scenario-owned
    /// definitions and Teams.txt from `scenario_path`.
    pub languages: &'a [String],
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
    /// Already-selected `Config.Network.LocalName` input.
    pub host_name: &'a str,
    /// Already-selected `Config.Network.Nick`; empty falls back to `host_name`.
    pub host_nick: &'a str,
    /// `Config.Network.Comment`, copied verbatim into the game reference.
    pub network_comment: &'a str,
    /// `Config.Network.PuncherAddress`, present even before a puncher ID exists.
    pub netpuncher_address: &'a str,
    /// Selected participant files in C++ module order, with their exact
    /// `Config.AtExeRelativePath` wire spellings.
    pub player_sources: &'a [HostInitialResourceSource],
    pub config: PreparedHostBootstrapConfig,
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
    admission: PreparedHostAdmission,
    start_time: i32,
    initial_host_player_info_control: PlayerInfoControlData,
    scenario_wire_name: LegacyCString,
    scenario_origin: String,
    dynamic_wire_name: LegacyCString,
    reference_icon: i32,
    reference_comment: LegacyCString,
    netpuncher_address: LegacyCString,
    local_player_resources: Vec<(NetworkResourceCore, PathBuf)>,
    lifetime: Arc<PreparedHostLifetime>,
}

impl PreparedHostBootstrap {
    pub fn host_config(&self) -> &HostConfig {
        &self.host_config
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

    pub fn admission(&self) -> PreparedHostAdmission {
        self.admission
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
        let summary = NetworkGameReference {
            title: parameters.title.to_string_lossy().into_owned(),
            host_name: self
                .host_config
                .local_core
                .name
                .to_string_lossy()
                .into_owned(),
            host_nick: self
                .host_config
                .local_core
                .nick
                .to_string_lossy()
                .into_owned(),
            state: state.to_string(),
            control_mode: self.host_config.initial_status.control_mode,
            start_time: i64::from(self.start_time),
            join_allowed,
            password_needed: !self.host_config.password.is_empty(),
            official_server: false,
            max_players: parameters.max_players,
            game: "LegacyClonk".to_string(),
            version: CURRENT_GAME_VERSION,
            build: CURRENT_GAME_BUILD,
            tcp_addresses: addresses
                .iter()
                .filter(|address| address.protocol == NetworkProtocol::Tcp)
                .map(|address| address.endpoint)
                .collect(),
        };
        let metadata = HostGameReferenceMetadata {
            icon: self.reference_icon,
            time: 0,
            frame: 0,
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

    /// Executes the local half of the host's direct Initial PlayerInfo and
    /// returns the only capability which can open lobby admission.
    pub fn install_initial_host_player_state(
        &self,
        registry: &mut lc_engine::ControlPlayerInfoRegistry,
        mut install_resource: impl FnMut(&NetworkResourceCore, &Path),
    ) -> Result<PreparedHostAdmissionReady, PreparedHostUseError> {
        self.lifetime
            .initial_player_info_installed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PreparedHostUseError::InitialPlayerInfoAlreadyInstalled)?;
        for (core, path) in &self.local_player_resources {
            install_resource(core, path);
        }
        let last_player_id = self
            .initial_host_player_info_control
            .players
            .iter()
            .map(|player| player.id)
            .max()
            .unwrap_or(0);
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
}

#[derive(Debug, Error)]
pub enum PrepareHostBootstrapError {
    #[error("the exact initial-host subset supports at most one local player ({count} selected)")]
    MultipleLocalPlayerFilesUnsupported { count: usize },
    #[error("local player startup with active scenario teams is not supported yet")]
    LocalPlayerTeamsUnsupported,
    #[error("the selected local player did not produce a published player resource")]
    LocalPlayerPublicationMissing,
    #[error("the selected local player could not be admitted into the scenario player slots")]
    LocalPlayerAdmissionRejected,
    #[error("scenario Parameters.txt is nonempty and cannot be applied exactly yet")]
    ScenarioParametersUnsupported,
    #[error("scenario Game.txt has non-player runtime data that cannot be applied exactly yet")]
    ScenarioGameStateUnsupported,
    #[error("scenario PlayerInfos.txt/replay player state is not supported")]
    ScenarioPlayerInfosUnsupported,
    #[error("scenario SavePlayerInfos.txt restore state is not supported")]
    RestorePlayerInfosUnsupported,
    #[error("savegame network hosting is not supported by the exact initial-host subset")]
    SavegameUnsupported,
    #[error("replay network hosting is rejected by C++")]
    ReplayUnsupported,
    #[error("a scenario already marked NetworkGame cannot be direct-started as a host")]
    NetworkGameScenarioUnsupported,
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
    #[error("resolved definition resource {index} has no UTF-8 Scenario.txt spelling")]
    DefinitionWireNameEncoding { index: usize },
    #[error("{field} is outside the exact ASCII input subset")]
    UnsupportedText { field: &'static str },
    #[error("{field} Unix time {value} does not fit the C++ signed 32-bit field")]
    UnixSecondsOutOfRange { field: &'static str, value: i64 },
    #[error("scenario max-player value {0} cannot be represented by HostConfig")]
    MaxPlayersOutOfRange(i32),
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

/// Builds the exact currently-supported initial host state without opening a
/// socket, registering with a masterserver, or making the game joinable.
pub fn prepare_host_bootstrap(
    spec: PreparedHostBootstrapSpec<'_>,
) -> Result<PreparedHostBootstrap, PrepareHostBootstrapError> {
    validate_inputs(&spec)?;
    let scenario_group = Group::open(spec.scenario_path).map_err(|source| {
        PrepareHostBootstrapError::ScenarioGroup {
            path: spec.scenario_path.to_path_buf(),
            source,
        }
    })?;
    let original_game_text = validate_scenario_group(&scenario_group)?;
    let scenario = Scenario::load_from_group_with_languages(
        &scenario_group,
        &InstallRootDefinitionResolver {
            roots: spec.install_roots,
        },
        spec.languages,
    )?;

    let (scenario_origin, scenario_wire_name, dynamic_group_filename, dynamic_wire_name) =
        network_names(
            spec.scenario_path,
            spec.install_roots,
            spec.network_work_path,
        )?;
    let scenario_metadata = scenario.initial_network_scenario_metadata()?;
    let team_metadata = scenario.initial_network_team_metadata()?;
    if !spec.player_sources.is_empty() && team_metadata.active {
        return Err(PrepareHostBootstrapError::LocalPlayerTeamsUnsupported);
    }
    let local_player_file = spec
        .player_sources
        .first()
        .map(|source| {
            let player = PlayerFile::load_from_path(&source.path)?;
            validate_network_name("local player name", &player.name, false)?;
            Ok::<_, PrepareHostBootstrapError>(player)
        })
        .transpose()?;
    let resource_sources = resolve_host_game_resource_sources(
        spec.scenario_path,
        spec.install_roots,
        &scenario_metadata,
    )?;
    // SaveCore writes Game.DefinitionFilenames, not the unmodified scenario
    // module list. OpenScenario appends every folder-local definitions group
    // before this save (src/C4Game.cpp:179-213; C4GameSave.cpp:89-92).
    let definition_modules = resource_sources
        .definitions
        .iter()
        .enumerate()
        .map(|(index, source)| {
            std::str::from_utf8(source.wire_name.as_bytes())
                .map(str::to_owned)
                .map_err(|_| PrepareHostBootstrapError::DefinitionWireNameEncoding { index })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let host_name = legacy_string(spec.host_name);
    let host_nick = if spec.host_nick.is_empty() {
        host_name.clone()
    } else {
        legacy_string(spec.host_nick)
    };
    let local_core = ClientCoreControlData {
        client_id: 0,
        activated: true,
        observer: false,
        name: host_name,
        nick: host_nick,
        lobby_ready: false,
    };
    let empty_players = PlayerInfoListSnapshot {
        last_player_id: 0,
        clients: Vec::new(),
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
        league_address: LegacyCString::default(),
        title: legacy_string(spec.scenario_title),
        scenario: NetworkResourceCore::default(),
        game_resources: Vec::new(),
        player_infos: initial_host_players,
        restore_player_infos: empty_players,
        teams: empty_team_snapshot(),
        clients: JoinClientRegistrySnapshot {
            clients: vec![local_core.clone()],
            local_client_id: Some(0),
        },
    };
    let scenario_defaults =
        fill_scenario_derived_join_parameters(&mut parameters, &scenario_metadata, team_metadata)?;
    let max_players = usize::try_from(parameters.max_players)
        .map_err(|_| PrepareHostBootstrapError::MaxPlayersOutOfRange(parameters.max_players))?;

    // Before InitGame, C++ runtime data is pristine apart from the stock
    // `speed` message-board command installed by InitSystem. Any non-player
    // Game.txt state was rejected above rather than silently discarded.
    let game = InitialNetworkGameData::default();
    let dynamic = compose_initial_network_dynamic(InitialNetworkDynamicSpec {
        group_filename: &dynamic_group_filename,
        maker: spec.group_maker,
        scenario: &scenario,
        scenario_title: spec.scenario_title,
        definition_modules: &definition_modules,
        scenario_origin: &scenario_origin,
        game: &game,
        original_game_text: original_game_text.as_deref(),
        parameters: &parameters,
        scenario_defaults: &scenario_defaults,
    })?;

    let mut publication = publish_host_initial_resources(HostInitialResourcePublicationSpec {
        network_directory: spec.network_directory.to_path_buf(),
        group_maker: spec.group_maker.to_owned(),
        max_load_file_size: spec.config.max_load_file_size,
        scenario: HostInitialResourceSource {
            path: spec.scenario_path.to_path_buf(),
            wire_name: scenario_wire_name.clone(),
        },
        definitions: resource_sources.definitions,
        system: resource_sources.system,
        materials: resource_sources.materials,
        players: spec.player_sources.to_vec(),
        dynamic,
        dynamic_wire_name: dynamic_wire_name.clone(),
        parameters,
        dynamic_tick: 0,
    })?;
    let temporary_files = publication
        .resource_files
        .iter()
        .filter(|resource| resource.ownership == ResourceFileOwnership::Temporary)
        .map(|resource| resource.path.clone())
        .collect();
    let temporary_files = PreparedTemporaryFiles::new(temporary_files);
    let requested_player_count = usize::from(local_player_file.is_some());
    let initial_players = match (
        local_player_file.as_ref(),
        spec.player_sources.first(),
        publication.player_cores.first(),
    ) {
        (Some(player), Some(source), Some(core)) => {
            let color = player.normalized_preferred_color();
            vec![ControlPlayerInfoEntry {
                name: legacy_string(&player.name),
                filename: source.wire_name.clone(),
                flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
                color,
                original_color: color,
                resource: Some(core.clone()),
                ..ControlPlayerInfoEntry::default()
            }]
        }
        (None, None, None) => Vec::new(),
        _ => return Err(PrepareHostBootstrapError::LocalPlayerPublicationMissing),
    };
    let mut player_allocator = lc_engine::ControlPlayerInfoRegistry::default();
    let initial_host_player_info_control = player_allocator
        .admit_request(
            PlayerInfoUpdateRequest {
                client_id: 0,
                flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: initial_players,
            },
            max_players,
        )
        .ok_or(PrepareHostBootstrapError::LocalPlayerAdmissionRejected)?;
    if initial_host_player_info_control.players.len() != requested_player_count {
        return Err(PrepareHostBootstrapError::LocalPlayerAdmissionRejected);
    }
    let last_player_id = initial_host_player_info_control
        .players
        .iter()
        .map(|player| player.id)
        .max()
        .unwrap_or(0);
    publication.join_snapshot.parameters.player_infos = PlayerInfoListSnapshot {
        last_player_id,
        clients: vec![ClientPlayerInfosSnapshot {
            client_id: initial_host_player_info_control.client_id,
            flags: initial_host_player_info_control.flags,
            players: initial_host_player_info_control.players.clone(),
        }],
    };
    let local_player_resources = spec
        .player_sources
        .iter()
        .zip(&publication.player_cores)
        .map(|(source, core)| (core.clone(), source.path.clone()))
        .collect();
    let resolved_dynamic_wire_name = publication.join_snapshot.dynamic.filename.clone();
    let mut host_config = HostConfig {
        max_players,
        start_tick: 0,
        local_core,
        initial_status: NetworkStatus {
            state: NETWORK_STATE_LOBBY,
            control_mode: spec.config.control_mode,
            target_tick: 0,
        },
        password: LegacyCString::default(),
        allow_join: false,
        ..HostConfig::default()
    };
    publication.apply_to(&mut host_config);
    let temporary_files = temporary_files.into_lifetime_paths();

    Ok(PreparedHostBootstrap {
        host_config,
        admission: PreparedHostAdmission {
            max_players: scenario_metadata.max_players,
            no_runtime_join: spec.config.no_runtime_join,
        },
        start_time: spec.start_unix_seconds as i32,
        initial_host_player_info_control,
        scenario_wire_name,
        scenario_origin,
        dynamic_wire_name: resolved_dynamic_wire_name,
        reference_icon: scenario_metadata.icon,
        reference_comment: legacy_string(spec.network_comment),
        netpuncher_address: legacy_string(spec.netpuncher_address),
        local_player_resources,
        lifetime: Arc::new(PreparedHostLifetime {
            temporary_files,
            host_launched: AtomicBool::new(false),
            initial_player_info_installed: AtomicBool::new(false),
        }),
    })
}

struct InstallRootDefinitionResolver<'a> {
    roots: &'a [PathBuf],
}

impl LegacyDefinitionResolver for InstallRootDefinitionResolver<'_> {
    fn resolve_definition_groups(
        &self,
        _scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let normalized = identifier.replace('\\', "/");
        let relative = Path::new(&normalized);
        for root in self.roots {
            let candidate = root.join(relative);
            match Group::open(&candidate) {
                Ok(group) => return Ok(vec![group]),
                Err(GroupError::Missing(_)) => {}
                Err(source) => return Err(ScenarioError::Resources(source)),
            }
        }
        Err(ScenarioError::LegacyDefinitionNotFound {
            path: identifier.to_owned(),
        })
    }
}

fn validate_inputs(spec: &PreparedHostBootstrapSpec<'_>) -> Result<(), PrepareHostBootstrapError> {
    if spec.player_sources.len() > 1 {
        return Err(
            PrepareHostBootstrapError::MultipleLocalPlayerFilesUnsupported {
                count: spec.player_sources.len(),
            },
        );
    }
    for source in spec.player_sources {
        let wire_name = std::str::from_utf8(source.wire_name.as_bytes()).map_err(|_| {
            PrepareHostBootstrapError::UnsupportedText {
                field: "local player filename",
            }
        })?;
        validate_ascii_text("local player filename", wire_name, false)?;
        if wire_name.contains(';') {
            return Err(PrepareHostBootstrapError::UnsupportedText {
                field: "local player filename",
            });
        }
    }
    validate_scenario_title(spec.scenario_title)?;
    validate_ascii_text("C4Group maker", spec.group_maker, true)?;
    validate_network_name("host network name", spec.host_name, false)?;
    validate_network_name("host network nick", spec.host_nick, true)?;
    validate_ascii_text("network comment", spec.network_comment, true)?;
    validate_ascii_text("netpuncher address", spec.netpuncher_address, true)?;
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

fn validate_scenario_title(value: &str) -> Result<(), PrepareHostBootstrapError> {
    validate_ascii_text("scenario title", value, false)?;
    if value.len() > 120
        || value.trim_matches(|character: char| character.is_ascii_whitespace()) != value
        || value
            .bytes()
            .any(|byte| byte != b' ' && !byte.is_ascii_graphic())
    {
        return Err(PrepareHostBootstrapError::UnsupportedText {
            field: "scenario title",
        });
    }
    Ok(())
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

fn validate_network_name(
    field: &'static str,
    value: &str,
    allow_empty: bool,
) -> Result<(), PrepareHostBootstrapError> {
    validate_ascii_text(field, value, allow_empty)?;
    if value.is_empty() && allow_empty {
        return Ok(());
    }
    let trimmed = value.trim_matches(|character: char| character.is_ascii_whitespace());
    if trimmed != value
        || trimmed.is_empty()
        || trimmed.len() > 30
        || trimmed.contains('{')
        || trimmed.contains('<')
        || trimmed.contains("}}")
    {
        return Err(PrepareHostBootstrapError::UnsupportedText { field });
    }
    Ok(())
}

fn validate_scenario_group(group: &Group) -> Result<Option<Vec<u8>>, PrepareHostBootstrapError> {
    if let Some(parameters) = read_direct_entry(group, "Parameters.txt")? {
        if !parameters.is_empty() {
            return Err(PrepareHostBootstrapError::ScenarioParametersUnsupported);
        }
    }
    if has_direct_entry(group, "SavePlayerInfos.txt")? {
        return Err(PrepareHostBootstrapError::RestorePlayerInfosUnsupported);
    }
    if has_direct_entry(group, "PlayerInfos.txt")? || has_direct_entry(group, "RecPlayerInfos.txt")?
    {
        return Err(PrepareHostBootstrapError::ScenarioPlayerInfosUnsupported);
    }

    let scenario_core = read_direct_entry(group, "Scenario.txt")?
        .ok_or(PrepareHostBootstrapError::ScenarioCoreMissing)?;
    let scenario_core = std::str::from_utf8(&scenario_core)
        .map_err(|_| PrepareHostBootstrapError::ScenarioCoreEncoding)?;
    if scenario_head_flag(scenario_core, "SaveGame")? {
        return Err(PrepareHostBootstrapError::SavegameUnsupported);
    }
    if scenario_head_flag(scenario_core, "Replay")? {
        return Err(PrepareHostBootstrapError::ReplayUnsupported);
    }
    if scenario_head_flag(scenario_core, "NetworkGame")? {
        return Err(PrepareHostBootstrapError::NetworkGameScenarioUnsupported);
    }

    let Some(game) = read_direct_entry(group, "Game.txt")? else {
        return Ok(None);
    };
    let effective = &game[..game
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(game.len())];
    let marker = effective
        .windows(b"[Player".len())
        .position(|window| window == b"[Player");
    let non_player_prefix = marker.map_or(effective, |position| &effective[..position]);
    if non_player_prefix
        .iter()
        .any(|byte| !byte.is_ascii_whitespace())
    {
        return Err(PrepareHostBootstrapError::ScenarioGameStateUnsupported);
    }
    if marker.is_some_and(|position| has_non_player_section(&effective[position..])) {
        return Err(PrepareHostBootstrapError::ScenarioGameStateUnsupported);
    }
    Ok(marker.map(|_| game))
}

fn has_non_player_section(player_tail: &[u8]) -> bool {
    player_tail
        .split(|byte| matches!(*byte, b'\n' | b'\r'))
        .any(|raw_line| {
            let line = raw_line.trim_ascii();
            let Some(section) = line.strip_prefix(b"[") else {
                return false;
            };
            let Some(close) = section.iter().position(|byte| *byte == b']') else {
                return false;
            };
            !section[..close].starts_with(b"Player")
        })
}

fn has_direct_entry(
    group: &Group,
    expected: &'static str,
) -> Result<bool, PrepareHostBootstrapError> {
    Ok(direct_entry_path(group, expected)?.is_some())
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
                            name.as_encoded_bytes()
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
    let dynamic_group_filename = format!("{network_work_path}/{dynamic_group_filename}");
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
    let normalized = value.trim_end_matches(['/', '\\']).replace('\\', "/");
    let path = Path::new(&normalized);
    if normalized.is_empty()
        || !normalized.is_ascii()
        || normalized.as_bytes().contains(&0)
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(PrepareHostBootstrapError::InvalidNetworkWorkPath(
            value.to_owned(),
        ));
    }
    Ok(normalized)
}

fn legacy_string(value: &str) -> LegacyCString {
    LegacyCString::from_bytes(value.as_bytes().to_vec())
        .expect("validated supported text is NUL-free")
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
