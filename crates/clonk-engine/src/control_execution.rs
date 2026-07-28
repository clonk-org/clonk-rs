use crate::control::PLAYER_INFO_FLAG_JOIN_ISSUED;
use crate::{
    player_file::PlayerFile, InitialNetworkTeam, InitialNetworkTeamDistribution,
    InitialNetworkTeamMetadata, JoinPlayerConfig, JoinPlayerControlData, JoinPlayerSource,
    LegacyCString, NetworkResourceCore, PlayerInfoUpdateRequest, ScenarioError,
    PLAYER_INFO_FLAG_ATTRIBUTES_FIXED, PLAYER_INFO_FLAG_HAS_RESOURCE, PLAYER_INFO_FLAG_INVISIBLE,
    PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK,
    PLAYER_INFO_FLAG_NO_SCENARIO_INIT, PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_FLAG_SAVEGAME_JOIN,
    PLAYER_INFO_TYPE_USER,
};
use crate::{
    ControlPlayerInfoEntry, PlayerInfoControlData, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
    CLIENT_PLAYER_INFO_FLAG_INITIAL, CLIENT_PLAYER_INFO_FLAG_UPDATED,
};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Process-local services used by C4TeamList while assigning initial players.
/// One object owns both operations because generated team colors can consume
/// the same C `rand()` stream as equal-team tie breaking.
pub trait InitialHostTeamAssignmentOracle {
    /// Mirrors one `SafeRandom(range)` call from the host process.
    fn safe_random(&mut self, range: i32) -> i32;

    /// Supplies the localized name and process-random color for a newly
    /// generated team. The assignment helper owns the remaining C4Team
    /// defaults and ignores those fields in the returned value.
    fn generate_team(
        &mut self,
        id: i32,
        existing_teams: &[InitialNetworkTeam],
    ) -> InitialNetworkTeam;
}

#[derive(Debug, Default)]
struct TeamAssignmentPreflightOracle {
    used_safe_random: bool,
    generated_team: bool,
}

impl TeamAssignmentPreflightOracle {
    fn used_process_randomness(&self) -> bool {
        self.used_safe_random || self.generated_team
    }
}

impl InitialHostTeamAssignmentOracle for TeamAssignmentPreflightOracle {
    fn safe_random(&mut self, range: i32) -> i32 {
        self.used_safe_random = true;
        range.saturating_sub(1)
    }

    fn generate_team(
        &mut self,
        id: i32,
        _existing_teams: &[InitialNetworkTeam],
    ) -> InitialNetworkTeam {
        self.generated_team = true;
        InitialNetworkTeam {
            id,
            name: LegacyCString::default(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        }
    }
}

/// Assigns scenario teams to a host's initial lobby player packet.
///
/// IDs must already have been allocated in packet order. The injected oracle
/// represents C++'s process-global `SafeRandom`; `Parameters.RandomSeed` is not
/// its seed (`src/C4Random.h:32-38,71-75`).
pub fn assign_initial_host_player_teams(
    teams: &mut InitialNetworkTeamMetadata,
    players: &mut [ControlPlayerInfoEntry],
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) {
    assign_initial_player_teams(teams, players, oracle, true, true);
}

/// Assigns scenario teams to already-ID-assigned ordinary offline players.
///
/// Unlike the host path, standalone initialization has no lobby. Custom users
/// in a non-random distribution therefore keep team zero for runtime choice;
/// all other required assignments share the host's exact team-selection path.
pub fn assign_initial_offline_player_teams(
    teams: &mut InitialNetworkTeamMetadata,
    players: &mut [ControlPlayerInfoEntry],
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) {
    assign_initial_player_teams(teams, players, oracle, false, true);
}

fn assign_initial_player_teams(
    teams: &mut InitialNetworkTeamMetadata,
    players: &mut [ControlPlayerInfoEntry],
    oracle: &mut impl InitialHostTeamAssignmentOracle,
    has_or_will_have_lobby: bool,
    by_host: bool,
) -> Vec<usize> {
    if !teams.active {
        return Vec::new();
    }

    let mut automatically_assigned = Vec::new();
    for (player_index, player) in players.iter_mut().enumerate() {
        let current_team = teams
            .teams
            .iter()
            .position(|team| team.player_ids.contains(&player.id));
        if player.team != 0 {
            if current_team.is_some_and(|index| teams.teams[index].id == player.team) {
                continue;
            }
            let host_may_select = matches!(
                teams.team_distribution,
                InitialNetworkTeamDistribution::Free
            ) || (by_host
                && matches!(
                    teams.team_distribution,
                    InitialNetworkTeamDistribution::Host
                ));
            let requested_team_is_available = player.team != -1
                && teams
                    .teams
                    .iter()
                    .find(|team| team.id == player.team)
                    .is_some_and(|team| !initial_team_is_full(team));
            if host_may_select && requested_team_is_available {
                continue;
            }
            player.team = current_team.map_or(0, |index| teams.teams[index].id);
            if current_team.is_some() {
                continue;
            }
        }

        let random_distribution = matches!(
            teams.team_distribution,
            InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        );
        let runtime_join_team_choice = teams.custom;
        let can_pick_team_at_runtime = !random_distribution
            && player.player_type == PLAYER_INFO_TYPE_USER
            && runtime_join_team_choice;
        let team_is_needed = runtime_join_team_choice || !teams.teams.is_empty();
        if !has_or_will_have_lobby && (!team_is_needed || can_pick_team_at_runtime) {
            continue;
        }
        let lowest_team = initial_random_smallest_team(
            &teams.teams,
            random_distribution,
            teams.random_team_count,
            oracle,
        );
        let assignment = if teams.auto_generate_teams && !random_distribution {
            lowest_team
                .filter(|&index| teams.teams[index].player_ids.is_empty())
                .or_else(|| Some(initial_generate_team(teams, oracle)))
        } else if lowest_team.is_none() && teams.teams.get(1).is_none() {
            initial_generate_teams_through(teams, 2, oracle);
            (!teams.teams.is_empty()).then_some(0)
        } else {
            lowest_team
        };
        let Some(index) = assignment else {
            continue;
        };
        let team = &mut teams.teams[index];
        team.player_ids.push(player.id);
        player.team = team.id;
        if teams.team_colors {
            player.color = team.color;
        }
        automatically_assigned.push(player_index);
    }
    automatically_assigned
}

fn initial_generate_teams_through(
    teams: &mut InitialNetworkTeamMetadata,
    last_id: i32,
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) {
    while teams.last_team_id < last_id {
        initial_generate_team(teams, oracle);
    }
}

fn initial_generate_team(
    teams: &mut InitialNetworkTeamMetadata,
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) -> usize {
    let id = teams.last_team_id.wrapping_add(1);
    let generated = oracle.generate_team(id, &teams.teams);
    teams.last_team_id = id;
    teams.teams.push(InitialNetworkTeam {
        id,
        name: generated.name,
        player_start_index: 0,
        player_ids: Vec::new(),
        color: generated.color,
        icon_spec: LegacyCString::default(),
        max_players: 0,
    });
    teams.teams.len() - 1
}

fn initial_team_is_full(team: &InitialNetworkTeam) -> bool {
    team.max_players != 0
        && i64::try_from(team.player_ids.len()).unwrap_or(i64::MAX) >= i64::from(team.max_players)
}

fn initial_random_smallest_team(
    teams: &[InitialNetworkTeam],
    limit_random_team_count: bool,
    random_team_count: i32,
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) -> Option<usize> {
    let team_count = if limit_random_team_count && random_team_count > 1 {
        usize::try_from(random_team_count).unwrap_or(usize::MAX)
    } else {
        teams.len()
    };
    let mut lowest: Option<usize> = None;
    let mut equal_lowest_count = 0;
    for (index, team) in teams.iter().take(team_count).enumerate() {
        if initial_team_is_full(team) {
            continue;
        }
        let is_lower = lowest.is_none_or(|lowest_index| {
            teams[lowest_index].player_ids.len() > team.player_ids.len()
        });
        if is_lower {
            lowest = Some(index);
            equal_lowest_count = 1;
        } else if lowest.is_some_and(|lowest_index| {
            teams[lowest_index].player_ids.len() == team.player_ids.len()
        }) {
            equal_lowest_count += 1;
            if oracle.safe_random(equal_lowest_count) == 0 {
                lowest = Some(index);
            }
        }
    }
    lowest
}

fn player_info_uses_color(player: &ControlPlayerInfoEntry) -> bool {
    player.flags & PLAYER_INFO_FLAG_REMOVED == 0 && player.savegame_player == 0
}

/// `IsColorConflict` from `C4PlayerInfoConflicts.cpp`, including the legacy
/// monitor-gamma and CIE u'v' conversion constants.
fn player_colors_conflict(first: u32, second: u32) -> bool {
    fn linear(channel: u32) -> f64 {
        (f64::from(channel) / 255.0).powf(1.0 / 2.2)
    }
    fn chromaticity(color: u32) -> (f64, f64, f64) {
        let r = linear((color >> 16) & 0xff);
        let g = linear((color >> 8) & 0xff);
        let b = linear(color & 0xff);
        let x_value = 0.412_453 * r + 0.357_580 * g + 0.180_423 * b;
        let luminance = 0.212_671 * r + 0.715_160 * g + 0.072_169 * b;
        let z_value = 0.019_334 * r + 0.119_193 * g + 0.950_227 * b;
        let sum = x_value + luminance + z_value;
        let (x, y) = if sum == 0.0 {
            (0.3, 0.3)
        } else {
            (x_value / sum, luminance / sum)
        };
        let denominator = -2.0 * x + 12.0 * y + 3.0;
        let (u, v) = if denominator == 0.0 {
            (0.0, 0.0)
        } else {
            (4.0 * x / denominator, 9.0 * y / denominator)
        };
        (u, v, luminance)
    }

    let (u1, v1, y1) = chromaticity(first);
    let (u2, v2, y2) = chromaticity(second);
    let luminance = (y1 + y2) / 2.0;
    let color_difference =
        ((u2 - u1).powi(2) + (v2 - v1).powi(2)).sqrt() * luminance.powi(2) * 150.0;
    let luminance_difference = ((y2 - y1).abs() / (luminance.powi(2) * 5.0).max(0.5)) / 0.10;
    color_difference + luminance_difference < 1.0
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlClientState {
    pub activated: bool,
    pub observer: bool,
    pub name: LegacyCString,
    pub nick: LegacyCString,
    pub lobby_ready: bool,
}

/// Synchronized `C4ClientList` state needed by player admission and lifecycle
/// controls (`src/C4Control.cpp:578-680`).
#[derive(Debug, Default)]
pub struct ControlClientRegistry {
    clients: BTreeMap<i32, ControlClientState>,
}

impl ControlClientRegistry {
    /// Replaces the synchronized client list with the complete registry from
    /// initial JoinData (`Game.Parameters.Clients`).
    pub fn replace_snapshot(
        &mut self,
        cores: impl IntoIterator<Item = crate::ClientCoreControlData>,
    ) {
        self.clients = cores
            .into_iter()
            .map(|core| {
                (
                    core.client_id,
                    ControlClientState {
                        activated: core.activated,
                        observer: core.observer,
                        name: core.name,
                        nick: core.nick,
                        lobby_ready: core.lobby_ready,
                    },
                )
            })
            .collect();
    }

    pub fn register(&mut self, client_id: i32, activated: bool, observer: bool) {
        self.clients.insert(
            client_id,
            ControlClientState {
                activated: activated && !observer,
                observer,
                name: LegacyCString::default(),
                nick: LegacyCString::default(),
                lobby_ready: false,
            },
        );
    }

    pub fn apply_join(&mut self, join: &crate::ClientJoinControlData) -> bool {
        if join.by_client != 0 || self.clients.contains_key(&join.core.client_id) {
            return false;
        }
        self.clients.insert(
            join.core.client_id,
            ControlClientState {
                activated: join.core.activated,
                observer: join.core.observer,
                name: join.core.name.clone(),
                nick: join.core.nick.clone(),
                lobby_ready: join.core.lobby_ready,
            },
        );
        true
    }

    pub fn apply_update(&mut self, update: &crate::ClientUpdateControlData) {
        if update.by_client != 0 {
            return;
        }
        let Some(client) = self.clients.get_mut(&update.client_id) else {
            return;
        };
        match update.update_type {
            crate::CLIENT_UPDATE_ACTIVATE => {
                // Native SetActivated is skipped entirely when the requested
                // activation bit already matches. This preserves even an
                // inconsistent recorded core with Activated+Observer set.
                if client.activated == (update.data != 0) {
                    return;
                }
                client.activated = update.data != 0;
                client.observer = false;
            }
            crate::CLIENT_UPDATE_SET_OBSERVER => {
                if client.observer {
                    return;
                }
                client.activated = false;
                client.observer = true;
            }
            _ => {}
        }
    }

    pub fn apply_remove(&mut self, remove: &crate::ClientRemoveControlData) -> bool {
        remove.by_client == 0
            && remove.client_id != 0
            && self.clients.remove(&remove.client_id).is_some()
    }

    pub fn contains(&self, client_id: i32) -> bool {
        self.clients.contains_key(&client_id)
    }

    pub fn state(&self, client_id: i32) -> Option<&ControlClientState> {
        self.clients.get(&client_id)
    }

    /// Apply `C4PacketReadyCheck` to the synchronized client core. Returns
    /// whether the serialized `LobbyReady` value actually changed.
    pub fn set_lobby_ready(&mut self, client_id: i32, ready: bool) -> bool {
        let Some(client) = self.clients.get_mut(&client_id) else {
            return false;
        };
        if client.lobby_ready == ready {
            return false;
        }
        client.lobby_ready = ready;
        true
    }

    /// `C4PacketReadyCheck::Request` clears every non-host ready flag before
    /// collecting the new responses.
    pub fn clear_nonhost_lobby_ready(&mut self) -> bool {
        let mut changed = false;
        for (&client_id, client) in &mut self.clients {
            if client_id != 0 && client.lobby_ready {
                client.lobby_ready = false;
                changed = true;
            }
        }
        changed
    }

    pub fn is_activated(&self, client_id: i32) -> bool {
        self.clients
            .get(&client_id)
            .is_some_and(|client| client.activated)
    }

    pub fn is_observer(&self, client_id: i32) -> bool {
        self.clients
            .get(&client_id)
            .is_some_and(|client| client.observer)
    }

    pub fn activated_client_ids(&self) -> Vec<i32> {
        self.clients
            .iter()
            .filter_map(|(&client_id, client)| client.activated.then_some(client_id))
            .collect()
    }

    /// Clone the live synchronized `C4ClientList` core values in client-ID
    /// order for JoinData/reference serialization.
    pub fn snapshot(&self) -> Vec<crate::ClientCoreControlData> {
        self.clients
            .iter()
            .map(|(&client_id, client)| crate::ClientCoreControlData {
                client_id,
                activated: client.activated,
                observer: client.observer,
                name: client.name.clone(),
                nick: client.nick.clone(),
                lobby_ready: client.lobby_ready,
            })
            .collect()
    }

    /// Apply the host-side admission rules from
    /// `C4Network2::HandleActivateReq` (`src/C4Network2.cpp:1553-1571`).
    #[allow(clippy::too_many_arguments)]
    pub fn activation_update_for_request(
        &self,
        client_id: i32,
        request_tick: i32,
        host_frame: i32,
        running: bool,
        waited_for: bool,
        ping_ms: i32,
        frames_per_second: i32,
    ) -> Option<crate::ClientUpdateControlData> {
        if client_id == 0 || !waited_for {
            return None;
        }
        let client = self.clients.get(&client_id)?;
        if client.observer || client.activated {
            return None;
        }
        if running {
            let lag_frames =
                (i64::from(ping_ms) * i64::from(frames_per_second) / 500).clamp(0, 100);
            let oldest_tick = i64::from(host_frame) - lag_frames - 20;
            if i64::from(request_tick) < oldest_tick {
                return None;
            }
        }
        Some(crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_ACTIVATE,
            client_id,
            data: 1,
            by_client: 0,
        })
    }
}

/// `C4PlayerInfoList`'s synchronized per-client player-info registry.
#[derive(Debug, Clone, Default)]
pub struct ControlPlayerInfoRegistry {
    clients: Vec<ClientPlayerInfos>,
    last_player_id: i32,
    issued_join_ids: HashSet<i32>,
    pending_reserved_join_ids: HashSet<i32>,
}

#[derive(Debug, Clone)]
struct ClientPlayerInfos {
    client_id: i32,
    flags: u32,
    players: Vec<ControlPlayerInfoEntry>,
}

/// Attribute resolution reached state unavailable from synchronized
/// `C4PlayerInfo`. Admission receives the process-random oracle explicitly;
/// only host-local `AlternateColorDw` remains unavailable there. The older
/// TeamColors-only path also uses this error for its conservative subset.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TeamColorUpdateError {
    #[error(
        "player {player_id} may need an unavailable host-local alternate color while resolving a conflict with player {other_player_id}"
    )]
    ConflictResolutionUnavailable {
        player_id: i32,
        other_player_id: i32,
    },
    #[error(
        "player {player_id} needs forced-name conflict resolution against player {other_player_id}"
    )]
    NameConflictResolutionUnavailable {
        player_id: i32,
        other_player_id: i32,
    },
}

/// The host-side result of admitting one `C4ClientPlayerInfos` packet.
///
/// `updated_existing` is the ordered `SendUpdatedPlayers` prefix: packets
/// already retained by the host that conflict resolution changed. `admitted`
/// remains separate until its own direct `CID_PlrInfo` executes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerInfoAdmission {
    pub updated_existing: Vec<PlayerInfoControlData>,
    pub admitted: PlayerInfoControlData,
    pub joined_player_team_updates: Vec<AdmittedPlayerTeamUpdate>,
}

/// Immediate live-player side effect of `C4Team::AddPlayer(..., true)` for
/// an incoming player-info row that had already joined the running game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmittedPlayerTeamUpdate {
    pub info_id: i32,
    pub team: i32,
    pub color: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedPlayerAttribute {
    Color,
    Name,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttributeBlockingPacket {
    Resolving(usize),
    Restore,
}

#[derive(Debug, Default)]
struct AttributeConflictMarks {
    current: bool,
    original: bool,
    alternate: bool,
    low_priority_original: Option<AttributeBlockingPacket>,
    low_priority_alternate: Option<AttributeBlockingPacket>,
    original_other_player_id: Option<i32>,
}

fn player_effective_name(player: &ControlPlayerInfoEntry) -> &[u8] {
    if !player.league_account.is_empty() {
        player.league_account.as_bytes()
    } else if !player.forced_name.is_empty() {
        player.forced_name.as_bytes()
    } else {
        player.name.as_bytes()
    }
}

fn c4_name_byte_capital(byte: u8) -> u8 {
    match byte {
        b'a'..=b'z' => byte - (b'a' - b'A'),
        0xe4 => 0xc4,
        0xf6 => 0xd6,
        0xfc => 0xdc,
        _ => byte,
    }
}

/// `SEqualNoCase`/`CharCapital` compare raw validated bytes and fold only
/// ASCII plus the three Latin-1 umlauts used by the original engine.
fn c4_names_equal(first: &[u8], second: &[u8]) -> bool {
    first.len() == second.len()
        && first
            .iter()
            .zip(second)
            .all(|(&first, &second)| c4_name_byte_capital(first) == c4_name_byte_capital(second))
}

fn generated_forced_player_name(original: &LegacyCString, suffix: i32) -> LegacyCString {
    let mut bytes = original.as_bytes().to_vec();
    bytes.extend_from_slice(format!(" ({suffix})").as_bytes());
    // SetForcedName uses VAL_NameAllowEmpty, whose only reachable change for
    // an already-validated original plus this ASCII suffix is C4MaxName's
    // byte-length truncation.
    bytes.truncate(30);
    LegacyCString::from_bytes(bytes)
        .expect("a validated player name plus an ASCII suffix contains no NUL")
}

fn generate_random_player_color(oracle: &mut impl InitialHostTeamAssignmentOracle) -> u32 {
    let mut channel = || oracle.safe_random(302).min(256) as u8;
    let red = channel();
    let green = channel();
    let blue = channel();
    // StdColors.h's RGB helper stores the first component in the low byte.
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16)
}

/// Build one `C4TeamList::GenerateDefaultTeams` entry from its caller-resolved
/// localized name, including the fixed color prefix and the process-random
/// conflict search used by `C4Team::RecheckColor` after that prefix is
/// exhausted.
pub fn generate_default_initial_team(
    id: i32,
    name: LegacyCString,
    existing_teams: &[InitialNetworkTeam],
    oracle: &mut impl InitialHostTeamAssignmentOracle,
) -> InitialNetworkTeam {
    let color = crate::default_generated_team_color(id).unwrap_or_else(|| {
        let mut candidate = 0;
        for _ in 1..100 {
            candidate = generate_random_player_color(oracle);
            if existing_teams
                .iter()
                .all(|team| !player_colors_conflict(team.color, candidate))
            {
                break;
            }
        }
        candidate
    });
    InitialNetworkTeam {
        id,
        name,
        player_start_index: 0,
        player_ids: Vec::new(),
        color,
        icon_spec: LegacyCString::default(),
        max_players: 0,
    }
}

impl ControlPlayerInfoRegistry {
    /// Replaces the complete player-info list and its raw allocation counter
    /// with the values compiled in JoinData.
    pub fn replace_snapshot(
        &mut self,
        last_player_id: i32,
        clients: impl IntoIterator<Item = PlayerInfoControlData>,
    ) {
        self.clients.clear();
        self.last_player_id = last_player_id;
        self.issued_join_ids.clear();
        self.pending_reserved_join_ids.clear();
        clients.into_iter().for_each(|client| self.apply(client));
    }

    /// Apply duplicate-resource filtering, ID allocation and slot pruning from
    /// the host's `HandlePlayerInfoUpdRequest` path. Nonzero IDs remain
    /// untouched exactly like `C4PlayerInfoList::AssignPlayerIDs`
    /// (src/C4Network2Players.cpp:166-191;
    /// src/C4PlayerInfo.cpp:781-807,1765-1775).
    pub fn admit_request(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
    ) -> Option<PlayerInfoControlData> {
        self.admit_request_with(request, max_players, |_| {})
    }

    /// Admits a remote runtime request and applies the host's Random-team
    /// assignment after allocating player IDs.
    pub fn admit_remote_request_with_runtime_teams(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: &mut InitialNetworkTeamMetadata,
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Option<PlayerInfoControlData> {
        self.admit_request_with(request, max_players, |players| {
            if matches!(
                teams.team_distribution,
                InitialNetworkTeamDistribution::Random
                    | InitialNetworkTeamDistribution::RandomInvisible
            ) {
                assign_initial_player_teams(teams, players, oracle, false, false);
            }
        })
    }

    /// Reject the synchronized shifted-color states whose missing host-local
    /// alternate could make a later admission non-transactional. This check
    /// deliberately runs before team assignment or process-random draws.
    ///
    /// It is conservative when another mutable current-color conflict could
    /// randomize before a shifted row is revisited: such rare packets are
    /// rejected even if one particular random sequence would avoid the
    /// shifted row's original color.
    fn preflight_player_attribute_admission(
        &self,
        incoming_players: &[ControlPlayerInfoEntry],
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        include_possible_team_colors: bool,
        alternate_color: &impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<(), TeamColorUpdateError> {
        let mut players = Vec::new();
        let mut existing_packet_ranges = Vec::new();
        for client in &self.clients {
            let start = players.len();
            players.extend(client.players.iter());
            existing_packet_ranges.push((start, players.len()));
        }
        let incoming_start = players.len();
        players.extend(incoming_players);
        let mut processing_rank = vec![usize::MAX; players.len()];
        let mut next_rank = 0;
        for player_index in incoming_start..players.len() {
            processing_rank[player_index] = next_rank;
            next_rank += 1;
        }
        for &(start, end) in existing_packet_ranges.iter().rev() {
            for player_index in start..end {
                processing_rank[player_index] = next_rank;
                next_rank += 1;
            }
        }
        let shifted_players = players
            .iter()
            .enumerate()
            .filter_map(|(player_index, &player)| {
                (Self::player_color_is_resolver_mutable(player, teams)
                    && player.color != player.original_color
                    && alternate_color(player).is_none())
                .then_some((player_index, player))
            })
            .collect::<Vec<_>>();
        if shifted_players.is_empty() {
            return Ok(());
        }
        let shifted_indices = shifted_players
            .iter()
            .map(|(player_index, _)| *player_index)
            .collect::<HashSet<_>>();
        for &(shifted_index, shifted) in &shifted_players {
            let mut original_marks = AttributeConflictMarks::default();
            for (other_index, &other) in players.iter().enumerate() {
                if other_index == shifted_index {
                    continue;
                }
                Self::mark_attribute_conflict(
                    &mut original_marks,
                    shifted,
                    usize::MAX,
                    usize::MAX,
                    other,
                    None,
                    AttributeBlockingPacket::Restore,
                    ResolvedPlayerAttribute::Color,
                    true,
                    None,
                );
            }
            for other in restore_players {
                Self::mark_attribute_conflict(
                    &mut original_marks,
                    shifted,
                    usize::MAX,
                    usize::MAX,
                    other,
                    None,
                    AttributeBlockingPacket::Restore,
                    ResolvedPlayerAttribute::Color,
                    true,
                    None,
                );
            }
            if original_marks.original
                || (shifted_players.len() > 1 && original_marks.low_priority_original.is_some())
            {
                return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                    player_id: shifted.id,
                    other_player_id: original_marks.original_other_player_id.unwrap_or(0),
                });
            }

            if include_possible_team_colors {
                if let Some(team_color_blocker) =
                    teams.filter(|teams| teams.team_colors).and_then(|teams| {
                        teams
                            .teams
                            .iter()
                            .find(|team| player_colors_conflict(shifted.original_color, team.color))
                    })
                {
                    return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                        player_id: shifted.id,
                        other_player_id: team_color_blocker
                            .player_ids
                            .first()
                            .copied()
                            .unwrap_or(0),
                    });
                }
            }
        }
        for (shifted_offset, (_, shifted)) in shifted_players.iter().enumerate() {
            if let Some((_, other)) =
                shifted_players
                    .iter()
                    .skip(shifted_offset + 1)
                    .find(|(_, other)| {
                        player_colors_conflict(shifted.original_color, other.original_color)
                    })
            {
                return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                    player_id: shifted.id,
                    other_player_id: other.id,
                });
            }
        }

        let randomization_conflict =
            players
                .iter()
                .enumerate()
                .find_map(|(player_index, &player)| {
                    if shifted_indices.contains(&player_index)
                        || !Self::player_color_is_resolver_mutable(player, teams)
                    {
                        return None;
                    }
                    players
                        .iter()
                        .enumerate()
                        .find(|(other_index, other)| {
                            *other_index != player_index
                                && (!shifted_indices.contains(other_index)
                                    || processing_rank[*other_index]
                                        >= processing_rank[player_index])
                                && (player.id == 0 || player.id != other.id)
                                && player_info_uses_color(other)
                                && player_colors_conflict(player.color, other.color)
                        })
                        .map(|(_, other)| other.id)
                        .or_else(|| {
                            restore_players
                                .iter()
                                .find(|other| {
                                    (player.id == 0 || player.id != other.id)
                                        && player_info_uses_color(other)
                                        && player_colors_conflict(player.color, other.color)
                                })
                                .map(|other| other.id)
                        })
                        .or_else(|| {
                            include_possible_team_colors
                                .then_some(teams)
                                .flatten()
                                .filter(|teams| teams.team_colors)
                                .and_then(|teams| {
                                    teams.teams.iter().find(|team| {
                                        player_colors_conflict(player.color, team.color)
                                    })
                                })
                                .map(|team| team.player_ids.first().copied().unwrap_or(0))
                        })
                });
        if let Some(other_player_id) = randomization_conflict {
            return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                player_id: shifted_players[0].1.id,
                other_player_id,
            });
        }
        Ok(())
    }

    fn preflight_unknown_generated_team_color(
        &self,
        incoming_players: &[ControlPlayerInfoEntry],
        teams: &InitialNetworkTeamMetadata,
    ) -> Result<(), TeamColorUpdateError> {
        if !teams.team_colors {
            return Ok(());
        }
        let shifted = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .chain(incoming_players)
            .find(|player| {
                Self::player_color_is_resolver_mutable(player, Some(teams))
                    && player.color != player.original_color
            });
        if let Some(shifted) = shifted {
            return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                player_id: shifted.id,
                other_player_id: 0,
            });
        }
        Ok(())
    }

    /// Host-side `C4Network2Players::UpdateSavegameAssignments` validation.
    ///
    /// Player IDs have already been allocated when native reaches this step.
    /// The reverse walk is observable: a later row wins an in-packet duplicate,
    /// and pruning uses `RemoveIndexedInfo`'s swap-with-last behavior rather
    /// than a stable erase. Retained registry rows always win, except that a
    /// replacement packet does not conflict with the packet it will replace.
    fn update_savegame_assignments(
        &self,
        admitted: &mut PlayerInfoControlData,
        restore_players: &[ControlPlayerInfoEntry],
    ) {
        let add_packet = admitted.flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0;
        let mut index = admitted.players.len();
        while index != 0 {
            index -= 1;
            let mut savegame_id = admitted.players[index].savegame_player;
            if savegame_id == 0 {
                continue;
            }

            if !restore_players
                .iter()
                .any(|restore| restore.id == savegame_id)
            {
                admitted.players[index].savegame_player = 0;
                admitted.flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
                savegame_id = 0;
            }

            if savegame_id != 0
                && admitted.players[index + 1..]
                    .iter()
                    .any(|player| player.savegame_player == savegame_id)
            {
                admitted.players[index].savegame_player = 0;
                admitted.flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
                savegame_id = 0;
            }

            if savegame_id != 0 {
                let duplicate = self.clients.iter().any(|client| {
                    if !add_packet && client.client_id == admitted.client_id {
                        return false;
                    }
                    client
                        .players
                        .iter()
                        .any(|player| player.savegame_player == savegame_id)
                });
                if duplicate {
                    admitted.players[index].savegame_player = 0;
                    admitted.flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
                    savegame_id = 0;
                }
            }

            if savegame_id == 0
                && admitted.players[index].flags & PLAYER_INFO_FLAG_SAVEGAME_JOIN != 0
            {
                admitted.players.swap_remove(index);
            }
        }
    }

    /// Transactionally allocate and resolve one admission whose team fields
    /// are already final. `None` supplies the no-team-list host path.
    ///
    /// On an unavailable host-local alternate-color branch, neither retained
    /// packets nor the player-ID high-water mark are changed.
    pub fn admit_request_with_attributes(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Result<Option<PlayerInfoAdmission>, TeamColorUpdateError> {
        self.admit_request_with_attributes_and_alternate_colors(
            request,
            max_players,
            teams,
            restore_players,
            oracle,
            |_| None,
        )
    }

    /// Host-local counterpart of [`Self::admit_request_with_attributes`].
    /// `dwAlternateColor` is not synchronized, so callers identify locally
    /// loaded rows and supply that sidecar value while admission is resolved.
    pub fn admit_request_with_attributes_and_alternate_colors(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<Option<PlayerInfoAdmission>, TeamColorUpdateError> {
        let mut updated_registry = self.clone();
        let Some(mut admitted) = updated_registry.admit_request(request, max_players) else {
            return Ok(None);
        };
        updated_registry.update_savegame_assignments(&mut admitted, restore_players);
        let admission = updated_registry.resolve_admitted_player_attributes_with_alternate_colors(
            admitted,
            teams,
            restore_players,
            oracle,
            alternate_color,
        )?;
        *self = updated_registry;
        Ok(Some(admission))
    }

    /// Compatibility wrapper for a client-authored post-lobby request.
    pub fn admit_remote_request_with_runtime_teams_and_attributes(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: &mut InitialNetworkTeamMetadata,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Result<Option<PlayerInfoAdmission>, TeamColorUpdateError> {
        self.admit_request_with_teams_and_attributes(
            request,
            max_players,
            teams,
            false,
            false,
            restore_players,
            oracle,
        )
    }

    /// Transactionally execute the native host admission order: allocate IDs,
    /// validate savegame associations, validate/assign teams for every active
    /// distribution, then update and resolve attributes. The registry and team
    /// list commit together.
    #[allow(clippy::too_many_arguments)]
    pub fn admit_request_with_teams_and_attributes(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: &mut InitialNetworkTeamMetadata,
        by_host: bool,
        has_or_will_have_lobby: bool,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Result<Option<PlayerInfoAdmission>, TeamColorUpdateError> {
        self.admit_request_with_teams_and_attributes_and_alternate_colors(
            request,
            max_players,
            teams,
            by_host,
            has_or_will_have_lobby,
            restore_players,
            oracle,
            |_| None,
        )
    }

    /// Host-local counterpart of
    /// [`Self::admit_request_with_teams_and_attributes`] retaining the
    /// unsynchronized `dwAlternateColor` sidecar through team assignment.
    #[allow(clippy::too_many_arguments)]
    pub fn admit_request_with_teams_and_attributes_and_alternate_colors(
        &mut self,
        request: PlayerInfoUpdateRequest,
        max_players: usize,
        teams: &mut InitialNetworkTeamMetadata,
        by_host: bool,
        has_or_will_have_lobby: bool,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<Option<PlayerInfoAdmission>, TeamColorUpdateError> {
        let mut updated_registry = self.clone();
        let mut updated_teams = teams.clone();
        let Some(mut admitted) = updated_registry.admit_request(request, max_players) else {
            return Ok(None);
        };
        updated_registry.update_savegame_assignments(&mut admitted, restore_players);
        let recheck_teams = updated_teams.active;
        let players_before_team_assignment = admitted.players.clone();
        let mut preflight_teams = updated_teams.clone();
        let mut preflight_players = admitted.players.clone();
        let mut preflight_oracle = TeamAssignmentPreflightOracle::default();
        if recheck_teams {
            assign_initial_player_teams(
                &mut preflight_teams,
                &mut preflight_players,
                &mut preflight_oracle,
                has_or_will_have_lobby,
                by_host,
            );
        }
        updated_registry.preflight_player_attribute_admission(
            &preflight_players,
            Some(&preflight_teams),
            restore_players,
            preflight_oracle.used_process_randomness(),
            &alternate_color,
        )?;
        if preflight_oracle.generated_team {
            updated_registry
                .preflight_unknown_generated_team_color(&preflight_players, &preflight_teams)?;
        }
        let automatically_assigned = if recheck_teams {
            assign_initial_player_teams(
                &mut updated_teams,
                &mut admitted.players,
                oracle,
                has_or_will_have_lobby,
                by_host,
            )
        } else {
            Vec::new()
        };
        let joined_player_team_updates = automatically_assigned
            .into_iter()
            .filter(|&player_index| players_before_team_assignment[player_index].is_joined())
            .map(|player_index| {
                let player = &admitted.players[player_index];
                AdmittedPlayerTeamUpdate {
                    info_id: player.id,
                    team: player.team,
                    color: updated_teams.team_colors.then_some(player.color),
                }
            })
            .collect();
        let mut admission = updated_registry
            .resolve_admitted_player_attributes_with_alternate_colors(
                admitted,
                Some(&updated_teams),
                restore_players,
                oracle,
                alternate_color,
            )?;
        admission.joined_player_team_updates = joined_player_team_updates;
        *self = updated_registry;
        *teams = updated_teams;
        Ok(Some(admission))
    }

    /// Reconciles each ordered team's membership against the complete player
    /// registry without changing player infos or team order.
    pub fn recheck_team_players(&self, teams: &mut InitialNetworkTeamMetadata) {
        // GetNextPlayerInfoByID starts at zero and repeatedly selects the
        // smallest greater ID, independent of client/packet storage order
        // (src/C4Teams.cpp:151-176; src/C4PlayerInfo.cpp:997-1009,1060-1074).
        let mut player_ids = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .map(|player| player.id)
            .filter(|&id| id > 0)
            .collect::<Vec<_>>();
        player_ids.sort_unstable();
        player_ids.dedup();

        for team in &mut teams.teams {
            let team_id = team.id;
            let is_eligible = |player_id| {
                player_id != 0
                    && self.get(player_id).is_some_and(|player| {
                        player.team == team_id && player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                    })
            };
            team.player_ids.retain(|&player_id| is_eligible(player_id));
            for player_id in player_ids.iter().copied().filter(|&id| is_eligible(id)) {
                if !team.player_ids.contains(&player_id) {
                    team.player_ids.push(player_id);
                }
            }
        }
    }

    /// Rebalances unissued players across automatic random teams.
    pub fn recheck_random_teams(
        &mut self,
        teams: &mut InitialNetworkTeamMetadata,
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Vec<PlayerInfoControlData> {
        if !matches!(
            teams.team_distribution,
            InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        ) {
            return Vec::new();
        }

        let generated_team_count = teams.random_team_count.max(2);
        if teams.auto_generate_teams
            && teams.teams.len() != usize::try_from(generated_team_count).unwrap_or(usize::MAX)
        {
            return self.reassign_all_teams(teams, oracle, true);
        }

        let mut touched_clients = HashSet::new();
        let team_count = if teams.random_team_count > 1 {
            usize::try_from(teams.random_team_count).unwrap_or(usize::MAX)
        } else {
            teams.teams.len()
        };
        while let Some(lowest_team) =
            initial_random_smallest_team(&teams.teams, true, teams.random_team_count, oracle)
        {
            let mut largest_team: Option<usize> = None;
            for index in 0..team_count.min(teams.teams.len()) {
                let is_larger = largest_team.is_none_or(|largest_index| {
                    teams.teams[index].player_ids.len()
                        > teams.teams[largest_index].player_ids.len()
                });
                if is_larger
                    && self
                        .first_unissued_team_player(&teams.teams[index])
                        .is_some()
                {
                    largest_team = Some(index);
                }
            }
            let Some(largest_team) = largest_team else {
                break;
            };
            if teams.teams[largest_team].player_ids.len()
                <= teams.teams[lowest_team].player_ids.len().saturating_add(1)
            {
                break;
            }
            let Some(player_id) = self.first_unissued_team_player(&teams.teams[largest_team])
            else {
                break;
            };
            let Some(player_index) = teams.teams[largest_team]
                .player_ids
                .iter()
                .position(|&id| id == player_id)
            else {
                break;
            };
            let target_team_id = teams.teams[lowest_team].id;
            let target_team_color = teams.teams[lowest_team].color;

            let Some((client_id, player)) = self.clients.iter_mut().find_map(|client| {
                let client_id = client.client_id;
                client
                    .players
                    .iter_mut()
                    .find(|player| player.id == player_id)
                    .map(|player| (client_id, player))
            }) else {
                break;
            };
            teams.teams[largest_team].player_ids.remove(player_index);
            teams.teams[lowest_team].player_ids.push(player_id);
            player.team = target_team_id;
            if teams.team_colors {
                player.color = target_team_color;
            }
            touched_clients.insert(client_id);
        }
        self.client_packets(&touched_clients)
    }

    /// Execute host-side `C4TeamList::ReassignAllTeams` after a synchronized
    /// distribution change. Player infos are visited by ascending positive
    /// ID, and every packet containing a resettable player is returned once
    /// in registry order even when its final values happen to match. That is
    /// the native `CIF_Updated`/`SendUpdatedPlayers` contract.
    pub fn reassign_all_teams(
        &mut self,
        teams: &mut InitialNetworkTeamMetadata,
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        has_or_will_have_lobby: bool,
    ) -> Vec<PlayerInfoControlData> {
        let mut touched_clients = HashSet::new();
        let issued_join_ids = &self.issued_join_ids;
        for client in &mut self.clients {
            for player in &mut client.players {
                if player.id <= 0
                    || issued_join_ids.contains(&player.id)
                    || player.flags & PLAYER_INFO_FLAG_JOINED != 0
                {
                    continue;
                }
                // Native marks the owning packet updated unconditionally,
                // even when the old team was already zero.
                player.team = 0;
                touched_clients.insert(client.client_id);
            }
        }

        // C4TeamList::RecheckPlayers removes reset/removed memberships before
        // any optional generated-team rebuild or reassignment.
        self.recheck_team_players(teams);

        let generated_team_count = teams.random_team_count.max(2);
        if matches!(
            teams.team_distribution,
            InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        ) && teams.auto_generate_teams
            && teams.teams.len() != usize::try_from(generated_team_count).unwrap_or(usize::MAX)
        {
            teams.teams.clear();
            teams.last_team_id = 0;
            initial_generate_teams_through(teams, generated_team_count, oracle);
        }

        let mut player_ids = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .map(|player| player.id)
            .collect::<Vec<_>>();
        player_ids.sort_unstable();
        player_ids.dedup();

        for player_id in player_ids {
            if self.issued_join_ids.contains(&player_id) {
                continue;
            }
            let Some(player) = self.get_mut(player_id) else {
                continue;
            };
            if player.flags & PLAYER_INFO_FLAG_JOINED != 0 {
                continue;
            }
            assign_initial_player_teams(
                teams,
                std::slice::from_mut(player),
                oracle,
                has_or_will_have_lobby,
                true,
            );
        }

        self.client_packets(&touched_clients)
    }

    /// Resolve attributes after a caller has allocated IDs and completed any
    /// team assignment that must precede `UpdatePlayerAttributes`. Existing
    /// retained packets commit only after the complete resolver succeeds;
    /// the admitted packet remains separate in the returned result.
    pub fn resolve_admitted_player_attributes(
        &mut self,
        admitted: PlayerInfoControlData,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Result<PlayerInfoAdmission, TeamColorUpdateError> {
        self.resolve_admitted_player_attributes_with_alternate_colors(
            admitted,
            teams,
            restore_players,
            oracle,
            |_| None,
        )
    }

    /// Initial local host players retain `C4PlayerInfo::dwAlternateColor`
    /// outside the synchronized player-info row. The callback resolves that
    /// process-local value from a stable row identity (normally the player
    /// resource ID) while the regular wire model remains unchanged.
    pub fn resolve_admitted_player_attributes_with_alternate_colors(
        &mut self,
        mut admitted: PlayerInfoControlData,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<PlayerInfoAdmission, TeamColorUpdateError> {
        let mut admitted_forced_color_changed = false;
        for player in &mut admitted.players {
            if player.flags & PLAYER_INFO_FLAG_JOINED != 0 {
                continue;
            }
            let restore_color = (player.savegame_player != 0)
                .then(|| {
                    restore_players
                        .iter()
                        .find(|restore| restore.id == player.savegame_player)
                        .map(|restore| restore.color)
                })
                .flatten();
            let team_color = teams
                .filter(|teams| teams.team_colors)
                .and_then(|teams| teams.teams.iter().find(|team| team.id == player.team))
                .map(|team| team.color);
            if let Some(color) = restore_color.or(team_color) {
                admitted_forced_color_changed |= player.color != color;
                player.color = color;
            }
        }
        self.preflight_player_attribute_admission(
            &admitted.players,
            teams,
            restore_players,
            false,
            &alternate_color,
        )?;

        let existing_count = self.clients.len();
        let mut resolving_packets = self.clients.clone();
        resolving_packets.push(ClientPlayerInfos {
            client_id: admitted.client_id,
            flags: admitted.flags,
            players: admitted.players,
        });
        let resolver_updated_packets = Self::resolve_player_attribute_conflicts(
            &mut resolving_packets,
            existing_count,
            Some(existing_count),
            teams,
            restore_players,
            oracle,
            &alternate_color,
        )?;

        let resolved_admitted = resolving_packets
            .pop()
            .expect("the admitted packet was appended to the resolver");
        let touched_clients = resolving_packets
            .iter()
            .enumerate()
            .filter_map(|(packet_index, packet)| {
                resolver_updated_packets
                    .contains(&packet_index)
                    .then_some(packet.client_id)
            })
            .collect::<HashSet<_>>();
        self.clients = resolving_packets;
        let updated_existing = self.client_packets(&touched_clients);

        admitted.players = resolved_admitted.players;
        if admitted_forced_color_changed || resolver_updated_packets.contains(&existing_count) {
            admitted.flags |= CLIENT_PLAYER_INFO_FLAG_UPDATED;
        }
        Ok(PlayerInfoAdmission {
            updated_existing,
            admitted,
            joined_player_team_updates: Vec::new(),
        })
    }

    /// Re-run the host's parameterless `C4PlayerInfoList::UpdatePlayerAttributes`
    /// over every retained packet. Forced restore/team colors are installed
    /// first, then the complete packet list is conflict-resolved in reverse
    /// storage order. The registry commits only after resolution succeeds.
    pub fn refresh_player_attributes(
        &mut self,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) -> Result<Vec<PlayerInfoControlData>, TeamColorUpdateError> {
        self.refresh_player_attributes_with_alternate_colors(teams, restore_players, oracle, |_| {
            None
        })
    }

    /// Host-local counterpart of [`Self::refresh_player_attributes`]. Every
    /// retained packet is revisited, so the callback must cover previously
    /// loaded local rows as well as the packet that triggered the refresh.
    pub fn refresh_player_attributes_with_alternate_colors(
        &mut self,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<Vec<PlayerInfoControlData>, TeamColorUpdateError> {
        let mut updated_clients = self.clients.clone();
        let mut updated_packets = HashSet::new();
        for (client_index, client) in updated_clients.iter_mut().enumerate() {
            for player in &mut client.players {
                if player.flags & PLAYER_INFO_FLAG_JOINED != 0 {
                    continue;
                }
                let restore_color = (player.savegame_player != 0)
                    .then(|| {
                        restore_players
                            .iter()
                            .find(|restore| restore.id == player.savegame_player)
                            .map(|restore| restore.color)
                    })
                    .flatten();
                let team_color = teams
                    .filter(|teams| teams.team_colors)
                    .and_then(|teams| teams.teams.iter().find(|team| team.id == player.team))
                    .map(|team| team.color);
                if let Some(color) = restore_color.or(team_color) {
                    if player.color != color {
                        updated_packets.insert(client_index);
                    }
                    player.color = color;
                }
            }
        }

        let primary_count = updated_clients.len();
        updated_packets.extend(Self::resolve_player_attribute_conflicts(
            &mut updated_clients,
            primary_count,
            None,
            teams,
            restore_players,
            oracle,
            &alternate_color,
        )?);
        let touched_clients = updated_clients
            .iter()
            .enumerate()
            .filter_map(|(client_index, client)| {
                updated_packets
                    .contains(&client_index)
                    .then_some(client.client_id)
            })
            .collect::<HashSet<_>>();
        self.clients = updated_clients;
        Ok(self.client_packets(&touched_clients))
    }

    fn resolve_player_attribute_conflicts(
        packets: &mut [ClientPlayerInfos],
        primary_count: usize,
        secondary_index: Option<usize>,
        teams: Option<&InitialNetworkTeamMetadata>,
        restore_players: &[ControlPlayerInfoEntry],
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: &impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<HashSet<usize>, TeamColorUpdateError> {
        let mut check_packets = (0..primary_count).collect::<Vec<_>>();
        if let Some(secondary_index) = secondary_index {
            check_packets.push(secondary_index);
        }
        let mut updated_packets = HashSet::new();

        while let Some(packet_index) = check_packets.pop() {
            let packet_players_before = packets[packet_index].players.clone();
            let player_count = packets[packet_index].players.len();
            for player_index in 0..player_count {
                if packets[packet_index].players[player_index].flags & PLAYER_INFO_FLAG_JOINED != 0
                {
                    continue;
                }

                let attributes_fixed = packets[packet_index].players[player_index].flags
                    & PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                    != 0;
                if !attributes_fixed
                    && packets[packet_index].players[player_index].savegame_player == 0
                    && !Self::player_has_forced_team_color(
                        &packets[packet_index].players[player_index],
                        teams,
                    )
                {
                    Self::resolve_player_color(
                        packets,
                        primary_count,
                        secondary_index,
                        restore_players,
                        packet_index,
                        player_index,
                        &mut check_packets,
                        oracle,
                        alternate_color,
                    )?;
                }
                if !attributes_fixed
                    && packets[packet_index].players[player_index]
                        .league_account
                        .is_empty()
                {
                    Self::resolve_player_name(
                        packets,
                        primary_count,
                        secondary_index,
                        restore_players,
                        packet_index,
                        player_index,
                        &mut check_packets,
                    );
                }
            }
            if packets[packet_index].players != packet_players_before {
                updated_packets.insert(packet_index);
            }
        }
        Ok(updated_packets)
    }

    fn player_has_forced_team_color(
        player: &ControlPlayerInfoEntry,
        teams: Option<&InitialNetworkTeamMetadata>,
    ) -> bool {
        teams.is_some_and(|teams| {
            teams.team_colors && teams.teams.iter().any(|team| team.id == player.team)
        })
    }

    fn player_color_is_resolver_mutable(
        player: &ControlPlayerInfoEntry,
        teams: Option<&InitialNetworkTeamMetadata>,
    ) -> bool {
        player.flags & (PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED) == 0
            && player.savegame_player == 0
            && !Self::player_has_forced_team_color(player, teams)
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_player_color(
        packets: &mut [ClientPlayerInfos],
        primary_count: usize,
        secondary_index: Option<usize>,
        restore_players: &[ControlPlayerInfoEntry],
        packet_index: usize,
        player_index: usize,
        check_packets: &mut Vec<usize>,
        oracle: &mut impl InitialHostTeamAssignmentOracle,
        alternate_color: &impl Fn(&ControlPlayerInfoEntry) -> Option<u32>,
    ) -> Result<(), TeamColorUpdateError> {
        let mut tries = 0;
        loop {
            let alternate = alternate_color(&packets[packet_index].players[player_index]);
            let marks = Self::mark_attribute_conflicts(
                packets,
                primary_count,
                secondary_index,
                restore_players,
                packet_index,
                player_index,
                ResolvedPlayerAttribute::Color,
                tries == 0,
                alternate,
            );
            let (current, original, player_id) = {
                let player = &packets[packet_index].players[player_index];
                (player.color, player.original_color, player.id)
            };

            if tries == 0 && current != original {
                if !marks.original {
                    packets[packet_index].players[player_index].color = original;
                    Self::readd_blocking_packet(check_packets, marks.low_priority_original);
                    break;
                }
                match alternate {
                    Some(alternate)
                        if alternate != 0 && current != alternate && !marks.alternate =>
                    {
                        packets[packet_index].players[player_index].color = alternate;
                        Self::readd_blocking_packet(check_packets, marks.low_priority_alternate);
                        break;
                    }
                    Some(_) => {
                        // The exact host-local value is known to be absent,
                        // already current, or blocked. Native keeps a free
                        // current color and otherwise enters its random loop.
                    }
                    None => {
                        return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                            player_id,
                            other_player_id: marks.original_other_player_id.unwrap_or(0),
                        });
                    }
                }
            }
            if !marks.current {
                break;
            }
            tries += 1;
            if tries > 100 {
                packets[packet_index].players[player_index].color = original;
                break;
            }
            packets[packet_index].players[player_index].color =
                generate_random_player_color(oracle);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn resolve_player_name(
        packets: &mut [ClientPlayerInfos],
        primary_count: usize,
        secondary_index: Option<usize>,
        restore_players: &[ControlPlayerInfoEntry],
        packet_index: usize,
        player_index: usize,
        check_packets: &mut Vec<usize>,
    ) {
        let mut tries = 0;
        let mut original_conflict = false;
        loop {
            let marks = Self::mark_attribute_conflicts(
                packets,
                primary_count,
                secondary_index,
                restore_players,
                packet_index,
                player_index,
                ResolvedPlayerAttribute::Name,
                tries == 0,
                None,
            );
            if tries == 0 {
                original_conflict = marks.original;
            }
            let player = &packets[packet_index].players[player_index];
            if !c4_names_equal(player_effective_name(player), player.name.as_bytes())
                && !original_conflict
            {
                packets[packet_index].players[player_index].forced_name = LegacyCString::default();
                Self::readd_blocking_packet(check_packets, marks.low_priority_original);
                break;
            }
            if !marks.current {
                break;
            }
            tries += 1;
            if tries > 100 {
                break;
            }
            let original = packets[packet_index].players[player_index].name.clone();
            packets[packet_index].players[player_index].forced_name =
                generated_forced_player_name(&original, tries + 1);
        }
    }

    fn readd_blocking_packet(
        check_packets: &mut Vec<usize>,
        blocking_packet: Option<AttributeBlockingPacket>,
    ) {
        let Some(AttributeBlockingPacket::Resolving(packet_index)) = blocking_packet else {
            return;
        };
        if !check_packets.contains(&packet_index) {
            check_packets.push(packet_index);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mark_attribute_conflicts(
        packets: &[ClientPlayerInfos],
        primary_count: usize,
        secondary_index: Option<usize>,
        restore_players: &[ControlPlayerInfoEntry],
        resolve_packet_index: usize,
        resolve_player_index: usize,
        attribute: ResolvedPlayerAttribute,
        test_original: bool,
        alternate_color: Option<u32>,
    ) -> AttributeConflictMarks {
        let resolve = &packets[resolve_packet_index].players[resolve_player_index];
        let mut marks = AttributeConflictMarks::default();

        for packet_index in 0..primary_count {
            for (player_index, check) in packets[packet_index].players.iter().enumerate() {
                Self::mark_attribute_conflict(
                    &mut marks,
                    resolve,
                    resolve_packet_index,
                    resolve_player_index,
                    check,
                    Some((packet_index, player_index)),
                    AttributeBlockingPacket::Resolving(packet_index),
                    attribute,
                    test_original,
                    alternate_color,
                );
            }
        }
        if attribute == ResolvedPlayerAttribute::Color {
            for check in restore_players {
                Self::mark_attribute_conflict(
                    &mut marks,
                    resolve,
                    resolve_packet_index,
                    resolve_player_index,
                    check,
                    None,
                    AttributeBlockingPacket::Restore,
                    attribute,
                    test_original,
                    alternate_color,
                );
            }
        }
        if let Some(secondary_index) = secondary_index {
            for (player_index, check) in packets[secondary_index].players.iter().enumerate() {
                Self::mark_attribute_conflict(
                    &mut marks,
                    resolve,
                    resolve_packet_index,
                    resolve_player_index,
                    check,
                    Some((secondary_index, player_index)),
                    AttributeBlockingPacket::Resolving(secondary_index),
                    attribute,
                    test_original,
                    alternate_color,
                );
            }
        }
        marks
    }

    #[allow(clippy::too_many_arguments)]
    fn mark_attribute_conflict(
        marks: &mut AttributeConflictMarks,
        resolve: &ControlPlayerInfoEntry,
        resolve_packet_index: usize,
        resolve_player_index: usize,
        check: &ControlPlayerInfoEntry,
        check_location: Option<(usize, usize)>,
        check_packet: AttributeBlockingPacket,
        attribute: ResolvedPlayerAttribute,
        test_original: bool,
        alternate_color: Option<u32>,
    ) {
        if check_location == Some((resolve_packet_index, resolve_player_index))
            || (resolve.id != 0 && resolve.id == check.id)
            || !Self::player_uses_attribute(check, attribute)
        {
            return;
        }

        let resolve_has_higher_priority =
            Self::player_attribute_priority_difference(resolve, check) > 0;
        if !resolve_has_higher_priority
            && Self::players_have_current_attribute_conflict(resolve, check, attribute)
        {
            marks.current = true;
        }
        if !test_original {
            return;
        }

        if Self::players_have_original_attribute_conflict(resolve, check, attribute) {
            marks.original_other_player_id.get_or_insert(check.id);
            if resolve_has_higher_priority
                && !marks.original
                && marks.low_priority_original.is_none()
            {
                marks.low_priority_original = Some(check_packet);
            } else {
                marks.low_priority_original = None;
                marks.original = true;
            }
        }

        if attribute == ResolvedPlayerAttribute::Color
            && alternate_color.is_some_and(|alternate| {
                alternate != 0 && player_colors_conflict(check.color, alternate)
            })
        {
            if resolve_has_higher_priority
                && !marks.alternate
                && marks.low_priority_alternate.is_none()
            {
                marks.low_priority_alternate = Some(check_packet);
            } else {
                marks.low_priority_alternate = None;
                marks.alternate = true;
            }
        }
    }

    fn player_uses_attribute(
        player: &ControlPlayerInfoEntry,
        attribute: ResolvedPlayerAttribute,
    ) -> bool {
        if player.flags & PLAYER_INFO_FLAG_REMOVED != 0 {
            return false;
        }
        match attribute {
            ResolvedPlayerAttribute::Color => player.savegame_player == 0,
            ResolvedPlayerAttribute::Name => player.league_account.is_empty(),
        }
    }

    fn player_attribute_priority_difference(
        first: &ControlPlayerInfoEntry,
        second: &ControlPlayerInfoEntry,
    ) -> i32 {
        if first.is_joined() {
            return i32::from(!second.is_joined());
        }
        if second.is_joined() {
            return -1;
        }
        let type_difference = i32::from(second.player_type) - i32::from(first.player_type);
        if type_difference != 0 {
            return type_difference;
        }
        first.league_score.wrapping_sub(second.league_score)
    }

    fn players_have_current_attribute_conflict(
        first: &ControlPlayerInfoEntry,
        second: &ControlPlayerInfoEntry,
        attribute: ResolvedPlayerAttribute,
    ) -> bool {
        match attribute {
            ResolvedPlayerAttribute::Color => player_colors_conflict(first.color, second.color),
            ResolvedPlayerAttribute::Name => {
                c4_names_equal(player_effective_name(first), player_effective_name(second))
            }
        }
    }

    fn players_have_original_attribute_conflict(
        resolve: &ControlPlayerInfoEntry,
        check: &ControlPlayerInfoEntry,
        attribute: ResolvedPlayerAttribute,
    ) -> bool {
        match attribute {
            ResolvedPlayerAttribute::Color => {
                player_colors_conflict(check.color, resolve.original_color)
            }
            ResolvedPlayerAttribute::Name => {
                c4_names_equal(player_effective_name(check), resolve.name.as_bytes())
            }
        }
    }

    /// Apply the exactly modelled portion of
    /// `C4PlayerInfoList::UpdatePlayerAttributes` for a TeamColors toggle.
    ///
    /// Joined players stay fixed. Unjoined savegame associations take their
    /// restore color first; otherwise an enabled, existing team forces its
    /// color. For ordinary unforced players the native resolver restores the
    /// original color when it is conflict-free. If that original still
    /// conflicts, native needs host-local `AlternateColorDw` or random color
    /// generation, neither of which exists in the synchronized registry, so
    /// this method fails before mutating live state.
    pub fn update_team_colors(
        &mut self,
        teams: &InitialNetworkTeamMetadata,
        enabled: bool,
        restore_players: &[ControlPlayerInfoEntry],
    ) -> Result<Vec<PlayerInfoControlData>, TeamColorUpdateError> {
        let mut updated_clients = self.clients.clone();

        for client in &mut updated_clients {
            for player in &mut client.players {
                if player.flags & PLAYER_INFO_FLAG_JOINED != 0 {
                    continue;
                }
                let restore_color = (player.savegame_player != 0)
                    .then(|| {
                        restore_players
                            .iter()
                            .find(|restore| restore.id == player.savegame_player)
                            .map(|restore| restore.color)
                    })
                    .flatten();
                // A missing associated restore row is not an error. Native
                // simply leaves the restore force unset and then tries the
                // ordinary enabled team-color force.
                let forced_color = restore_color.or_else(|| {
                    enabled
                        .then(|| {
                            teams
                                .teams
                                .iter()
                                .find(|team| team.id == player.team)
                                .map(|team| team.color)
                        })
                        .flatten()
                });
                if let Some(color) = forced_color {
                    player.color = color;
                }
            }
        }

        let mut resolver_players = Vec::new();
        for (client_index, client) in updated_clients.iter().enumerate() {
            for (player_index, player) in client.players.iter().enumerate() {
                let forced_team_color =
                    enabled && teams.teams.iter().any(|team| team.id == player.team);
                if player.flags & (PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED) == 0
                    && player.savegame_player == 0
                    && !forced_team_color
                {
                    resolver_players.push((client_index, player_index));
                }
            }
        }

        // Native tests an original candidate against the other players'
        // current colors before it changes anything. Ordered cases where a
        // current color blocks a candidate may later resolve differently, so
        // conservatively reject them instead of applying all originals at
        // once and diverging from that traversal.
        for &(client_index, player_index) in &resolver_players {
            let player = &updated_clients[client_index].players[player_index];
            let player_id = player.id;
            let candidate = player.original_color;
            let current_conflict = updated_clients
                .iter()
                .enumerate()
                .flat_map(|(other_client_index, client)| {
                    client.players.iter().enumerate().filter_map(
                        move |(other_player_index, other)| {
                            ((other_client_index, other_player_index)
                                != (client_index, player_index)
                                && (player_id == 0 || other.id != player_id)
                                && player_info_uses_color(other)
                                && player_colors_conflict(candidate, other.color))
                            .then_some(other.id)
                        },
                    )
                })
                .chain(restore_players.iter().filter_map(|other| {
                    ((player_id == 0 || other.id != player_id)
                        && player_info_uses_color(other)
                        && player_colors_conflict(candidate, other.color))
                    .then_some(other.id)
                }))
                .next();
            if let Some(other_player_id) = current_conflict {
                return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                    player_id,
                    other_player_id,
                });
            }
        }

        // Every candidate is free against the current state. Install all
        // originals, then also reject conflicts among the candidate results.
        for &(client_index, player_index) in &resolver_players {
            let player = &mut updated_clients[client_index].players[player_index];
            player.color = player.original_color;
        }
        for &(client_index, player_index) in &resolver_players {
            let player = &updated_clients[client_index].players[player_index];
            let player_id = player.id;
            let color = player.color;
            let current_conflict = updated_clients
                .iter()
                .enumerate()
                .flat_map(|(other_client_index, client)| {
                    client.players.iter().enumerate().filter_map(
                        move |(other_player_index, other)| {
                            ((other_client_index, other_player_index)
                                != (client_index, player_index)
                                && (player_id == 0 || other.id != player_id)
                                && player_info_uses_color(other)
                                && player_colors_conflict(color, other.color))
                            .then_some(other.id)
                        },
                    )
                })
                .chain(restore_players.iter().filter_map(|other| {
                    ((player_id == 0 || other.id != player_id)
                        && player_info_uses_color(other)
                        && player_colors_conflict(color, other.color))
                    .then_some(other.id)
                }))
                .next();
            if let Some(other_player_id) = current_conflict {
                return Err(TeamColorUpdateError::ConflictResolutionUnavailable {
                    player_id,
                    other_player_id,
                });
            }
        }

        // The same native resolver always checks names after colors. On the
        // exactly modelled path, every mutable non-league player can return
        // to its original name, represented by clearing ForcedName. Install
        // all candidates first, then reject any original-name collision
        // before committing the cloned registry. Colliding originals require
        // the resolver's ordered "Name (n)" cascade, which is intentionally
        // outside this synchronized subset.
        let mut name_resolver_players = Vec::new();
        for (client_index, client) in updated_clients.iter().enumerate() {
            for (player_index, player) in client.players.iter().enumerate() {
                if player.flags & (PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED) == 0
                    && player.league_account.is_empty()
                {
                    name_resolver_players.push((client_index, player_index));
                }
            }
        }
        // Names have the same ordered-current hazard as colors. An original
        // blocked by another current effective name needs the native ordered
        // ForcedName cascade, so reject that state transactionally.
        for &(client_index, player_index) in &name_resolver_players {
            let player = &updated_clients[client_index].players[player_index];
            let player_id = player.id;
            let original_name = player.name.as_bytes();
            let conflict = updated_clients
                .iter()
                .enumerate()
                .flat_map(|(other_client_index, client)| {
                    client.players.iter().enumerate().filter_map(
                        move |(other_player_index, other)| {
                            let other_name = if !other.forced_name.is_empty() {
                                other.forced_name.as_bytes()
                            } else {
                                other.name.as_bytes()
                            };
                            ((other_client_index, other_player_index)
                                != (client_index, player_index)
                                && (player_id == 0 || other.id != player_id)
                                && other.flags & PLAYER_INFO_FLAG_REMOVED == 0
                                && other.league_account.is_empty()
                                && original_name.eq_ignore_ascii_case(other_name))
                            .then_some(other.id)
                        },
                    )
                })
                .next();
            if let Some(other_player_id) = conflict {
                return Err(TeamColorUpdateError::NameConflictResolutionUnavailable {
                    player_id,
                    other_player_id,
                });
            }
        }
        for &(client_index, player_index) in &name_resolver_players {
            updated_clients[client_index].players[player_index].forced_name =
                LegacyCString::default();
        }
        for &(client_index, player_index) in &name_resolver_players {
            let player = &updated_clients[client_index].players[player_index];
            let player_id = player.id;
            let original_name = player.name.as_bytes();
            let conflict = updated_clients
                .iter()
                .enumerate()
                .flat_map(|(other_client_index, client)| {
                    client.players.iter().enumerate().filter_map(
                        move |(other_player_index, other)| {
                            let other_name = if !other.forced_name.is_empty() {
                                other.forced_name.as_bytes()
                            } else {
                                other.name.as_bytes()
                            };
                            ((other_client_index, other_player_index)
                                != (client_index, player_index)
                                && (player_id == 0 || other.id != player_id)
                                && other.flags & PLAYER_INFO_FLAG_REMOVED == 0
                                && other.league_account.is_empty()
                                && original_name.eq_ignore_ascii_case(other_name))
                            .then_some(other.id)
                        },
                    )
                })
                .next();
            if let Some(other_player_id) = conflict {
                return Err(TeamColorUpdateError::NameConflictResolutionUnavailable {
                    player_id,
                    other_player_id,
                });
            }
        }

        let touched_clients = self
            .clients
            .iter()
            .zip(&updated_clients)
            .filter_map(|(before, after)| {
                (before.players != after.players).then_some(after.client_id)
            })
            .collect::<HashSet<_>>();
        self.clients = updated_clients;
        Ok(self.client_packets(&touched_clients))
    }

    /// Clone the selected retained client packets in registry order for a
    /// host-authored `SendUpdatedPlayers` flush. The host-local Updated marker
    /// is cleared before the packet is copied, matching the native wire form.
    pub fn client_packets(&self, client_ids: &HashSet<i32>) -> Vec<PlayerInfoControlData> {
        self.clients
            .iter()
            .filter(|client| client_ids.contains(&client.client_id))
            .map(|client| PlayerInfoControlData {
                client_id: client.client_id,
                // SendUpdatedPlayers resets the host-local marker before it
                // constructs the direct control packet.
                flags: client.flags & !CLIENT_PLAYER_INFO_FLAG_UPDATED,
                players: client.players.clone(),
                by_client: 0,
            })
            .collect()
    }

    /// Admits a request and exposes its retained, ID-assigned players before
    /// the synchronized control packet is built.
    pub fn admit_request_with(
        &mut self,
        mut request: PlayerInfoUpdateRequest,
        max_players: usize,
        assign_players: impl FnOnce(&mut [ControlPlayerInfoEntry]),
    ) -> Option<PlayerInfoControlData> {
        if request.players.is_empty() && request.flags & CLIENT_PLAYER_INFO_FLAG_INITIAL == 0 {
            return None;
        }
        if request.flags & CLIENT_PLAYER_INFO_FLAG_INITIAL == 0 {
            if let Some(existing) = self
                .clients
                .iter()
                .find(|client| client.client_id == request.client_id)
            {
                // Native scans backward and RemoveIndexedInfo swaps the last
                // row into the removed slot. Preserve that observable order
                // before IDs, slots, teams, or attributes are assigned.
                let mut index = request.players.len();
                while index != 0 {
                    index -= 1;
                    let player = &request.players[index];
                    let is_duplicate = player.id == 0
                        && player.resource.as_ref().is_some_and(|resource| {
                            existing.players.iter().any(|existing_player| {
                                existing_player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                                    && existing_player.resource.as_ref().is_some_and(
                                        |existing_resource| existing_resource.id == resource.id,
                                    )
                            })
                        });
                    if is_duplicate {
                        request.players.swap_remove(index);
                    }
                }
            }
            if request.players.is_empty() {
                return None;
            }
        }
        let startup_count = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .filter(|player| player.flags & PLAYER_INFO_FLAG_REMOVED == 0)
            .count();
        let free_slots = max_players.saturating_sub(startup_count);
        let mut joins_granted = 0usize;
        request.players.retain_mut(|player| {
            if player.id != 0 {
                return true;
            }
            if joins_granted >= free_slots {
                return false;
            }
            self.last_player_id = self.last_player_id.wrapping_add(1);
            player.id = self.last_player_id;
            joins_granted += 1;
            true
        });
        if request.players.is_empty() && request.flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0 {
            return None;
        }
        assign_players(&mut request.players);
        Some(PlayerInfoControlData {
            client_id: request.client_id,
            flags: request.flags,
            players: request.players,
            by_client: 0,
        })
    }

    pub fn apply(&mut self, info: PlayerInfoControlData) {
        let PlayerInfoControlData {
            client_id,
            flags,
            players,
            ..
        } = info;
        if let Some(existing) = self
            .clients
            .iter_mut()
            .find(|client| client.client_id == client_id)
        {
            if flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0 {
                existing.players.extend(players);
            } else {
                existing.flags = flags & !CLIENT_PLAYER_INFO_FLAG_UPDATED;
                existing.players = players;
            }
        } else {
            // C4PlayerInfoList::AddInfo clears CIF_AddPlayers when a packet
            // establishes a new client row, and HandlePlayerInfo clears the
            // transient CIF_Updated bit after applying the control.
            self.clients.push(ClientPlayerInfos {
                client_id,
                flags: flags
                    & !(CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_UPDATED),
                players,
            });
        }
    }

    /// Conditionally applies the host session's post-broadcast resource
    /// normalization without replaying AddPlayers or replacing a client row.
    /// A later preexecuted packet wins when it has already changed that row's
    /// resource state.
    pub fn apply_player_resource_normalization(
        &mut self,
        original: &PlayerInfoControlData,
        echoed: &PlayerInfoControlData,
    ) -> bool {
        if original.client_id != echoed.client_id {
            return false;
        }
        let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.client_id == original.client_id)
        else {
            return false;
        };
        let mut changed = false;
        for echoed_player in &echoed.players {
            let Some(original_player) = original
                .players
                .iter()
                .find(|original_player| original_player.id == echoed_player.id)
            else {
                continue;
            };
            let Some(retained) = client
                .players
                .iter_mut()
                .find(|retained| retained.id == echoed_player.id)
            else {
                continue;
            };
            if retained != original_player {
                continue;
            }
            let normalized_resource_flag = echoed_player.flags & PLAYER_INFO_FLAG_HAS_RESOURCE;
            if retained.flags & PLAYER_INFO_FLAG_HAS_RESOURCE != normalized_resource_flag {
                retained.flags =
                    (retained.flags & !PLAYER_INFO_FLAG_HAS_RESOURCE) | normalized_resource_flag;
                changed = true;
            }
            if retained.resource != echoed_player.resource {
                retained.resource.clone_from(&echoed_player.resource);
                changed = true;
            }
        }
        changed
    }

    pub fn issue_unjoined_players(
        &mut self,
        client_id: i32,
        path_for_resource: impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
    ) -> Vec<JoinPlayerControlData> {
        let Some(client) = self
            .clients
            .iter()
            .find(|client| client.client_id == client_id)
        else {
            return Vec::new();
        };
        Self::issue_unjoined_player_rows(
            &mut self.issued_join_ids,
            client_id,
            &client.players,
            path_for_resource,
        )
    }

    /// Queues unjoined players from the exact ordered PlayerInfo view that
    /// existed at an earlier direct-control boundary. The current retained
    /// client packet may already have been replaced or removed.
    pub fn issue_unjoined_player_snapshots(
        &mut self,
        client_id: i32,
        players: &[ControlPlayerInfoEntry],
        path_for_resource: impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
    ) -> Vec<JoinPlayerControlData> {
        Self::issue_unjoined_player_rows(
            &mut self.issued_join_ids,
            client_id,
            players,
            path_for_resource,
        )
    }

    /// Marks the exact unjoined rows captured for a delayed direct-control
    /// echo as JoinIssued before any later team recheck can relocate them.
    pub fn reserve_unjoined_player_snapshots(&mut self, players: &[ControlPlayerInfoEntry]) {
        for player in players {
            if player.flags & PLAYER_INFO_FLAG_JOINED == 0
                && player.savegame_player == 0
                && self.issued_join_ids.insert(player.id)
            {
                self.pending_reserved_join_ids.insert(player.id);
            }
        }
    }

    /// Consumes delayed-echo reservations in supplied player-info order.
    /// Reserved IDs are already in `issued_join_ids`, so this path deliberately
    /// bypasses the ordinary duplicate-issuance gate.
    pub fn issue_reserved_player_snapshots(
        &mut self,
        client_id: i32,
        players: &[ControlPlayerInfoEntry],
        mut path_for_resource: impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
    ) -> Vec<JoinPlayerControlData> {
        players
            .iter()
            .filter_map(|player| {
                self.pending_reserved_join_ids
                    .remove(&player.id)
                    .then(|| {
                        Self::join_player_from_snapshot(client_id, player, &mut path_for_resource)
                    })
                    .flatten()
            })
            .collect()
    }

    fn issue_unjoined_player_rows(
        issued_join_ids: &mut HashSet<i32>,
        client_id: i32,
        players: &[ControlPlayerInfoEntry],
        mut path_for_resource: impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
    ) -> Vec<JoinPlayerControlData> {
        players
            .iter()
            .filter_map(|player| {
                if player.flags & PLAYER_INFO_FLAG_JOINED != 0
                    || player.savegame_player != 0
                    || issued_join_ids.contains(&player.id)
                {
                    return None;
                }
                // C++ sets PIF_JoinIssued before validating the resource so a
                // failed fileless user join is not retried forever.
                issued_join_ids.insert(player.id);
                Self::join_player_from_snapshot(client_id, player, &mut path_for_resource)
            })
            .collect()
    }

    fn join_player_from_snapshot(
        client_id: i32,
        player: &ControlPlayerInfoEntry,
        path_for_resource: &mut impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
    ) -> Option<JoinPlayerControlData> {
        let (filename, source) = match player.resource.as_ref() {
            Some(resource) => (
                // LoadResources installs a resource before its transfer
                // completes, and native queues the by-resource join at that
                // point. The filename is only authoritative for the issuer;
                // synchronized execution resolves this form from the core.
                path_for_resource(resource).unwrap_or_else(|| resource.filename.clone()),
                JoinPlayerSource::Resource(resource.clone()),
            ),
            None if player.is_script_player() => (
                LegacyCString::default(),
                JoinPlayerSource::Embedded(Vec::new()),
            ),
            None => return None,
        };
        Some(JoinPlayerControlData {
            filename,
            at_client: client_id,
            info_id: player.id,
            source,
            by_client: 0,
        })
    }

    /// Queues the local game's non-resource JoinPlayer controls in player-info
    /// order, matching `C4PlayerInfoList::LocalJoinUnjoinedPlayersInQueue`.
    pub fn issue_unjoined_local_players(
        &mut self,
        client_id: i32,
        mut filename_for_player: impl FnMut(&ControlPlayerInfoEntry) -> Option<LegacyCString>,
    ) -> Vec<JoinPlayerControlData> {
        let Some(client) = self
            .clients
            .iter()
            .find(|client| client.client_id == client_id)
        else {
            return Vec::new();
        };
        let issued_join_ids = &mut self.issued_join_ids;
        client
            .players
            .iter()
            .filter_map(|player| {
                if player.flags & PLAYER_INFO_FLAG_JOINED != 0
                    || issued_join_ids.contains(&player.id)
                {
                    return None;
                }
                // C++ sets PIF_JoinIssued before checking whether a user
                // player has a filename, so a failed entry is never retried
                // (pristine 9ffa0a5d src/C4PlayerInfo.cpp:1292-1322).
                issued_join_ids.insert(player.id);
                let filename = match filename_for_player(player) {
                    Some(filename) => filename,
                    None if player.is_script_player() => LegacyCString::default(),
                    None => return None,
                };
                Some(JoinPlayerControlData {
                    filename,
                    at_client: client_id,
                    info_id: player.id,
                    source: JoinPlayerSource::Embedded(Vec::new()),
                    by_client: client_id,
                })
            })
            .collect()
    }

    pub fn on_client_part(&mut self, client_id: i32) {
        let mut removed_ids = Vec::new();
        if let Some(client) = self
            .clients
            .iter_mut()
            .find(|client| client.client_id == client_id)
        {
            client.players.retain(|player| {
                let retain = player.flags & PLAYER_INFO_FLAG_JOINED != 0;
                if !retain {
                    removed_ids.push(player.id);
                }
                retain
            });
        }
        for id in removed_ids {
            self.issued_join_ids.remove(&id);
        }
        self.clients
            .retain(|client| client.client_id != client_id || !client.players.is_empty());
    }

    pub fn client_info_ids(&self, client_id: i32) -> Vec<i32> {
        self.clients
            .iter()
            .find(|client| client.client_id == client_id)
            .map(|client| client.players.iter().map(|player| player.id).collect())
            .unwrap_or_default()
    }

    /// Clone one retained client packet for a host-authored follow-up direct
    /// control. Stored rows are authoritative; AddPlayers and the host-local
    /// Updated marker are never emitted on this clean snapshot.
    pub fn client_packet(&self, client_id: i32) -> Option<PlayerInfoControlData> {
        self.clients
            .iter()
            .find(|client| client.client_id == client_id)
            .map(|client| PlayerInfoControlData {
                client_id,
                flags: client.flags
                    & !(CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_UPDATED),
                players: client.players.clone(),
                by_client: 0,
            })
    }

    /// Clone one retained `C4ClientPlayerInfos` exactly for a client-authored
    /// update request. Unlike [`Self::client_packet`], this preserves every
    /// stored flag because lobby ComboBox callbacks copy the complete value
    /// before changing one player's team (`C4PlayerInfoListBox.cpp:627-643`).
    pub fn client_update_request(&self, client_id: i32) -> Option<PlayerInfoUpdateRequest> {
        self.clients
            .iter()
            .find(|client| client.client_id == client_id)
            .map(|client| PlayerInfoUpdateRequest {
                client_id,
                flags: client.flags,
                players: client.players.clone(),
            })
    }

    /// Empty Initial packets are semantically present in C++ and mark a
    /// client as having completed its first player-info exchange.
    pub fn contains_client(&self, client_id: i32) -> bool {
        self.clients
            .iter()
            .any(|client| client.client_id == client_id)
    }

    /// `C4PlayerInfoList::HasSameTeamPlayers`: compare every non-removed
    /// player row for two clients and accept the first equal team ID.
    pub fn has_same_team_players(&self, first_client: i32, second_client: i32) -> bool {
        let Some(first) = self
            .clients
            .iter()
            .find(|client| client.client_id == first_client)
        else {
            return false;
        };
        let Some(second) = self
            .clients
            .iter()
            .find(|client| client.client_id == second_client)
        else {
            return false;
        };
        first.players.iter().any(|first_player| {
            first_player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                && second.players.iter().any(|second_player| {
                    second_player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                        && second_player.team == first_player.team
                })
        })
    }

    /// Color used by `C4Network2Players::GetClientChatColor`: the first user
    /// player row owned by an activated client (the activation check remains
    /// app-owned alongside the client registry).
    pub fn first_user_color(&self, client_id: i32) -> Option<u32> {
        self.first_user_lobby_color(client_id, false, |_| false)
    }

    /// `C4PlayerInfo::GetLobbyColor` hides an unjoined player's assigned team
    /// color while RandomInvisible surprise teams are active, but only when
    /// that assigned team still exists in `C4TeamList`.
    pub fn first_user_lobby_color(
        &self,
        client_id: i32,
        random_invisible_team_colors: bool,
        team_exists: impl FnOnce(i32) -> bool,
    ) -> Option<u32> {
        self.clients
            .iter()
            .find(|client| client.client_id == client_id)?
            .players
            .iter()
            .find(|player| player.player_type == PLAYER_INFO_TYPE_USER)
            .map(|player| {
                if random_invisible_team_colors
                    && team_exists(player.team)
                    && player.flags & PLAYER_INFO_FLAG_JOINED == 0
                    && player.savegame_player == 0
                {
                    player.original_color
                } else {
                    player.color
                }
            })
    }

    /// Clone the complete live PlayerInfo packets for a
    /// game-parameter/reference snapshot, including packet flags, rows, and
    /// the allocation high-water mark.
    pub fn retained_rows_snapshot(&self) -> (i32, Vec<(i32, u32, Vec<ControlPlayerInfoEntry>)>) {
        (
            self.last_player_id,
            self.clients
                .iter()
                .map(|client| (client.client_id, client.flags, client.players.clone()))
                .collect(),
        )
    }

    /// Advance `C4PlayerInfoList::iLastPlayerID` past a player that already
    /// exists in the runtime engine. Ordinary C++ joins always have a
    /// retained PlayerInfo row; this seam keeps low-level/synthetic engine
    /// registrations from colliding with the next admitted zero-ID request.
    pub fn reserve_player_ids_through(&mut self, player_info_id: i32) {
        self.last_player_id = self.last_player_id.max(player_info_id);
    }

    /// Apply `C4PlayerInfo::SetJoined`: retain the runtime player number and
    /// exact join frame together with the Joined bit.
    pub fn mark_joined(&mut self, info_id: i32, game_number: i32, game_join_frame: i32) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.game_number = game_number;
        info.game_join_frame = game_join_frame;
        info.flags |= PLAYER_INFO_FLAG_JOINED;
        true
    }

    /// Mirror `C4Team::AddPlayer(..., true)` into the retained PlayerInfo.
    /// Team color replaces only the active color; OriginalColor remains the
    /// player's pre-team preference.
    pub fn set_team_and_color(&mut self, info_id: i32, team: i32, color: Option<u32>) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        let mut changed = false;
        if info.team != team {
            info.team = team;
            changed = true;
        }
        if let Some(color) = color.filter(|color| info.color != *color) {
            info.color = color;
            changed = true;
        }
        changed
    }

    /// Apply `C4PlayerInfo::SetWinner` after `DoGameOver` has marked a
    /// surviving runtime player as the winner.
    pub fn mark_winner(&mut self, info_id: i32) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.flags |= crate::PLAYER_INFO_FLAG_WON;
        true
    }

    /// Apply the `C4ControlVoteEnd` pre-abort/pre-kick PlayerInfo history
    /// update. `None` marks every nonremoved row for an approved Cancel;
    /// `Some(client_id)` marks only that client's rows for an approved Kick.
    pub fn mark_voted_out(&mut self, client_id: Option<i32>) -> usize {
        let mut changed = 0;
        for client in &mut self.clients {
            if client_id.is_some_and(|client_id| client.client_id != client_id) {
                continue;
            }
            for player in &mut client.players {
                if player.flags & PLAYER_INFO_FLAG_REMOVED == 0
                    && player.flags & crate::PLAYER_INFO_FLAG_VOTED_OUT == 0
                {
                    player.flags |= crate::PLAYER_INFO_FLAG_VOTED_OUT;
                    changed += 1;
                }
            }
        }
        changed
    }

    pub fn mark_removed(&mut self, info_id: i32, disconnected: bool, game_part_frame: i32) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.flags |= PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED;
        if disconnected {
            info.flags |= crate::PLAYER_INFO_FLAG_DISCONNECTED;
        }
        info.game_part_frame = game_part_frame;
        true
    }

    pub fn get(&self, info_id: i32) -> Option<&ControlPlayerInfoEntry> {
        self.clients
            .iter()
            .flat_map(|client| &client.players)
            .find(|player| player.id == info_id)
    }

    /// Apply `C4PlayerInfo::SetLeagueProgressData` to the retained row.
    /// `None` clears the `StdStrBuf`; `Some([])` is an allocated empty string.
    pub fn set_league_progress_data(&mut self, info_id: i32, data: Option<Vec<u8>>) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.league_progress_data_is_null = data.is_none();
        info.league_progress_data = LegacyCString::from_bytes(data.unwrap_or_default())
            .expect("engine-normalized league progress data contains no interior NUL");
        true
    }

    /// Deterministic script-visible projection of every retained
    /// `C4PlayerInfo` row's league progress data.
    ///
    /// `GetPlayerInfoByID` returns the first matching row in packet storage
    /// order. IDs are expected to be unique after host allocation, but keep
    /// that first-row rule for malformed/imported snapshots and sort the
    /// resulting projection by ID so engine bootstrap is deterministic.
    /// Compiled strings, including empty ones, are allocated; freshly created
    /// in-memory rows retain their explicit null provenance.
    pub fn league_progress_data_snapshot(&self) -> Vec<(i32, Option<Vec<u8>>)> {
        let mut entries = BTreeMap::new();
        for player in self.clients.iter().flat_map(|client| &client.players) {
            if player.id != 0 {
                entries.entry(player.id).or_insert_with(|| {
                    (!player.league_progress_data_is_null
                        || !player.league_progress_data.is_empty())
                    .then(|| player.league_progress_data.as_bytes().to_vec())
                });
            }
        }
        entries.into_iter().collect()
    }

    /// Deterministic script-visible projection of every retained
    /// `C4PlayerInfo` row's league score.
    ///
    /// `GetPlayerInfoByID` returns the first matching row in packet storage
    /// order. Preserve that rule for duplicate IDs and sort the projection by
    /// ID for deterministic engine bootstrap. A score of zero is a retained
    /// value, not an absent entry.
    pub fn league_scores_snapshot(&self) -> Vec<(i32, i32)> {
        let mut entries = BTreeMap::new();
        for player in self.clients.iter().flat_map(|client| &client.players) {
            if player.id != 0 {
                entries.entry(player.id).or_insert(player.league_score);
            }
        }
        entries.into_iter().collect()
    }

    /// Applies the league server's projected-gain list by global player-info
    /// ID and returns one clean direct-update packet per changed owner.
    pub fn apply_league_projected_gains(
        &mut self,
        gains: impl IntoIterator<Item = (i32, i32)>,
    ) -> Vec<PlayerInfoControlData> {
        let mut first_gain_by_id = HashMap::new();
        for (player_id, gain) in gains {
            first_gain_by_id.entry(player_id).or_insert(gain);
        }
        let mut touched_clients = HashSet::new();
        for client in &mut self.clients {
            for player in &mut client.players {
                let Some(&gain) = first_gain_by_id.get(&player.id) else {
                    continue;
                };
                if player.league_projected_gain != gain {
                    player.league_projected_gain = gain;
                    touched_clients.insert(client.client_id);
                }
            }
        }
        self.client_packets(&touched_clients)
    }

    /// Reset every valid projected league gain and return each changed owner
    /// once in retained packet order. Native validity is exactly `>= 0`;
    /// negative sentinel values remain untouched.
    pub fn reset_league_projected_gains(&mut self) -> Vec<PlayerInfoControlData> {
        let mut touched_clients = HashSet::new();
        for client in &mut self.clients {
            for player in &mut client.players {
                if player.league_projected_gain >= 0 {
                    player.league_projected_gain = -1;
                    touched_clients.insert(client.client_id);
                }
            }
        }
        self.client_packets(&touched_clients)
    }

    /// Executes the projected-gain reset inside
    /// `HandlePlayerInfoUpdRequest` after attribute resolution.
    ///
    /// Existing valid gains mark their retained owner for the ordered
    /// `SendUpdatedPlayers` prefix. Every incoming row is reset regardless of
    /// its prior value. Rebuilding the prefix from the registry also folds in
    /// any packets already changed by attribute conflict resolution, so each
    /// owner is emitted exactly once with its final pre-admission state.
    pub fn reset_projected_gains_for_admission(&mut self, admission: &mut PlayerInfoAdmission) {
        let mut touched_clients = admission
            .updated_existing
            .iter()
            .map(|packet| packet.client_id)
            .collect::<HashSet<_>>();
        for client in &mut self.clients {
            let mut reset_any = false;
            for player in &mut client.players {
                if player.league_projected_gain >= 0 {
                    player.league_projected_gain = -1;
                    reset_any = true;
                }
            }
            if reset_any {
                touched_clients.insert(client.client_id);
            }
        }
        for player in &mut admission.admitted.players {
            player.league_projected_gain = -1;
        }
        admission.updated_existing = self.client_packets(&touched_clients);
    }

    pub fn client_id_for_info(&self, info_id: i32) -> Option<i32> {
        self.clients.iter().find_map(|client| {
            client
                .players
                .iter()
                .any(|player| player.id == info_id)
                .then_some(client.client_id)
        })
    }

    /// C4PlayerInfoList::RecreatePlayers walks client packets and each
    /// packet's player list in storage order. InitControl collision handling
    /// therefore observes this order before restored runtime player numbers
    /// re-sort C4PlayerList.
    pub fn recreation_players(&self) -> Vec<(i32, i32)> {
        self.clients
            .iter()
            .flat_map(|client| {
                client
                    .players
                    .iter()
                    .filter(|player| player.is_joined())
                    .map(|player| (client.client_id, player.id))
            })
            .collect()
    }

    pub fn recreation_info_ids(&self) -> Vec<i32> {
        self.recreation_players()
            .into_iter()
            .map(|(_, info_id)| info_id)
            .collect()
    }

    /// Applies `C4PlayerInfo::SetSavegameResume` from the exact restore-info
    /// row associated with the first current player in registry order.
    ///
    /// File/resource, identity and league fields remain those of the current
    /// joining player. Native replaces only the saved ID, takeover flag mask,
    /// current color and team; the player-ID high-water mark was seeded from
    /// `RestorePlayerInfos` earlier and is deliberately unchanged here.
    pub fn resume_savegame_player_from_info(
        &mut self,
        restore_info: &ControlPlayerInfoEntry,
    ) -> bool {
        if restore_info.id == 0 {
            return false;
        }
        let Some(player) = self
            .clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
            .find(|player| player.savegame_player == restore_info.id)
        else {
            return false;
        };

        const SAVEGAME_TAKEOVER_FLAGS: u16 = PLAYER_INFO_FLAG_JOINED
            | PLAYER_INFO_FLAG_REMOVED
            | PLAYER_INFO_FLAG_JOIN_ISSUED
            | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
            | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
            | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
            | PLAYER_INFO_FLAG_INVISIBLE;
        player.id = restore_info.id;
        player.flags = (player.flags & !SAVEGAME_TAKEOVER_FLAGS)
            | (restore_info.flags & SAVEGAME_TAKEOVER_FLAGS);
        player.color = restore_info.color;
        player.team = restore_info.team;
        true
    }

    /// Applies the `C4PlayerInfo::SetSavegameResume` fields that are
    /// authoritative for a still-live saved runtime player. The current
    /// entry keeps its player-file/name/type identity, while the associated
    /// savegame ID, joined state, team and no-elimination flag are restored
    /// before `RecreatePlayers` filters the registry.
    pub fn resume_joined_savegame_player(
        &mut self,
        savegame_info_id: i32,
        team: i32,
        no_elimination_check: bool,
    ) -> bool {
        if savegame_info_id <= 0 {
            return false;
        }
        let Some(player) = self
            .clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
            .find(|player| player.savegame_player == savegame_info_id)
        else {
            return false;
        };

        player.id = savegame_info_id;
        player.flags &= !(PLAYER_INFO_FLAG_JOINED
            | PLAYER_INFO_FLAG_REMOVED
            | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK);
        player.flags |= PLAYER_INFO_FLAG_JOINED;
        if no_elimination_check {
            player.flags |= PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK;
        }
        player.team = team;
        self.last_player_id = self.last_player_id.max(savegame_info_id);
        true
    }

    fn first_unissued_team_player(&self, team: &InitialNetworkTeam) -> Option<i32> {
        team.player_ids.iter().copied().find(|id| {
            !self.issued_join_ids.contains(id)
                && self
                    .get(*id)
                    .is_some_and(|player| player.flags & PLAYER_INFO_FLAG_JOINED == 0)
        })
    }

    fn get_mut(&mut self, info_id: i32) -> Option<&mut ControlPlayerInfoEntry> {
        self.clients
            .iter_mut()
            .flat_map(|client| &mut client.players)
            .find(|player| player.id == info_id)
    }

    pub fn player_count(&self) -> usize {
        self.clients.iter().map(|client| client.players.len()).sum()
    }

    /// `C4PlayerInfoList::GetStartupCount`: every retained player-info row
    /// except rows carrying PIF_Removed, irrespective of joined/type state.
    pub fn nonremoved_player_count(&self) -> usize {
        self.clients
            .iter()
            .flat_map(|client| &client.players)
            .filter(|player| player.flags & PLAYER_INFO_FLAG_REMOVED == 0)
            .count()
    }
}

pub struct JoinPlayerPreparation<'a> {
    pub join: &'a JoinPlayerControlData,
    pub info: &'a ControlPlayerInfoEntry,
    pub player_file: Option<&'a PlayerFile>,
    pub startup_player_count: i32,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrepareJoinPlayerError {
    #[error("join references player info {control_id}, but entry has id {info_id}")]
    PlayerInfoIdMismatch { control_id: i32, info_id: i32 },
    #[error("user player {info_id} has no player file data")]
    MissingPlayerData { info_id: i32 },
}

#[derive(Debug)]
pub enum RemoteEmbeddedPlayerData {
    PlayerFile(PlayerFile),
    ScriptWithoutFile,
}

#[derive(Debug, thiserror::Error)]
pub enum ResolveRemoteEmbeddedPlayerDataError {
    #[error("player {info_id} join is resource-backed, not embedded")]
    ResourceBacked { info_id: i32 },
    #[error("user player {info_id} has no embedded player file data")]
    MissingPlayerData { info_id: i32 },
    #[error("embedded player data for player {info_id} is not a gzip archive")]
    UnsupportedArchiveMagic { info_id: i32 },
    #[error("failed to load embedded player data for player {info_id}: {source}")]
    PlayerDataLoad {
        info_id: i32,
        #[source]
        source: ScenarioError,
    },
}

pub fn resolve_remote_embedded_player_data(
    join: &JoinPlayerControlData,
    info: &ControlPlayerInfoEntry,
) -> Result<RemoteEmbeddedPlayerData, ResolveRemoteEmbeddedPlayerDataError> {
    resolve_remote_embedded_player_data_with_resolution(join, info, None)
}

/// Resolve a runtime-save player group against the already-loaded scenario's
/// shared Strings.txt IDs and live object numbers. C++ denumerates these
/// player values only after the scenario objects exist.
pub fn resolve_remote_embedded_player_data_with_engine(
    engine: &crate::Engine,
    join: &JoinPlayerControlData,
    info: &ControlPlayerInfoEntry,
) -> Result<RemoteEmbeddedPlayerData, ResolveRemoteEmbeddedPlayerDataError> {
    let resolution = crate::player_file::PersistedC4ValueResolution {
        strings: engine.legacy_string_table_snapshot(),
        object_numbers: engine
            .objects
            .iter()
            .filter(|object| object.state.status != crate::ObjectStatus::Deleted)
            .map(|object| object.id.as_u64())
            .collect(),
    };
    resolve_remote_embedded_player_data_with_resolution(join, info, Some(&resolution))
}

fn resolve_remote_embedded_player_data_with_resolution(
    join: &JoinPlayerControlData,
    info: &ControlPlayerInfoEntry,
    resolution: Option<&crate::player_file::PersistedC4ValueResolution>,
) -> Result<RemoteEmbeddedPlayerData, ResolveRemoteEmbeddedPlayerDataError> {
    let JoinPlayerSource::Embedded(data) = &join.source else {
        return Err(ResolveRemoteEmbeddedPlayerDataError::ResourceBacked { info_id: info.id });
    };
    if data.is_empty() {
        if info.is_script_player() {
            return Ok(RemoteEmbeddedPlayerData::ScriptWithoutFile);
        }
        return Err(ResolveRemoteEmbeddedPlayerDataError::MissingPlayerData { info_id: info.id });
    }
    if !matches!(data.as_slice(), [0x1e, 0x8c, ..] | [0x1f, 0x8b, ..]) {
        return Err(
            ResolveRemoteEmbeddedPlayerDataError::UnsupportedArchiveMagic { info_id: info.id },
        );
    }
    let label = std::path::PathBuf::from(join.filename.to_string_lossy().into_owned());
    let loaded = match resolution {
        Some(resolution) => PlayerFile::load_from_bytes_with_portraits_and_value_resolution(
            label,
            data.clone(),
            false,
            resolution,
        ),
        None => PlayerFile::load_from_bytes_with_portraits(label, data.clone(), false),
    };
    loaded
        .map(RemoteEmbeddedPlayerData::PlayerFile)
        .map_err(
            |source| ResolveRemoteEmbeddedPlayerDataError::PlayerDataLoad {
                info_id: info.id,
                source,
            },
        )
}

pub fn prepare_join_player_config(
    input: JoinPlayerPreparation<'_>,
) -> Result<JoinPlayerConfig, PrepareJoinPlayerError> {
    if input.join.info_id != input.info.id {
        return Err(PrepareJoinPlayerError::PlayerInfoIdMismatch {
            control_id: input.join.info_id,
            info_id: input.info.id,
        });
    }
    let script_defaults =
        (input.player_file.is_none() && input.info.is_script_player()).then(PlayerFile::default);
    let file = input.player_file.or(script_defaults.as_ref()).ok_or(
        PrepareJoinPlayerError::MissingPlayerData {
            info_id: input.info.id,
        },
    )?;
    let name = [
        &input.info.league_account,
        &input.info.forced_name,
        &input.info.name,
    ]
    .into_iter()
    .find(|name| !name.is_empty())
    .map(|name| clonk_script::c4_string_from_bytes(name.as_bytes()))
    .unwrap_or_default();

    Ok(JoinPlayerConfig {
        name,
        player_info_id: input.info.id,
        score: file.score,
        rounds: file.rounds,
        rounds_won: file.rounds_won,
        rounds_lost: file.rounds_lost,
        total_playing_time: file.total_playing_time,
        team: (input.info.team != 0).then_some(input.info.team),
        color_dw: input.info.color & 0x00ff_ffff,
        pref_color: file.pref_color,
        pref_position: file.pref_position,
        crew: file.crew.clone(),
        control_style: file.pref_control_style,
        auto_context_menu: file.pref_auto_context_menu,
        startup_player_count: input.startup_player_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RecordingTeamAssignmentOracle {
        outcomes: std::collections::VecDeque<i32>,
        ranges: Vec<i32>,
    }

    impl InitialHostTeamAssignmentOracle for RecordingTeamAssignmentOracle {
        fn safe_random(&mut self, range: i32) -> i32 {
            self.ranges.push(range);
            self.outcomes
                .pop_front()
                .expect("recorded SafeRandom result")
        }

        fn generate_team(
            &mut self,
            _id: i32,
            _existing_teams: &[crate::InitialNetworkTeam],
        ) -> crate::InitialNetworkTeam {
            panic!("explicit-team assignment must not generate a team")
        }
    }

    #[derive(Default)]
    struct GeneratingTeamAssignmentOracle {
        outcomes: std::collections::VecDeque<i32>,
        ranges: Vec<i32>,
        generation_calls: Vec<(i32, Vec<i32>)>,
    }

    impl InitialHostTeamAssignmentOracle for GeneratingTeamAssignmentOracle {
        fn safe_random(&mut self, range: i32) -> i32 {
            self.ranges.push(range);
            self.outcomes
                .pop_front()
                .expect("recorded SafeRandom result")
        }

        fn generate_team(
            &mut self,
            id: i32,
            existing_teams: &[crate::InitialNetworkTeam],
        ) -> crate::InitialNetworkTeam {
            self.generation_calls
                .push((id, existing_teams.iter().map(|team| team.id).collect()));
            crate::InitialNetworkTeam {
                id,
                name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
                player_start_index: 0,
                player_ids: Vec::new(),
                color: 0x0010_0000 + u32::try_from(id).unwrap(),
                icon_spec: LegacyCString::default(),
                max_players: 0,
            }
        }
    }

    #[test]
    fn default_generated_team_uses_fixed_then_conflict_checked_process_colors() {
        let mut fixed_oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };
        let fixed = generate_default_initial_team(
            2,
            LegacyCString::from_bytes(b"Team 2".to_vec()).unwrap(),
            &[],
            &mut fixed_oracle,
        );
        assert_eq!(fixed.name.as_bytes(), b"Team 2");
        assert_eq!(fixed.color, 0x0000_c800);
        assert!(fixed_oracle.ranges.is_empty());

        let first_candidate = 255;
        let mut random_oracle = RecordingTeamAssignmentOracle {
            outcomes: [255, 0, 0, 0, 0, 255].into(),
            ranges: Vec::new(),
        };
        let generated = generate_default_initial_team(
            11,
            LegacyCString::from_bytes(b"Team 11".to_vec()).unwrap(),
            &[crate::InitialNetworkTeam {
                id: 1,
                name: LegacyCString::default(),
                player_start_index: 0,
                player_ids: Vec::new(),
                color: first_candidate,
                icon_spec: LegacyCString::default(),
                max_players: 0,
            }],
            &mut random_oracle,
        );
        assert_eq!(generated.name.as_bytes(), b"Team 11");
        assert_eq!(generated.color, 255 << 16);
        assert_eq!(random_oracle.ranges, vec![302; 6]);
    }

    fn player(id: i32) -> ControlPlayerInfoEntry {
        ControlPlayerInfoEntry {
            id,
            ..Default::default()
        }
    }

    #[test]
    fn startup_count_excludes_only_removed_player_infos() {
        let mut removed = player(2);
        removed.flags = PLAYER_INFO_FLAG_REMOVED;
        let mut joined = player(3);
        joined.flags = PLAYER_INFO_FLAG_JOINED;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 1,
            players: vec![player(1), removed, joined],
            ..Default::default()
        });

        assert_eq!(registry.player_count(), 3);
        assert_eq!(registry.nonremoved_player_count(), 2);
    }

    #[test]
    fn initial_offline_custom_free_user_stays_teamless_but_random_uses_recorded_tie() {
        // Offline has no lobby. A custom Free user may choose at runtime and
        // therefore remains teamless, while Random disables that choice and
        // runs the complete least-used SafeRandom tie scan (pristine 9ffa0a5d
        // src/C4PlayerInfo.cpp:717-730; src/C4Teams.cpp:446-462,474-543).
        let team = |id, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let initial_teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, 0x00f4_0000), team(2, 0x0000_c800)],
        };

        let mut free_teams = initial_teams.clone();
        let mut free_players = [ControlPlayerInfoEntry {
            id: 1,
            color: 0x0011_1111,
            original_color: 0x0011_1111,
            ..Default::default()
        }];
        let mut free_oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };
        assign_initial_offline_player_teams(&mut free_teams, &mut free_players, &mut free_oracle);

        assert_eq!(free_players[0].team, 0);
        assert_eq!(free_players[0].color, 0x0011_1111);
        assert!(free_teams
            .teams
            .iter()
            .all(|team| team.player_ids.is_empty()));
        assert!(free_oracle.ranges.is_empty());

        let mut random_teams = initial_teams;
        random_teams.team_distribution = crate::InitialNetworkTeamDistribution::Random;
        let mut random_players = [ControlPlayerInfoEntry {
            id: 2,
            color: 0x0022_2222,
            original_color: 0x0022_2222,
            ..Default::default()
        }];
        let mut random_oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };
        assign_initial_offline_player_teams(
            &mut random_teams,
            &mut random_players,
            &mut random_oracle,
        );

        assert_eq!(random_oracle.ranges, vec![2]);
        assert_eq!(random_players[0].team, 2);
        assert_eq!(random_players[0].color, 0x0000_c800);
        assert_eq!(random_teams.teams[1].player_ids, vec![2]);
    }

    #[test]
    fn initial_offline_team_need_gate_assigns_script_and_existing_noncustom_teams() {
        // A script player cannot make the runtime team choice. A non-custom
        // list is also assigned when teams already exist, but empty default
        // melee metadata is not team-needed yet and must not generate a team
        // (pristine 9ffa0a5d src/C4Teams.cpp:503-507,510-541,605-610).
        let team = |id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let initial_teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1), team(2)],
        };

        let mut script_teams = initial_teams.clone();
        let mut script_players = [ControlPlayerInfoEntry {
            id: 3,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        }];
        let mut script_oracle = RecordingTeamAssignmentOracle {
            outcomes: [1].into(),
            ranges: Vec::new(),
        };
        assign_initial_offline_player_teams(
            &mut script_teams,
            &mut script_players,
            &mut script_oracle,
        );
        assert_eq!(script_oracle.ranges, vec![2]);
        assert_eq!(script_players[0].team, 1);

        let mut noncustom_teams = initial_teams;
        noncustom_teams.custom = false;
        let mut noncustom_players = [player(4)];
        let mut noncustom_oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };
        assign_initial_offline_player_teams(
            &mut noncustom_teams,
            &mut noncustom_players,
            &mut noncustom_oracle,
        );
        assert_eq!(noncustom_oracle.ranges, vec![2]);
        assert_eq!(noncustom_players[0].team, 2);

        let mut empty_autogenerated = crate::InitialNetworkTeamMetadata {
            teams: Vec::new(),
            auto_generate_teams: true,
            last_team_id: 0,
            ..noncustom_teams
        };
        let mut teamless_players = [player(5)];
        let mut empty_oracle = GeneratingTeamAssignmentOracle::default();
        assign_initial_offline_player_teams(
            &mut empty_autogenerated,
            &mut teamless_players,
            &mut empty_oracle,
        );
        assert_eq!(teamless_players[0].team, 0);
        assert!(empty_autogenerated.teams.is_empty());
        assert!(empty_oracle.ranges.is_empty());
        assert!(empty_oracle.generation_calls.is_empty());
    }

    #[test]
    fn initial_host_team_assignment_matches_cpp_order_full_skip_and_team_colors() {
        // RecheckPlayerInfoTeams processes the packet in order. Its
        // GetRandomSmallestTeam reservoir scan skips full teams and calls
        // SafeRandom(2), SafeRandom(3), ... for equal minima in team-list
        // order. AddPlayer changes the current color, not OriginalColor
        // (src/C4PlayerInfo.cpp:810-817; src/C4Teams.cpp:53-81,446-462,474-543).
        let team = |id, player_ids, color, max_players| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color,
            icon_spec: LegacyCString::default(),
            max_players,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 4,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![], 0x00f4_0000, 0),
                team(2, vec![99], 0x0000_c800, 1),
                team(3, vec![], 0x0020_20ff, 0),
                team(4, vec![], 0x00fc_f41c, 0),
            ],
        };
        let mut players = vec![
            ControlPlayerInfoEntry {
                id: 1,
                color: 0x0011_1111,
                original_color: 0x0011_1111,
                ..Default::default()
            },
            ControlPlayerInfoEntry {
                id: 2,
                color: 0x0022_2222,
                original_color: 0x0022_2222,
                ..Default::default()
            },
            ControlPlayerInfoEntry {
                id: 3,
                color: 0x0033_3333,
                original_color: 0x0033_3333,
                ..Default::default()
            },
        ];
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [0, 2, 1, 0].into(),
            ranges: Vec::new(),
        };

        assign_initial_host_player_teams(&mut teams, &mut players, &mut oracle);

        assert_eq!(oracle.ranges, vec![2, 3, 2, 2]);
        assert_eq!(
            players
                .iter()
                .map(|player| (player.team, player.color, player.original_color))
                .collect::<Vec<_>>(),
            vec![
                (3, 0x0020_20ff, 0x0011_1111),
                (1, 0x00f4_0000, 0x0022_2222),
                (4, 0x00fc_f41c, 0x0033_3333),
            ]
        );
        assert_eq!(teams.teams[0].player_ids, vec![2]);
        assert_eq!(teams.teams[1].player_ids, vec![99]);
        assert_eq!(teams.teams[2].player_ids, vec![1]);
        assert_eq!(teams.teams[3].player_ids, vec![3]);
    }

    #[test]
    fn initial_host_team_assignment_generates_empty_active_teams_at_cpp_timing() {
        // Existing Teams.txt with no Team sections forces AutoGenerateTeams
        // but remains empty until RecheckPlayerInfoTeams. Each player first
        // performs the complete smallest-team scan (and its SafeRandom ties),
        // then GenerateDefaultTeams creates last_team_id + 1 when no empty
        // team exists (src/C4Teams.cpp:386-395,446-462,510-524,605-611).
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: true,
            last_team_id: 0,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        };
        let mut players = (1..=4)
            .map(|id| ControlPlayerInfoEntry {
                id,
                color: 0x0000_1000 + u32::try_from(id).unwrap(),
                original_color: 0x0000_1000 + u32::try_from(id).unwrap(),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let original_colors = players
            .iter()
            .map(|player| player.original_color)
            .collect::<Vec<_>>();
        let mut oracle = GeneratingTeamAssignmentOracle {
            outcomes: [0, 1, 0].into(),
            ..Default::default()
        };

        assign_initial_host_player_teams(&mut teams, &mut players, &mut oracle);

        assert_eq!(oracle.ranges, vec![2, 2, 3]);
        assert_eq!(
            oracle.generation_calls,
            vec![
                (1, vec![]),
                (2, vec![1]),
                (3, vec![1, 2]),
                (4, vec![1, 2, 3]),
            ]
        );
        assert_eq!(teams.last_team_id, 4);
        assert_eq!(
            teams
                .teams
                .iter()
                .map(|team| (team.id, team.player_ids.clone()))
                .collect::<Vec<_>>(),
            vec![(1, vec![1]), (2, vec![2]), (3, vec![3]), (4, vec![4])]
        );
        assert_eq!(
            players
                .iter()
                .map(|player| (player.team, player.color))
                .collect::<Vec<_>>(),
            vec![
                (1, 0x0010_0001),
                (2, 0x0010_0002),
                (3, 0x0010_0003),
                (4, 0x0010_0004),
            ]
        );
        assert_eq!(
            players
                .iter()
                .map(|player| player.original_color)
                .collect::<Vec<_>>(),
            original_colors
        );
    }

    #[test]
    fn initial_host_team_assignment_matches_cpp_two_team_fallback() {
        // If every configured team is full but fewer than two list entries
        // exist, the non-autogenerate branch calls GenerateDefaultTeams(2)
        // and then selects list index zero without rechecking fullness. Team 2
        // is therefore created empty while the full first team is overfilled;
        // skipped full teams and fixed default colors consume no SafeRandom
        // draws (src/C4Teams.cpp:53-81,181-218,386-395,446-462,525-539).
        let first_team_color = 0x00aa_5500;
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 1,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![crate::InitialNetworkTeam {
                id: 1,
                name: LegacyCString::from_bytes(b"Only team".to_vec()).unwrap(),
                player_start_index: 0,
                player_ids: vec![99],
                color: first_team_color,
                icon_spec: LegacyCString::default(),
                max_players: 1,
            }],
        };
        let mut players = vec![ControlPlayerInfoEntry {
            id: 7,
            color: 0x0012_3456,
            original_color: 0x0012_3456,
            ..Default::default()
        }];
        let mut oracle = GeneratingTeamAssignmentOracle::default();

        assign_initial_host_player_teams(&mut teams, &mut players, &mut oracle);

        assert!(oracle.ranges.is_empty());
        assert_eq!(oracle.generation_calls, vec![(2, vec![1])]);
        assert_eq!(teams.last_team_id, 2);
        assert_eq!(teams.teams[0].player_ids, vec![99, 7]);
        assert!(teams.teams[1].player_ids.is_empty());
        assert_eq!(teams.teams[1].color, 0x0010_0002);
        assert_eq!(
            (players[0].team, players[0].color, players[0].original_color),
            (1, first_team_color, 0x0012_3456)
        );
    }

    #[test]
    fn non_add_packet_replaces_the_clients_player_list() {
        // C4PlayerInfoList::AddInfo replaces an existing client's entire
        // C4ClientPlayerInfos unless CIF_AddPlayers is set
        // (src/C4PlayerInfo.cpp:834-880).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(8)],
            ..Default::default()
        });

        assert!(registry.get(7).is_none());
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 1);
    }

    #[test]
    fn add_packet_appends_to_the_clients_player_list() {
        // CIF_AddPlayers makes C4PlayerInfoList::AddInfo call
        // C4ClientPlayerInfos::GrabMergeFrom, which appends in packet order
        // (src/C4PlayerInfo.cpp:458-482,834-880).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![player(8)],
            ..Default::default()
        });

        assert_eq!(registry.get(7).map(|entry| entry.id), Some(7));
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 2);
    }

    #[test]
    fn queued_add_resource_normalization_updates_only_its_original_row() {
        let resource = |id| NetworkResourceCore {
            id,
            filename: LegacyCString::from_bytes(format!("Player{id}.ocp").into_bytes()).unwrap(),
            ..NetworkResourceCore::default()
        };
        let mut first = player(7);
        first.name = LegacyCString::from_bytes(b"First retained".to_vec()).unwrap();
        first.flags = PLAYER_INFO_FLAG_HAS_RESOURCE | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK;
        first.resource = Some(resource(7));
        let mut second = player(8);
        second.flags = PLAYER_INFO_FLAG_HAS_RESOURCE | PLAYER_INFO_FLAG_JOINED;
        second.resource = Some(resource(8));
        let original = PlayerInfoControlData {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![first.clone()],
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(original.clone());
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![second.clone()],
            ..Default::default()
        });
        let mut echoed_first = first;
        echoed_first.name = LegacyCString::from_bytes(b"Must not replace".to_vec()).unwrap();
        echoed_first.flags = PLAYER_INFO_FLAG_JOINED;
        echoed_first.resource = None;
        let echo = PlayerInfoControlData {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![echoed_first, player(99)],
            ..Default::default()
        };

        assert!(registry.apply_player_resource_normalization(&original, &echo));
        let normalized = registry.get(7).expect("the first Add row remains retained");
        assert_eq!(normalized.name.as_bytes(), b"First retained");
        assert_eq!(normalized.flags, PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK);
        assert!(normalized.resource.is_none());
        assert_eq!(registry.get(8), Some(&second));
        assert!(!registry.apply_player_resource_normalization(&original, &echo));
        assert_eq!(registry.client_info_ids(3), vec![7, 8]);
    }

    #[test]
    fn stale_same_id_scenario_flag_replacement_skips_resource_normalization() {
        let resource = |id| NetworkResourceCore {
            id,
            filename: LegacyCString::from_bytes(format!("Player{id}.ocp").into_bytes()).unwrap(),
            ..NetworkResourceCore::default()
        };
        let mut original_player = player(7);
        original_player.flags = PLAYER_INFO_FLAG_HAS_RESOURCE;
        original_player.resource = Some(resource(7));
        let original = PlayerInfoControlData {
            client_id: 3,
            players: vec![original_player.clone()],
            ..Default::default()
        };
        let mut replacement = original_player;
        replacement.flags |= crate::PLAYER_INFO_FLAG_IN_SCENARIO_FILE;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(original.clone());
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![replacement.clone()],
            ..Default::default()
        });
        let mut echoed_player = original.players[0].clone();
        echoed_player.flags &= !PLAYER_INFO_FLAG_HAS_RESOURCE;
        echoed_player.resource = None;
        let echo = PlayerInfoControlData {
            client_id: 3,
            players: vec![echoed_player],
            ..Default::default()
        };

        assert!(!registry.apply_player_resource_normalization(&original, &echo));
        assert_eq!(registry.get(7), Some(&replacement));
    }

    #[test]
    fn retained_player_snapshot_keeps_packet_and_join_lifecycle_fields() {
        // InitLocal copies the complete retained C4ClientPlayerInfos rows.
        // SetJoined supplies both in-game fields, and DoGameOver adds Won
        // before the reference is rebuilt (src/C4PlayerInfo.cpp:319-325;
        // src/C4Game.cpp:3660-3675; src/C4Network2Reference.cpp:49-66).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            41,
            [PlayerInfoControlData {
                client_id: 7,
                flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                players: vec![player(41)],
                ..Default::default()
            }],
        );

        assert!(registry.mark_joined(41, 3, 77));
        assert!(registry.mark_winner(41));

        let (last_player_id, clients) = registry.retained_rows_snapshot();
        assert_eq!(last_player_id, 41);
        let [(client_id, flags, players)] = clients.as_slice() else {
            panic!("expected one retained client packet");
        };
        assert_eq!((*client_id, *flags), (7, CLIENT_PLAYER_INFO_FLAG_INITIAL));
        let [retained] = players.as_slice() else {
            panic!("expected one retained player");
        };
        assert_eq!((retained.game_number, retained.game_join_frame), (3, 77));
        assert_ne!(retained.flags & PLAYER_INFO_FLAG_JOINED, 0);
        assert_ne!(retained.flags & crate::PLAYER_INFO_FLAG_WON, 0);
    }

    #[test]
    fn approved_vote_marks_only_active_player_info_rows_voted_out() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            30,
            [
                PlayerInfoControlData {
                    client_id: 7,
                    players: vec![
                        player(10),
                        ControlPlayerInfoEntry {
                            id: 11,
                            flags: PLAYER_INFO_FLAG_REMOVED,
                            ..Default::default()
                        },
                    ],
                    ..Default::default()
                },
                PlayerInfoControlData {
                    client_id: 8,
                    players: vec![player(20)],
                    ..Default::default()
                },
            ],
        );

        assert_eq!(registry.mark_voted_out(Some(7)), 1);
        assert_ne!(
            registry.get(10).expect("active target row").flags & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );
        assert_eq!(
            registry.get(11).expect("removed target row").flags & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );
        assert_eq!(
            registry.get(20).expect("other client row").flags & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );
        assert_eq!(registry.mark_voted_out(None), 1);
        assert_ne!(
            registry.get(20).expect("all-client cancel row").flags
                & crate::PLAYER_INFO_FLAG_VOTED_OUT,
            0
        );
    }

    #[test]
    fn league_progress_snapshot_is_sorted_and_keeps_retained_empty_rows() {
        let mut first_nine = player(9);
        first_nine.league_progress_data = LegacyCString::from_bytes(b"first".to_vec()).unwrap();
        let mut five = player(5);
        five.league_progress_data = LegacyCString::from_bytes(b"five".to_vec()).unwrap();
        let mut duplicate_nine = player(9);
        duplicate_nine.league_progress_data =
            LegacyCString::from_bytes(b"duplicate".to_vec()).unwrap();
        let mut retained_empty = player(2);
        retained_empty.flags = PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED;
        retained_empty.league_progress_data_is_null = false;
        let retained_null = player(3);

        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![first_nine, retained_empty, retained_null],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![five, duplicate_nine],
            ..Default::default()
        });

        assert_eq!(
            registry.league_progress_data_snapshot(),
            vec![
                (2, Some(Vec::new())),
                (3, None),
                (5, Some(b"five".to_vec())),
                (9, Some(b"first".to_vec())),
            ]
        );

        assert!(registry.set_league_progress_data(3, Some(Vec::new())));
        assert_eq!(
            registry.league_progress_data_snapshot()[1],
            (3, Some(Vec::new()))
        );
        assert!(registry.set_league_progress_data(3, None));
        assert_eq!(registry.league_progress_data_snapshot()[1], (3, None));
        assert!(!registry.set_league_progress_data(99, Some(b"missing".to_vec())));
    }

    #[test]
    fn league_scores_snapshot_is_sorted_and_keeps_first_duplicate_and_zero() {
        let mut first_nine = player(9);
        first_nine.league_score = 90;
        let five_with_zero = player(5);
        let mut duplicate_nine = player(9);
        duplicate_nine.league_score = 900;
        let mut two = player(2);
        two.league_score = -20;

        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![first_nine, five_with_zero],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![duplicate_nine, two],
            ..Default::default()
        });

        assert_eq!(
            registry.league_scores_snapshot(),
            vec![(2, -20), (5, 0), (9, 90)]
        );
    }

    #[test]
    fn league_projected_gains_match_global_ids_and_return_changed_owners_once() {
        let mut unchanged = player(10);
        unchanged.league_projected_gain = 3;
        let mut first_changed = player(11);
        first_changed.league_projected_gain = -1;
        let mut second_changed = player(20);
        second_changed.league_projected_gain = 1;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL | CLIENT_PLAYER_INFO_FLAG_UPDATED,
            players: vec![unchanged, first_changed],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![second_changed],
            ..Default::default()
        });

        let updates =
            registry.apply_league_projected_gains([(20, 8), (999, 44), (10, 3), (11, 5), (11, 99)]);

        assert_eq!(registry.get(10).unwrap().league_projected_gain, 3);
        assert_eq!(registry.get(11).unwrap().league_projected_gain, 5);
        assert_eq!(registry.get(20).unwrap().league_projected_gain, 8);
        assert_eq!(
            updates
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![4, 7]
        );
        assert!(updates
            .iter()
            .all(|packet| packet.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED == 0));
    }

    #[test]
    fn league_projected_gain_reset_clears_nonnegative_values_once_per_owner() {
        let mut unknown = player(10);
        unknown.league_projected_gain = -1;
        let mut other_negative = player(11);
        other_negative.league_projected_gain = -2;
        let mut zero = player(20);
        zero.league_projected_gain = 0;
        let mut positive = player(21);
        positive.league_projected_gain = 7;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![unknown, other_negative],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            flags: CLIENT_PLAYER_INFO_FLAG_UPDATED,
            players: vec![zero, positive],
            ..Default::default()
        });

        let updates = registry.reset_league_projected_gains();

        assert_eq!(registry.get(10).unwrap().league_projected_gain, -1);
        assert_eq!(registry.get(11).unwrap().league_projected_gain, -2);
        assert_eq!(registry.get(20).unwrap().league_projected_gain, -1);
        assert_eq!(registry.get(21).unwrap().league_projected_gain, -1);
        assert_eq!(
            updates
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert_eq!(updates[0].flags & CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);
    }

    #[test]
    fn admission_projected_gain_reset_merges_existing_updates_and_resets_every_incoming_row() {
        let mut unchanged_invalid = player(10);
        unchanged_invalid.league_projected_gain = -1;
        let mut valid = player(20);
        valid.league_projected_gain = 7;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![unchanged_invalid],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            players: vec![valid],
            ..Default::default()
        });
        let mut incoming_valid = player(30);
        incoming_valid.league_projected_gain = 9;
        let mut admission = PlayerInfoAdmission {
            updated_existing: vec![registry.client_packets(&HashSet::from([4]))[0].clone()],
            admitted: PlayerInfoControlData {
                client_id: 9,
                players: vec![incoming_valid],
                ..Default::default()
            },
            joined_player_team_updates: Vec::new(),
        };

        registry.reset_projected_gains_for_admission(&mut admission);

        assert_eq!(registry.get(10).unwrap().league_projected_gain, -1);
        assert_eq!(registry.get(20).unwrap().league_projected_gain, -1);
        assert_eq!(admission.admitted.players[0].league_projected_gain, -1);
        assert_eq!(
            admission
                .updated_existing
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![4, 7]
        );
    }

    #[test]
    fn recreation_order_contains_only_currently_joined_players() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 7,
                    flags: PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 8,
                    flags: PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED,
                    ..Default::default()
                },
                player(9),
            ],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![ControlPlayerInfoEntry {
                id: 10,
                flags: PLAYER_INFO_FLAG_JOINED,
                ..Default::default()
            }],
            ..Default::default()
        });

        assert_eq!(registry.recreation_players(), vec![(3, 7), (4, 10)]);
        assert_eq!(registry.recreation_info_ids(), vec![7, 10]);
        assert_eq!(
            registry.get(8).map(|info| info.flags),
            Some(PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED)
        );
    }

    #[test]
    fn join_data_snapshot_replaces_registries_and_preserves_the_player_id_counter() {
        // HandleJoinData replaces Game.Parameters, including the complete
        // client/player lists and raw LastPlayerID. Later host assignment uses
        // ++iLastPlayerID (pristine 9ffa0a5d src/C4Network2.cpp:1574-1605;
        // src/C4PlayerInfo.cpp:781-807,1733-1760).
        let mut clients = ControlClientRegistry::default();
        clients.register(99, true, false);
        clients.replace_snapshot([
            crate::ClientCoreControlData {
                client_id: 0,
                activated: true,
                name: LegacyCString::from_bytes(b"Host".to_vec()).unwrap(),
                ..Default::default()
            },
            crate::ClientCoreControlData {
                client_id: 7,
                observer: true,
                name: LegacyCString::from_bytes(b"Local observer".to_vec()).unwrap(),
                lobby_ready: true,
                ..Default::default()
            },
        ]);
        assert!(!clients.contains(99));
        assert!(clients.is_activated(0));
        let local = clients.state(7).expect("local client restored");
        assert!(local.observer);
        assert_eq!(local.name.as_bytes(), b"Local observer");
        assert!(local.lobby_ready);
        let live_clients = clients.snapshot();
        assert_eq!(
            live_clients
                .iter()
                .map(|client| client.client_id)
                .collect::<Vec<_>>(),
            vec![0, 7]
        );
        assert!(live_clients[0].activated);
        assert!(live_clients[1].observer);
        assert_eq!(live_clients[1].name.as_bytes(), b"Local observer");
        assert!(live_clients[1].lobby_ready);

        let mut players = ControlPlayerInfoRegistry::default();
        players.apply(PlayerInfoControlData {
            client_id: 99,
            players: vec![player(3)],
            ..Default::default()
        });
        players.replace_snapshot(
            40,
            [PlayerInfoControlData {
                client_id: 0,
                players: vec![player(12)],
                ..Default::default()
            }],
        );
        assert!(players.get(3).is_none());
        assert!(players.get(12).is_some());
        let admitted = players
            .admit_request(
                PlayerInfoUpdateRequest {
                    client_id: 7,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![player(0)],
                },
                4,
            )
            .expect("snapshot leaves a free player slot");
        assert_eq!(admitted.players[0].id, 41);
    }

    #[test]
    fn client_update_requires_host_and_applies_activation_then_observer() {
        // C4ControlClientUpdate ignores non-host authors. Activate toggles the
        // synchronized bit; SetObserver is one-way and deactivates the client
        // (src/C4Control.cpp:578-620).
        let mut clients = ControlClientRegistry::default();
        clients.register(3, false, false);

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 3,
        });
        assert!(!clients.is_activated(3));

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        });
        assert!(clients.is_activated(3));

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_SET_OBSERVER,
            client_id: 3,
            data: 99,
            by_client: 0,
        });
        assert!(!clients.is_activated(3));
        assert!(clients.is_observer(3));

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 0,
            by_client: 0,
        });
        assert!(
            clients.is_observer(3),
            "equal Activate request is a full native no-op"
        );
    }

    #[test]
    fn client_join_requires_host_preserves_core_and_rejects_duplicates() {
        // C4ControlClientJoin is host-only; C4ClientList::Add rejects duplicate
        // IDs and copies the wire core as-is (src/C4Control.cpp:552-568;
        // src/C4Client.cpp:255-265).
        let mut clients = ControlClientRegistry::default();
        let non_host = crate::ClientJoinControlData {
            core: crate::ClientCoreControlData {
                client_id: 3,
                activated: true,
                observer: true,
                name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                nick: LegacyCString::from_bytes(b"Ali".to_vec()).unwrap(),
                lobby_ready: true,
            },
            by_client: 3,
        };
        assert!(!clients.apply_join(&non_host));

        let mut host_join = non_host.clone();
        host_join.by_client = 0;
        assert!(clients.apply_join(&host_join));
        let state = clients.state(3).expect("client registered");
        assert!(state.activated);
        assert!(state.observer);
        assert_eq!(state.name.as_bytes(), b"Alice");
        assert_eq!(state.nick.as_bytes(), b"Ali");
        assert!(state.lobby_ready);

        let mut duplicate = host_join;
        duplicate.core.activated = false;
        duplicate.core.name = LegacyCString::from_bytes(b"Replacement".to_vec()).unwrap();
        assert!(!clients.apply_join(&duplicate));
        let state = clients.state(3).expect("original client retained");
        assert!(state.activated);
        assert_eq!(state.name.as_bytes(), b"Alice");
    }

    #[test]
    fn activation_request_matches_cpp_host_eligibility_and_lag_window() {
        // HandleActivateReq accepts only a known, fully joined, inactive,
        // non-observer remote. While running, its oldest accepted frame is
        // host_frame - clamp(ping_ms * FPS / 500, 0, 100) - 20
        // (src/C4Network2.cpp:1553-1571; src/C4Network2.h:57-60).
        let mut clients = ControlClientRegistry::default();
        clients.register(0, true, false);
        clients.register(3, false, false);

        let admitted =
            clients.activation_update_for_request(3, 1_880, 2_000, true, true, 2_000, 36);
        assert_eq!(
            admitted,
            Some(crate::ClientUpdateControlData {
                update_type: crate::CLIENT_UPDATE_ACTIVATE,
                client_id: 3,
                data: 1,
                by_client: 0,
            })
        );
        assert!(clients
            .activation_update_for_request(3, 1_879, 2_000, true, true, 2_000, 36)
            .is_none());
        assert!(clients
            .activation_update_for_request(3, -1, 2_000, false, true, 0, 36)
            .is_some());
        assert!(clients
            .activation_update_for_request(3, 2_000, 2_000, true, false, 0, 36)
            .is_none());
        assert!(clients
            .activation_update_for_request(0, 2_000, 2_000, true, true, 0, 36)
            .is_none());
        assert!(clients
            .activation_update_for_request(99, 2_000, 2_000, true, true, 0, 36)
            .is_none());

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_SET_OBSERVER,
            client_id: 3,
            data: 0,
            by_client: 0,
        });
        assert!(clients
            .activation_update_for_request(3, 2_000, 2_000, true, true, 0, 36)
            .is_none());

        clients.apply_update(&crate::ClientUpdateControlData {
            update_type: crate::CLIENT_UPDATE_ACTIVATE,
            client_id: 3,
            data: 1,
            by_client: 0,
        });
        assert!(clients
            .activation_update_for_request(3, 2_000, 2_000, true, true, 0, 36)
            .is_none());
    }

    #[test]
    fn client_part_drops_unjoined_infos_but_keeps_joined_history() {
        // OnClientPart deletes never-joined infos while preserving joined
        // history for evaluation and replay state
        // (src/C4Network2Players.cpp:425-459).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![
                player(7),
                ControlPlayerInfoEntry {
                    id: 8,
                    flags: PLAYER_INFO_FLAG_JOINED,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });

        registry.on_client_part(3);

        assert!(registry.get(7).is_none());
        assert_eq!(registry.get(8).map(|entry| entry.id), Some(8));
        assert_eq!(registry.player_count(), 1);
    }

    #[test]
    fn client_player_removal_marks_joined_history_disconnected() {
        // C4PlayerList::Remove marks the live player's info Joined|Removed and
        // ClientRemove additionally marks it Disconnected before OnClientPart
        // prunes unjoined records (src/C4PlayerList.cpp:219-239;
        // src/C4PlayerInfo.cpp:327-334).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![player(7), player(8)],
            ..Default::default()
        });

        assert_eq!(registry.client_info_ids(3), vec![7, 8]);
        assert!(registry.mark_joined(7, 3, 17));
        assert!(registry.mark_removed(7, true, 42));
        registry.on_client_part(3);

        let retained = registry.get(7).expect("joined history remains");
        assert_ne!(retained.flags & PLAYER_INFO_FLAG_JOINED, 0);
        assert_ne!(retained.flags & PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(retained.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
        assert_eq!((retained.game_number, retained.game_join_frame), (3, 17));
        assert_eq!(retained.game_part_frame, 42);
        assert!(registry.get(8).is_none());
    }

    #[test]
    fn host_admission_assigns_the_next_id_and_preserves_the_claimed_client() {
        // AssignPlayerIDs changes only zero IDs to ++iLastPlayerID, then the
        // host constructs C4ControlPlayerInfo without rebinding the packet's
        // client ID (src/C4PlayerInfo.cpp:781-807;
        // src/C4Network2Players.cpp:160-205,232-239).
        let mut registry = ControlPlayerInfoRegistry::default();
        let existing = registry
            .admit_request(
                crate::PlayerInfoUpdateRequest {
                    client_id: 1,
                    flags: 0,
                    players: vec![player(0); 7],
                },
                8,
            )
            .expect("seven free player slots accept the first request");
        registry.apply(existing);
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![player(0)],
        };

        let admitted = registry
            .admit_request(request, 8)
            .expect("one free player slot accepts the request");

        assert_eq!((admitted.client_id, admitted.by_client), (3, 0));
        assert_eq!(admitted.flags, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS);
        let [admitted_player] = admitted.players.as_slice() else {
            panic!("expected one admitted player");
        };
        assert_eq!(admitted_player.id, 8);
    }

    #[test]
    fn host_admission_duplicate_resource_filter_uses_native_swap_order() {
        // HandlePlayerInfoUpdRequest scans backward and RemoveIndexedInfo
        // replaces a removed row with the final row before AssignPlayerIDs
        // (src/C4Network2Players.cpp:166-191;
        // src/C4PlayerInfo.cpp:492-500,568-580).
        let resource_player = |id, resource_id| ControlPlayerInfoEntry {
            id,
            flags: crate::PLAYER_INFO_FLAG_HAS_RESOURCE,
            resource: Some(NetworkResourceCore {
                resource_type: 3,
                id: resource_id,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![resource_player(7, 61)],
            ..Default::default()
        });
        registry.reserve_player_ids_through(7);
        let mut callback_resource_ids = Vec::new();

        let admitted = registry
            .admit_request_with(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![
                        resource_player(0, 61),
                        resource_player(0, 62),
                        resource_player(0, 63),
                    ],
                },
                8,
                |players| {
                    callback_resource_ids = players
                        .iter()
                        .map(|player| player.resource.as_ref().unwrap().id)
                        .collect();
                },
            )
            .expect("the two distinct resources remain in the request");

        assert_eq!(
            admitted
                .players
                .iter()
                .map(|player| (player.id, player.resource.as_ref().unwrap().id))
                .collect::<Vec<_>>(),
            vec![(8, 63), (9, 62)]
        );
        assert_eq!(callback_resource_ids, vec![63, 62]);
    }

    #[test]
    fn host_admission_all_duplicate_resources_return_before_assignment() {
        let resource_player = |id, resource_id, flags| ControlPlayerInfoEntry {
            id,
            flags: flags | crate::PLAYER_INFO_FLAG_HAS_RESOURCE,
            resource: Some(NetworkResourceCore {
                resource_type: 3,
                id: resource_id,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![resource_player(7, 61, 0), resource_player(8, 62, 0)],
            ..Default::default()
        });
        registry.reserve_player_ids_through(8);
        let before = registry.retained_rows_snapshot();
        let mut callback_called = false;

        let admitted = registry.admit_request_with(
            PlayerInfoUpdateRequest {
                client_id: 3,
                flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                players: vec![
                    resource_player(0, 61, 0),
                    resource_player(0, 62, PLAYER_INFO_FLAG_REMOVED),
                ],
            },
            8,
            |_| callback_called = true,
        );

        assert_eq!(admitted, None);
        assert!(!callback_called);
        assert_eq!(registry.retained_rows_snapshot(), before);
    }

    #[test]
    fn host_admission_duplicate_resource_filter_preserves_cpp_exemptions() {
        // Initial packets bypass the filter, assigned IDs are updates, only
        // the requesting client's active resources are searched, and native
        // does not deduplicate two new rows within one request
        // (src/C4Network2Players.cpp:166-181;
        // src/C4PlayerInfo.h:207-213; src/C4PlayerInfo.cpp:568-580).
        let resource_player = |id, resource_id, flags| ControlPlayerInfoEntry {
            id,
            flags: flags | crate::PLAYER_INFO_FLAG_HAS_RESOURCE,
            resource: Some(NetworkResourceCore {
                resource_type: 3,
                id: resource_id,
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![
                resource_player(7, 61, 0),
                resource_player(8, 62, PLAYER_INFO_FLAG_REMOVED),
            ],
            ..Default::default()
        });
        registry.reserve_player_ids_through(8);

        let same_client = registry
            .admit_request(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![
                        resource_player(77, 61, 0),
                        resource_player(0, 62, 0),
                        player(0),
                    ],
                },
                8,
            )
            .expect("assigned, resource-less, and removed-resource rows are exempt");
        assert_eq!(
            same_client
                .players
                .iter()
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![77, 9, 10]
        );

        let other_client = registry
            .admit_request(
                PlayerInfoUpdateRequest {
                    client_id: 4,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![resource_player(0, 61, 0)],
                },
                8,
            )
            .expect("another client's matching resource is not a duplicate");
        assert_eq!(other_client.players[0].id, 11);

        let initial = registry
            .admit_request(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
                    players: vec![resource_player(0, 61, 0)],
                },
                8,
            )
            .expect("initial packets bypass duplicate-resource filtering");
        assert_eq!(initial.players[0].id, 12);

        let within_request = registry
            .admit_request(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![resource_player(0, 64, 0), resource_player(0, 64, 0)],
                },
                8,
            )
            .expect("new rows are not deduplicated against each other");
        assert_eq!(
            within_request
                .players
                .iter()
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![13, 14]
        );
    }

    #[test]
    fn admission_resolves_incoming_duplicate_name_with_c4_byte_folding() {
        let named = |id, name: &[u8], color| ControlPlayerInfoEntry {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            color,
            original_color: color,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 1,
            players: vec![named(1, b"\xc4lice", 0x00f4_0000)],
            ..Default::default()
        });
        registry.reserve_player_ids_through(1);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![named(0, b"\xe4LICE", 0x0000_00f4)],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("the deterministic name cascade is available")
            .expect("the player is admitted");

        assert!(admission.updated_existing.is_empty());
        assert_eq!(admission.admitted.players[0].id, 2);
        assert_eq!(
            admission.admitted.players[0].forced_name.as_bytes(),
            b"\xe4LICE (2)"
        );
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert!(
            registry.get(2).is_none(),
            "the direct control applies it later"
        );
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn higher_priority_incoming_name_displaces_existing_packets_in_registry_order() {
        let named = |id, player_type, color| ControlPlayerInfoEntry {
            id,
            player_type,
            name: LegacyCString::from_bytes(b"Same".to_vec()).unwrap(),
            color,
            original_color: color,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 9,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL,
            players: vec![named(1, crate::PLAYER_INFO_TYPE_SCRIPT, 0x00f4_0000)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![named(2, crate::PLAYER_INFO_TYPE_SCRIPT, 0x0000_c800)],
            ..Default::default()
        });
        registry.reserve_player_ids_through(2);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };
        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 7,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![named(0, PLAYER_INFO_TYPE_USER, 0x0000_00f4)],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("name resolution succeeds")
            .expect("the player is admitted");

        assert_eq!(
            admission
                .updated_existing
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![9, 3]
        );
        assert_eq!(
            admission.updated_existing[0].players[0]
                .forced_name
                .as_bytes(),
            b"Same (3)"
        );
        assert_eq!(
            admission.updated_existing[1].players[0]
                .forced_name
                .as_bytes(),
            b"Same (2)"
        );
        assert_eq!(
            admission.updated_existing[0].flags,
            CLIENT_PLAYER_INFO_FLAG_INITIAL
        );
        assert_eq!(admission.admitted.players[0].forced_name.as_bytes(), b"");
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn readded_packet_keeps_updated_marker_after_returning_to_initial_bytes() {
        let named =
            |id, player_type, name: &[u8], forced_name: &[u8], color| ControlPlayerInfoEntry {
                id,
                player_type,
                name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
                forced_name: LegacyCString::from_bytes(forced_name.to_vec()).unwrap(),
                color,
                original_color: color,
                ..Default::default()
            };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 10,
            players: vec![named(
                1,
                PLAYER_INFO_TYPE_USER,
                b"Same (2)",
                b"Same (3)",
                0x00f4_0000,
            )],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 20,
            players: vec![named(
                2,
                crate::PLAYER_INFO_TYPE_SCRIPT,
                b"Same",
                b"Same (3)",
                0x0000_c800,
            )],
            ..Default::default()
        });
        registry.reserve_player_ids_through(2);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };
        let mut fixed_incoming = named(0, PLAYER_INFO_TYPE_USER, b"Same", b"", 0x0000_00f4);
        fixed_incoming.flags |= PLAYER_INFO_FLAG_ATTRIBUTES_FIXED;

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 30,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![fixed_incoming],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("the ordered name re-add is exactly modelled")
            .expect("the player is admitted");

        assert_eq!(
            admission
                .updated_existing
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );
        assert!(admission.updated_existing[0].players[0]
            .forced_name
            .is_empty());
        assert_eq!(
            admission.updated_existing[1].players[0]
                .forced_name
                .as_bytes(),
            b"Same (3)"
        );
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn retained_attribute_refresh_keeps_readded_packet_updates_in_registry_order() {
        let named =
            |id, player_type, name: &[u8], forced_name: &[u8], color| ControlPlayerInfoEntry {
                id,
                player_type,
                name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
                forced_name: LegacyCString::from_bytes(forced_name.to_vec()).unwrap(),
                color,
                original_color: color,
                ..Default::default()
            };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 10,
            players: vec![named(
                1,
                PLAYER_INFO_TYPE_USER,
                b"Same (2)",
                b"Same (3)",
                0x00f4_0000,
            )],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 20,
            players: vec![named(
                2,
                crate::PLAYER_INFO_TYPE_SCRIPT,
                b"Same",
                b"Same (3)",
                0x0000_c800,
            )],
            ..Default::default()
        });
        let mut fixed = named(3, PLAYER_INFO_TYPE_USER, b"Same", b"", 0x0000_00f4);
        fixed.flags |= PLAYER_INFO_FLAG_ATTRIBUTES_FIXED;
        registry.apply(PlayerInfoControlData {
            client_id: 30,
            players: vec![fixed],
            ..Default::default()
        });
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let updates = registry
            .refresh_player_attributes(None, &[], &mut oracle)
            .expect("the retained ordered name re-add is exactly modelled");

        assert_eq!(
            updates
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![10, 20]
        );
        assert!(updates[0].players[0].forced_name.is_empty());
        assert_eq!(updates[1].players[0].forced_name.as_bytes(), b"Same (3)");
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn admission_color_conflict_uses_three_injected_safe_random_draws() {
        let named = |id, name: &[u8]| ControlPlayerInfoEntry {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            color: 0x00f4_0000,
            original_color: 0x00f4_0000,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 1,
            players: vec![named(1, b"One")],
            ..Default::default()
        });
        registry.reserve_player_ids_through(1);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [10, 20, 30].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![named(0, b"Two")],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("random replacement color is available")
            .expect("the player is admitted");

        assert_eq!(oracle.ranges, vec![302, 302, 302]);
        assert_eq!(admission.admitted.players[0].color, 0x001e_140a);
        assert_eq!(admission.admitted.players[0].original_color, 0x00f4_0000);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert!(admission.updated_existing.is_empty());
    }

    #[test]
    fn admission_reclaims_free_original_color_without_random_draws() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 1,
            players: vec![ControlPlayerInfoEntry {
                id: 1,
                name: LegacyCString::from_bytes(b"Existing".to_vec()).unwrap(),
                color: 0x0000_00f4,
                original_color: 0x0000_00f4,
                ..Default::default()
            }],
            ..Default::default()
        });
        registry.reserve_player_ids_through(1);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Incoming".to_vec()).unwrap(),
                        color: 0x0000_00f4,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("the free original color is exactly modelled")
            .expect("the player is admitted");

        assert_eq!(admission.admitted.players[0].color, 0x00f4_0000);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert!(admission.updated_existing.is_empty());
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn earlier_unshifted_row_conflicting_with_later_shifted_current_rejects_before_random() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Shifted".to_vec()).unwrap(),
                    color: 0x0000_00f4,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let before = registry.retained_rows_snapshot();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [10, 20, 30].into(),
            ranges: Vec::new(),
        };

        let error = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Ordinary".to_vec()).unwrap(),
                        color: 0x0000_00f4,
                        original_color: 0x0000_00f4,
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect_err("the earlier row could randomize onto the shifted original");

        assert!(matches!(
            error,
            TeamColorUpdateError::ConflictResolutionUnavailable { player_id: 1, .. }
        ));
        assert_eq!(registry.retained_rows_snapshot(), before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn higher_priority_shifted_color_reclaims_from_one_lower_blocker_and_readds_it() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 1,
            players: vec![ControlPlayerInfoEntry {
                id: 1,
                player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                name: LegacyCString::from_bytes(b"Lower".to_vec()).unwrap(),
                color: 0x00f4_0000,
                original_color: 0x00f4_0000,
                ..Default::default()
            }],
            ..Default::default()
        });
        registry.reserve_player_ids_through(1);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [10, 20, 30].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Higher".to_vec()).unwrap(),
                        color: 0x0000_00f4,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("one lower-priority blocker is resolved by native re-add")
            .expect("the player is admitted");

        assert_eq!(admission.admitted.players[0].color, 0x00f4_0000);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert_eq!(admission.updated_existing.len(), 1);
        assert_eq!(admission.updated_existing[0].client_id, 1);
        assert_eq!(admission.updated_existing[0].players[0].color, 0x001e_140a);
        assert_eq!(oracle.ranges, vec![302, 302, 302]);
    }

    #[test]
    fn unavailable_alternate_color_rolls_back_id_and_registry() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Blocker".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let before = registry.retained_rows_snapshot();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let error = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Candidate".to_vec()).unwrap(),
                        color: 0x0000_c800,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect_err("the missing host-local alternate can change the result");

        assert_eq!(
            error,
            TeamColorUpdateError::ConflictResolutionUnavailable {
                player_id: 2,
                other_player_id: 1,
            }
        );
        assert_eq!(registry.retained_rows_snapshot(), before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn known_host_local_alternate_color_is_resolved_by_resource_identity() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Blocker".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let resource_id = 77;
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };
        let admission = registry
            .admit_request_with_attributes_and_alternate_colors(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Candidate".to_vec()).unwrap(),
                        color: 0x0000_c800,
                        original_color: 0x00f4_0000,
                        resource: Some(NetworkResourceCore {
                            id: resource_id,
                            ..Default::default()
                        }),
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
                |player| {
                    player
                        .resource
                        .as_ref()
                        .filter(|resource| resource.id == resource_id)
                        .map(|_| 0x0000_00e8)
                },
            )
            .expect("known C4PlayerInfo::dwAlternateColor resolves the conflict")
            .expect("candidate receives an ID before attribute resolution");

        assert_eq!(admission.admitted.players[0].color, 0x0000_00e8);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn known_zero_alternate_color_enters_the_native_random_fallback() {
        let named = |id, name: &[u8], color| ControlPlayerInfoEntry {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            color,
            original_color: color,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            2,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![
                    named(1, b"Original blocker", 0x00f4_0000),
                    named(2, b"Current blocker", 0x0000_00f4),
                ],
                ..Default::default()
            }],
        );
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [10, 20, 30].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes_and_alternate_colors(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Candidate".to_vec()).unwrap(),
                        color: 0x0000_00f4,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                None,
                &[],
                &mut oracle,
                |_| Some(0),
            )
            .expect("wire-deserialized zero alternate is complete native state")
            .expect("candidate is admitted");

        assert_eq!(admission.admitted.players[0].color, 0x001e_140a);
        assert_eq!(oracle.ranges, vec![302, 302, 302]);
    }

    #[test]
    fn runtime_team_admission_preflights_alternate_error_before_random_draw() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Blocker".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let team = |id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1), team(2)],
        };
        let teams_before = teams.clone();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [1].into(),
            ranges: Vec::new(),
        };

        let error = registry
            .admit_remote_request_with_runtime_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Candidate".to_vec()).unwrap(),
                        color: 0x0000_c800,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                &[],
                &mut oracle,
            )
            .expect_err("preflight rejects before the random team tie draw");

        assert_eq!(
            error,
            TeamColorUpdateError::ConflictResolutionUnavailable {
                player_id: 2,
                other_player_id: 1,
            }
        );
        assert_eq!(teams, teams_before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn same_client_add_keeps_updated_existing_and_admitted_packets_separate() {
        let named = |id, player_type, color| ControlPlayerInfoEntry {
            id,
            player_type,
            name: LegacyCString::from_bytes(b"Same".to_vec()).unwrap(),
            color,
            original_color: color,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![named(1, crate::PLAYER_INFO_TYPE_SCRIPT, 0x00f4_0000)],
            ..Default::default()
        });
        registry.reserve_player_ids_through(1);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![named(0, PLAYER_INFO_TYPE_USER, 0x0000_00f4)],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("name resolution succeeds")
            .expect("the add packet is admitted");

        assert_eq!(admission.updated_existing.len(), 1);
        assert_eq!(admission.updated_existing[0].client_id, 3);
        assert_eq!(admission.updated_existing[0].players.len(), 1);
        assert_eq!(admission.updated_existing[0].players[0].id, 1);
        assert_eq!(admission.admitted.client_id, 3);
        assert_eq!(admission.admitted.players.len(), 1);
        assert_eq!(admission.admitted.players[0].id, 2);
        assert_eq!(registry.client_info_ids(3), vec![1]);
        assert!(registry.get(2).is_none());
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn lobby_free_admission_uses_safe_random_tie_for_smallest_team() {
        let team = |id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1), team(2)],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Lobby player".to_vec()).unwrap(),
                        color: 0x0012_3456,
                        original_color: 0x0012_3456,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                false,
                true,
                &[],
                &mut oracle,
            )
            .expect("Free-distribution lobby assignment is available")
            .expect("the player is admitted");

        assert_eq!(oracle.ranges, vec![2]);
        assert_eq!(
            (
                admission.admitted.players[0].id,
                admission.admitted.players[0].team
            ),
            (1, 2)
        );
        assert!(teams.teams[0].player_ids.is_empty());
        assert_eq!(teams.teams[1].player_ids, vec![1]);
    }

    #[test]
    fn requested_team_validation_matches_distribution_origin_capacity_and_existence() {
        struct Case {
            distribution: crate::InitialNetworkTeamDistribution,
            by_host: bool,
            target_team: i32,
            second_team: Option<(i32, Vec<i32>)>,
            expected_team: i32,
        }
        let cases = [
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Free,
                by_host: false,
                target_team: 2,
                second_team: Some((0, Vec::new())),
                expected_team: 2,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Free,
                by_host: false,
                target_team: 2,
                second_team: Some((1, vec![99])),
                expected_team: 1,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Free,
                by_host: false,
                target_team: 99,
                second_team: Some((0, Vec::new())),
                expected_team: 1,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::None,
                by_host: false,
                target_team: 2,
                second_team: Some((0, Vec::new())),
                expected_team: 1,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Random,
                by_host: false,
                target_team: 2,
                second_team: Some((0, Vec::new())),
                expected_team: 1,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Host,
                by_host: false,
                target_team: 2,
                second_team: Some((0, Vec::new())),
                expected_team: 1,
            },
            Case {
                distribution: crate::InitialNetworkTeamDistribution::Host,
                by_host: true,
                target_team: 2,
                second_team: Some((0, Vec::new())),
                expected_team: 2,
            },
        ];

        for case in cases {
            let Case {
                distribution,
                by_host,
                target_team,
                second_team,
                expected_team,
            } = case;
            let team = |id, max_players, player_ids| crate::InitialNetworkTeam {
                id,
                name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
                player_start_index: 0,
                player_ids,
                color: 0x0010_0000 + u32::try_from(id).unwrap(),
                icon_spec: LegacyCString::default(),
                max_players,
            };
            let mut team_rows = vec![team(1, 0, vec![10])];
            if let Some((max_players, player_ids)) = second_team {
                team_rows.push(team(2, max_players, player_ids));
            }
            let mut teams = crate::InitialNetworkTeamMetadata {
                active: true,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: false,
                auto_generate_teams: false,
                last_team_id: 2,
                team_distribution: distribution,
                team_colors: false,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: team_rows,
            };
            let existing = ControlPlayerInfoEntry {
                id: 10,
                name: LegacyCString::from_bytes(b"Existing".to_vec()).unwrap(),
                team: 1,
                color: 0x0012_3456,
                original_color: 0x0012_3456,
                ..Default::default()
            };
            let mut registry = ControlPlayerInfoRegistry::default();
            registry.replace_snapshot(
                10,
                [PlayerInfoControlData {
                    client_id: 3,
                    players: vec![existing.clone()],
                    ..Default::default()
                }],
            );
            let mut requested = existing;
            requested.team = target_team;
            let mut oracle = RecordingTeamAssignmentOracle {
                outcomes: [].into(),
                ranges: Vec::new(),
            };

            let admission = registry
                .admit_request_with_teams_and_attributes(
                    PlayerInfoUpdateRequest {
                        client_id: 3,
                        flags: 0,
                        players: vec![requested],
                    },
                    8,
                    &mut teams,
                    by_host,
                    true,
                    &[],
                    &mut oracle,
                )
                .expect("team validation has no unavailable attribute state")
                .expect("the non-add update remains admitted");

            assert_eq!(
                admission.admitted.players[0].team, expected_team,
                "distribution {:?}, by_host {}, target {}",
                distribution, by_host, target_team
            );
            assert!(oracle.ranges.is_empty());
        }
    }

    #[test]
    fn host_authored_script_runtime_packet_is_team_assigned_without_lobby() {
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![41]), team(2, Vec::new())],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 0,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                        name: LegacyCString::from_bytes(b"Host script".to_vec()).unwrap(),
                        color: 0x0012_3456,
                        original_color: 0x0012_3456,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                true,
                false,
                &[],
                &mut oracle,
            )
            .expect("host-authored team assignment succeeds")
            .expect("the script player is admitted");

        assert_eq!(admission.admitted.players[0].team, 2);
        assert_eq!(teams.teams[1].player_ids, vec![1]);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn free_custom_user_team_assignment_obeys_lobby_state() {
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let base_teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![41]), team(2, Vec::new())],
        };

        for (has_lobby, expected_team) in [(false, 0), (true, 2)] {
            let mut teams = base_teams.clone();
            let mut registry = ControlPlayerInfoRegistry::default();
            let mut oracle = RecordingTeamAssignmentOracle {
                outcomes: [].into(),
                ranges: Vec::new(),
            };
            let admission = registry
                .admit_request_with_teams_and_attributes(
                    PlayerInfoUpdateRequest {
                        client_id: 3,
                        flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                        players: vec![ControlPlayerInfoEntry {
                            name: LegacyCString::from_bytes(b"Runtime user".to_vec()).unwrap(),
                            color: 0x0012_3456,
                            original_color: 0x0012_3456,
                            ..Default::default()
                        }],
                    },
                    8,
                    &mut teams,
                    false,
                    has_lobby,
                    &[],
                    &mut oracle,
                )
                .expect("team assignment is available")
                .expect("the user player is admitted");

            assert_eq!(admission.admitted.players[0].team, expected_team);
            assert!(oracle.ranges.is_empty());
        }
    }

    #[test]
    fn free_custom_runtime_shifted_color_ignores_unreachable_team_color() {
        let team = |id, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let original_color = 0x00f4_0000;
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, original_color), team(2, 0x0000_00f4)],
        };
        let teams_before = teams.clone();
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Runtime user".to_vec()).unwrap(),
                        color: 0x0000_c800,
                        original_color,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                false,
                false,
                &[],
                &mut oracle,
            )
            .expect("the runtime-choice path needs no unavailable alternate")
            .expect("the user player is admitted");

        assert_eq!(admission.admitted.players[0].team, 0);
        assert_eq!(admission.admitted.players[0].color, original_color);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert!(admission.joined_player_team_updates.is_empty());
        assert_eq!(teams, teams_before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn unique_team_assignment_forces_color_before_shifted_conflict_preflight() {
        let team = |id, player_ids, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let conflicting_original = 0x00f4_0000;
        let assigned_color = 0x0000_c800;
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![1], conflicting_original),
                team(2, Vec::new(), assigned_color),
            ],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Existing".to_vec()).unwrap(),
                    team: 1,
                    color: conflicting_original,
                    original_color: conflicting_original,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Shifted".to_vec()).unwrap(),
                        color: 0x0000_00f4,
                        original_color: conflicting_original,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                false,
                false,
                &[],
                &mut oracle,
            )
            .expect("the forced team color removes the alternate-color branch")
            .expect("the shifted player is admitted");

        assert_eq!(
            (
                admission.admitted.players[0].id,
                admission.admitted.players[0].team,
                admission.admitted.players[0].color,
            ),
            (2, 2, assigned_color)
        );
        assert_eq!(teams.teams[1].player_ids, vec![2]);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn automatic_team_assignment_reports_exact_duplicate_id_team_color_for_joined_player() {
        let team = |id, player_ids, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let joined = ControlPlayerInfoEntry {
            id: 7,
            name: LegacyCString::from_bytes(b"Joined".to_vec()).unwrap(),
            flags: PLAYER_INFO_FLAG_JOINED,
            color: 0x0012_3456,
            original_color: 0x0012_3456,
            ..Default::default()
        };
        let assigned_color = 0x0000_c800;
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 1,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![99], 0x00f4_0000),
                team(1, Vec::new(), assigned_color),
            ],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            7,
            [PlayerInfoControlData {
                client_id: 3,
                players: vec![joined.clone()],
                ..Default::default()
            }],
        );
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: 0,
                    players: vec![joined],
                },
                8,
                &mut teams,
                false,
                false,
                &[],
                &mut oracle,
            )
            .expect("the joined row is assignable")
            .expect("the update is admitted");

        assert_eq!(
            (
                admission.admitted.players[0].team,
                admission.admitted.players[0].color
            ),
            (1, assigned_color)
        );
        assert_eq!(
            admission.joined_player_team_updates,
            vec![AdmittedPlayerTeamUpdate {
                info_id: 7,
                team: 1,
                color: Some(assigned_color),
            }]
        );
        assert_eq!(teams.teams[1].player_ids, vec![7]);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn free_lobby_attribute_rejection_precedes_team_tie_rng() {
        let team = |id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1), team(2)],
        };
        let teams_before = teams.clone();
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            1,
            [PlayerInfoControlData {
                client_id: 1,
                players: vec![ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Existing".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let registry_before = registry.retained_rows_snapshot();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };

        let error = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 2,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![ControlPlayerInfoEntry {
                        name: LegacyCString::from_bytes(b"Shifted".to_vec()).unwrap(),
                        color: 0x0000_c800,
                        original_color: 0x00f4_0000,
                        ..Default::default()
                    }],
                },
                8,
                &mut teams,
                false,
                true,
                &[],
                &mut oracle,
            )
            .expect_err("unavailable alternate color rejects before team tie RNG");

        assert!(matches!(
            error,
            TeamColorUpdateError::ConflictResolutionUnavailable { .. }
        ));
        assert_eq!(registry.retained_rows_snapshot(), registry_before);
        assert_eq!(teams, teams_before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn host_runtime_admission_assigns_a_teamless_remote_player_after_allocating_its_id() {
        // The host allocates IDs before AssignTeams. A runtime Random join then
        // performs C4TeamList's ordered reservoir tie scan and AddPlayer forces
        // the current color without changing OriginalColor
        // (src/C4Network2Players.cpp:160-205;
        // src/C4Teams.cpp:53-81,474-542).
        let team = |id, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, 0x00f4_0000), team(2, 0x0000_c800)],
        };
        let original_color = 0x0012_3456;
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![ControlPlayerInfoEntry {
                color: original_color,
                original_color,
                ..Default::default()
            }],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };

        let admitted = registry
            .admit_remote_request_with_runtime_teams(request, 8, &mut teams, &mut oracle)
            .expect("one free runtime player slot accepts the request");

        assert_eq!(oracle.ranges, vec![2]);
        let [player] = admitted.players.as_slice() else {
            panic!("expected one admitted player");
        };
        assert_eq!((player.id, player.team), (1, 2));
        assert_eq!(
            (player.color, player.original_color),
            (0x0000_c800, original_color)
        );
        assert!(teams.teams[0].player_ids.is_empty());
        assert_eq!(teams.teams[1].player_ids, vec![1]);
    }

    #[test]
    fn host_runtime_admission_random_invisible_disables_user_team_choice() {
        // RandomInvisible is a random distribution, so a runtime user cannot
        // defer team selection: the host runs the same least-used reservoir
        // assignment as Random (src/C4Teams.cpp:465-471,474-542).
        let team = |id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: Vec::new(),
            color: 0x0010_0000 + u32::try_from(id).unwrap(),
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::RandomInvisible,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1), team(2)],
        };
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![player(0)],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [0].into(),
            ranges: Vec::new(),
        };

        let admitted = registry
            .admit_remote_request_with_runtime_teams(request, 8, &mut teams, &mut oracle)
            .expect("one free runtime player slot accepts the request");

        assert_eq!(oracle.ranges, vec![2]);
        assert_eq!((admitted.players[0].id, admitted.players[0].team), (1, 2));
        assert!(teams.teams[0].player_ids.is_empty());
        assert_eq!(teams.teams[1].player_ids, vec![1]);
    }

    #[test]
    fn team_player_recheck_retains_valid_order_then_appends_by_player_id() {
        // C4Team::RecheckPlayers first removes stale, wrong-team, and removed
        // entries without disturbing the order of survivors. It then walks
        // GetNextPlayerInfoByID and appends missing, non-removed members in
        // ascending positive ID order; PIF_Joined alone does not exclude one
        // (src/C4Teams.cpp:151-176; src/C4PlayerInfo.h:212;
        // src/C4PlayerInfo.cpp:997-1009,1060-1074).
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![5, 99, 1, 2, 5]), team(2, vec![4, 6, 77])],
        };
        let info = |id, team, flags| ControlPlayerInfoEntry {
            id,
            team,
            flags,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 9,
            players: vec![
                info(5, 1, 0),
                info(1, 2, 0),
                info(4, 2, PLAYER_INFO_FLAG_REMOVED),
            ],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![
                info(6, 2, PLAYER_INFO_FLAG_JOINED),
                info(3, 1, 0),
                info(2, 1, 0),
            ],
            ..Default::default()
        });

        registry.recheck_team_players(&mut teams);

        assert_eq!(teams.teams[0].player_ids, vec![5, 2, 5, 3]);
        assert_eq!(teams.teams[1].player_ids, vec![6, 1]);
    }

    #[test]
    fn lobby_same_team_check_uses_non_removed_player_info_rows() {
        let info = |id, team, flags| ControlPlayerInfoEntry {
            id,
            team,
            flags,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            players: vec![info(1, 0, PLAYER_INFO_FLAG_REMOVED), info(2, 3, 0)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 9,
            players: vec![info(3, 0, 0), info(4, 3, 0)],
            ..Default::default()
        });

        assert!(registry.has_same_team_players(7, 9));
        assert!(!registry.has_same_team_players(7, 8));

        registry.apply(PlayerInfoControlData {
            client_id: 9,
            players: vec![info(4, 3, PLAYER_INFO_FLAG_REMOVED)],
            ..Default::default()
        });
        assert!(!registry.has_same_team_players(7, 9));
    }

    #[test]
    fn random_invisible_lobby_color_requires_the_assigned_team_to_exist() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            players: vec![ControlPlayerInfoEntry {
                team: 4,
                color: 0x0012_3456,
                original_color: 0x0065_4321,
                ..Default::default()
            }],
            ..Default::default()
        });

        assert_eq!(
            registry.first_user_lobby_color(7, true, |team| team == 5),
            Some(0x0012_3456)
        );
        assert_eq!(
            registry.first_user_lobby_color(7, true, |team| team == 4),
            Some(0x0065_4321)
        );
    }

    #[test]
    fn random_team_recheck_moves_only_the_first_unjoined_member_until_balanced() {
        // RecheckTeams chooses the uniquely largest team that still has an
        // unjoined member, moves its first such member to the uniquely
        // smallest team, and stops once the count delta is at most one
        // (src/C4Teams.cpp:688-729).
        for distribution in [
            crate::InitialNetworkTeamDistribution::Random,
            crate::InitialNetworkTeamDistribution::RandomInvisible,
        ] {
            let team = |id, player_ids, color| crate::InitialNetworkTeam {
                id,
                name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
                player_start_index: 0,
                player_ids,
                color,
                icon_spec: LegacyCString::default(),
                max_players: 0,
            };
            let team_one_color = 0x00f4_0000;
            let team_two_color = 0x0000_c800;
            let original_color = 0x0012_3456;
            let mut teams = crate::InitialNetworkTeamMetadata {
                active: true,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: false,
                auto_generate_teams: false,
                last_team_id: 2,
                team_distribution: distribution,
                team_colors: true,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count: 0,
                teams: vec![
                    team(1, vec![1, 2, 3, 4], team_one_color),
                    team(2, vec![5], team_two_color),
                ],
            };
            let mut registry = ControlPlayerInfoRegistry::default();
            registry.apply(PlayerInfoControlData {
                client_id: 3,
                players: (1..=5)
                    .map(|id| ControlPlayerInfoEntry {
                        id,
                        flags: if id == 1 { PLAYER_INFO_FLAG_JOINED } else { 0 },
                        team: if id == 5 { 2 } else { 1 },
                        color: if id == 5 {
                            team_two_color
                        } else {
                            team_one_color
                        },
                        original_color: if id == 2 {
                            original_color
                        } else {
                            team_one_color
                        },
                        ..Default::default()
                    })
                    .collect(),
                ..Default::default()
            });
            let mut oracle = RecordingTeamAssignmentOracle {
                outcomes: [].into(),
                ranges: Vec::new(),
            };

            let updates = registry.recheck_random_teams(&mut teams, &mut oracle);

            assert!(oracle.ranges.is_empty(), "{distribution:?}");
            assert_eq!(updates.len(), 1, "{distribution:?}");
            assert_eq!(updates[0].client_id, 3, "{distribution:?}");
            assert_eq!(updates[0].flags & CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);
            assert_eq!(teams.teams[0].player_ids, vec![1, 3, 4]);
            assert_eq!(teams.teams[1].player_ids, vec![5, 2]);
            assert_eq!(
                teams.teams[0].player_ids.len() - teams.teams[1].player_ids.len(),
                1
            );
            let joined = registry.get(1).expect("joined player remains registered");
            assert_eq!((joined.team, joined.color), (1, team_one_color));
            assert_ne!(joined.flags & PLAYER_INFO_FLAG_JOINED, 0);
            let moved = registry
                .get(2)
                .expect("first unjoined player remains registered");
            assert_eq!((moved.team, moved.color), (2, team_two_color));
            assert_eq!(moved.original_color, original_color);
            assert_eq!(registry.get(3).expect("later unjoined player").team, 1);
            assert_eq!(registry.get(4).expect("last unjoined player").team, 1);
        }
    }

    #[test]
    fn random_team_recheck_three_to_zero_moves_one_and_flushes_one_clean_packet() {
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![1, 2, 3]), team(2, vec![])],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            flags: CLIENT_PLAYER_INFO_FLAG_INITIAL | CLIENT_PLAYER_INFO_FLAG_UPDATED,
            players: (1..=3)
                .map(|id| ControlPlayerInfoEntry {
                    id,
                    team: 1,
                    ..Default::default()
                })
                .collect(),
            by_client: 0,
        });
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let updates = registry.recheck_random_teams(&mut teams, &mut oracle);

        assert!(oracle.ranges.is_empty());
        assert_eq!(teams.teams[0].player_ids, vec![2, 3]);
        assert_eq!(teams.teams[1].player_ids, vec![1]);
        assert_eq!(registry.get(1).map(|player| player.team), Some(2));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].client_id, 7);
        assert_eq!(updates[0].flags, CLIENT_PLAYER_INFO_FLAG_INITIAL);
    }

    #[test]
    fn delayed_echo_reservation_excludes_player_from_random_team_recheck() {
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![1, 2, 3]), team(2, Vec::new())],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            players: (1..=3)
                .map(|id| ControlPlayerInfoEntry {
                    id,
                    team: 1,
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        });
        let reserved = registry.get(1).expect("reserved player exists").clone();
        registry.reserve_unjoined_player_snapshots(&[reserved]);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        registry.recheck_random_teams(&mut teams, &mut oracle);

        assert_eq!(teams.teams[0].player_ids, vec![1, 3]);
        assert_eq!(teams.teams[1].player_ids, vec![2]);
        assert_eq!(registry.get(1).unwrap().team, 1);
        assert_eq!(registry.get(2).unwrap().team, 2);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn balanced_random_team_recheck_still_consumes_the_lowest_team_tie_draw() {
        let team = |id, player_id| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids: vec![player_id],
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, 1), team(2, 2)],
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    team: 1,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    team: 2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [1].into(),
            ranges: Vec::new(),
        };

        let updates = registry.recheck_random_teams(&mut teams, &mut oracle);

        assert!(updates.is_empty());
        assert_eq!(oracle.ranges, vec![2]);
        assert_eq!(teams.teams[0].player_ids, vec![1]);
        assert_eq!(teams.teams[1].player_ids, vec![2]);
    }

    #[test]
    fn random_team_recheck_deduplicates_owners_in_registry_order() {
        let team = |id, player_ids| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color: 0,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::RandomInvisible,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![team(1, vec![1, 2, 3, 4, 5]), team(2, vec![])],
        };
        let info = |id| ControlPlayerInfoEntry {
            id,
            team: 1,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 9,
            players: vec![info(2), info(3), info(4), info(5)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![info(1)],
            ..Default::default()
        });
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let updates = registry.recheck_random_teams(&mut teams, &mut oracle);

        assert!(oracle.ranges.is_empty());
        assert_eq!(teams.teams[0].player_ids, vec![3, 4, 5]);
        assert_eq!(teams.teams[1].player_ids, vec![1, 2]);
        assert_eq!(
            updates
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![9, 3]
        );
    }

    #[test]
    fn random_team_recheck_rebuilds_generated_teams_before_id_order_reassignment() {
        // ReassignAllTeams resets only players without HasJoinIssued, clears
        // a wrong auto-generated team count, generates teams 1 through the
        // configured count (default two), and reassigns in ascending player
        // ID order (src/C4Teams.cpp:386-400,688-700,731-769;
        // src/C4PlayerInfo.cpp:1060-1074).
        for (
            distribution,
            random_team_count,
            oracle_outcomes,
            expected_ranges,
            expected_memberships,
        ) in [
            (
                crate::InitialNetworkTeamDistribution::Random,
                0,
                vec![0],
                vec![2],
                vec![(1, vec![3]), (2, vec![1])],
            ),
            (
                crate::InitialNetworkTeamDistribution::RandomInvisible,
                3,
                vec![0, 0, 1],
                vec![2, 3, 2],
                vec![(1, vec![3]), (2, vec![]), (3, vec![1])],
            ),
        ] {
            let old_team_color = 0x00aa_5500;
            let joined_original_color = 0x0012_3456;
            let mut teams = crate::InitialNetworkTeamMetadata {
                active: true,
                custom: true,
                allow_hostility_change: false,
                allow_team_switch: false,
                auto_generate_teams: true,
                last_team_id: 7,
                team_distribution: distribution,
                team_colors: true,
                max_script_players: 0,
                script_player_names: LegacyCString::default(),
                random_team_count,
                teams: vec![crate::InitialNetworkTeam {
                    id: 7,
                    name: LegacyCString::from_bytes(b"Old team".to_vec()).unwrap(),
                    player_start_index: 0,
                    player_ids: vec![3, 2, 1],
                    color: old_team_color,
                    icon_spec: LegacyCString::default(),
                    max_players: 0,
                }],
            };
            let mut registry = ControlPlayerInfoRegistry::default();
            registry.apply(PlayerInfoControlData {
                client_id: 9,
                players: vec![
                    ControlPlayerInfoEntry {
                        id: 3,
                        team: 7,
                        color: old_team_color,
                        original_color: 0x0033_3333,
                        ..Default::default()
                    },
                    ControlPlayerInfoEntry {
                        id: 1,
                        team: 7,
                        color: old_team_color,
                        original_color: 0x0011_1111,
                        ..Default::default()
                    },
                    ControlPlayerInfoEntry {
                        id: 2,
                        flags: PLAYER_INFO_FLAG_JOINED,
                        team: 7,
                        color: old_team_color,
                        original_color: joined_original_color,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            });
            let mut oracle = GeneratingTeamAssignmentOracle {
                outcomes: oracle_outcomes.into(),
                ..Default::default()
            };

            let updates = registry.recheck_random_teams(&mut teams, &mut oracle);

            assert_eq!(teams.last_team_id, random_team_count.max(2));
            assert_eq!(updates.len(), 1, "{distribution:?}");
            assert_eq!(updates[0].client_id, 9, "{distribution:?}");
            assert_eq!(updates[0].flags & CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);
            assert_eq!(
                teams
                    .teams
                    .iter()
                    .map(|team| (team.id, team.player_ids.clone()))
                    .collect::<Vec<_>>(),
                expected_memberships,
                "{distribution:?}"
            );
            assert_eq!(oracle.ranges, expected_ranges, "{distribution:?}");
            assert_eq!(
                oracle.generation_calls,
                (1..=random_team_count.max(2))
                    .map(|id| (id, (1..id).collect::<Vec<_>>()))
                    .collect::<Vec<_>>(),
                "{distribution:?}"
            );
            let first = registry.get(1).expect("first unjoined player");
            let last = registry.get(3).expect("last unjoined player");
            assert_eq!(
                (first.team, first.color, first.original_color),
                (
                    if random_team_count > 1 { 3 } else { 2 },
                    0x0010_0000 + u32::try_from(if random_team_count > 1 { 3 } else { 2 }).unwrap(),
                    0x0011_1111,
                )
            );
            assert_eq!(
                (last.team, last.color, last.original_color),
                (1, 0x0010_0001, 0x0033_3333)
            );
            let joined = registry.get(2).expect("joined player remains registered");
            assert_eq!(
                (joined.team, joined.color, joined.original_color),
                (7, old_team_color, joined_original_color)
            );
            assert_ne!(joined.flags & PLAYER_INFO_FLAG_JOINED, 0);
        }
    }

    #[test]
    fn full_reassignment_walks_player_ids_but_flushes_packets_in_registry_order() {
        let team = |id, player_ids, color| crate::InitialNetworkTeam {
            id,
            name: LegacyCString::from_bytes(format!("Team {id}").into_bytes()).unwrap(),
            player_start_index: 0,
            player_ids,
            color,
            icon_spec: LegacyCString::default(),
            max_players: 0,
        };
        let mut teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Random,
            team_colors: true,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                team(1, vec![2, 3, 4, 5], 0x00f4_0000),
                team(2, vec![1], 0x0000_00f4),
            ],
        };
        let info = |id, team, flags| ControlPlayerInfoEntry {
            id,
            team,
            flags,
            color: 0x0012_3456,
            original_color: 0x0012_3456,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 9,
            players: vec![info(3, 1, 0), info(1, 2, PLAYER_INFO_FLAG_JOINED)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![info(4, 1, 0), info(2, 1, PLAYER_INFO_FLAG_JOINED)],
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 7,
            players: vec![info(5, 1, 0)],
            ..Default::default()
        });
        registry.issued_join_ids.insert(5);
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [1].into(),
            ranges: Vec::new(),
        };

        let packets = registry.reassign_all_teams(&mut teams, &mut oracle, true);

        assert_eq!(oracle.ranges, vec![2]);
        assert_eq!(teams.teams[0].player_ids, vec![2, 5, 4]);
        assert_eq!(teams.teams[1].player_ids, vec![1, 3]);
        assert_eq!(registry.get(3).map(|player| player.team), Some(2));
        assert_eq!(registry.get(4).map(|player| player.team), Some(1));
        assert_eq!(registry.get(1).map(|player| player.team), Some(2));
        assert_eq!(registry.get(2).map(|player| player.team), Some(1));
        assert_eq!(registry.get(5).map(|player| player.team), Some(1));
        assert_eq!(
            packets
                .iter()
                .map(|packet| packet.client_id)
                .collect::<Vec<_>>(),
            vec![9, 3]
        );
    }

    #[test]
    fn team_color_update_clears_safe_forced_names_and_rolls_back_name_conflicts() {
        let teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 2,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![
                crate::InitialNetworkTeam {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Red".to_vec()).unwrap(),
                    player_start_index: 0,
                    player_ids: vec![1],
                    color: 0x00f4_0000,
                    icon_spec: LegacyCString::default(),
                    max_players: 0,
                },
                crate::InitialNetworkTeam {
                    id: 2,
                    name: LegacyCString::from_bytes(b"Blue".to_vec()).unwrap(),
                    player_start_index: 0,
                    player_ids: vec![2],
                    color: 0x0000_00f4,
                    icon_spec: LegacyCString::default(),
                    max_players: 0,
                },
            ],
        };
        let named =
            |id, team, name: &[u8], forced_name: &[u8], original_color| ControlPlayerInfoEntry {
                id,
                team,
                name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
                forced_name: LegacyCString::from_bytes(forced_name.to_vec()).unwrap(),
                color: original_color,
                original_color,
                ..Default::default()
            };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 5,
            players: vec![
                named(1, 1, b"Alice", b"Alice (2)", 0x00f4_0000),
                named(2, 2, b"Bob", b"", 0x0000_00f4),
            ],
            ..Default::default()
        });

        let packets = registry
            .update_team_colors(&teams, true, &[])
            .expect("distinct original names take the deterministic safe path");
        assert_eq!(packets.len(), 1);
        assert!(registry.get(1).unwrap().forced_name.is_empty());
        assert_eq!(registry.get(1).unwrap().color, 0x00f4_0000);
        assert_eq!(registry.get(2).unwrap().color, 0x0000_00f4);

        let mut conflicting = registry.clone();
        conflicting.get_mut(2).unwrap().name =
            LegacyCString::from_bytes(b"Alice".to_vec()).unwrap();
        conflicting.get_mut(1).unwrap().forced_name =
            LegacyCString::from_bytes(b"Alias".to_vec()).unwrap();
        let before = conflicting.retained_rows_snapshot();
        let error = conflicting
            .update_team_colors(&teams, false, &[])
            .expect_err("colliding originals need native ordered forced-name generation");
        assert!(matches!(
            error,
            TeamColorUpdateError::NameConflictResolutionUnavailable { .. }
        ));
        assert_eq!(conflicting.retained_rows_snapshot(), before);
    }

    #[test]
    fn team_color_conflict_error_preserves_the_complete_registry() {
        let teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 0,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"One".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    name: LegacyCString::from_bytes(b"Two".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let before = registry.retained_rows_snapshot();

        let error = registry
            .update_team_colors(&teams, true, &[])
            .expect_err("equal original colors need alternate/random native state");

        assert!(matches!(
            error,
            TeamColorUpdateError::ConflictResolutionUnavailable { .. }
        ));
        assert_eq!(registry.retained_rows_snapshot(), before);
    }

    #[test]
    fn ordered_current_attribute_blockers_fail_before_mutation() {
        let teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 0,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        };

        let mut color_blocked = ControlPlayerInfoRegistry::default();
        color_blocked.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"One".to_vec()).unwrap(),
                    color: 0x0000_f400,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    name: LegacyCString::from_bytes(b"Two".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x0000_00f4,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let color_before = color_blocked.retained_rows_snapshot();
        assert!(matches!(
            color_blocked.update_team_colors(&teams, true, &[]),
            Err(TeamColorUpdateError::ConflictResolutionUnavailable { .. })
        ));
        assert_eq!(color_blocked.retained_rows_snapshot(), color_before);

        let mut name_blocked = ControlPlayerInfoRegistry::default();
        name_blocked.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                    forced_name: LegacyCString::from_bytes(b"Alias".to_vec()).unwrap(),
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    name: LegacyCString::from_bytes(b"Bob".to_vec()).unwrap(),
                    forced_name: LegacyCString::from_bytes(b"Alice".to_vec()).unwrap(),
                    color: 0x0000_00f4,
                    original_color: 0x0000_00f4,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });
        let name_before = name_blocked.retained_rows_snapshot();
        assert!(matches!(
            name_blocked.update_team_colors(&teams, true, &[]),
            Err(TeamColorUpdateError::NameConflictResolutionUnavailable { .. })
        ));
        assert_eq!(name_blocked.retained_rows_snapshot(), name_before);
    }

    #[test]
    fn missing_restore_color_falls_back_to_team_or_preserves_current_color() {
        let team_color = 0x0000_00f4;
        let teams = crate::InitialNetworkTeamMetadata {
            active: true,
            custom: true,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 1,
            team_distribution: crate::InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: vec![crate::InitialNetworkTeam {
                id: 1,
                name: LegacyCString::from_bytes(b"Blue".to_vec()).unwrap(),
                player_start_index: 0,
                player_ids: vec![1],
                color: team_color,
                icon_spec: LegacyCString::default(),
                max_players: 0,
            }],
        };
        let unchanged_color = 0x0012_3456;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 4,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    name: LegacyCString::from_bytes(b"One".to_vec()).unwrap(),
                    savegame_player: 91,
                    team: 1,
                    color: 0x00f4_0000,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    name: LegacyCString::from_bytes(b"Two".to_vec()).unwrap(),
                    savegame_player: 92,
                    team: 0,
                    color: unchanged_color,
                    original_color: 0x00f4_0000,
                    ..Default::default()
                },
            ],
            ..Default::default()
        });

        let packets = registry
            .update_team_colors(&teams, true, &[])
            .expect("missing restore rows follow native fallback semantics");

        assert_eq!(packets.len(), 1);
        assert_eq!(registry.get(1).unwrap().color, team_color);
        assert_eq!(registry.get(2).unwrap().color, unchanged_color);
    }

    #[test]
    fn host_admission_rejects_an_empty_non_initial_request() {
        // HandlePlayerInfoUpdRequest drops an empty packet unless it carries
        // CIF_Initial, before ID assignment or direct PlayerInfo emission
        // (src/C4Network2Players.cpp:167-190).
        let mut registry = ControlPlayerInfoRegistry::default();
        let request = crate::PlayerInfoUpdateRequest {
            client_id: 3,
            flags: 0,
            players: Vec::new(),
        };

        assert_eq!(registry.admit_request(request, 8), None);
    }

    #[test]
    fn host_issues_a_fileless_script_join_only_once() {
        // JoinUnjoinedPlayersInControlQueue marks JoinIssued before queuing;
        // a script player without a resource uses the filename-null embedded
        // JoinPlayer constructor (src/C4Network2Players.cpp:353-388;
        // src/C4Control.cpp:695-707).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![ControlPlayerInfoEntry {
                id: 7,
                player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                ..Default::default()
            }],
            by_client: 0,
            ..Default::default()
        });

        let joins = registry.issue_unjoined_players(3, |_| None);

        assert_eq!(
            joins,
            vec![JoinPlayerControlData {
                at_client: 3,
                info_id: 7,
                source: JoinPlayerSource::Embedded(Vec::new()),
                by_client: 0,
                ..Default::default()
            }]
        );
        assert!(registry.issue_unjoined_players(3, |_| None).is_empty());
    }

    #[test]
    fn host_issues_resource_join_before_local_resource_completion() {
        // LoadResources creates the resource node while it is still loading;
        // JoinUnjoinedPlayersInControlQueue immediately queues the by-resource
        // control and PreExecute performs the later wait
        // (src/C4Network2Players.cpp:353-388; src/C4Control.cpp:811-825).
        let resource = NetworkResourceCore {
            resource_type: 3,
            id: 61,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Remote.c4p".to_vec())
                .expect("valid resource filename"),
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![ControlPlayerInfoEntry {
                id: 7,
                flags: crate::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(resource.clone()),
                ..Default::default()
            }],
            by_client: 0,
            ..Default::default()
        });

        let joins = registry.issue_unjoined_players(3, |_| None);

        assert_eq!(
            joins,
            vec![JoinPlayerControlData {
                filename: resource.filename.clone(),
                at_client: 3,
                info_id: 7,
                source: JoinPlayerSource::Resource(resource),
                by_client: 0,
            }]
        );
        assert!(registry
            .issue_unjoined_players(3, |_| {
                Some(LegacyCString::from_bytes(b"Complete.c4p".to_vec()).unwrap())
            })
            .is_empty());
    }

    #[test]
    fn host_marks_fileless_user_join_issued_before_dropping_it() {
        // Native sets PIF_JoinIssued before rejecting a user player without a
        // resource, so a later refresh cannot resurrect the failed join
        // (src/C4Network2Players.cpp:353-378).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![ControlPlayerInfoEntry {
                id: 7,
                ..Default::default()
            }],
            by_client: 0,
            ..Default::default()
        });

        assert!(registry.issue_unjoined_players(3, |_| None).is_empty());

        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![ControlPlayerInfoEntry {
                id: 7,
                flags: crate::PLAYER_INFO_FLAG_HAS_RESOURCE,
                resource: Some(NetworkResourceCore {
                    resource_type: 3,
                    id: 61,
                    loadable: true,
                    ..Default::default()
                }),
                ..Default::default()
            }],
            by_client: 0,
            ..Default::default()
        });
        assert!(registry.issue_unjoined_players(3, |_| None).is_empty());
    }

    #[test]
    fn snapshot_join_issuance_survives_later_client_packet_replacement() {
        let script = |id| ControlPlayerInfoEntry {
            id,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };
        let earlier_players = vec![script(7)];
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: earlier_players.clone(),
            ..Default::default()
        });
        registry.apply(PlayerInfoControlData {
            client_id: 3,
            players: vec![script(8)],
            ..Default::default()
        });

        let earlier_echo = registry.issue_unjoined_player_snapshots(3, &earlier_players, |_| None);

        assert_eq!(registry.client_info_ids(3), vec![8]);
        assert_eq!(
            earlier_echo
                .iter()
                .map(|join| join.info_id)
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert!(registry
            .issue_unjoined_player_snapshots(3, &earlier_players, |_| None)
            .is_empty());
        assert_eq!(
            registry
                .issue_unjoined_players(3, |_| None)
                .into_iter()
                .map(|join| join.info_id)
                .collect::<Vec<_>>(),
            vec![8]
        );
    }

    #[test]
    fn reserved_snapshot_join_is_consumed_once_with_incomplete_resource_fallback() {
        let resource = NetworkResourceCore {
            resource_type: 3,
            id: 61,
            loadable: true,
            filename: LegacyCString::from_bytes(b"Remote.c4p".to_vec()).unwrap(),
            ..Default::default()
        };
        let players = vec![ControlPlayerInfoEntry {
            id: 7,
            flags: PLAYER_INFO_FLAG_HAS_RESOURCE,
            resource: Some(resource.clone()),
            ..Default::default()
        }];
        let mut registry = ControlPlayerInfoRegistry::default();

        registry.reserve_unjoined_player_snapshots(&players);

        assert!(
            registry
                .issue_unjoined_player_snapshots(3, &players, |_| None)
                .is_empty(),
            "ordinary issuance sees the reservation's JoinIssued marker"
        );
        assert_eq!(
            registry.issue_reserved_player_snapshots(3, &players, |_| None),
            vec![JoinPlayerControlData {
                filename: resource.filename.clone(),
                at_client: 3,
                info_id: 7,
                source: JoinPlayerSource::Resource(resource),
                by_client: 0,
            }]
        );
        assert!(registry
            .issue_reserved_player_snapshots(3, &players, |_| None)
            .is_empty());
    }

    #[test]
    fn local_issuance_keeps_duplicates_and_marks_missing_filenames_issued() {
        // LocalJoinUnjoinedPlayersInQueue marks JoinIssued before checking the
        // filename, retains packet order and creates the non-resource
        // C4ControlJoinPlayer form for both user files and fileless script
        // players (pristine 9ffa0a5d src/C4PlayerInfo.cpp:1292-1322;
        // src/C4Control.cpp:38-45,695-708).
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 0,
            players: vec![
                ControlPlayerInfoEntry {
                    id: 1,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 2,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 3,
                    ..Default::default()
                },
                ControlPlayerInfoEntry {
                    id: 4,
                    player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
                    ..Default::default()
                },
            ],
            by_client: 0,
            ..Default::default()
        });
        let duplicate = LegacyCString::from_bytes(b"Alice.c4p".to_vec()).expect("valid path");

        let joins = registry.issue_unjoined_local_players(0, |player| {
            matches!(player.id, 1 | 2).then(|| duplicate.clone())
        });

        assert_eq!(
            joins,
            vec![
                JoinPlayerControlData {
                    filename: duplicate.clone(),
                    at_client: 0,
                    info_id: 1,
                    source: JoinPlayerSource::Embedded(Vec::new()),
                    by_client: 0,
                },
                JoinPlayerControlData {
                    filename: duplicate,
                    at_client: 0,
                    info_id: 2,
                    source: JoinPlayerSource::Embedded(Vec::new()),
                    by_client: 0,
                },
                JoinPlayerControlData {
                    at_client: 0,
                    info_id: 4,
                    source: JoinPlayerSource::Embedded(Vec::new()),
                    by_client: 0,
                    ..Default::default()
                },
            ]
        );
        assert!(registry
            .issue_unjoined_local_players(0, |_| panic!("issued entries must not retry"))
            .is_empty());
    }

    #[test]
    fn user_join_combines_player_info_with_player_file_core() {
        // C4Player::Init takes ID/team/name/color from C4PlayerInfo, while
        // C4Player::Load supplies score, preferences and crew from the .c4p
        // (src/C4Player.cpp:246-284,1089-1106).
        let info = ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(b"Network Tyler".to_vec())
                .expect("valid legacy name"),
            id: 7,
            team: 2,
            color: 0x0011_2233,
            ..Default::default()
        };
        let crew = vec![crate::player_file::CrewInfo {
            id: "CLNK".to_string(),
            name: "Ada".to_string(),
            rank: 3,
            rank_name: "Captain".to_string(),
            experience: 50,
            rounds: 6,
            physical: crate::PhysicalInfo::default(),
            ..Default::default()
        }];
        let file = PlayerFile {
            info_core: Default::default(),
            name: "File Tyler".to_string(),
            score: 250,
            rounds: 11,
            rounds_won: 7,
            rounds_lost: 4,
            total_playing_time: 1_234,
            pref_color: 4,
            pref_color_dw: 0x00aa_bbcc,
            pref_color2_dw: 0,
            pref_position: 2,
            pref_control: 0,
            pref_mouse: true,
            pref_control_style: true,
            pref_auto_context_menu: false,
            crew: crew.clone(),
        };
        let join = JoinPlayerControlData {
            info_id: 7,
            ..Default::default()
        };

        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: Some(&file),
            startup_player_count: 2,
        })
        .expect("user join prepares");

        assert_eq!(
            config,
            JoinPlayerConfig {
                name: "Network Tyler".to_string(),
                player_info_id: 7,
                score: 250,
                rounds: 11,
                rounds_won: 7,
                rounds_lost: 4,
                total_playing_time: 1_234,
                team: Some(2),
                color_dw: 0x0011_2233,
                pref_color: 4,
                pref_position: 2,
                crew,
                control_style: true,
                auto_context_menu: false,
                startup_player_count: 2,
            }
        );
    }

    #[test]
    fn savegame_assignment_missing_rows_reset_prune_and_swap_in_attribute_admission() {
        let candidate = |name: &[u8], savegame_player, flags, color| ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            flags,
            color,
            original_color: color,
            savegame_player,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![
                        candidate(b"Retained", 70, 0, 0x00f4_0000),
                        candidate(b"Pruned", 71, PLAYER_INFO_FLAG_SAVEGAME_JOIN, 0x0000_c800),
                        candidate(
                            b"Zero association",
                            0,
                            PLAYER_INFO_FLAG_SAVEGAME_JOIN,
                            0x0000_00f4,
                        ),
                    ],
                },
                8,
                None,
                &[],
                &mut oracle,
            )
            .expect("attribute resolution remains available")
            .expect("post-validation emptiness is not checked here");

        assert_eq!(
            admission
                .admitted
                .players
                .iter()
                .map(|player| player.id)
                .collect::<Vec<_>>(),
            vec![1, 3],
            "RemoveIndexedInfo swaps the already-visited last row into the hole"
        );
        assert!(admission
            .admitted
            .players
            .iter()
            .all(|player| player.savegame_player == 0));
        assert_ne!(
            admission.admitted.players[1].flags & PLAYER_INFO_FLAG_SAVEGAME_JOIN,
            0,
            "an association that arrived as zero is not a failed association"
        );
        assert_eq!(
            admission.admitted.flags,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_UPDATED
        );
        assert_eq!(registry.retained_rows_snapshot().0, 3);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn all_pruned_savegame_add_stays_admitted_in_team_attribute_path() {
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut teams = InitialNetworkTeamMetadata {
            active: false,
            custom: false,
            allow_hostility_change: false,
            allow_team_switch: false,
            auto_generate_teams: false,
            last_team_id: 0,
            team_distribution: InitialNetworkTeamDistribution::Free,
            team_colors: false,
            max_script_players: 0,
            script_player_names: LegacyCString::default(),
            random_team_count: 0,
            teams: Vec::new(),
        };
        let teams_before = teams.clone();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_teams_and_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![
                        ControlPlayerInfoEntry {
                            savegame_player: 80,
                            flags: PLAYER_INFO_FLAG_SAVEGAME_JOIN,
                            ..Default::default()
                        },
                        ControlPlayerInfoEntry {
                            savegame_player: 81,
                            flags: PLAYER_INFO_FLAG_SAVEGAME_JOIN,
                            ..Default::default()
                        },
                    ],
                },
                8,
                &mut teams,
                false,
                false,
                &[],
                &mut oracle,
            )
            .expect("empty post-validation packets need no attribute oracle")
            .expect("native emits an Add packet emptied after ID allocation");

        assert!(admission.admitted.players.is_empty());
        assert_eq!(
            admission.admitted.flags,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS | CLIENT_PLAYER_INFO_FLAG_UPDATED
        );
        assert_eq!(registry.retained_rows_snapshot().0, 2);
        assert_eq!(teams, teams_before);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn same_packet_savegame_duplicate_keeps_the_last_row() {
        let restore = ControlPlayerInfoEntry {
            id: 40,
            color: 0x0012_3456,
            ..Default::default()
        };
        let candidate = |name: &[u8], flags| ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            flags,
            color: restore.color,
            original_color: restore.color,
            savegame_player: restore.id,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        let mut oracle = RecordingTeamAssignmentOracle {
            outcomes: [].into(),
            ranges: Vec::new(),
        };

        let admission = registry
            .admit_request_with_attributes(
                PlayerInfoUpdateRequest {
                    client_id: 3,
                    flags: CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
                    players: vec![
                        candidate(b"Earlier", PLAYER_INFO_FLAG_SAVEGAME_JOIN),
                        candidate(b"Later", 0),
                    ],
                },
                8,
                None,
                std::slice::from_ref(&restore),
                &mut oracle,
            )
            .expect("association validation is deterministic")
            .expect("the later duplicate survives");

        assert_eq!(admission.admitted.players.len(), 1);
        assert_eq!(admission.admitted.players[0].id, 2);
        assert_eq!(admission.admitted.players[0].name.as_bytes(), b"Later");
        assert_eq!(admission.admitted.players[0].savegame_player, 40);
        assert_ne!(
            admission.admitted.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED,
            0
        );
        assert_eq!(registry.retained_rows_snapshot().0, 2);
        assert!(oracle.ranges.is_empty());
    }

    #[test]
    fn retained_savegame_association_blocks_add_and_other_client_but_not_replacement() {
        let restore = ControlPlayerInfoEntry {
            id: 40,
            color: 0x0012_3456,
            ..Default::default()
        };
        let associated = |id, name: &[u8]| ControlPlayerInfoEntry {
            id,
            name: LegacyCString::from_bytes(name.to_vec()).unwrap(),
            color: 0x0000_00f4,
            original_color: 0x0000_00f4,
            savegame_player: restore.id,
            ..Default::default()
        };
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            10,
            [PlayerInfoControlData {
                client_id: 3,
                players: vec![associated(10, b"Existing")],
                ..Default::default()
            }],
        );
        let admit = |registry: &mut ControlPlayerInfoRegistry, client_id, flags, name: &[u8]| {
            let mut oracle = RecordingTeamAssignmentOracle {
                outcomes: [].into(),
                ranges: Vec::new(),
            };
            let admission = registry
                .admit_request_with_attributes(
                    PlayerInfoUpdateRequest {
                        client_id,
                        flags,
                        players: vec![associated(0, name)],
                    },
                    8,
                    None,
                    std::slice::from_ref(&restore),
                    &mut oracle,
                )
                .expect("attributes remain conflict-free")
                .expect("one row remains admitted");
            assert!(oracle.ranges.is_empty());
            admission.admitted
        };

        let same_client_add = admit(
            &mut registry,
            3,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            b"Same-client add",
        );
        assert_eq!(same_client_add.players[0].savegame_player, 0);
        assert_ne!(same_client_add.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);

        let same_client_replacement = admit(&mut registry, 3, 0, b"Replacement");
        assert_eq!(same_client_replacement.players[0].savegame_player, 40);

        let other_client_add = admit(
            &mut registry,
            4,
            CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            b"Other-client add",
        );
        assert_eq!(other_client_add.players[0].savegame_player, 0);
        assert_ne!(other_client_add.flags & CLIENT_PLAYER_INFO_FLAG_UPDATED, 0);
    }

    #[test]
    fn savegame_resume_promotes_associated_unjoined_info_before_recreation() {
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.apply(PlayerInfoControlData {
            client_id: 0,
            players: vec![ControlPlayerInfoEntry {
                id: 91,
                savegame_player: 7,
                flags: PLAYER_INFO_FLAG_REMOVED
                    | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
                    | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                    | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK
                    | PLAYER_INFO_FLAG_INVISIBLE,
                team: 3,
                ..Default::default()
            }],
            ..Default::default()
        });

        assert!(registry.resume_joined_savegame_player(7, 5, false));

        let resumed = registry.get(7).expect("associated info takes saved ID");
        assert_eq!(resumed.team, 5);
        assert_eq!(
            resumed.flags & PLAYER_INFO_FLAG_JOINED,
            PLAYER_INFO_FLAG_JOINED
        );
        assert_eq!(resumed.flags & PLAYER_INFO_FLAG_REMOVED, 0);
        assert_eq!(resumed.flags & PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK, 0);
        assert_ne!(resumed.flags & PLAYER_INFO_FLAG_ATTRIBUTES_FIXED, 0);
        assert_ne!(resumed.flags & PLAYER_INFO_FLAG_NO_SCENARIO_INIT, 0);
        assert_ne!(resumed.flags & PLAYER_INFO_FLAG_INVISIBLE, 0);
        assert_eq!(registry.recreation_players(), vec![(0, 7)]);
    }

    #[test]
    fn exact_savegame_resume_copies_only_native_takeover_fields() {
        let current_name = LegacyCString::from_bytes(b"Current identity".to_vec()).unwrap();
        let current_filename = LegacyCString::from_bytes(b"Current.c4p".to_vec()).unwrap();
        let current_flags = PLAYER_INFO_FLAG_SAVEGAME_JOIN
            | crate::PLAYER_INFO_FLAG_DISCONNECTED
            | crate::PLAYER_INFO_FLAG_WON
            | PLAYER_INFO_FLAG_REMOVED
            | PLAYER_INFO_FLAG_ATTRIBUTES_FIXED
            | PLAYER_INFO_FLAG_NO_ELIMINATION_CHECK;
        let mut registry = ControlPlayerInfoRegistry::default();
        registry.replace_snapshot(
            100,
            [PlayerInfoControlData {
                client_id: 3,
                players: vec![ControlPlayerInfoEntry {
                    name: current_name.clone(),
                    filename: current_filename.clone(),
                    flags: current_flags,
                    id: 91,
                    color: 0x0011_2233,
                    original_color: 0x0044_5566,
                    savegame_player: 7,
                    team: 3,
                    league_score: 55,
                    ..Default::default()
                }],
                ..Default::default()
            }],
        );
        let restore = ControlPlayerInfoEntry {
            name: LegacyCString::from_bytes(b"Saved identity".to_vec()).unwrap(),
            filename: LegacyCString::from_bytes(b"Saved.c4p".to_vec()).unwrap(),
            flags: PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_JOIN_ISSUED
                | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | PLAYER_INFO_FLAG_INVISIBLE
                | crate::PLAYER_INFO_FLAG_VOTED_OUT,
            id: 7,
            color: 0x00aa_bbcc,
            original_color: 0x00dd_eeff,
            team: 5,
            league_score: 99,
            ..Default::default()
        };

        assert!(registry.resume_savegame_player_from_info(&restore));

        let resumed = registry.get(7).expect("the saved ID replaces the join ID");
        assert_eq!(resumed.name, current_name);
        assert_eq!(resumed.filename, current_filename);
        assert_eq!(resumed.original_color, 0x0044_5566);
        assert_eq!(resumed.savegame_player, 7);
        assert_eq!(resumed.league_score, 55);
        assert_eq!((resumed.color, resumed.team), (0x00aa_bbcc, 5));
        assert_eq!(
            resumed.flags,
            PLAYER_INFO_FLAG_SAVEGAME_JOIN
                | crate::PLAYER_INFO_FLAG_DISCONNECTED
                | crate::PLAYER_INFO_FLAG_WON
                | PLAYER_INFO_FLAG_JOINED
                | PLAYER_INFO_FLAG_JOIN_ISSUED
                | PLAYER_INFO_FLAG_NO_SCENARIO_INIT
                | PLAYER_INFO_FLAG_INVISIBLE
        );
        assert_eq!(registry.retained_rows_snapshot().0, 100);
        assert_eq!(registry.recreation_players(), vec![(3, 7)]);
    }

    #[test]
    fn script_player_without_file_prepares_cpp_core_defaults() {
        // C4Player::Init permits a missing core file only for script players;
        // C4PlayerInfoCore defaults remain in force before PlayerInfo supplies
        // name/team/color (src/C4Player.cpp:256-284;
        // src/C4InfoCore.cpp:66-85).
        let info = ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(b"Script Tyler".to_vec())
                .expect("valid legacy name"),
            id: 9,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            color: 0x0044_5566,
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: 9,
            ..Default::default()
        };

        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 1,
        })
        .expect("script player prepares without a file");

        assert_eq!(config.name, "Script Tyler");
        assert_eq!(config.player_info_id, 9);
        assert_eq!((config.score, config.total_playing_time), (0, 0));
        assert_eq!(config.color_dw, 0x0044_5566);
        assert_eq!((config.pref_color, config.pref_position), (0, 0));
        assert!(config.crew.is_empty());
        assert!(!config.control_style);
        assert!(!config.auto_context_menu);
    }

    #[test]
    fn script_player_join_preserves_non_utf8_name_bytes() {
        let info = ControlPlayerInfoEntry {
            name: crate::LegacyCString::from_bytes(vec![0xff]).expect("name has no NUL"),
            id: 9,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };
        let join = JoinPlayerControlData {
            info_id: 9,
            ..Default::default()
        };

        let config = prepare_join_player_config(JoinPlayerPreparation {
            join: &join,
            info: &info,
            player_file: None,
            startup_player_count: 1,
        })
        .expect("script player prepares without a file");

        assert_eq!(clonk_script::c4_string_bytes(&config.name), [0xff]);
    }

    #[test]
    fn remote_embedded_join_uses_player_data_not_the_transmitted_path() {
        // Remote non-resource joins save PlrData and load that temporary .c4p;
        // the transmitted Filename is not opened (src/C4Control.cpp:731-744).
        let join = JoinPlayerControlData {
            filename: crate::LegacyCString::from_bytes(
                b"/definitely/missing/RemotePlayer.c4p".to_vec(),
            )
            .expect("valid legacy filename"),
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(
                include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec(),
            ),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let RemoteEmbeddedPlayerData::PlayerFile(file) =
            resolve_remote_embedded_player_data(&join, &info)
                .expect("embedded player data resolves")
        else {
            panic!("user player data must resolve to a player file");
        };

        assert_eq!((file.name.as_str(), file.score), ("Embedded Tyler", 42));
    }

    #[test]
    fn remote_embedded_join_rejects_non_gzip_player_data() {
        // CStdFile recognizes packed C4Groups only by the custom 1e8c or
        // standard 1f8b gzip magic (src/CStdFile.cpp:92-107;
        // src/StdGzCompressedFile.cpp:62-114).
        let mut data = include_bytes!("../tests/fixtures/embedded_player.c4p").to_vec();
        data[..2].copy_from_slice(&[0, 0]);
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(data),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let error = resolve_remote_embedded_player_data(&join, &info)
            .expect_err("raw or non-gzip player data must be rejected");

        assert!(matches!(
            error,
            ResolveRemoteEmbeddedPlayerDataError::UnsupportedArchiveMagic { info_id: 7 }
        ));
    }

    #[test]
    fn remote_user_join_requires_embedded_player_data() {
        // A remote non-resource user join with empty PlrData is rejected as a
        // ghost player (src/C4Control.cpp:750-755).
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(Vec::new()),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            ..Default::default()
        };

        let error = resolve_remote_embedded_player_data(&join, &info)
            .expect_err("empty user player data must be rejected");

        assert!(matches!(
            error,
            ResolveRemoteEmbeddedPlayerDataError::MissingPlayerData { info_id: 7 }
        ));
    }

    #[test]
    fn remote_script_join_accepts_empty_player_data() {
        // Remote script players join without a player filename when PlrData is
        // empty (src/C4Control.cpp:745-749).
        let join = JoinPlayerControlData {
            info_id: 7,
            source: crate::JoinPlayerSource::Embedded(Vec::new()),
            ..Default::default()
        };
        let info = ControlPlayerInfoEntry {
            id: 7,
            player_type: crate::PLAYER_INFO_TYPE_SCRIPT,
            ..Default::default()
        };

        let resolved = resolve_remote_embedded_player_data(&join, &info)
            .expect("empty script player data is valid");

        assert!(matches!(
            resolved,
            RemoteEmbeddedPlayerData::ScriptWithoutFile
        ));
    }
}
