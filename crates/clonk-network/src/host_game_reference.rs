//! Exact host-side `C4GameParameters` reference serialization.
//!
//! Search results intentionally remain a compact display projection. This
//! module couples that projection to the complete synchronized parameters so
//! a host cannot advertise the former while silently dropping the latter.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::{SocketAddr, SocketAddrV6};

use clonk_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE, PLAYER_INFO_FLAG_JOINED,
    PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
};
use thiserror::Error;

use crate::{
    ClientPlayerInfosSnapshot, JoinGameParametersEnvelope, JoinTeamListSnapshot, JoinTeamSnapshot,
    NetpuncherGameIds, NetworkAddress, NetworkGameReference, NetworkProtocol,
    PlayerInfoListSnapshot,
};

const MAX_REFERENCE_LIST_ENTRIES: usize = 5_000;
const PLAYER_INFO_SYNC_FLAGS: u16 = 0x7fcd;

/// A search/display projection coupled to the complete game parameters copied
/// by `C4Network2Reference::InitLocal`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostGameReference {
    summary: NetworkGameReference,
    metadata: HostGameReferenceMetadata,
    parameters: JoinGameParametersEnvelope,
}

/// Top-level `C4Network2Reference` values that are not part of
/// `C4GameParameters` or the search/display projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostGameReferenceMetadata {
    pub icon: i32,
    pub time: i32,
    pub frame: i32,
    pub league_performance: i32,
    pub comment: LegacyCString,
    pub addresses: Vec<NetworkAddress>,
    pub netpuncher_ipv4: u32,
    pub netpuncher_ipv6: u32,
    pub netpuncher_address: LegacyCString,
}

impl HostGameReference {
    pub fn new(
        summary: NetworkGameReference,
        metadata: HostGameReferenceMetadata,
        mut parameters: JoinGameParametersEnvelope,
    ) -> Result<Self, HostGameReferenceError> {
        discard_live_player_resources(&mut parameters.player_infos);
        let reference = Self {
            summary,
            metadata,
            parameters,
        };
        reference.validate()?;
        Ok(reference)
    }

    pub fn summary(&self) -> &NetworkGameReference {
        &self.summary
    }

    pub fn parameters(&self) -> &JoinGameParametersEnvelope {
        &self.parameters
    }

    pub fn metadata(&self) -> &HostGameReferenceMetadata {
        &self.metadata
    }

    /// Rebuilds the two reference fields changed by the live lobby password
    /// and comment controls.
    ///
    /// Neither value belongs to [`JoinGameParametersEnvelope`]: password
    /// presence is part of the search summary, while the comment is top-level
    /// `C4Network2Reference` metadata. Rebuilding them together keeps the
    /// published reference atomic and runs the complete reference validation
    /// without disturbing the synchronized game parameters.
    pub fn replacing_lobby_options(
        &self,
        password_needed: bool,
        comment: LegacyCString,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        summary.password_needed = password_needed;
        summary.comment = clonk_resources::decode_legacy_script_text(comment.as_bytes());
        let mut metadata = self.metadata.clone();
        metadata.comment = comment;
        Self::new(summary, metadata, self.parameters.clone())
    }

    /// Rebuild the exact reference after a live C4GameParameters mutation.
    /// The display projection duplicates title and MaxPlayers and therefore
    /// must advance atomically with the serialized parameters.
    pub fn replacing_parameters(
        &self,
        parameters: JoinGameParametersEnvelope,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        refresh_parameter_summary(&mut summary, &parameters);
        Self::new(summary, self.metadata.clone(), parameters)
    }

    /// Rebuilds the reference after a control-mode policy change.
    ///
    /// `C4Network2Reference::InitLocal` copies the current control mode into
    /// its independent display projection. Keeping this mutation explicit
    /// prevents a later parameter/runtime replacement from advertising the
    /// stale pre-league mode.
    pub fn replacing_control_mode(
        &self,
        control_mode: i32,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        summary.control_mode = control_mode;
        Self::new(summary, self.metadata.clone(), self.parameters.clone())
    }

    /// Rebuilds the socket-facing fields invalidated after the netpuncher
    /// assigns an ID. C++ owns one address container and one ID pair; update
    /// both Rust projections atomically so serialization cannot expose a
    /// stale or internally inconsistent reference.
    pub fn replacing_netpuncher_state(
        &self,
        game_ids: NetpuncherGameIds,
        addresses: Vec<NetworkAddress>,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        summary.addresses = addresses.clone();
        summary.tcp_addresses = addresses
            .iter()
            .filter_map(|address| {
                (address.protocol == NetworkProtocol::Tcp).then_some(address.endpoint)
            })
            .collect();
        summary.netpuncher_ipv4 = game_ids.ipv4;
        summary.netpuncher_ipv6 = game_ids.ipv6;

        let mut metadata = self.metadata.clone();
        metadata.addresses = addresses;
        metadata.netpuncher_ipv4 = game_ids.ipv4;
        metadata.netpuncher_ipv6 = game_ids.ipv6;
        Self::new(summary, metadata, self.parameters.clone())
    }

    /// Rebuild the fields refreshed by `C4Network2Reference::InitLocal`
    /// while a game is live. Per-player league performance remains untouched
    /// until the game-over branch below.
    pub fn replacing_runtime(
        &self,
        parameters: JoinGameParametersEnvelope,
        state: impl Into<String>,
        time: i32,
        frame: i32,
        join_allowed: bool,
        league_performance: i32,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        summary.state = state.into();
        summary.join_allowed = join_allowed;
        summary.time = time;
        refresh_parameter_summary(&mut summary, &parameters);
        let mut metadata = self.metadata.clone();
        metadata.time = time;
        metadata.frame = frame;
        metadata.league_performance = league_performance;
        Self::new(summary, metadata, parameters)
    }

    /// Rebuild `C4Network2Reference::InitLocal`'s game-over projection.
    /// C++ copies live parameters, then overlays the independent global
    /// performance and one performance value per retained PlayerInfo ID.
    // Keep the C++ projection inputs explicit so callers cannot accidentally
    // reuse one of the independently refreshed reference fields.
    #[allow(clippy::too_many_arguments)]
    pub fn replacing_game_over<I>(
        &self,
        parameters: JoinGameParametersEnvelope,
        state: impl Into<String>,
        time: i32,
        frame: i32,
        join_allowed: bool,
        league_performance: i32,
        player_league_performance: I,
    ) -> Result<Self, HostGameReferenceError>
    where
        I: IntoIterator<Item = (i32, i32)>,
    {
        let performance = player_league_performance
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let mut parameters = parameters;
        for player in parameters
            .player_infos
            .clients
            .iter_mut()
            .flat_map(|client| client.players.iter_mut())
        {
            player.league_performance = performance.get(&player.id).copied().unwrap_or(0);
        }
        self.replacing_runtime(
            parameters,
            state,
            time,
            frame,
            join_allowed,
            league_performance,
        )
    }

    pub(crate) fn validate(&self) -> Result<(), HostGameReferenceError> {
        validate_metadata(&self.metadata)?;
        validate_summary(&self.summary, &self.metadata, &self.parameters)?;
        validate_parameters(&self.parameters)
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum HostGameReferenceError {
    #[error("reference state `{0}` is not a C4Network2Status state")]
    InvalidState(String),
    #[error("reference start time {0} does not fit the C++ signed 32-bit field")]
    StartTimeOutOfRange(i64),
    #[error("reference MaxPlayers {reference} differs from parameters value {parameters}")]
    MaxPlayersMismatch { reference: i32, parameters: i32 },
    #[error("reference title differs from the byte-preserving parameters title")]
    TitleMismatch,
    #[error("reference icon differs from the top-level reference metadata")]
    IconMetadataMismatch,
    #[error("reference time differs from the top-level reference metadata")]
    TimeMetadataMismatch,
    #[error("reference comment differs from the byte-preserving top-level metadata")]
    CommentMetadataMismatch,
    #[error("reference UseFairCrew differs from the parameters value")]
    UseFairCrewMismatch,
    #[error("reference goals differ from the parameters goal list")]
    GoalsMismatch,
    #[error("reference league differs from the byte-preserving parameters league")]
    LeagueMismatch,
    #[error("reference league address differs from the byte-preserving parameters value")]
    LeagueAddressMismatch,
    #[error("reference player names differ from the active parameters player projection")]
    PlayerNamesMismatch,
    #[error("parameters contain no unique host client with ID 0")]
    HostClientMissingOrDuplicate,
    #[error("reference host Name differs from the parameters host client")]
    HostNameMismatch,
    #[error("reference host Nick differs from the parameters host client")]
    HostNickMismatch,
    #[error("reference canonical addresses differ from the metadata address set")]
    AddressSetMismatch,
    #[error("reference netpuncher summary differs from the metadata values")]
    NetpuncherMetadataMismatch,
    #[error("reference TCP display addresses differ from the metadata TCP projection")]
    TcpAddressProjectionMismatch,
    #[error("reference address protocol byte {0} has no C++ text representation")]
    InvalidAddressProtocol(u8),
    #[error("client ID {0} occurs more than once")]
    DuplicateClientId(i32),
    #[error("scenario resource has type {0}, expected Scenario")]
    InvalidScenarioResourceType(u8),
    #[error("resource type {0} has no C++ reference name")]
    InvalidResourceType(u8),
    #[error("loadable resource {0} has zero chunk size")]
    ZeroResourceChunkSize(i32),
    #[error("{kind} list contains {count} entries, above the C++ limit")]
    ListTooLarge { kind: &'static str, count: usize },
    #[error("player {player_id} has unsupported type {player_type}")]
    InvalidPlayerType { player_id: i32, player_type: u8 },
    #[error("player {0} has HasResource without a resource core")]
    MissingPlayerResource(i32),
    #[error("player {0} carries a resource core without HasResource")]
    UnexpectedPlayerResource(i32),
    #[error("team boolean `{field}` has non-boolean byte {value}")]
    InvalidTeamBoolean { field: &'static str, value: u8 },
    #[error("team distribution {0} has no C++ reference name")]
    InvalidTeamDistribution(u8),
    #[error("team name has {0} bytes, above C4MaxName")]
    TeamNameTooLong(usize),
    #[error("raw C4Team name contains a line break")]
    TeamNameContainsLineBreak,
}

pub(crate) fn serialize_reference_parameters(
    output: &mut String,
    parameters: &JoinGameParametersEnvelope,
) -> Result<(), HostGameReferenceError> {
    validate_parameters(parameters)?;

    push_i32(output, "RandomSeed", parameters.random_seed, 0, 0);
    push_i32(
        output,
        "StartupPlayerCount",
        parameters.startup_player_count,
        0,
        0,
    );
    push_i32(output, "MaxPlayers", parameters.max_players, 0, 0);
    push_bool(output, "UseFairCrew", parameters.use_fair_crew, false, 0);
    push_bool(
        output,
        "FairCrewForced",
        parameters.fair_crew_forced,
        false,
        0,
    );
    push_i32(
        output,
        "FairCrewStrength",
        parameters.fair_crew_strength,
        0,
        0,
    );
    push_bool(output, "AllowDebug", parameters.allow_debug, true, 0);
    push_bool(
        output,
        "IsNetworkGame",
        parameters.is_network_game,
        false,
        0,
    );
    push_i32(output, "ControlRate", parameters.control_rate, -1, 0);
    push_bool(
        output,
        "AutoFrameSkip",
        parameters.auto_frame_skip,
        false,
        0,
    );
    if !parameters.rules.is_empty() {
        push_line(output, 0, "Rules", &encode_id_list(&parameters.rules));
    }
    if !parameters.goals.is_empty() {
        push_line(output, 0, "Goals", &encode_id_list(&parameters.goals));
    }
    push_legacy_string(output, "League", &parameters.league, 0);
    push_legacy_string(output, "LeagueAddress", &parameters.league_address, 0);
    if parameters.title.as_bytes() != b"No title" {
        push_line(output, 0, "Title", &quote_legacy(&parameters.title));
    }

    push_resource_section(output, "Scenario", &parameters.scenario, 2);
    for resource in &parameters.game_resources {
        push_resource_section(output, "Resource", resource, 2);
    }
    push_player_info_list(output, "PlayerInfos", &parameters.player_infos);
    push_player_info_list(
        output,
        "RestorePlayerInfos",
        &parameters.restore_player_infos,
    );
    push_team_list(output, &parameters.teams);
    push_clients(output, &parameters.clients.clients)?;
    Ok(())
}

fn validate_summary(
    summary: &NetworkGameReference,
    metadata: &HostGameReferenceMetadata,
    parameters: &JoinGameParametersEnvelope,
) -> Result<(), HostGameReferenceError> {
    if !matches!(
        summary.state.as_str(),
        "None" | "Init" | "Lobby" | "Paused" | "Running"
    ) {
        return Err(HostGameReferenceError::InvalidState(summary.state.clone()));
    }
    i32::try_from(summary.start_time)
        .map(|_| ())
        .map_err(|_| HostGameReferenceError::StartTimeOutOfRange(summary.start_time))?;
    if summary.icon != metadata.icon {
        return Err(HostGameReferenceError::IconMetadataMismatch);
    }
    if summary.time != metadata.time {
        return Err(HostGameReferenceError::TimeMetadataMismatch);
    }
    if summary.comment != clonk_resources::decode_legacy_script_text(metadata.comment.as_bytes()) {
        return Err(HostGameReferenceError::CommentMetadataMismatch);
    }
    if summary.max_players != parameters.max_players {
        return Err(HostGameReferenceError::MaxPlayersMismatch {
            reference: summary.max_players,
            parameters: parameters.max_players,
        });
    }
    if summary.title != clonk_resources::decode_legacy_script_text(parameters.title.as_bytes()) {
        return Err(HostGameReferenceError::TitleMismatch);
    }
    if summary.use_fair_crew != parameters.use_fair_crew {
        return Err(HostGameReferenceError::UseFairCrewMismatch);
    }
    if summary.goals != project_goal_ids(parameters) {
        return Err(HostGameReferenceError::GoalsMismatch);
    }
    if summary.league != clonk_resources::decode_legacy_script_text(parameters.league.as_bytes()) {
        return Err(HostGameReferenceError::LeagueMismatch);
    }
    if summary.league_address
        != clonk_resources::decode_legacy_script_text(parameters.league_address.as_bytes())
    {
        return Err(HostGameReferenceError::LeagueAddressMismatch);
    }
    if summary.player_names != project_active_player_names(parameters) {
        return Err(HostGameReferenceError::PlayerNamesMismatch);
    }
    let mut hosts = parameters
        .clients
        .clients
        .iter()
        .filter(|client| client.client_id == 0);
    let host = hosts
        .next()
        .filter(|_| hosts.next().is_none())
        .ok_or(HostGameReferenceError::HostClientMissingOrDuplicate)?;
    if summary.host_name != clonk_resources::decode_legacy_script_text(host.name.as_bytes()) {
        return Err(HostGameReferenceError::HostNameMismatch);
    }
    if summary.host_nick != clonk_resources::decode_legacy_script_text(host.nick.as_bytes()) {
        return Err(HostGameReferenceError::HostNickMismatch);
    }
    if summary.addresses != metadata.addresses {
        return Err(HostGameReferenceError::AddressSetMismatch);
    }
    if summary.netpuncher_ipv4 != metadata.netpuncher_ipv4
        || summary.netpuncher_ipv6 != metadata.netpuncher_ipv6
        || summary.netpuncher_address
            != clonk_resources::decode_legacy_script_text(metadata.netpuncher_address.as_bytes())
    {
        return Err(HostGameReferenceError::NetpuncherMetadataMismatch);
    }
    let tcp_projection = metadata
        .addresses
        .iter()
        .filter_map(|address| match address.protocol {
            NetworkProtocol::Tcp => Some(reference_projection_endpoint(
                NetworkAddress::new(NetworkProtocol::Tcp, address.endpoint).endpoint,
            )),
            NetworkProtocol::Udp | NetworkProtocol::Unknown(_) => None,
        })
        .collect::<Vec<_>>();
    let display_addresses = summary
        .tcp_addresses
        .iter()
        .map(|address| {
            reference_projection_endpoint(
                NetworkAddress::new(NetworkProtocol::Tcp, *address).endpoint,
            )
        })
        .collect::<Vec<_>>();
    if display_addresses != tcp_projection {
        return Err(HostGameReferenceError::TcpAddressProjectionMismatch);
    }
    Ok(())
}

fn refresh_parameter_summary(
    summary: &mut NetworkGameReference,
    parameters: &JoinGameParametersEnvelope,
) {
    summary.title = clonk_resources::decode_legacy_script_text(parameters.title.as_bytes());
    summary.max_players = parameters.max_players;
    summary.use_fair_crew = parameters.use_fair_crew;
    summary.goals = project_goal_ids(parameters);
    summary.league = clonk_resources::decode_legacy_script_text(parameters.league.as_bytes());
    summary.league_address =
        clonk_resources::decode_legacy_script_text(parameters.league_address.as_bytes());
    summary.player_names = project_active_player_names(parameters);
}

fn project_goal_ids(parameters: &JoinGameParametersEnvelope) -> Vec<String> {
    parameters
        .goals
        .iter()
        .map(|goal| goal.id.as_bytes().iter().copied().map(char::from).collect())
        .collect()
}

fn project_active_player_names(parameters: &JoinGameParametersEnvelope) -> Vec<String> {
    parameters
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
            let name = if !player.league_account.is_empty() {
                &player.league_account
            } else if !player.forced_name.is_empty() {
                &player.forced_name
            } else {
                &player.name
            };
            clonk_resources::decode_legacy_script_text(name.as_bytes())
        })
        .collect()
}

fn reference_projection_endpoint(endpoint: SocketAddr) -> SocketAddr {
    match endpoint {
        SocketAddr::V4(_) => endpoint,
        SocketAddr::V6(endpoint) => {
            SocketAddr::V6(SocketAddrV6::new(*endpoint.ip(), endpoint.port(), 0, 0))
        }
    }
}

fn validate_metadata(metadata: &HostGameReferenceMetadata) -> Result<(), HostGameReferenceError> {
    metadata.addresses.iter().try_for_each(|address| {
        if let NetworkProtocol::Unknown(value) = address.protocol {
            Err(HostGameReferenceError::InvalidAddressProtocol(value))
        } else {
            Ok(())
        }
    })
}

fn discard_live_player_resources(list: &mut PlayerInfoListSnapshot) {
    for client in &mut list.clients {
        for player in &mut client.players {
            player.flags &= !PLAYER_INFO_FLAG_HAS_RESOURCE;
            player.resource = None;
        }
    }
}

fn validate_parameters(
    parameters: &JoinGameParametersEnvelope,
) -> Result<(), HostGameReferenceError> {
    if parameters.scenario.resource_type != 1 {
        return Err(HostGameReferenceError::InvalidScenarioResourceType(
            parameters.scenario.resource_type,
        ));
    }
    validate_resource(&parameters.scenario)?;
    for resource in &parameters.game_resources {
        validate_resource(resource)?;
    }
    validate_player_info_list(&parameters.player_infos)?;
    validate_player_info_list(&parameters.restore_player_infos)?;
    validate_teams(&parameters.teams)?;
    validate_clients(&parameters.clients.clients)
}

fn validate_resource(resource: &NetworkResourceCore) -> Result<(), HostGameReferenceError> {
    resource_type_name(resource.resource_type).ok_or(
        HostGameReferenceError::InvalidResourceType(resource.resource_type),
    )?;
    if resource.loadable && resource.chunk_size == 0 {
        return Err(HostGameReferenceError::ZeroResourceChunkSize(resource.id));
    }
    Ok(())
}

fn validate_player_info_list(list: &PlayerInfoListSnapshot) -> Result<(), HostGameReferenceError> {
    ensure_list_size("player-info client", list.clients.len())?;
    for client in &list.clients {
        ensure_list_size("player", client.players.len())?;
        for player in &client.players {
            if !matches!(
                player.player_type,
                PLAYER_INFO_TYPE_USER | PLAYER_INFO_TYPE_SCRIPT
            ) {
                return Err(HostGameReferenceError::InvalidPlayerType {
                    player_id: player.id,
                    player_type: player.player_type,
                });
            }
            match (
                player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != 0,
                player.resource.as_ref(),
            ) {
                (true, Some(resource)) => validate_resource(resource)?,
                (true, None) => {
                    return Err(HostGameReferenceError::MissingPlayerResource(player.id));
                }
                (false, Some(_)) => {
                    return Err(HostGameReferenceError::UnexpectedPlayerResource(player.id));
                }
                (false, None) => {}
            }
        }
    }
    Ok(())
}

fn validate_teams(teams: &JoinTeamListSnapshot) -> Result<(), HostGameReferenceError> {
    for (field, value) in [
        ("Active", teams.active),
        ("Custom", teams.custom),
        ("AllowHostilityChange", teams.allow_hostility_change),
        ("AllowTeamSwitch", teams.allow_team_switch),
        ("AutoGenerateTeams", teams.auto_generate_teams),
        ("TeamColors", teams.team_colors),
    ] {
        if value > 1 {
            return Err(HostGameReferenceError::InvalidTeamBoolean { field, value });
        }
    }
    team_distribution_name(teams.team_distribution).ok_or(
        HostGameReferenceError::InvalidTeamDistribution(teams.team_distribution),
    )?;
    ensure_list_size("team", teams.teams.len())?;
    for team in &teams.teams {
        if team.name.as_bytes().len() > 30 {
            return Err(HostGameReferenceError::TeamNameTooLong(
                team.name.as_bytes().len(),
            ));
        }
        if team
            .name
            .as_bytes()
            .iter()
            .any(|byte| matches!(byte, b'\r' | b'\n'))
        {
            return Err(HostGameReferenceError::TeamNameContainsLineBreak);
        }
        ensure_list_size("team player", team.player_ids.len())?;
    }
    Ok(())
}

fn validate_clients(clients: &[ClientCoreControlData]) -> Result<(), HostGameReferenceError> {
    let mut clients = clients.iter().collect::<Vec<_>>();
    clients.sort_by_key(|client| client.client_id);
    if let Some(id) = clients
        .windows(2)
        .find(|pair| pair[0].client_id == pair[1].client_id)
        .map(|pair| pair[0].client_id)
    {
        return Err(HostGameReferenceError::DuplicateClientId(id));
    }
    Ok(())
}

fn ensure_list_size(kind: &'static str, count: usize) -> Result<(), HostGameReferenceError> {
    if count > MAX_REFERENCE_LIST_ENTRIES {
        Err(HostGameReferenceError::ListTooLarge { kind, count })
    } else {
        Ok(())
    }
}

fn push_resource_section(
    output: &mut String,
    name: &str,
    resource: &NetworkResourceCore,
    indent: usize,
) {
    begin_section(output, indent, name);
    let type_name = resource_type_name(resource.resource_type)
        .expect("validated resource type has a reference name");
    if resource.resource_type != 0 {
        push_line(output, indent, "Type", type_name);
    }
    push_i32(output, "ID", resource.id, -1, indent);
    push_i32(output, "DerID", resource.derived_id, -1, indent);
    push_bool(output, "Loadable", resource.loadable, true, indent);
    if resource.loadable {
        push_u32(output, "FileSize", resource.file_size, 0, indent);
        push_u32(output, "FileCRC", resource.file_crc, 0, indent);
        push_u32(output, "ChunkSize", resource.chunk_size, 100 * 1024, indent);
    }
    push_u32(output, "ContentsCRC", resource.contents_crc, 0, indent);
    if let Some(hash) = resource.file_sha {
        let mut encoded = String::with_capacity(hash.len() * 2);
        for byte in hash {
            let _ = write!(encoded, "{byte:02x}");
        }
        push_line(output, indent, "FileSHA", &encoded);
    }
    push_network_filename(output, "Filename", &resource.filename, indent);
    push_network_filename(output, "Author", &resource.author, indent);
}

fn push_player_info_list(output: &mut String, name: &str, list: &PlayerInfoListSnapshot) {
    if list.last_player_id == 0 && list.clients.is_empty() {
        return;
    }
    begin_section(output, 2, name);
    push_i32(output, "LastPlayerID", list.last_player_id, 0, 2);
    for client in &list.clients {
        push_client_player_infos(output, client, 4);
    }
}

fn push_client_player_infos(
    output: &mut String,
    client: &ClientPlayerInfosSnapshot,
    indent: usize,
) {
    begin_section(output, indent, "Client");
    push_i32(output, "ID", client.client_id, -1, indent);
    if client.flags != 0 {
        push_line(
            output,
            indent,
            "Flags",
            &encode_bitfield(
                client.flags,
                &[(1, "AddPlayers"), (2, "Updated"), (4, "Initial")],
            ),
        );
    }
    for player in &client.players {
        push_player(output, player, indent + 2);
    }
}

fn push_player(output: &mut String, player: &ControlPlayerInfoEntry, indent: usize) {
    begin_section(output, indent, "Player");
    append_player_info_fields(output, player, indent);
}

/// Serializes the named `C4PlayerInfoList` form stored in `PlayerInfos.txt`
/// and `SavePlayerInfos.txt`.
pub fn encode_player_info_list_ini(
    list: &PlayerInfoListSnapshot,
) -> Result<Vec<u8>, HostGameReferenceError> {
    validate_player_info_list(list)?;
    let mut output = String::from("[PlayerInfoList]\r\n");
    push_i32(&mut output, "LastPlayerID", list.last_player_id, 0, 0);
    for client in &list.clients {
        push_client_player_infos(&mut output, client, 2);
    }
    Ok(output
        .chars()
        .map(|character| u8::try_from(u32::from(character)).unwrap_or(b'?'))
        .collect())
}

pub(crate) fn append_player_info_fields(
    output: &mut String,
    player: &ControlPlayerInfoEntry,
    indent: usize,
) {
    push_legacy_string(output, "Name", &player.name, indent);
    push_legacy_string(output, "ForcedName", &player.forced_name, indent);
    push_legacy_string(output, "Filename", &player.filename, indent);
    let flags = player.flags & PLAYER_INFO_SYNC_FLAGS;
    if flags != 0 {
        push_line(
            output,
            indent,
            "Flags",
            &encode_bitfield(
                flags,
                &[
                    (1 << 0, "Joined"),
                    (1 << 2, "Removed"),
                    (1 << 3, "HasResource"),
                    (1 << 4, "JoinIssued"),
                    (1 << 7, "SavegameJoin"),
                    (1 << 8, "Disconnected"),
                    (1 << 10, "VotedOut"),
                    (1 << 9, "Won"),
                    (1 << 11, "AttributesFixed"),
                    (1 << 12, "NoScenarioInit"),
                    (1 << 13, "NoEliminationCheck"),
                    (1 << 14, "Invisible"),
                ],
            ),
        );
    }
    push_i32(output, "ID", player.id, 0, indent);
    if player.player_type == PLAYER_INFO_TYPE_SCRIPT {
        push_line(output, indent, "Type", "Script");
    }
    push_u32(output, "Color", player.color, 0, indent);
    push_u32(
        output,
        "OriginalColor",
        player.original_color,
        player.color,
        indent,
    );
    push_i32(output, "SavgamePlayer", player.savegame_player, 0, indent);
    push_i32(output, "Team", player.team, 0, indent);
    push_legacy_string(output, "AUID", &player.auth_id, indent);
    if flags & PLAYER_INFO_FLAG_JOINED != 0 {
        push_i32(output, "GameNumber", player.game_number, -1, indent);
        push_i32(output, "GameJoinFrame", player.game_join_frame, -1, indent);
    }
    if flags & PLAYER_INFO_FLAG_REMOVED != 0 {
        push_i32(output, "GamePartFrame", player.game_part_frame, -1, indent);
    }
    if player.extra_data != *b"NONE" {
        let extra = player
            .extra_data
            .iter()
            .copied()
            .map(char::from)
            .collect::<String>();
        push_line(output, indent, "ExtraData", &extra);
    }
    push_legacy_string(output, "LeagueAccount", &player.league_account, indent);
    push_i32(output, "LeagueScore", player.league_score, 0, indent);
    push_i32(output, "LeagueRank", player.league_rank, 0, indent);
    push_i32(
        output,
        "LeagueRankSymbol",
        player.league_rank_symbol,
        0,
        indent,
    );
    push_i32(
        output,
        "ProjectedGain",
        player.league_projected_gain,
        -1,
        indent,
    );
    push_raw_legacy_string(output, "ClanTag", &player.clan_tag, indent);
    push_i32(
        output,
        "LeaguePerformance",
        player.league_performance,
        0,
        indent,
    );
    push_legacy_string(
        output,
        "LeagueProgressData",
        &player.league_progress_data,
        indent,
    );
    if let Some(resource) = player.resource.as_ref() {
        push_resource_section(output, "ResCore", resource, indent + 2);
    }
}

fn push_team_list(output: &mut String, teams: &JoinTeamListSnapshot) {
    let has_values = teams.active != 1
        || teams.custom != 1
        || teams.allow_hostility_change != 0
        || teams.allow_team_switch != 0
        || teams.auto_generate_teams != 0
        || teams.last_team_id != 0
        || teams.team_distribution != 0
        || teams.team_colors != 0
        || teams.max_script_players != 0
        || !teams.script_player_names.is_empty()
        || teams.random_team_count != 0
        || !teams.teams.is_empty();
    if !has_values {
        return;
    }
    begin_section(output, 2, "Teams");
    push_bool(output, "Active", teams.active != 0, true, 2);
    push_bool(output, "Custom", teams.custom != 0, true, 2);
    push_bool(
        output,
        "AllowHostilityChange",
        teams.allow_hostility_change != 0,
        false,
        2,
    );
    push_bool(
        output,
        "AllowTeamSwitch",
        teams.allow_team_switch != 0,
        false,
        2,
    );
    push_bool(
        output,
        "AutoGenerateTeams",
        teams.auto_generate_teams != 0,
        false,
        2,
    );
    push_i32(output, "LastTeamID", teams.last_team_id, 0, 2);
    if teams.team_distribution != 0 {
        push_line(
            output,
            2,
            "TeamDistribution",
            team_distribution_name(teams.team_distribution)
                .expect("validated team distribution has a name"),
        );
    }
    push_bool(output, "TeamColors", teams.team_colors != 0, false, 2);
    push_i32(output, "MaxScriptPlayers", teams.max_script_players, 0, 2);
    push_legacy_string(output, "ScriptPlayerNames", &teams.script_player_names, 2);
    push_i32(output, "RandomTeamCount", teams.random_team_count, 0, 2);
    for team in &teams.teams {
        push_team(output, team);
    }
}

fn push_team(output: &mut String, team: &JoinTeamSnapshot) {
    begin_section(output, 4, "Team");
    push_i32(output, "id", team.id, 0, 4);
    push_raw_legacy_string(output, "Name", &team.name, 4);
    push_i32(output, "PlrStartIndex", team.player_start_index, 0, 4);
    push_i32(output, "PlayerCount", team.player_ids.len() as i32, 0, 4);
    if !team.player_ids.is_empty() {
        let players = team
            .player_ids
            .iter()
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        push_line(output, 4, "Players", &players);
    }
    push_u32(output, "Color", team.color, 0, 4);
    push_legacy_string(output, "IconSpec", &team.icon_spec, 4);
    push_i32(output, "MaxPlayer", team.max_players, 0, 4);
}

fn push_clients(
    output: &mut String,
    clients: &[ClientCoreControlData],
) -> Result<(), HostGameReferenceError> {
    validate_clients(clients)?;
    let mut clients = clients.iter().collect::<Vec<_>>();
    clients.sort_by_key(|client| client.client_id);
    for client in clients {
        begin_section(output, 2, "Client");
        push_i32(output, "ID", client.client_id, -1, 2);
        push_bool(output, "Activated", client.activated, false, 2);
        push_bool(output, "Observer", client.observer, false, 2);
        push_legacy_string(output, "Name", &client.name, 2);
        push_legacy_string(output, "Nick", &client.nick, 2);
        push_bool(output, "LobbyReady", client.lobby_ready, false, 2);
    }
    Ok(())
}

fn begin_section(output: &mut String, indent: usize, name: &str) {
    output.push_str("\r\n");
    output.push_str(&" ".repeat(indent));
    let _ = writeln!(output, "[{name}]\r");
}

fn push_line(output: &mut String, indent: usize, name: &str, value: &str) {
    output.push_str(&" ".repeat(indent));
    let _ = writeln!(output, "{name}={value}\r");
}

fn push_i32(output: &mut String, name: &str, value: i32, default: i32, indent: usize) {
    if value != default {
        push_line(output, indent, name, &value.to_string());
    }
}

fn push_u32(output: &mut String, name: &str, value: u32, default: u32, indent: usize) {
    if value != default {
        push_line(output, indent, name, &value.to_string());
    }
}

fn push_bool(output: &mut String, name: &str, value: bool, default: bool, indent: usize) {
    if value != default {
        push_line(output, indent, name, if value { "true" } else { "false" });
    }
}

fn push_legacy_string(output: &mut String, name: &str, value: &LegacyCString, indent: usize) {
    if !value.is_empty() {
        push_line(output, indent, name, &quote_legacy(value));
    }
}

fn push_raw_legacy_string(output: &mut String, name: &str, value: &LegacyCString, indent: usize) {
    if !value.is_empty() {
        let raw = value
            .as_bytes()
            .iter()
            .copied()
            .map(char::from)
            .collect::<String>();
        push_line(output, indent, name, &raw);
    }
}

fn push_network_filename(output: &mut String, name: &str, value: &LegacyCString, indent: usize) {
    if value.is_empty() {
        return;
    }
    let normalized = LegacyCString::from_bytes(
        value
            .as_bytes()
            .iter()
            .map(|byte| if *byte == b'/' { b'\\' } else { *byte })
            .collect(),
    )
    .expect("normalizing separators cannot introduce NUL");
    push_line(output, indent, name, &quote_legacy(&normalized));
}

pub(crate) fn quote_legacy(value: &LegacyCString) -> String {
    let mut output = String::with_capacity(value.as_bytes().len() + 2);
    output.push('"');
    let mut last_was_numeric_escape = false;
    for byte in value.as_bytes() {
        let numeric_escape = last_was_numeric_escape && byte.is_ascii_digit();
        last_was_numeric_escape = false;
        if (!byte.is_ascii_graphic() && *byte != b' ')
            || *byte == b'\\'
            || *byte == b'"'
            || numeric_escape
        {
            match byte {
                b'\x07' => output.push_str("\\a"),
                b'\x08' => output.push_str("\\b"),
                b'\x0c' => output.push_str("\\f"),
                b'\n' => output.push_str("\\n"),
                b'\r' => output.push_str("\\r"),
                b'\t' => output.push_str("\\t"),
                b'\x0b' => output.push_str("\\v"),
                b'"' => output.push_str("\\\""),
                b'\\' => output.push_str("\\\\"),
                _ => {
                    let _ = write!(output, "\\{byte:o}");
                    last_was_numeric_escape = true;
                }
            }
        } else {
            output.push(char::from(*byte));
        }
    }
    output.push('"');
    output
}

fn encode_id_list(entries: &[crate::JoinDataIdListEntry]) -> String {
    entries
        .iter()
        .map(|entry| {
            let id = entry
                .id
                .as_bytes()
                .iter()
                .copied()
                .map(char::from)
                .collect::<String>();
            format!("{id}={}", entry.count)
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn encode_bitfield<T>(mut value: T, names: &[(T, &'static str)]) -> String
where
    T: Copy
        + PartialEq
        + std::ops::BitAnd<Output = T>
        + std::ops::BitAndAssign
        + std::ops::Not<Output = T>
        + std::fmt::Display
        + From<u8>,
{
    let zero = T::from(0);
    let mut output = Vec::new();
    for (bit, name) in names {
        if value & *bit == *bit {
            output.push((*name).to_owned());
            value &= !*bit;
        }
    }
    if value != zero {
        output.push(value.to_string());
    }
    output.join("|")
}

fn resource_type_name(value: u8) -> Option<&'static str> {
    match value {
        0 => Some("Null"),
        1 => Some("Scenario"),
        2 => Some("Dynamic"),
        3 => Some("Player"),
        4 => Some("Definitions"),
        5 => Some("System"),
        6 => Some("Material"),
        _ => None,
    }
}

fn team_distribution_name(value: u8) -> Option<&'static str> {
    match value {
        0 => Some("Free"),
        1 => Some("Host"),
        2 => Some("None"),
        3 => Some("Random"),
        4 => Some("RandomInv"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use clonk_engine::{ControlPlayerInfoEntry, LegacyCString};

    use super::*;

    fn exact_reference() -> HostGameReference {
        let host_config = crate::HostConfig::default();
        let parameters = host_config
            .initial_join_snapshot
            .as_ref()
            .expect("default host JoinData")
            .parameters
            .clone();
        let host = parameters
            .clients
            .clients
            .iter()
            .find(|client| client.client_id == 0)
            .expect("default host client");
        HostGameReference::new(
            NetworkGameReference {
                icon: 7,
                title: clonk_resources::decode_legacy_script_text(parameters.title.as_bytes()),
                host_name: clonk_resources::decode_legacy_script_text(host.name.as_bytes()),
                host_nick: clonk_resources::decode_legacy_script_text(host.nick.as_bytes()),
                state: "Lobby".to_string(),
                control_mode: host_config.initial_status.control_mode,
                time: 23,
                comment: "old comment".to_string(),
                password_needed: false,
                official_server: true,
                max_players: parameters.max_players,
                game: "LegacyClonk".to_string(),
                version: [4, 9, 11, 0],
                build: 362,
                ..NetworkGameReference::default()
            },
            HostGameReferenceMetadata {
                icon: 7,
                time: 23,
                frame: 24,
                league_performance: 25,
                comment: LegacyCString::from_bytes(b"old comment".to_vec()).unwrap(),
                ..HostGameReferenceMetadata::default()
            },
            parameters,
        )
        .expect("exact reference fixture validates")
    }

    #[test]
    fn lobby_option_rebuild_updates_only_password_presence_and_comment() {
        let reference = exact_reference();
        let original_summary = reference.summary.clone();
        let original_metadata = reference.metadata.clone();
        let original_parameters = reference.parameters.clone();
        let comment = LegacyCString::from_bytes(b"new \x80 comment".to_vec()).unwrap();

        let updated = reference
            .replacing_lobby_options(true, comment.clone())
            .expect("live lobby options rebuild validates");

        let mut expected_summary = original_summary;
        expected_summary.password_needed = true;
        expected_summary.comment = clonk_resources::decode_legacy_script_text(comment.as_bytes());
        let mut expected_metadata = original_metadata;
        expected_metadata.comment = comment;
        assert_eq!(updated.summary, expected_summary);
        assert_eq!(updated.metadata, expected_metadata);
        assert_eq!(updated.parameters, original_parameters);
    }

    #[test]
    fn constructor_rejects_stale_display_projections() {
        let reference = exact_reference();

        let mut summary = reference.summary.clone();
        summary.icon += 1;
        assert_eq!(
            HostGameReference::new(
                summary,
                reference.metadata.clone(),
                reference.parameters.clone(),
            )
            .unwrap_err(),
            HostGameReferenceError::IconMetadataMismatch
        );

        let mut summary = reference.summary.clone();
        summary.comment.push('!');
        assert_eq!(
            HostGameReference::new(
                summary,
                reference.metadata.clone(),
                reference.parameters.clone(),
            )
            .unwrap_err(),
            HostGameReferenceError::CommentMetadataMismatch
        );

        let mut summary = reference.summary.clone();
        summary.player_names.push("stale".to_string());
        assert_eq!(
            HostGameReference::new(
                summary,
                reference.metadata.clone(),
                reference.parameters.clone(),
            )
            .unwrap_err(),
            HostGameReferenceError::PlayerNamesMismatch
        );
    }

    #[test]
    fn parameter_rebuild_refreshes_every_display_projection() {
        let reference = exact_reference();
        let mut parameters = reference.parameters.clone();
        parameters.title = LegacyCString::from_bytes(b"New title".to_vec()).unwrap();
        parameters.max_players = 12;
        parameters.use_fair_crew = true;
        parameters.goals = vec![crate::JoinDataIdListEntry {
            id: crate::JoinDataC4Id::from_bytes(*b"MELE").unwrap(),
            count: 0,
        }];
        parameters.league = LegacyCString::from_bytes(b"New league".to_vec()).unwrap();
        parameters.league_address =
            LegacyCString::from_bytes(b"https://league.invalid/new".to_vec()).unwrap();
        parameters.player_infos = PlayerInfoListSnapshot {
            last_player_id: 5,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 0,
                flags: 0,
                players: vec![
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Original".to_vec()).unwrap(),
                        forced_name: LegacyCString::from_bytes(b"Forced".to_vec()).unwrap(),
                        league_account: LegacyCString::from_bytes(b"League name".to_vec()).unwrap(),
                        id: 1,
                        ..ControlPlayerInfoEntry::default()
                    },
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Original two".to_vec()).unwrap(),
                        forced_name: LegacyCString::from_bytes(b"Forced two".to_vec()).unwrap(),
                        id: 2,
                        ..ControlPlayerInfoEntry::default()
                    },
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Plain".to_vec()).unwrap(),
                        id: 3,
                        ..ControlPlayerInfoEntry::default()
                    },
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Removed".to_vec()).unwrap(),
                        flags: PLAYER_INFO_FLAG_REMOVED,
                        id: 4,
                        ..ControlPlayerInfoEntry::default()
                    },
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Invisible".to_vec()).unwrap(),
                        flags: PLAYER_INFO_FLAG_INVISIBLE,
                        id: 5,
                        ..ControlPlayerInfoEntry::default()
                    },
                    ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Invisible script".to_vec()).unwrap(),
                        flags: PLAYER_INFO_FLAG_INVISIBLE,
                        id: 6,
                        player_type: PLAYER_INFO_TYPE_SCRIPT,
                        ..ControlPlayerInfoEntry::default()
                    },
                ],
            }],
        };

        let updated = reference.replacing_parameters(parameters.clone()).unwrap();

        assert_eq!(updated.summary.title, "New title");
        assert_eq!(updated.summary.max_players, 12);
        assert!(updated.summary.use_fair_crew);
        assert_eq!(updated.summary.goals, ["MELE"]);
        assert_eq!(updated.summary.league, "New league");
        assert_eq!(updated.summary.league_address, "https://league.invalid/new");
        assert_eq!(
            updated.summary.player_names,
            ["League name", "Forced two", "Plain", "Invisible"]
        );

        parameters.league = LegacyCString::from_bytes(b"Runtime league".to_vec()).unwrap();
        let runtime = updated
            .replacing_runtime(parameters, "Running", 99, 100, false, 0)
            .unwrap();
        assert_eq!(runtime.summary.time, 99);
        assert_eq!(runtime.metadata.time, 99);
        assert_eq!(runtime.summary.league, "Runtime league");
    }

    #[test]
    fn lobby_option_rebuild_revalidates_the_complete_reference() {
        let mut reference = exact_reference();
        reference.summary.max_players += 1;
        let expected = HostGameReferenceError::MaxPlayersMismatch {
            reference: reference.summary.max_players,
            parameters: reference.parameters.max_players,
        };

        assert_eq!(
            reference
                .replacing_lobby_options(false, reference.metadata.comment.clone())
                .unwrap_err(),
            expected
        );
    }

    #[test]
    fn standalone_player_info_list_round_trips_through_the_cpp_ini_shape() {
        let list = PlayerInfoListSnapshot {
            last_player_id: 7,
            clients: vec![ClientPlayerInfosSnapshot {
                client_id: 3,
                flags: 0,
                players: vec![ControlPlayerInfoEntry {
                    name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                    id: 7,
                    league_progress_data_is_null: false,
                    ..ControlPlayerInfoEntry::default()
                }],
            }],
        };

        let encoded = encode_player_info_list_ini(&list).unwrap();
        assert!(encoded.starts_with(b"[PlayerInfoList]\r\nLastPlayerID=7\r\n"));
        assert_eq!(crate::decode_player_info_list_ini(&encoded).unwrap(), list);
    }
}
