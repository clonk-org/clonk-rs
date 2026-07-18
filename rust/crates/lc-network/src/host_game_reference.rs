//! Exact host-side `C4GameParameters` reference serialization.
//!
//! Search results intentionally remain a compact display projection. This
//! module couples that projection to the complete synchronized parameters so
//! a host cannot advertise the former while silently dropping the latter.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::net::{SocketAddr, SocketAddrV6};

use lc_engine::{
    ClientCoreControlData, ControlPlayerInfoEntry, LegacyCString, NetworkResourceCore,
    PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_REMOVED,
    PLAYER_INFO_TYPE_SCRIPT, PLAYER_INFO_TYPE_USER,
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

    /// Rebuild the exact reference after a live C4GameParameters mutation.
    /// The display projection duplicates title and MaxPlayers and therefore
    /// must advance atomically with the serialized parameters.
    pub fn replacing_parameters(
        &self,
        parameters: JoinGameParametersEnvelope,
    ) -> Result<Self, HostGameReferenceError> {
        let mut summary = self.summary.clone();
        summary.title = lc_resources::decode_legacy_script_text(parameters.title.as_bytes());
        summary.max_players = parameters.max_players;
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
        summary.title = lc_resources::decode_legacy_script_text(parameters.title.as_bytes());
        summary.max_players = parameters.max_players;
        let mut metadata = self.metadata.clone();
        metadata.time = time;
        metadata.frame = frame;
        metadata.league_performance = league_performance;
        Self::new(summary, metadata, parameters)
    }

    /// Rebuild `C4Network2Reference::InitLocal`'s game-over projection.
    /// C++ copies live parameters, then overlays the independent global
    /// performance and one performance value per retained PlayerInfo ID.
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
    if summary.max_players != parameters.max_players {
        return Err(HostGameReferenceError::MaxPlayersMismatch {
            reference: summary.max_players,
            parameters: parameters.max_players,
        });
    }
    if summary.title != lc_resources::decode_legacy_script_text(parameters.title.as_bytes()) {
        return Err(HostGameReferenceError::TitleMismatch);
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
    if summary.host_name != lc_resources::decode_legacy_script_text(host.name.as_bytes()) {
        return Err(HostGameReferenceError::HostNameMismatch);
    }
    if summary.host_nick != lc_resources::decode_legacy_script_text(host.nick.as_bytes()) {
        return Err(HostGameReferenceError::HostNickMismatch);
    }
    if summary.addresses != metadata.addresses {
        return Err(HostGameReferenceError::AddressSetMismatch);
    }
    if summary.netpuncher_ipv4 != metadata.netpuncher_ipv4
        || summary.netpuncher_ipv6 != metadata.netpuncher_ipv6
        || summary.netpuncher_address
            != lc_resources::decode_legacy_script_text(metadata.netpuncher_address.as_bytes())
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
        push_client_player_infos(output, client);
    }
}

fn push_client_player_infos(output: &mut String, client: &ClientPlayerInfosSnapshot) {
    begin_section(output, 4, "Client");
    push_i32(output, "ID", client.client_id, -1, 4);
    if client.flags != 0 {
        push_line(
            output,
            4,
            "Flags",
            &encode_bitfield(
                client.flags,
                &[(1, "AddPlayers"), (2, "Updated"), (4, "Initial")],
            ),
        );
    }
    for player in &client.players {
        push_player(output, player);
    }
}

fn push_player(output: &mut String, player: &ControlPlayerInfoEntry) {
    begin_section(output, 6, "Player");
    append_player_info_fields(output, player, 6);
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
    push_legacy_string(output, "ClanTag", &player.clan_tag, indent);
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
