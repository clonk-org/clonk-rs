use crate::{
    player_file::PlayerFile, InitialNetworkTeam, InitialNetworkTeamDistribution,
    InitialNetworkTeamMetadata, JoinPlayerConfig, JoinPlayerControlData, JoinPlayerSource,
    LegacyCString, NetworkResourceCore, PlayerInfoUpdateRequest, ScenarioError,
    PLAYER_INFO_FLAG_JOINED, PLAYER_INFO_FLAG_REMOVED, PLAYER_INFO_TYPE_USER,
};
use crate::{
    ControlPlayerInfoEntry, PlayerInfoControlData, CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
    CLIENT_PLAYER_INFO_FLAG_INITIAL,
};
use std::collections::{BTreeMap, HashSet};

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
    assign_initial_player_teams(teams, players, oracle, true);
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
    assign_initial_player_teams(teams, players, oracle, false);
}

fn assign_initial_player_teams(
    teams: &mut InitialNetworkTeamMetadata,
    players: &mut [ControlPlayerInfoEntry],
    oracle: &mut impl InitialHostTeamAssignmentOracle,
    has_or_will_have_lobby: bool,
) {
    if !teams.active {
        return;
    }

    for player in players {
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
                InitialNetworkTeamDistribution::Free | InitialNetworkTeamDistribution::Host
            );
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
    }
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
                client.activated = update.data != 0;
                client.observer = false;
            }
            crate::CLIENT_UPDATE_SET_OBSERVER => {
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
            let lag_frames = (i64::from(ping_ms) * i64::from(frames_per_second) / 500)
                .clamp(0, 100);
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
#[derive(Debug, Default)]
pub struct ControlPlayerInfoRegistry {
    clients: Vec<ClientPlayerInfos>,
    last_player_id: i32,
    issued_join_ids: HashSet<i32>,
}

#[derive(Debug)]
struct ClientPlayerInfos {
    client_id: i32,
    players: Vec<ControlPlayerInfoEntry>,
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
        clients.into_iter().for_each(|client| self.apply(client));
    }

    /// Apply the ID-allocation and slot-pruning portion of the host's
    /// `HandlePlayerInfoUpdRequest` path. Nonzero IDs remain untouched exactly
    /// like `C4PlayerInfoList::AssignPlayerIDs`
    /// (src/C4PlayerInfo.cpp:781-807,1765-1775).
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
                assign_initial_player_teams(teams, players, oracle, false);
            }
        })
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
    ) {
        if !matches!(
            teams.team_distribution,
            InitialNetworkTeamDistribution::Random
                | InitialNetworkTeamDistribution::RandomInvisible
        ) {
            return;
        }

        let generated_team_count = teams.random_team_count.max(2);
        if teams.auto_generate_teams
            && teams.teams.len()
                != usize::try_from(generated_team_count).unwrap_or(usize::MAX)
        {
            self.reassign_all_random_teams(teams, generated_team_count, oracle);
            return;
        }

        let team_count = if teams.random_team_count > 1 {
            usize::try_from(teams.random_team_count).unwrap_or(usize::MAX)
        } else {
            teams.teams.len()
        };
        loop {
            let Some(lowest_team) = initial_random_smallest_team(
                &teams.teams,
                true,
                teams.random_team_count,
                oracle,
            ) else {
                break;
            };
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
            let Some(player_id) =
                self.first_unissued_team_player(&teams.teams[largest_team])
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

            teams.teams[largest_team].player_ids.remove(player_index);
            teams.teams[lowest_team].player_ids.push(player_id);
            let team_colors = teams.team_colors;
            let Some(player) = self.get_mut(player_id) else {
                break;
            };
            player.team = target_team_id;
            if team_colors {
                player.color = target_team_color;
            }
        }
    }

    fn reassign_all_random_teams(
        &mut self,
        teams: &mut InitialNetworkTeamMetadata,
        generated_team_count: i32,
        oracle: &mut impl InitialHostTeamAssignmentOracle,
    ) {
        let mut player_ids = self
            .clients
            .iter()
            .flat_map(|client| &client.players)
            .map(|player| player.id)
            .collect::<Vec<_>>();
        player_ids.sort_unstable();
        player_ids.dedup();

        for &player_id in &player_ids {
            if self.issued_join_ids.contains(&player_id) {
                continue;
            }
            let Some(player) = self.get_mut(player_id) else {
                continue;
            };
            if player.flags & PLAYER_INFO_FLAG_JOINED == 0 {
                player.team = 0;
            }
        }

        teams.teams.clear();
        teams.last_team_id = 0;
        initial_generate_teams_through(teams, generated_team_count, oracle);

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
            assign_initial_player_teams(teams, std::slice::from_mut(player), oracle, true);
        }
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
        if request.players.is_empty()
            && request.flags & CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS != 0
        {
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
                existing.players = players;
            }
        } else {
            self.clients.push(ClientPlayerInfos { client_id, players });
        }
    }

    pub fn issue_unjoined_players(
        &mut self,
        client_id: i32,
        mut path_for_resource: impl FnMut(&NetworkResourceCore) -> Option<LegacyCString>,
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
                    || player.savegame_player != 0
                    || issued_join_ids.contains(&player.id)
                {
                    return None;
                }
                // C++ sets PIF_JoinIssued before validating the resource so a
                // failed fileless user join is not retried forever.
                issued_join_ids.insert(player.id);
                let (filename, source) = match player.resource.as_ref() {
                    Some(resource) => (
                        path_for_resource(resource)?,
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
            })
            .collect()
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

    /// Empty Initial packets are semantically present in C++ and mark a
    /// client as having completed its first player-info exchange.
    pub fn contains_client(&self, client_id: i32) -> bool {
        self.clients
            .iter()
            .any(|client| client.client_id == client_id)
    }

    pub fn mark_joined(&mut self, info_id: i32) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.flags |= PLAYER_INFO_FLAG_JOINED;
        true
    }

    pub fn mark_removed(&mut self, info_id: i32, disconnected: bool) -> bool {
        let Some(info) = self.get_mut(info_id) else {
            return false;
        };
        info.flags |= PLAYER_INFO_FLAG_JOINED | PLAYER_INFO_FLAG_REMOVED;
        if disconnected {
            info.flags |= crate::PLAYER_INFO_FLAG_DISCONNECTED;
        }
        true
    }

    pub fn get(&self, info_id: i32) -> Option<&ControlPlayerInfoEntry> {
        self.clients
            .iter()
            .flat_map(|client| &client.players)
            .find(|player| player.id == info_id)
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
    #[error("script player {info_id} is not supported yet")]
    UnsupportedScriptPlayer { info_id: i32 },
    #[error("NoScenarioInit player {info_id} is not supported yet")]
    UnsupportedNoScenarioInit { info_id: i32 },
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
    PlayerFile::load_from_bytes(label, data.clone())
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
    if input.info.no_scenario_init() {
        return Err(PrepareJoinPlayerError::UnsupportedNoScenarioInit {
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
    .map(|name| name.to_string_lossy().into_owned())
    .unwrap_or_default();

    Ok(JoinPlayerConfig {
        name,
        player_info_id: input.info.id,
        score: file.score,
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
            self.outcomes.pop_front().expect("recorded SafeRandom result")
        }

        fn generate_team(
            &mut self,
            id: i32,
            existing_teams: &[crate::InitialNetworkTeam],
        ) -> crate::InitialNetworkTeam {
            self.generation_calls.push((
                id,
                existing_teams.iter().map(|team| team.id).collect(),
            ));
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

    fn player(id: i32) -> ControlPlayerInfoEntry {
        ControlPlayerInfoEntry {
            id,
            ..Default::default()
        }
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
        assign_initial_offline_player_teams(
            &mut free_teams,
            &mut free_players,
            &mut free_oracle,
        );

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

        let admitted = clients.activation_update_for_request(
            3,
            1_880,
            2_000,
            true,
            true,
            2_000,
            36,
        );
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
        assert!(registry.mark_joined(7));
        assert!(registry.mark_removed(7, true));
        registry.on_client_part(3);

        let retained = registry.get(7).expect("joined history remains");
        assert_ne!(retained.flags & PLAYER_INFO_FLAG_JOINED, 0);
        assert_ne!(retained.flags & PLAYER_INFO_FLAG_REMOVED, 0);
        assert_ne!(retained.flags & crate::PLAYER_INFO_FLAG_DISCONNECTED, 0);
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
                        flags: if id == 1 {
                            PLAYER_INFO_FLAG_JOINED
                        } else {
                            0
                        },
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

            registry.recheck_random_teams(&mut teams, &mut oracle);

            assert!(oracle.ranges.is_empty(), "{distribution:?}");
            assert_eq!(teams.teams[0].player_ids, vec![1, 3, 4]);
            assert_eq!(teams.teams[1].player_ids, vec![5, 2]);
            assert_eq!(teams.teams[0].player_ids.len() - teams.teams[1].player_ids.len(), 1);
            let joined = registry.get(1).expect("joined player remains registered");
            assert_eq!((joined.team, joined.color), (1, team_one_color));
            assert_ne!(joined.flags & PLAYER_INFO_FLAG_JOINED, 0);
            let moved = registry.get(2).expect("first unjoined player remains registered");
            assert_eq!((moved.team, moved.color), (2, team_two_color));
            assert_eq!(moved.original_color, original_color);
            assert_eq!(registry.get(3).expect("later unjoined player").team, 1);
            assert_eq!(registry.get(4).expect("last unjoined player").team, 1);
        }
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

            registry.recheck_random_teams(&mut teams, &mut oracle);

            assert_eq!(teams.last_team_id, random_team_count.max(2));
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
                    0x0010_0000 + u32::try_from(if random_team_count > 1 { 3 } else { 2 })
                        .unwrap(),
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
            experience: 50,
            participation: 1,
            in_action: false,
            has_died: false,
        }];
        let file = PlayerFile {
            name: "File Tyler".to_string(),
            score: 250,
            total_playing_time: 1_234,
            pref_color: 4,
            pref_color_dw: 0x00aa_bbcc,
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
