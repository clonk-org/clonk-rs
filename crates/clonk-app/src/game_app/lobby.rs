//! `impl GameApp` — lobby methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn console_lobby_active(&self) -> bool {
        self.league_player_auth_lobby_active()
    }

    fn configured_console_lobby_countdown(&self) -> i32 {
        self.staged_network_host_scenario
            .as_ref()
            .map(|staged| staged.lobby.countdown_seconds)
            .or_else(|| {
                native_config_text(
                    &load_native_config_bytes(self.app_paths.as_ref()),
                    "Lobby",
                    "CountdownTime",
                )
                .and_then(|value| value.trim().parse::<i32>().ok())
            })
            .unwrap_or(DEFAULT_LOBBY_COUNTDOWN_SECONDS)
    }

    pub(crate) fn process_console_lobby_start(&mut self, line: &str) -> Result<()> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            let message = self.classic_lobby_resource_text("IDS_MSG_CMD_HOSTONLY", "Host only!");
            tracing::warn!(%message, command = line, "console lobby command rejected");
            self.append_lobby_command_log(message);
            return Ok(());
        }

        let countdown_seconds = match line.find(' ') {
            None => self.configured_console_lobby_countdown(),
            Some(space) => {
                let parameter = line[space..].trim_start_matches(' ');
                match legacy_sscanf_decimal_prefix(&clonk_script::c4_string_bytes(parameter))
                    .filter(|seconds| *seconds >= 0)
                {
                    Some(seconds) => seconds,
                    None => {
                        let message = self.classic_lobby_resource_text(
                            "IDS_MSG_CMD_START_USAGE",
                            "Usage: /start [timer]",
                        );
                        tracing::warn!(%message, command = line, "console lobby command rejected");
                        self.append_lobby_command_log(message);
                        return Ok(());
                    }
                }
            }
        };

        // C4Network2::StartLobbyCountdown replaces an existing timer.
        self.abort_network_lobby_countdown();
        self.start_console_lobby_countdown_with(countdown_seconds)?;
        Ok(())
    }

    pub(crate) fn sync_network_lobby_game_option_state(&mut self) {
        if self.classic_host_lobby.is_some() {
            // The exact host lobby owns the retained strip lifecycle.
            return;
        }
        let Some((is_host, countdown)) = self
            .network_lobby
            .as_ref()
            .map(|lobby| (lobby.is_host, lobby.controller.countdown().is_locked()))
        else {
            return;
        };
        let context = if is_host {
            GameOptionContext::LobbyHost
        } else {
            GameOptionContext::LobbyClient
        };

        let league = self.network_is_league;
        let fair_crew = self.engine.use_fair_crew();
        let fair_crew_strength = self.engine.fair_crew_strength();
        let fair_crew_forced = self.engine.fair_crew_forced();
        if self.scenario_game_options.context() != context {
            let mut values = self.scenario_game_options.values().clone();
            values.lobby_is_league = league;
            values.fair_crew = fair_crew;
            values.fair_crew_strength = fair_crew_strength;
            values.lobby_fair_crew_forced = fair_crew_forced;
            values.countdown = countdown;
            self.scenario_game_options = GameOptionButtons::new(context, values);
        } else {
            self.scenario_game_options.set_lobby_league(league);
            self.scenario_game_options.set_lobby_fair_crew_state(
                fair_crew,
                fair_crew_strength,
                fair_crew_forced,
            );
            self.scenario_game_options.set_countdown(countdown);
        }
        self.sync_scenario_game_option_bounds();
    }

    /// The reconstructed (non-exact-host) network lobby adapter is receiving
    /// startup input.
    pub(crate) fn joined_network_lobby_active(&self) -> bool {
        self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.classic_host_lobby.is_none()
            && self.network_lobby.is_some()
    }

    pub(crate) fn classic_host_lobby_active(&self) -> bool {
        self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.classic_host_lobby.is_some()
    }

    pub(crate) fn note_classic_lobby_non_pointer_input(&mut self) {
        if self.mode != AppMode::Menu || self.startup_view != StartupView::NetworkLobby {
            return;
        }
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.note_non_pointer_input();
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.sync_classic_controller();
            lobby.controller.note_non_pointer_input();
        } else {
            return;
        }
        self.scenario_game_options.note_non_pointer_input();
    }

    pub(crate) fn has_or_will_have_network_lobby(&self) -> bool {
        self.network.is_some()
            && matches!(self.mode, AppMode::Menu)
            && (self.network_lobby.is_some() || self.classic_host_lobby.is_some())
    }

    /// `C4Network2::isLobbyActive` is false as soon as a non-lobby status is
    /// installed, even while the UI still renders its transition screen.
    pub(crate) fn league_player_auth_lobby_active(&self) -> bool {
        if self.mode != AppMode::Menu
            || self
                .pending_client_start_status
                .is_some_and(|status| status.state != clonk_network::NETWORK_STATE_LOBBY)
            || self
                .runtime_network_status_barrier
                .is_some_and(|pending| pending.status.state != clonk_network::NETWORK_STATE_LOBBY)
        {
            return false;
        }
        self.network_lobby.is_some() || self.classic_host_lobby_active()
    }

    pub(crate) fn acknowledge_initial_lobby_status_if_ready(&mut self) {
        if !self.initial_lobby_status_ack_pending
            || self.network_lobby.is_none()
            || self.startup_view != StartupView::NetworkLobby
        {
            return;
        }
        let Some(current_control_tick) = self
            .network_control_clock
            .map(NetworkControlClock::current_tick)
        else {
            tracing::error!("cannot acknowledge initial lobby status without a control clock");
            return;
        };
        let current_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        match self.network.as_mut().map(|network| {
            network.acknowledge_requested_status_at_frame(current_control_tick, current_frame)
        }) {
            Some(Ok(())) => self.initial_lobby_status_ack_pending = false,
            Some(Err(error)) => {
                tracing::error!(%error, "failed to acknowledge initial lobby status");
            }
            None => {}
        }
    }

    fn submit_lobby_network_player(
        &mut self,
        source_path: PathBuf,
        wire_name: &str,
    ) -> Result<(), String> {
        self.submit_network_player_path(&source_path, wire_name, false)
    }

    pub(crate) fn control_message_has_lobby(&self) -> bool {
        self.classic_host_lobby_active() || self.joined_network_lobby_active()
    }

    pub(crate) fn control_message_lobby_chat_color(&self, client_id: i32) -> u32 {
        if self.control_clients.is_activated(client_id) {
            let hide_assigned_team_color = self.control_message_has_lobby()
                && self.engine.team_distribution() == 4
                && self.engine.team_colors();
            let teams = self.engine.teams();
            self.control_player_infos
                .first_user_lobby_color(client_id, hide_assigned_team_color, |team_id| {
                    teams.iter().any(|team| team.id == team_id)
                })
                .unwrap_or(0x00ff_ffff)
        } else {
            0x00ff_ffff
        }
    }

    pub(crate) fn classic_lobby_labels(&self) -> LobbyLabels {
        let mut labels = LobbyLabels::default();
        let resource = |key, fallback: &str| self.runtime_resource_text(key, fallback);
        labels.lobby = resource("IDS_DLG_LOBBY", &labels.lobby);
        labels.players_template = resource("IDS_DLG_PLAYERS", "&Players (%d/%d)")
            .replacen("%d", "{active}", 1)
            .replacen("%d", "{maximum}", 1);
        labels.options = resource("IDS_DLG_OPTIONS", &labels.options);
        labels.chat = resource("IDS_CTL_CHAT", &labels.chat);
        labels.exit = resource("IDS_DLG_EXIT", &labels.exit);
        labels.start = resource("IDS_DLG_GAMEGO", &labels.start);
        labels.cancel = resource("IDS_DLG_CANCEL", &labels.cancel);
        labels.ready = resource("IDS_DLG_READY", &labels.ready);
        labels.preload = resource("IDS_DLG_PRELOAD", &labels.preload);
        labels.still_loading = resource("IDS_DLG_STILLLOADING", &labels.still_loading);
        labels.countdown_template = resource("IDS_PRC_COUNTDOWN", &labels.countdown_template)
            .replacen("%d", "{seconds}", 1);
        labels.start_aborted = resource("IDS_PRC_STARTABORTED", &labels.start_aborted);
        labels.tooltip_chat = resource("IDS_DLGTIP_CHAT", &labels.tooltip_chat);
        labels.tooltip_exit = resource("IDS_DLGTIP_EXIT", &labels.tooltip_exit);
        labels.tooltip_start = resource("IDS_DLGTIP_GAMEGO", &labels.tooltip_start);
        labels.tooltip_ready = resource("IDS_DLGTIP_READY", &labels.tooltip_ready);
        labels.tooltip_ready_unavailable = resource(
            "IDS_DLGTIP_READYNOTAVAILABLE",
            &labels.tooltip_ready_unavailable,
        );
        labels.tooltip_preload = resource("IDS_DLGTIP_PRELOAD", &labels.tooltip_preload);
        labels.tooltip_ping = resource("IDS_DLGTIP_PING", &labels.tooltip_ping);
        labels.tooltip_unassigned_savegame_players = resource(
            "IDS_DESC_UNASSOCIATEDSAVEGAMEPLAYE",
            &labels.tooltip_unassigned_savegame_players,
        );
        labels.tooltip_script_players = resource(
            "IDS_DESC_PLAYERSCONTROLLEDBYCOMPUT",
            &labels.tooltip_script_players,
        );
        labels.tooltip_replay_players =
            resource("IDS_MSG_REPLAYPLRS_DESC", &labels.tooltip_replay_players);
        labels.tooltip_team_template =
            resource("IDS_DESC_TEAM", "Team %s").replacen("%s", "{team}", 1);
        labels
    }

    pub(crate) fn classic_lobby_option_labels(&self) -> LobbyOptionLabels {
        let mut labels = LobbyOptionLabels::default();
        let resource = |key, fallback: &str| self.runtime_resource_text(key, fallback);
        labels.control_mode = resource("IDS_TEXT_CONTROLMODE", &labels.control_mode);
        labels.control_mode_tooltip = resource(
            "IDS_DESC_CHANGESTHEWAYCONTROLDATAI",
            &labels.control_mode_tooltip,
        );
        labels.control_mode_central =
            resource("IDS_NET_CTRLMODE_CENTRAL", &labels.control_mode_central);
        labels.control_mode_decentral =
            resource("IDS_NET_CTRLMODE_DECENTRAL", &labels.control_mode_decentral);
        labels.control_mode_none = resource("IDS_NET_CTRLMODE_NONE", &labels.control_mode_none);
        labels.control_rate = resource("IDS_CTL_CONTROLRATE", &labels.control_rate);
        labels.control_rate_tooltip =
            resource("IDS_CTL_CONTROLRATE_DESC", &labels.control_rate_tooltip);
        labels.runtime_join = resource("IDS_NET_RUNTIMEJOIN", &labels.runtime_join);
        labels.runtime_join_tooltip =
            resource("IDS_NET_RUNTIMEJOIN_DESC", &labels.runtime_join_tooltip);
        labels.runtime_join_barred =
            resource("IDS_NET_RUNTIMEJOINBARRED", &labels.runtime_join_barred);
        labels.runtime_join_free = resource("IDS_NET_RUNTIMEJOINFREE", &labels.runtime_join_free);
        labels.team_distribution = resource("IDS_MSG_TEAMDIST", &labels.team_distribution);
        labels.team_distribution_tooltip =
            resource("IDS_MSG_TEAMDIST_DESC", &labels.team_distribution_tooltip);
        labels.team_distribution_free =
            resource("IDS_MSG_TEAMDIST_FREE", &labels.team_distribution_free);
        labels.team_distribution_host =
            resource("IDS_MSG_TEAMDIST_HOST", &labels.team_distribution_host);
        labels.team_distribution_none =
            resource("IDS_MSG_TEAMDIST_NONE", &labels.team_distribution_none);
        labels.team_distribution_random =
            resource("IDS_MSG_TEAMDIST_RND", &labels.team_distribution_random);
        labels.team_distribution_random_invisible = resource(
            "IDS_MSG_TEAMDIST_RNDINV",
            &labels.team_distribution_random_invisible,
        );
        labels.team_colors = resource("IDS_MSG_TEAMCOLORS", &labels.team_colors);
        labels.team_colors_tooltip =
            resource("IDS_MSG_TEAMCOLORS_DESC", &labels.team_colors_tooltip);
        labels.enabled = resource("IDS_MSG_ENABLED", &labels.enabled);
        labels.disabled = resource("IDS_MSG_DISABLED", &labels.disabled);
        labels.random_team_count = resource("IDS_MSG_RANDOMTEAMCOUNT", &labels.random_team_count);
        labels.random_team_count_tooltip = resource(
            "IDS_MSG_RANDOMTEAMCOUNT_DESC",
            &labels.random_team_count_tooltip,
        );
        labels.automatic = resource("IDS_MSG_TEAMCOUNT_AUTO", &labels.automatic);
        labels.automatic_tooltip =
            resource("IDS_MSG_TEAMCOUNT_AUTO_DESC", &labels.automatic_tooltip);
        labels.select_template = resource("IDS_MSG_SELECT", &labels.select_template);
        labels
    }

    fn classic_lobby_option_rows_for(
        &self,
        mode: &NetworkMode,
        control_rate: i32,
        runtime_join_allowed: bool,
        teams: Option<LobbyTeamOptionState>,
    ) -> Vec<LobbyOptionRow> {
        let (role, control_mode) = match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => (
                LobbyRole::Host,
                prepared.host_config().initial_status.control_mode,
            ),
            NetworkMode::Host(_) => (LobbyRole::Host, 0),
            NetworkMode::Client(_) => (
                LobbyRole::Client,
                self.pending_network_join_data
                    .as_ref()
                    .map_or(-1, |join| join.status.control_mode),
            ),
        };
        let labels = self.classic_lobby_option_labels();
        let mut rows = core_lobby_option_rows(
            role,
            &labels,
            control_mode,
            control_rate,
            runtime_join_allowed,
        );
        if let Some(teams) = teams {
            rows.extend(team_lobby_option_rows(role, &labels, teams));
        }
        rows
    }

    fn engine_team_option_state(
        teams: &clonk_engine::InitialNetworkTeamMetadata,
        active_player_count: i32,
    ) -> LobbyTeamOptionState {
        LobbyTeamOptionState {
            active: teams.active,
            auto_generate_teams: teams.auto_generate_teams,
            distribution: teams.team_distribution as i32,
            team_colors: teams.team_colors,
            random_team_count: teams.random_team_count,
            active_player_count,
            team_count: i32::try_from(teams.teams.len()).unwrap_or(i32::MAX),
        }
    }

    /// `C4TeamList` reaches every client verbatim through JoinData, and
    /// `C4GameOptionsList` reads that same live list rather than a host-only
    /// projection (src/C4GameOptions.cpp:203-231; src/C4Teams.cpp:560-590).
    fn joined_team_option_state(
        teams: &clonk_network::JoinTeamListSnapshot,
        active_player_count: i32,
    ) -> LobbyTeamOptionState {
        LobbyTeamOptionState {
            active: teams.active != 0,
            auto_generate_teams: teams.auto_generate_teams != 0,
            distribution: i32::from(teams.team_distribution),
            team_colors: teams.team_colors != 0,
            random_team_count: teams.random_team_count,
            active_player_count,
            team_count: i32::try_from(teams.teams.len()).unwrap_or(i32::MAX),
        }
    }

    fn current_classic_lobby_option_rows(&self) -> Option<Vec<LobbyOptionRow>> {
        let mode = self.network_mode.as_ref()?;
        let runtime_join_allowed = self
            .classic_host_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.runtime_join_allowed);
        let control_rate = self
            .network_control_clock
            .map(NetworkControlClock::control_rate)
            .unwrap_or_else(|| self.engine.control_rate());
        let active_player_count = i32::try_from(
            self.control_player_infos
                .retained_rows_snapshot()
                .1
                .iter()
                .flat_map(|(_, _, players)| players)
                .filter(|player| player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
                .count(),
        )
        .unwrap_or(i32::MAX);
        let teams = match mode {
            NetworkMode::Host(_) => self
                .network_team_assignment
                .as_ref()
                .map(NetworkTeamAssignmentState::teams)
                .map(|teams| Self::engine_team_option_state(teams, active_player_count)),
            NetworkMode::Client(_) => self.pending_network_join_data.as_ref().map(|join| {
                Self::joined_team_option_state(&join.parameters.teams, active_player_count)
            }),
        };
        Some(self.classic_lobby_option_rows_for(mode, control_rate, runtime_join_allowed, teams))
    }

    /// Mirrors `C4GameOptionsList::Activate`/its one-second callback. An
    /// activation calls through even when values compare equal; inactive
    /// sheets retain their last projection and do no periodic work.
    ///
    /// `C4GameLobby::MainDlg` builds one options list per participant
    /// (src/C4GameLobby.cpp:223,247), so the joined adapter's retained
    /// controller is refreshed on exactly the same cadence as the host's.
    pub(crate) fn refresh_classic_lobby_options(&mut self, force: bool) -> bool {
        let host_active = self
            .classic_host_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.controller.active_sheet() == LobbySheet::Options);
        let joined_active = self
            .network_lobby
            .as_ref()
            .is_some_and(|lobby| lobby.active_sheet == LobbySheet::Options);
        if !host_active && !joined_active {
            return false;
        }
        let Some(rows) = self.current_classic_lobby_option_rows() else {
            return false;
        };
        let mut changed = false;
        if host_active {
            if let Some(lobby) = self.classic_host_lobby.as_mut() {
                let host_changed = lobby.controller.option_rows() != rows;
                if force || host_changed {
                    lobby.controller.set_option_rows(rows.clone());
                }
                changed |= host_changed;
            }
        }
        if joined_active {
            if let Some(lobby) = self.network_lobby.as_mut() {
                let joined_changed = lobby.controller.option_rows() != rows;
                if force || joined_changed {
                    lobby.controller.set_option_rows(rows);
                }
                changed |= joined_changed;
            }
        }
        if changed {
            self.close_stale_classic_lobby_team_combo();
        }
        changed
    }

    fn classic_host_lobby_roster_rows(
        &self,
        mode: &NetworkMode,
        local_name: &str,
        nick: &str,
    ) -> (Vec<LobbyRosterRow>, i32) {
        let parameters = match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => prepared
                .host_config()
                .initial_join_snapshot
                .as_ref()
                .map(|snapshot| &snapshot.parameters),
            NetworkMode::Client(_) => self
                .pending_network_join_data
                .as_ref()
                .map(|join| &join.parameters),
            NetworkMode::Host(_) => None,
        };
        let teams = parameters.map(|parameters| &parameters.teams);
        let league_mode = parameters.is_some_and(synchronized_parameters_are_league);
        let default_player_icon = self
            .staged_network_host_scenario
            .as_ref()
            .map(|staged| &staged.default_player_icon);
        let default_crew_icon = self.staged_network_host_scenario.as_ref().map(|staged| {
            clonk_frontend::classic_gui::blacken_transparent_pixels(&staged.default_crew_icon)
        });
        let replay = self
            .staged_network_host_scenario
            .as_ref()
            .and_then(|staged| staged.scenario.lobby_metadata())
            .is_some_and(|metadata| metadata.head().is_replay());
        let (_, retained) = self.control_player_infos.retained_rows_snapshot();
        let retained_players = retained
            .iter()
            .flat_map(|(client_id, _, players)| {
                players.iter().map(move |player| (*client_id, player))
            })
            .collect::<Vec<_>>();
        let is_visible = |player: &clonk_engine::ControlPlayerInfoEntry| {
            player.flags
                & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                    | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                == 0
        };
        let active_players = i32::try_from(
            retained_players
                .iter()
                .filter(|(_, player)| player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
                .count(),
        )
        .unwrap_or(i32::MAX);
        let player_name = |player: &clonk_engine::ControlPlayerInfoEntry| {
            if !player.league_account.is_empty() {
                let account = legacy_presentation_text(player.league_account.as_bytes());
                if player.clan_tag.is_empty() {
                    account
                } else {
                    format!(
                        "<c afafaf>{}</c> {account}",
                        legacy_presentation_text(player.clan_tag.as_bytes())
                    )
                }
            } else if !player.forced_name.is_empty() {
                legacy_presentation_text(player.forced_name.as_bytes())
            } else {
                legacy_presentation_text(player.name.as_bytes())
            }
        };
        let restore_player = |id: i32| {
            parameters.and_then(|parameters| {
                parameters
                    .restore_player_infos
                    .clients
                    .iter()
                    .flat_map(|client| &client.players)
                    .find(|player| player.id == id)
            })
        };
        let lobby_color = |player: &clonk_engine::ControlPlayerInfoEntry| {
            let random_invisible = teams.is_some_and(|teams| {
                teams.team_distribution == 4
                    && teams.team_colors != 0
                    && teams.teams.iter().any(|team| team.id == player.team)
                    && !player.is_joined()
                    && player.savegame_player == 0
            });
            if random_invisible {
                player.original_color
            } else {
                player.color
            }
        };
        let player_row = |client_id: i32,
                          player: &clonk_engine::ControlPlayerInfoEntry,
                          joined_player: Option<&clonk_engine::ControlPlayerInfoEntry>,
                          selectable: bool| {
            let color = lobby_color(player);
            let team = teams.filter(|teams| teams.active != 0).map(|teams| {
                let visible = teams.team_distribution != 4;
                let name = if visible {
                    teams
                        .teams
                        .iter()
                        .find(|team| team.id == player.team)
                        .map(|team| legacy_presentation_text(team.name.as_bytes()))
                        .unwrap_or_default()
                } else {
                    self.runtime_resource_text("IDS_MSG_RNDTEAM", "Random team")
                };
                LobbyTeamValue {
                    id: player.team,
                    name,
                    selectable: selectable
                        && visible
                        && classic_lobby_player_can_choose_team(
                            teams,
                            player,
                            joined_player.is_some(),
                        ),
                }
            });
            let league_score = (player.league_score != 0 || player.league_projected_gain >= 0)
                .then(|| {
                    if player.league_projected_gain >= 0
                        && teams.is_none_or(|teams| teams.team_distribution != 4)
                    {
                        format!(
                            "{} ({:+})",
                            player.league_score, player.league_projected_gain
                        )
                    } else {
                        player.league_score.to_string()
                    }
                });
            let icon = player
                .resource
                .as_ref()
                .filter(|_| player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0)
                .and_then(|resource| self.admission_resources.complete_path(resource.id))
                .map(|path| {
                    if let Some(icon) = load_network_player_big_icon(path) {
                        LobbyRosterIcon::Raster(
                            clonk_frontend::classic_gui::blacken_transparent_pixels(&icon),
                        )
                    } else if let Some(default_player_icon) = default_player_icon {
                        LobbyRosterIcon::Raster(
                            clonk_frontend::game_lobby::compose_classic_lobby_player_fallback_icon(
                                default_player_icon,
                                Color::opaque(
                                    ((color >> 16) & 0xff) as u8,
                                    ((color >> 8) & 0xff) as u8,
                                    (color & 0xff) as u8,
                                ),
                            )
                            .expect("the staged active Player graphic is nonempty"),
                        )
                    } else {
                        LobbyRosterIcon::Standard(7)
                    }
                })
                .unwrap_or_else(|| {
                    LobbyRosterIcon::Standard(if player.is_script_player() { 4 } else { 7 })
                });
            LobbyRosterRow::Player(LobbyPlayerRow {
                id: player.id,
                client_id,
                name: player_name(player),
                color: readable_lobby_rgba(color),
                icon,
                joined_player_overlay: joined_player.and_then(|joined_player| {
                    default_crew_icon.as_ref().map(|crew| {
                        let color = lobby_color(joined_player);
                        LobbyJoinedPlayerOverlay {
                            crew: crew.clone(),
                            color: [
                                ((color >> 16) & 0xff) as u8,
                                ((color >> 8) & 0xff) as u8,
                                (color & 0xff) as u8,
                                255,
                            ],
                        }
                    })
                }),
                team,
                league_score,
                league_rank: (league_mode && player.league_rank_symbol != 0)
                    .then(|| player.league_rank_symbol.clamp(1, 9) as u8),
            })
        };
        let local_client_color = retained_players
            .iter()
            .find(|(client_id, player)| {
                *client_id == 0 && player.player_type == clonk_engine::PLAYER_INFO_TYPE_USER
            })
            .map(|(_, player)| lobby_color(player))
            .unwrap_or(0x00ff_ffff);

        let mut rows = Vec::new();
        if let Some(parameters) = parameters {
            let associated = retained_players
                .iter()
                .map(|(_, player)| player.savegame_player)
                .filter(|id| *id != 0)
                .collect::<HashSet<_>>();
            let has_restore_players = parameters
                .restore_player_infos
                .clients
                .iter()
                .flat_map(|client| &client.players)
                .any(|player| {
                    player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
                        && !player.is_script_player()
                });
            let mut free_restore = parameters
                .restore_player_infos
                .clients
                .iter()
                .flat_map(|client| &client.players)
                .collect::<Vec<_>>();
            free_restore.sort_by_key(|player| player.id);
            free_restore.retain(|player| player.id > 0);
            free_restore.dedup_by_key(|player| player.id);
            free_restore
                .retain(|player| !player.is_script_player() && !associated.contains(&player.id));
            if has_restore_players {
                rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::UnassignedSavegamePlayers,
                    label: self
                        .runtime_resource_text("IDS_MSG_FREESAVEGAMEPLRS", "Player assignment"),
                    icon: LobbyRosterIcon::Standard(12),
                    can_add_player: false,
                }));
                rows.extend(
                    free_restore
                        .into_iter()
                        .map(|player| player_row(-1, player, Some(player), false)),
                );
            }
        }

        if replay {
            rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
                kind: LobbyRosterHeader::ReplayPlayers,
                label: self.runtime_resource_text("IDS_MSG_REPLAYPLRS", "Replay players"),
                icon: LobbyRosterIcon::Standard(21),
                can_add_player: false,
            }));
            // C++ first constructs every visible replay row with client -1.
            // UpdateScriptPlayers then moves each active script row by its
            // semantic ID, retaining that client ownership; removed scripts
            // stay in the replay group.
            let mut replay_players = retained_players
                .iter()
                .filter(|(_, player)| {
                    player.id > 0
                        && player.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE == 0
                        && !(player.is_script_player()
                            && player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
                })
                .copied()
                .collect::<Vec<_>>();
            replay_players.sort_by_key(|(_, player)| player.id);
            rows.extend(replay_players.into_iter().map(|(_, player)| {
                player_row(-1, player, restore_player(player.savegame_player), true)
            }));
        }

        let script_players = retained_players
            .iter()
            .filter(|(_, player)| player.is_script_player() && is_visible(player))
            .copied()
            .collect::<Vec<_>>();
        let active_script_players = retained_players
            .iter()
            .filter(|(_, player)| {
                player.is_script_player()
                    && player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
            })
            .count();
        let max_script_players = teams.map_or(0, |teams| teams.max_script_players);
        if max_script_players != 0 || !script_players.is_empty() {
            rows.push(LobbyRosterRow::Header(LobbyHeaderRow {
                kind: LobbyRosterHeader::ScriptPlayers,
                label: self.runtime_resource_text("IDS_CTL_SCRIPTPLAYERS", "Script players"),
                icon: LobbyRosterIcon::Standard(21),
                can_add_player: max_script_players
                    .saturating_sub(i32::try_from(active_script_players).unwrap_or(i32::MAX))
                    > 0,
            }));
            rows.extend(script_players.into_iter().map(|(client_id, player)| {
                player_row(
                    if replay { -1 } else { client_id },
                    player,
                    restore_player(player.savegame_player),
                    true,
                )
            }));
        }

        rows.push(LobbyRosterRow::Client(LobbyClientRow {
            id: 0,
            name: c4_presentation_text(local_name),
            nick: c4_presentation_text(nick),
            color: readable_lobby_rgba(local_client_color),
            status: LobbyClientStatus::Host,
            local: true,
            connected: false,
            resource_progress: None,
            ping_ms: None,
        }));
        if !replay {
            rows.extend(
                retained_players
                    .into_iter()
                    .filter(|(_, player)| !player.is_script_player() && is_visible(player))
                    .map(|(client_id, player)| {
                        player_row(
                            client_id,
                            player,
                            restore_player(player.savegame_player),
                            true,
                        )
                    }),
            );
        }
        (rows, active_players)
    }

    pub(crate) fn build_classic_host_lobby(
        &self,
        mode: &NetworkMode,
        manager: &NetworkManager,
    ) -> Result<(ClassicHostLobbyState, GameOptionButtons)> {
        let NetworkMode::Host(settings) = mode else {
            return Err(classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "client sessions do not have an app-wired classic lobby".to_string(),
            }));
        };
        if manager.local_client_id() != 0 {
            return Err(classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "classic host lobby requires local client ID zero".to_string(),
            }));
        }
        let staged = self.staged_network_host_scenario.as_ref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "host connection completed without a staged scenario".to_string(),
            })
        })?;
        if settings.player_name != staged.lobby.local_name {
            return Err(classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "connected host identity differs from the pre-bind accepted model"
                    .to_string(),
            }));
        }
        self.assets.game_lobby_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        self.assets.game_option_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let loader = self.loader_screen.as_ref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "staged scenario loader is not installed".to_string(),
            })
        })?;
        let (rows, active_players) =
            self.classic_host_lobby_roster_rows(mode, &staged.lobby.local_name, &staged.lobby.nick);
        let mut controller = ClassicGameLobby::new(
            LobbyRole::Host,
            loader.state().title(),
            active_players,
            staged.lobby.max_players,
            staged.lobby.has_teams,
            self.startup_irc_client_active(),
            self.admission_resources.lobby_ready_available(),
            false,
            staged.lobby.countdown_seconds,
            rows,
        );
        controller.set_labels(self.classic_lobby_labels());
        let preload = LobbyPreloadState::new(
            load_options_program_state(
                self.app_paths.as_ref(),
                Some(&self.startup_tooltip_resources),
            )
            .preloading,
        );
        controller.set_preload_button_state(preload.manual_button_present, preload.eligible);
        let runtime_join_allowed = settings
            .prepared
            .as_ref()
            .is_some_and(|prepared| prepared.admission().runtime_join_allowed());
        let control_rate = initial_network_control_clock(Some(mode))
            .map(NetworkControlClock::control_rate)
            .unwrap_or_else(|| self.engine.control_rate());
        let teams = match mode {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => Some(Self::engine_team_option_state(
                prepared.runtime_team_metadata(),
                active_players,
            )),
            NetworkMode::Host(_) | NetworkMode::Client(_) => None,
        };
        controller.set_option_rows(self.classic_lobby_option_rows_for(
            mode,
            control_rate,
            runtime_join_allowed,
            teams,
        ));
        controller.set_league_mode(initial_network_is_league(Some(mode)));
        let mut values = staged.options.clone();
        values.lobby_is_league = initial_network_is_league(Some(mode));
        values.selector_fair_crew_constraint = FairCrewConstraint::Free;
        values.lobby_fair_crew_forced = staged.lobby.fair_crew_forced;
        values.fair_crew = staged.lobby.fair_crew;
        values.fair_crew_strength = staged.lobby.fair_crew_strength;
        values.countdown = false;
        let mut options = GameOptionButtons::new(GameOptionContext::LobbyHost, values);
        let fonts = self.assets.clonk_fonts.as_deref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
            })
        })?;
        let surface = self.graphics.surface();
        let layout = controller.layout(surface.width() as i32, surface.height() as i32, fonts);
        options.set_bounds(layout.game_option_strip);
        // Prime scroll metrics before the first pointer or wheel event.
        let _ = controller.right_list_layout(&layout, fonts);
        let resource_rows = initial_classic_lobby_resource_rows(
            settings
                .prepared
                .as_ref()
                .and_then(|prepared| prepared.host_config().initial_join_snapshot.as_ref()),
        );
        Ok((
            ClassicHostLobbyState {
                controller,
                preload,
                pointer: None,
                last_roster_click: None,
                chat_history_index: -1,
                runtime_join_allowed,
                resource_rows,
                scenario_description: LobbyScenarioDescriptionState::default(),
            },
            options,
        ))
    }

    pub(crate) fn visible_classic_lobby_controller(&self) -> Option<&ClassicGameLobby> {
        self.classic_host_lobby
            .as_ref()
            .map(|lobby| &lobby.controller)
            .or_else(|| self.network_lobby.as_ref().map(|lobby| &lobby.controller))
    }

    pub(crate) fn visible_classic_lobby_controller_mut(&mut self) -> Option<&mut ClassicGameLobby> {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            Some(&mut lobby.controller)
        } else {
            self.network_lobby
                .as_mut()
                .map(|lobby| &mut lobby.controller)
        }
    }

    fn visible_classic_lobby_player_context_target(&self, player_id: i32) -> Option<(i32, bool)> {
        self.visible_classic_lobby_controller()
            .and_then(|controller| {
                let mut free_savegame_group = false;
                controller.rows().iter().find_map(|row| match row {
                    LobbyRosterRow::Header(header) => {
                        free_savegame_group =
                            matches!(header.kind, LobbyRosterHeader::UnassignedSavegamePlayers);
                        None
                    }
                    LobbyRosterRow::Client(_) => {
                        free_savegame_group = false;
                        None
                    }
                    LobbyRosterRow::Player(player) if player.id == player_id => {
                        Some((player.client_id, free_savegame_group))
                    }
                    LobbyRosterRow::Player(_) => None,
                })
            })
    }

    pub(crate) fn classic_lobby_restore_player(
        &self,
        player_id: i32,
    ) -> Option<&clonk_engine::ControlPlayerInfoEntry> {
        fn find(
            snapshot: &clonk_network::PlayerInfoListSnapshot,
            player_id: i32,
        ) -> Option<&clonk_engine::ControlPlayerInfoEntry> {
            snapshot
                .clients
                .iter()
                .flat_map(|client| &client.players)
                .find(|player| player.id == player_id)
        }
        self.pending_network_join_data
            .as_ref()
            .and_then(|join| find(&join.parameters.restore_player_infos, player_id))
            .or_else(|| {
                self.host_join_snapshot
                    .as_ref()
                    .and_then(|snapshot| find(&snapshot.parameters.restore_player_infos, player_id))
            })
            .or_else(|| {
                self.network_mode.as_ref().and_then(|mode| match mode {
                    NetworkMode::Host(HostSettings {
                        prepared: Some(prepared),
                        ..
                    }) => prepared
                        .host_config()
                        .initial_join_snapshot
                        .as_ref()
                        .and_then(|snapshot| {
                            find(&snapshot.parameters.restore_player_infos, player_id)
                        }),
                    NetworkMode::Host(_) | NetworkMode::Client(_) => None,
                })
            })
    }

    fn visible_classic_lobby_team_metadata(
        &self,
    ) -> Option<clonk_engine::InitialNetworkTeamMetadata> {
        self.network_team_assignment
            .as_ref()
            .map(|assignment| assignment.teams().clone())
            .or_else(|| {
                self.pending_network_join_data.as_ref().and_then(|join| {
                    initial_team_metadata_from_join_snapshot(&join.parameters.teams)
                })
            })
    }

    pub(crate) fn set_context_menu_lobby_team_player(&mut self, player_id: Option<i32>) {
        self.context_menu_lobby_team_player = player_id;
        if let Some(controller) = self.visible_classic_lobby_controller_mut() {
            controller.set_open_team_combo_player(player_id);
        }
    }

    pub(crate) fn set_context_menu_lobby_option(&mut self, option: Option<LobbyOptionKind>) {
        self.context_menu_lobby_option = option;
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_open_option_combo(option);
        }
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.set_open_option(option);
        }
    }

    pub(crate) fn close_stale_classic_lobby_team_combo(&mut self) {
        let roster_active = self
            .visible_classic_lobby_controller()
            .is_some_and(|controller| controller.active_sheet().is_roster());
        let stale_team_combo = self
            .context_menu_lobby_team_player
            .is_some_and(|player_id| {
                !roster_active
                    || self
                        .visible_classic_lobby_controller()
                        .and_then(ClassicGameLobby::open_team_combo_player)
                        != Some(player_id)
            });
        let stale_kick = self
            .context_menu_lobby_kick_client
            .is_some_and(|client_id| {
                !roster_active
                    || !self.control_clients.contains(client_id)
                    || self.visible_lobby_client_is_local(client_id).is_none()
            });
        let stale_player = self.context_menu_lobby_player.is_some_and(
            |(client_id, player_id, opened_as_free_savegame)| {
                !roster_active
                    || !self
                        .visible_classic_lobby_player_context_target(player_id)
                        .is_some_and(|(visible_client_id, free_savegame_player)| {
                            visible_client_id == client_id
                                && free_savegame_player == opened_as_free_savegame
                        })
            },
        );
        let stale_option = self.context_menu_lobby_option.is_some_and(|option| {
            let lobby_owns = self.classic_host_lobby.as_ref().is_some_and(|lobby| {
                lobby.controller.active_sheet() == LobbySheet::Options
                    && lobby.controller.open_option_combo() == Some(option)
            });
            let runtime_owns = self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.open_option() == Some(option));
            !lobby_owns && !runtime_owns
        });
        if stale_team_combo || stale_kick || stale_player || stale_option {
            // ComboBox::SetReadOnly aborts its menu without a DoorClose sound.
            self.close_context_menu_silently();
        }
    }

    pub(crate) fn refresh_classic_lobby_client_telemetry(&mut self) -> bool {
        if self.classic_host_lobby.is_none() && self.network_lobby.is_none() {
            return false;
        }
        let Some(network) = self.network.as_ref() else {
            return false;
        };
        let local_client_id = network.local_client_id();
        let mut client_ids = self
            .classic_host_lobby
            .as_ref()
            .into_iter()
            .flat_map(|lobby| lobby.controller.rows())
            .chain(
                self.network_lobby
                    .as_ref()
                    .into_iter()
                    .flat_map(|lobby| lobby.roster_rows.iter()),
            )
            .filter_map(|row| match row {
                LobbyRosterRow::Client(client) => ClientId::try_from(client.id).ok(),
                _ => None,
            })
            .chain(
                self.network_lobby
                    .as_ref()
                    .into_iter()
                    .flat_map(|lobby| lobby.participants.keys().copied()),
            )
            .filter(|client_id| *client_id != local_client_id)
            .collect::<Vec<_>>();
        client_ids.sort_unstable();
        client_ids.dedup();
        if client_ids.is_empty() {
            return false;
        }
        let telemetry = match network.lobby_client_telemetry(client_ids) {
            Ok(telemetry) => telemetry,
            Err(error) => {
                tracing::debug!(%error, "classic lobby client telemetry is not available");
                return false;
            }
        };

        let mut changed = false;
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            let mut rows = lobby.controller.rows().to_vec();
            if apply_classic_lobby_client_telemetry(&mut rows, local_client_id, &telemetry) {
                lobby.controller.set_rows(rows);
                changed = true;
            }
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            changed |= lobby.set_client_telemetry(telemetry);
        }
        changed
    }

    pub(crate) fn sync_classic_lobby_roster(&mut self) {
        if self.classic_host_lobby.is_none() && self.network_lobby.is_none() {
            return;
        }
        self.submit_restart_restore_team_updates_for_new_roster_items();
        let active_sheet = self
            .classic_host_lobby
            .as_ref()
            .map(|lobby| lobby.controller.active_sheet())
            .or_else(|| {
                self.network_lobby
                    .as_ref()
                    .map(|lobby| lobby.controller.active_sheet())
            })
            .unwrap_or(LobbySheet::Players);
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        // The generic projection owns live client ordering, status, and team
        // authorization. Overlay the C4PlayerInfoListBox presentation built
        // from the same retained PlayerInfo state so a post-admission sync
        // does not discard BigIcon/fallback graphics, savegame crew overlays,
        // league metadata, localized headers, or unassigned restore rows.
        let (rich_rows, rich_active_players) = self
            .network_mode
            .as_ref()
            .map(|mode| self.classic_host_lobby_roster_rows(mode, "", ""))
            .unwrap_or_default();
        let has_rich_projection = !rich_rows.is_empty();
        let rich_players = rich_rows
            .iter()
            .filter_map(|row| match row {
                LobbyRosterRow::Player(player) => {
                    Some(((player.client_id, player.id), player.clone()))
                }
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        let rich_script_header = rich_rows.iter().find_map(|row| match row {
            LobbyRosterRow::Header(header)
                if matches!(header.kind, LobbyRosterHeader::ScriptPlayers) =>
            {
                Some(header.clone())
            }
            _ => None,
        });
        let mut rich_replay_rows = Vec::new();
        let mut collecting_replay_rows = false;
        for row in &rich_rows {
            match row {
                LobbyRosterRow::Header(header)
                    if matches!(header.kind, LobbyRosterHeader::ReplayPlayers) =>
                {
                    collecting_replay_rows = true;
                    rich_replay_rows.push(row.clone());
                }
                LobbyRosterRow::Client(_) if collecting_replay_rows => break,
                _ if collecting_replay_rows => rich_replay_rows.push(row.clone()),
                _ => {}
            }
        }
        let mut rich_restore_rows = Vec::new();
        let mut collecting_restore_rows = false;
        for row in &rich_rows {
            match row {
                LobbyRosterRow::Header(header)
                    if matches!(header.kind, LobbyRosterHeader::UnassignedSavegamePlayers) =>
                {
                    collecting_restore_rows = true;
                    rich_restore_rows.push(row.clone());
                }
                LobbyRosterRow::Player(player)
                    if collecting_restore_rows && player.client_id == -1 =>
                {
                    rich_restore_rows.push(row.clone());
                }
                _ if collecting_restore_rows => break,
                _ => {}
            }
        }
        let joined_teams = self
            .pending_network_join_data
            .as_ref()
            .and_then(|join| initial_team_metadata_from_join_snapshot(&join.parameters.teams));
        let teams = self
            .network_team_assignment
            .as_ref()
            .map(NetworkTeamAssignmentState::teams)
            .or(joined_teams.as_ref());
        let has_teams = teams.is_some_and(|teams| teams.active);
        let (mut rows, generic_active_players) = classic_lobby_roster_projection(
            &self.control_clients,
            &self.control_player_infos,
            teams,
            local_client_id,
            active_sheet,
        );
        if active_sheet == LobbySheet::Teams {
            let random_team = self.runtime_resource_text("IDS_MSG_RNDTEAM", "Random team");
            for row in &mut rows {
                if let LobbyRosterRow::Header(LobbyHeaderRow {
                    kind: LobbyRosterHeader::RandomTeam,
                    label,
                    ..
                }) = row
                {
                    label.clone_from(&random_team);
                }
            }
        }
        if !rich_replay_rows.is_empty() && active_sheet == LobbySheet::Players {
            let live_player_teams = rows
                .iter()
                .filter_map(|row| match row {
                    LobbyRosterRow::Player(player) => Some((player.id, player.team.clone())),
                    _ => None,
                })
                .collect::<BTreeMap<_, _>>();
            for row in &mut rich_replay_rows {
                let LobbyRosterRow::Player(player) = row else {
                    continue;
                };
                if let Some(team) = live_player_teams.get(&player.id) {
                    player.team.clone_from(team);
                }
            }
            rows.retain(|row| matches!(row, LobbyRosterRow::Client(_)));
            rich_replay_rows.extend(rows);
            rows = rich_replay_rows;
        } else if !rich_replay_rows.is_empty()
            && active_sheet == LobbySheet::Teams
            && teams.is_some_and(|teams| {
                matches!(
                    teams.team_distribution,
                    clonk_engine::InitialNetworkTeamDistribution::RandomInvisible
                )
            })
        {
            rows.clear();
        }
        let active_players = if has_rich_projection {
            rich_active_players
        } else {
            generic_active_players
        };
        for row in &mut rows {
            match row {
                LobbyRosterRow::Player(player) => {
                    let Some(mut rich) = rich_players.get(&(player.client_id, player.id)).cloned()
                    else {
                        continue;
                    };
                    rich.team = player.team.clone();
                    *player = rich;
                }
                LobbyRosterRow::Header(header)
                    if matches!(header.kind, LobbyRosterHeader::ScriptPlayers) =>
                {
                    if let Some(rich) = rich_script_header.as_ref() {
                        let can_add_player = header.can_add_player;
                        *header = rich.clone();
                        header.can_add_player = can_add_player;
                    }
                }
                _ => {}
            }
        }
        if !rich_restore_rows.is_empty() {
            rich_restore_rows.extend(rows);
            rows = rich_restore_rows;
        }
        let previous_clients = self
            .classic_host_lobby
            .as_ref()
            .map(|lobby| lobby.controller.rows())
            .or_else(|| {
                self.network_lobby
                    .as_ref()
                    .map(|lobby| lobby.controller.rows())
            })
            .into_iter()
            .flatten()
            .filter_map(|row| match row {
                LobbyRosterRow::Client(client) => Some((client.id, client.clone())),
                _ => None,
            })
            .collect::<BTreeMap<_, _>>();
        for row in &mut rows {
            let LobbyRosterRow::Client(client) = row else {
                continue;
            };
            let Some(previous) = previous_clients.get(&client.id) else {
                continue;
            };
            if client.name.is_empty() {
                client.name.clone_from(&previous.name);
            }
            if client.nick.is_empty() {
                client.nick.clone_from(&previous.nick);
            }
            if client.id == 0
                && client.status == LobbyClientStatus::Unknown
                && previous.status == LobbyClientStatus::Host
            {
                // The staged controller is constructed from the prepared
                // host core before the empty initial PlayerInfo packet is
                // projected. Preserve that known host identity across this
                // transitional synchronization pass.
                client.status = LobbyClientStatus::Host;
            }
        }
        let maximum = i32::try_from(self.network_max_players).unwrap_or(i32::MAX);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_rows(rows.clone());
            lobby.controller.set_player_count(active_players, maximum);
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.roster_rows = rows.clone();
            lobby.roster_rows_authoritative = true;
            lobby.active_players = active_players;
            lobby.max_players = maximum;
            lobby.has_teams = has_teams;
            lobby.league_mode = self.network_is_league;
            lobby.controller.set_has_teams(has_teams);
            lobby.controller.set_league_mode(self.network_is_league);
            lobby.controller.set_rows(rows);
            lobby.controller.set_player_count(active_players, maximum);
        }
        self.close_stale_classic_lobby_team_combo();
        self.refresh_classic_lobby_client_telemetry();
    }

    fn sync_visible_classic_lobby_resources(&mut self) {
        let Some(lobby) = self.classic_host_lobby.as_mut() else {
            return;
        };
        if lobby.controller.resource_sheet_active() {
            lobby
                .controller
                .set_resource_rows(lobby.resource_rows.values().cloned().collect());
        }
    }

    fn lobby_resource_work_directory(&self) -> PathBuf {
        match self.network_mode.as_ref() {
            Some(NetworkMode::Client(settings)) => settings.resource_directory.clone(),
            Some(NetworkMode::Host(settings)) => settings
                .prepared
                .as_ref()
                .and_then(|prepared| prepared.host_config().resource_directory.clone())
                .or_else(|| network_work_directory(self.app_paths.as_ref()))
                .unwrap_or_else(|| PathBuf::from("Network")),
            None => network_work_directory(self.app_paths.as_ref())
                .unwrap_or_else(|| PathBuf::from("Network")),
        }
    }

    fn lobby_resource_exe_path(&self, work_directory: &Path) -> PathBuf {
        self.app_paths
            .as_ref()
            .map(|paths| paths.install_root().to_path_buf())
            .or_else(|| work_directory.parent().map(Path::to_path_buf))
            .unwrap_or_default()
    }

    fn lobby_resource_player_path(&self) -> PathBuf {
        let config = load_native_config_bytes(self.app_paths.as_ref());
        native_config_text(&config, "General", "PlayerPath")
            .map(|path| PathBuf::from(path.trim()))
            .unwrap_or_default()
    }

    fn lobby_allow_player_save(&self) -> bool {
        let config = load_native_config_bytes(self.app_paths.as_ref());
        native_config_text(&config, "Lobby", "AllowPlayerSave")
            .is_some_and(|value| parse_config_bool(value.trim()))
    }

    fn lobby_resource_save_possible(&self, resource_id: i32) -> bool {
        let Some(core) = self.admission_resources.resource_cores.get(&resource_id) else {
            return false;
        };
        let Some(AdmissionResourceState::Complete {
            path,
            removed: false,
            local,
        }) = self.admission_resources.status(resource_id)
        else {
            return false;
        };
        lobby_resource_save_possible(
            *local,
            true,
            core.resource_type,
            self.lobby_allow_player_save(),
            path,
            &self.lobby_resource_work_directory(),
        )
    }

    pub(crate) fn request_lobby_resource_save(
        &mut self,
        resource_id: i32,
        overwrite: bool,
    ) -> Result<(), EngineError> {
        let Some(core) = self
            .admission_resources
            .resource_cores
            .get(&resource_id)
            .cloned()
        else {
            return Ok(());
        };
        let Some(source) = self
            .admission_resources
            .complete_path(resource_id)
            .map(Path::to_path_buf)
        else {
            return Ok(());
        };
        let work_directory = self.lobby_resource_work_directory();
        let error_caption =
            self.runtime_resource_text("IDS_NET_ERR_COPYFILE", "Error copying file");
        if !path_has_raw_directory_prefix(&source, &work_directory) {
            let message = self
                .runtime_resource_text("IDS_NET_ERR_COPYFILE_LOCAL", "The file is local already");
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    message,
                    error_caption,
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        }
        let exe_path = self.lobby_resource_exe_path(&work_directory);
        let player_path = self.lobby_resource_player_path();
        let Some((target, basename)) = lobby_resource_save_target(&exe_path, &player_path, &core)
        else {
            return Ok(());
        };
        if !overwrite && fs::symlink_metadata(&target).is_ok() {
            let template = self
                .runtime_resource_text("IDS_NET_RES_SAVE_OVERWRITE", "File %s exists. Overwrite?");
            let caption = self.runtime_resource_text("IDS_NET_RES_SAVE", "Save resource");
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::new(
                    format_resource_string(template, &[&basename]),
                    caption,
                    clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                    clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
                    clonk_frontend::message_dialog::MessageDialogSize::Regular,
                    false,
                ),
                MessageDialogContinuation::LobbyResourceOverwrite { resource_id },
            )?;
            return Ok(());
        }
        if let Err(error) = copy_lobby_resource_item(&source, &target) {
            tracing::warn!(
                resource_id,
                source = %source.display(),
                target = %target.display(),
                %error,
                "failed to save lobby resource"
            );
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    error_caption.clone(),
                    error_caption,
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        }
        let template = self.runtime_resource_text(
            "IDS_NET_RES_SAVED_DESC",
            "Resource successfully saved to %s",
        );
        let caption = self.runtime_resource_text("IDS_NET_RES_SAVED", "Resource saved");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                format_resource_string(template, &[&basename]),
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(13),
            ),
            MessageDialogContinuation::None,
        )?;
        Ok(())
    }

    pub(crate) fn register_classic_lobby_resource(
        &mut self,
        core: &clonk_engine::NetworkResourceCore,
        present_percent: u8,
    ) {
        if core.id < 0 {
            return;
        }
        let save_possible = self.lobby_resource_save_possible(core.id);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.resource_rows.insert(
                core.id,
                LobbyResourceRow {
                    id: core.id,
                    filename: legacy_presentation_text(core.filename.as_bytes()),
                    present_percent: present_percent.min(100),
                    save_possible,
                },
            );
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.resource_rows.insert(
                core.id,
                LobbyResourceRow {
                    id: core.id,
                    filename: legacy_presentation_text(core.filename.as_bytes()),
                    present_percent: present_percent.min(100),
                    save_possible,
                },
            );
        }
        self.sync_visible_classic_lobby_resources();
    }

    pub(crate) fn register_classic_lobby_player_resources(
        &mut self,
        players: &[clonk_engine::ControlPlayerInfoEntry],
    ) {
        let cores = players
            .iter()
            .filter(|player| {
                player.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0
                    && player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0
                    && player.flags & clonk_engine::PLAYER_INFO_FLAG_IN_SCENARIO_FILE == 0
            })
            .filter_map(|player| player.resource.as_ref())
            .filter_map(|core| {
                let available = matches!(
                    self.admission_resources.status(core.id),
                    Some(AdmissionResourceState::Loading { removed: false })
                        | Some(AdmissionResourceState::Complete { removed: false, .. })
                );
                available.then(|| {
                    let percent = self
                        .admission_resources
                        .present_percent
                        .get(&core.id)
                        .copied()
                        .unwrap_or(0);
                    (core.clone(), percent)
                })
            })
            .collect::<Vec<_>>();
        for (core, percent) in cores {
            self.register_classic_lobby_resource(&core, percent);
        }
    }

    pub(crate) fn update_classic_lobby_resource_progress(&mut self, resource_id: i32, percent: u8) {
        if let Some(row) = self
            .classic_host_lobby
            .as_mut()
            .and_then(|lobby| lobby.resource_rows.get_mut(&resource_id))
        {
            row.present_percent = percent.min(100);
        }
        if let Some(row) = self
            .network_lobby
            .as_mut()
            .and_then(|lobby| lobby.resource_rows.get_mut(&resource_id))
        {
            row.present_percent = percent.min(100);
        }
        self.sync_visible_classic_lobby_resources();
    }

    pub(crate) fn remove_classic_lobby_resource(&mut self, resource_id: i32) {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.resource_rows.remove(&resource_id);
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.resource_rows.remove(&resource_id);
        }
        self.sync_visible_classic_lobby_resources();
    }

    pub(crate) fn remove_classic_lobby_resources_at_client(&mut self, client_id: i32) {
        let owned = |resource_id: &i32| *resource_id >= 0 && (*resource_id >> 16) == client_id;
        self.admission_resources
            .resources
            .retain(|resource_id, _| !owned(resource_id));
        self.admission_resources
            .resource_cores
            .retain(|resource_id, _| !owned(resource_id));
        self.admission_resources
            .present_percent
            .retain(|resource_id, _| !owned(resource_id));
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby
                .resource_rows
                .retain(|resource_id, _| !owned(resource_id));
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby
                .resource_rows
                .retain(|resource_id, _| !owned(resource_id));
        }
        self.sync_visible_classic_lobby_resources();
        self.sync_classic_lobby_resource_ready();
    }

    fn lobby_scenario_loading_text(&self, present_percent: u8) -> String {
        let present_percent = present_percent.to_string();
        let template =
            self.runtime_resource_text("IDS_MSG_SCENARIODESC_LOADING", "Loading... (%d%%)");
        format_resource_string(template, &[&present_percent]).replace("%%", "%")
    }

    pub(crate) fn completed_lobby_scenario_description(
        &self,
        path: &Path,
        title: String,
    ) -> LobbyScenarioText {
        let languages = startup_language_sequence(self.app_paths.as_ref());
        let language_packs = self
            .app_paths
            .as_ref()
            .map(classic_language_packs)
            .unwrap_or_default();
        match load_lobby_scenario_description(path, &languages, &language_packs) {
            Ok(Some(description)) => LobbyScenarioText::Description(description),
            Ok(None) => LobbyScenarioText::Title(title),
            Err(_) => LobbyScenarioText::Message("scenario file load error".to_string()),
        }
    }

    fn current_lobby_scenario_description_update(&self) -> Option<LobbyScenarioDescriptionUpdate> {
        match self.network_mode.as_ref()? {
            NetworkMode::Host(HostSettings {
                prepared: Some(prepared),
                ..
            }) => {
                let config = prepared.host_config();
                let snapshot = config.initial_join_snapshot.as_ref()?;
                let scenario = &snapshot.parameters.scenario;
                let title = legacy_presentation_text(snapshot.parameters.title.as_bytes());
                let path = config
                    .resource_files
                    .iter()
                    .find(|resource| resource.core.id == scenario.id)
                    .map(|resource| resource.path.clone())?;
                Some(LobbyScenarioDescriptionUpdate::Complete(
                    self.completed_lobby_scenario_description(&path, title),
                ))
            }
            NetworkMode::Client(_) => {
                let join_data = self.pending_network_join_data.as_ref()?;
                let scenario = &join_data.parameters.scenario;
                let resource_id = scenario.id;
                let title = legacy_presentation_text(join_data.parameters.title.as_bytes());
                match self.admission_resources.status(resource_id).cloned()? {
                    AdmissionResourceState::Loading { removed: false } => {
                        let percent = self
                            .admission_resources
                            .present_percent
                            .get(&resource_id)
                            .copied()
                            .unwrap_or(0);
                        Some(LobbyScenarioDescriptionUpdate::Loading(
                            self.lobby_scenario_loading_text(percent),
                        ))
                    }
                    AdmissionResourceState::Complete { path, .. } => {
                        Some(LobbyScenarioDescriptionUpdate::Complete(
                            self.completed_lobby_scenario_description(&path, title),
                        ))
                    }
                    AdmissionResourceState::Loading { removed: true }
                    | AdmissionResourceState::Unavailable(_) => None,
                }
            }
            NetworkMode::Host(_) => None,
        }
    }

    pub(crate) fn refresh_lobby_scenario_description(&mut self) -> bool {
        let host_active = self.classic_host_lobby.as_ref().is_some_and(|lobby| {
            lobby.controller.active_sheet() == LobbySheet::Scenario
                && !lobby.scenario_description.finished
        });
        let client_active = self.network_lobby.as_ref().is_some_and(|lobby| {
            lobby.active_sheet == LobbySheet::Scenario && !lobby.scenario_description.finished
        });
        if !host_active && !client_active {
            return false;
        }

        let update = self.current_lobby_scenario_description_update();
        let mut changed = false;
        if host_active {
            if let Some(lobby) = self.classic_host_lobby.as_mut() {
                let host_changed = lobby.scenario_description.apply(update.clone());
                if host_changed {
                    lobby
                        .controller
                        .set_scenario_text(lobby.scenario_description.text.clone());
                }
                changed |= host_changed;
            }
        }
        if client_active {
            if let Some(lobby) = self.network_lobby.as_mut() {
                changed |= lobby.scenario_description.apply(update);
            }
        }
        changed
    }

    fn exit_startup_lobby_to_main(&mut self) {
        self.restart_restore_roster_items.clear();
        self.show_main_menu();
        self.resume_startup_music_after_failed_open_game();
    }

    pub(crate) fn select_classic_lobby_sheet(&mut self, sheet: LobbySheet) -> bool {
        let has_teams = self
            .network_team_assignment
            .as_ref()
            .is_some_and(|assignment| assignment.teams().active);
        if sheet == LobbySheet::Teams && !has_teams {
            return false;
        }
        if (!sheet.is_roster()
            && (self.context_menu_lobby_team_player.is_some()
                || self.context_menu_lobby_kick_client.is_some()
                || self.context_menu_lobby_player.is_some()))
            || (sheet != LobbySheet::Options && self.context_menu_lobby_option.is_some())
        {
            self.close_context_menu_silently();
        }
        {
            let Some(lobby) = self.classic_host_lobby.as_mut() else {
                return false;
            };
            lobby.last_roster_click = None;
            lobby.controller.set_active_sheet(sheet);
            if sheet == LobbySheet::Resources {
                lobby
                    .controller
                    .set_resource_rows(lobby.resource_rows.values().cloned().collect());
            }
        }
        if sheet == LobbySheet::Options {
            let _ = self.refresh_classic_lobby_options(true);
        }
        if sheet.is_roster() {
            self.sync_classic_lobby_roster();
        }
        if sheet == LobbySheet::Scenario {
            let _ = self.refresh_lobby_scenario_description();
        }
        true
    }

    pub(crate) fn lobby_tab_context_entries(
        has_teams: bool,
        options_available: bool,
    ) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
        let mut entries = vec![ContextMenuEntry::new("Players")
            .with_icon(ContextMenuIcon::Phase(9))
            .with_action(AppContextMenuCommand::LobbySheet(LobbySheet::Players))];
        if has_teams {
            entries.push(
                ContextMenuEntry::new("Teams")
                    .with_icon(ContextMenuIcon::Phase(19))
                    .with_action(AppContextMenuCommand::LobbySheet(LobbySheet::Teams)),
            );
        }
        entries.push(
            ContextMenuEntry::new("Resources")
                .with_icon(ContextMenuIcon::Phase(10))
                .with_action(AppContextMenuCommand::LobbySheet(LobbySheet::Resources)),
        );
        if options_available {
            entries.push(
                ContextMenuEntry::new("Options")
                    .with_icon(ContextMenuIcon::Phase(14))
                    .with_action(AppContextMenuCommand::LobbySheet(LobbySheet::Options)),
            );
        }
        entries
    }

    fn open_lobby_tab_context(&mut self, position: GuiPoint) -> Result<bool, EngineError> {
        let (has_teams, options_available) = if self.classic_host_lobby_active() {
            (
                self.network_team_assignment
                    .as_ref()
                    .is_some_and(|assignment| assignment.teams().active),
                true,
            )
        } else if let Some(lobby) = self.network_lobby.as_ref() {
            // C++ offers Options to every participant (src/C4GameLobby.cpp:223).
            (lobby.has_teams, true)
        } else {
            return Ok(false);
        };
        self.open_context_menu_at(
            Self::lobby_tab_context_entries(has_teams, options_available),
            position,
        )
    }

    fn classic_lobby_player_is_owned(&self, client_id: i32) -> bool {
        matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self.control_clients.contains(client_id)
            || self
                .network
                .as_ref()
                .and_then(|network| i32::try_from(network.local_client_id()).ok())
                == Some(client_id)
    }

    pub(crate) fn classic_lobby_takeover_entries(
        &self,
        savegame_player_id: i32,
    ) -> Vec<ContextMenuEntry<AppContextMenuCommand>> {
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return Vec::new();
        };
        let Some(request) = self
            .control_player_infos
            .client_update_request(local_client_id)
        else {
            return Vec::new();
        };
        let using_player = self.runtime_resource_text("IDS_MSG_USINGPLR", "Using %s");
        let tooltip = self.runtime_resource_text(
            "IDS_MSG_USINGPLR_DESC",
            "Use this player to continue the savegame",
        );
        request
            .players
            .into_iter()
            .filter(|player| {
                player.flags
                    & (clonk_engine::PLAYER_INFO_FLAG_JOINED
                        | clonk_engine::PLAYER_INFO_FLAG_JOIN_ISSUED)
                    == 0
                    && player.savegame_player == 0
            })
            .map(|player| {
                let name = legacy_presentation_text(control_player_effective_name(&player));
                ContextMenuEntry::new(format_resource_string(using_player.clone(), &[&name]))
                    .with_tooltip(tooltip.clone())
                    .with_icon(ContextMenuIcon::Phase(9))
                    .with_action(AppContextMenuCommand::LobbyPlayerTakeOver {
                        savegame_player_id,
                        player_id: player.id,
                    })
            })
            .collect()
    }

    pub(crate) fn classic_lobby_player_context_entries(
        &self,
        player_id: i32,
    ) -> Option<(i32, Vec<ContextMenuEntry<AppContextMenuCommand>>)> {
        let (client_id, free_savegame_player) =
            self.visible_classic_lobby_player_context_target(player_id)?;

        if free_savegame_player {
            if self
                .classic_lobby_restore_player(player_id)
                .is_some_and(clonk_engine::ControlPlayerInfoEntry::is_script_player)
            {
                return Some((client_id, Vec::new()));
            }
            return Some((
                client_id,
                // C++ attaches a CBContextHandler here and fills the children
                // in OnContextTakeOver only when the submenu opens
                // (src/C4PlayerInfoListBox.cpp:503-505,535-556), so the
                // candidate set reflects PlayerInfo updates that arrive while
                // the root menu is open.
                vec![ContextMenuEntry::new(
                    self.runtime_resource_text("IDS_MSG_TAKEOVERPLR", "&Take over"),
                )
                .with_tooltip(self.runtime_resource_text(
                    "IDS_MSG_TAKEOVERPLR_DESC",
                    "Control the player in the game",
                ))
                .with_icon(ContextMenuIcon::Phase(9))
                .with_deferred_submenu(
                    AppContextMenuCommand::LobbyPlayerTakeOverSubmenu {
                        savegame_player_id: player_id,
                    },
                )],
            ));
        }

        let player = if client_id == -1 {
            // Replay rows deliberately use the synthetic client -1 while
            // looking up presentation data in the global PlayerInfos list.
            self.control_player_infos.get(player_id).cloned()
        } else {
            self.control_player_infos
                .client_update_request(client_id)
                .and_then(|request| {
                    request
                        .players
                        .into_iter()
                        .find(|player| player.id == player_id)
                })
        };
        let Some(player) = player else {
            return Some((client_id, Vec::new()));
        };
        if !self.classic_lobby_player_is_owned(client_id) {
            return Some((client_id, Vec::new()));
        }

        let mut entries = Vec::new();
        if !player.is_script_player() || player.savegame_player == 0 {
            entries.push(
                ContextMenuEntry::new(self.runtime_resource_text("IDS_MSG_REMOVEPLR", "&Remove"))
                    .with_tooltip(self.runtime_resource_text(
                        "IDS_MSG_REMOVEPLR_DESC",
                        "Do not join with this player",
                    ))
                    .with_icon(ContextMenuIcon::Phase(34))
                    .with_action(AppContextMenuCommand::LobbyPlayerRemove {
                        client_id,
                        player_id,
                    }),
            );
        }
        let team_colors = self
            .visible_classic_lobby_team_metadata()
            .is_some_and(|teams| teams.team_colors);
        if player.color != player.original_color && (!team_colors || player.team == 0) {
            entries.push(
                ContextMenuEntry::new(
                    self.runtime_resource_text("IDS_MSG_NEWPLRCOLOR", "New &color"),
                )
                .with_tooltip(self.runtime_resource_text(
                    "IDS_MSG_NEWPLRCOLOR_DESC",
                    "Generate a new random player color",
                ))
                .with_icon(ContextMenuIcon::Phase(9))
                .with_action(AppContextMenuCommand::LobbyPlayerNewColor {
                    client_id,
                    player_id,
                }),
            );
        }
        Some((client_id, entries))
    }

    fn visible_lobby_client_is_local(&self, client_id: i32) -> Option<bool> {
        self.classic_host_lobby
            .as_ref()
            .and_then(|lobby| {
                lobby.controller.rows().iter().find_map(|row| match row {
                    LobbyRosterRow::Client(client) if client.id == client_id => Some(client.local),
                    _ => None,
                })
            })
            .or_else(|| {
                self.network_lobby
                    .as_ref()
                    .and_then(|lobby| lobby.visible_client_is_local(client_id))
            })
    }

    pub(crate) fn classic_lobby_client_context_entries(
        &self,
        client_id: i32,
    ) -> Option<Vec<ContextMenuEntry<AppContextMenuCommand>>> {
        if self.network.is_none() || !self.control_clients.contains(client_id) {
            return None;
        }
        let local = self.visible_lobby_client_is_local(client_id)?;
        let mut entries = Vec::new();
        if !local {
            let muted = self.control_messages.is_muted(client_id);
            entries.push(
                ContextMenuEntry::new(if muted {
                    self.runtime_resource_text("IDS_NET_UNMUTE", "&Unmute")
                } else {
                    self.runtime_resource_text("IDS_NET_MUTE", "&Mute")
                })
                .with_tooltip(if muted {
                    self.runtime_resource_text(
                        "IDS_NET_UNMUTE_DESC",
                        "Unmute /sound-commands by this client",
                    )
                } else {
                    self.runtime_resource_text(
                        "IDS_NET_MUTE_DESC",
                        "Mute /sound commands of by this client",
                    )
                })
                .with_action(AppContextMenuCommand::LobbyClientToggleMute(client_id)),
            );
        }
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            entries.push(
                ContextMenuEntry::new(self.runtime_resource_text("IDS_NET_KICKCLIENT", "&Kick"))
                    .with_tooltip(
                        self.runtime_resource_text(
                            "IDS_NET_KICKCLIENT_DESC",
                            "Disconnect this client",
                        ),
                    )
                    .with_action(AppContextMenuCommand::LobbyKick(client_id)),
            );
            let activated = self.control_clients.is_activated(client_id);
            entries.push(
                ContextMenuEntry::new(if activated {
                    self.runtime_resource_text("IDS_NET_DEACTIVATECLIENT", "De&activate")
                } else {
                    self.runtime_resource_text("IDS_NET_ACTIVATECLIENT", "&Activate")
                })
                .with_tooltip(self.runtime_resource_text(
                    "IDS_NET_ACTIVATECLIENT_DESC",
                    "Toggle player/observer-status",
                ))
                .with_action(AppContextMenuCommand::LobbyClientToggleActivate(client_id)),
            );
        }
        entries.push(
            ContextMenuEntry::new(self.runtime_resource_text("IDS_NET_CLIENTINFO", "&Info"))
                .with_tooltip(
                    self.runtime_resource_text("IDS_NET_CLIENTINFO_DESC", "Show extended info"),
                )
                .with_action(AppContextMenuCommand::LobbyClientInfo(client_id)),
        );
        Some(entries)
    }

    fn open_classic_lobby_roster_context(
        &mut self,
        row: LobbyRosterId,
        position: GuiPoint,
    ) -> Result<bool, EngineError> {
        match row {
            LobbyRosterId::Player(player_id) => {
                let Some((client_id, entries)) =
                    self.classic_lobby_player_context_entries(player_id)
                else {
                    return Ok(false);
                };
                let opened = self.open_context_menu_at(entries, position)?;
                if opened {
                    let opened_as_free_savegame = self
                        .visible_classic_lobby_player_context_target(player_id)
                        .is_some_and(|(_, free_savegame_player)| free_savegame_player);
                    self.context_menu_lobby_player =
                        Some((client_id, player_id, opened_as_free_savegame));
                }
                Ok(opened)
            }
            LobbyRosterId::Client(client_id) => {
                let Some(entries) = self.classic_lobby_client_context_entries(client_id) else {
                    return Ok(false);
                };
                let opened = self.open_context_menu_at(entries, position)?;
                if opened {
                    self.context_menu_lobby_kick_client = Some(client_id);
                }
                Ok(opened)
            }
            row @ LobbyRosterId::Header(_) => Err(classic_game_lobby_child_error(
                ClassicGameLobbyChild::RosterContext(row),
            )),
        }
    }

    pub(crate) fn toggle_classic_lobby_client_mute(&mut self, client_id: i32) {
        if self.control_clients.contains(client_id) {
            let muted = !self.control_messages.is_muted(client_id);
            self.control_messages.set_muted(client_id, muted);
        }
    }

    pub(crate) fn toggle_classic_lobby_client_activation(&mut self, client_id: i32) {
        if self.network.is_none()
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || self.visible_lobby_client_is_local(client_id).is_none()
            || !self.control_clients.contains(client_id)
        {
            return;
        }
        let update = clonk_engine::ClientUpdateControlData::new(
            clonk_engine::CLIENT_UPDATE_ACTIVATE,
            client_id,
            i32::from(!self.control_clients.is_activated(client_id)),
            0,
        );
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_client_update(update))
        {
            tracing::error!(%error, client_id, "failed to toggle lobby client activation");
        }
    }

    pub(crate) fn open_classic_lobby_client_info(
        &mut self,
        client_id: i32,
    ) -> Result<bool, EngineError> {
        if self.network.is_none() {
            return Ok(false);
        }
        Self::guard_gui_overlay_result(
            "C4Network2ClientDlg",
            self.assets
                .runtime_client_list_resources()
                .context("exact C4Network2ClientDlg resource set is absent")
                .and_then(|resources| resources.validate()),
        )?;
        // C4Network2ClientDlg is constructed from the id alone and resolves the
        // client inside UpdateText, so a stale context entry opens the dialog on
        // its unknown-id text instead of doing nothing
        // (src/C4Network2Dialogs.cpp:42-59).
        let (_, rows, _) = self.runtime_client_list_snapshot();
        let row = rows.into_iter().find(|row| row.client_id == client_id);
        self.cancel_underlying_interaction();
        self.runtime_client_list_consumed_keys.clear();
        self.runtime_client_list = Some(
            clonk_frontend::runtime_client_list::RuntimeClientListDialog::new_info(
                self.runtime_resource_string("IDS_NET_CLIENT_INFO"),
                client_id,
                row,
            )
            .with_info_resources(self.runtime_client_info_resources()),
        );
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ClientList);
        Ok(true)
    }

    pub(crate) fn kick_classic_lobby_client(&mut self, client_id: i32) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || client_id == 0
            || !self.control_clients.contains(client_id)
        {
            return;
        }
        let league_vote = self.network_is_league
            && self
                .control_player_infos
                .retained_rows_snapshot()
                .1
                .into_iter()
                .find(|(packet_client, _, _)| *packet_client == client_id)
                .is_some_and(|(_, _, players)| {
                    players
                        .iter()
                        .any(|player| player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
                });
        if league_vote {
            self.submit_own_league_vote(
                LeagueVoteSubject {
                    vote_type: clonk_engine::VOTE_TYPE_KICK,
                    data: client_id,
                },
                true,
            );
            return;
        }
        let remove = clonk_engine::ClientRemoveControlData {
            client_id,
            reason: LegacyCString::from_bytes(b"kicked from startup waiting dialog".to_vec())
                .unwrap_or_default(),
            by_client: 0,
        };
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_client_remove(remove))
        {
            tracing::error!(%error, client_id, "failed to kick lobby client");
        }
    }

    pub(crate) fn report_classic_lobby_error(&mut self, detail: impl Into<String>) {
        let detail = detail.into();
        self.status_text.clone_from(&detail);
        self.append_control_message_log(detail, 0x00ff_1f1f, None);
    }

    pub(crate) fn submit_selected_classic_lobby_player(
        &mut self,
        client_id: i32,
        source_path: &Path,
        wire_filename: &str,
    ) {
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        if local_client_id != Some(client_id) || !self.control_clients.contains(client_id) {
            self.report_classic_lobby_error(
                "The selected lobby client is no longer available for adding a player.",
            );
            return;
        }
        if !source_path.exists() {
            self.report_classic_lobby_error(format!(
                "The selected player file no longer exists: {}",
                source_path.display()
            ));
            return;
        }
        let countdown_active = self
            .visible_classic_lobby_controller()
            .is_some_and(|controller| controller.countdown().is_locked());
        if countdown_active {
            if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
                return;
            }
            let abort = clonk_network::LobbyCountdownPacket::new(
                clonk_network::LobbyCountdownPacket::ABORT,
            );
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_lobby_countdown(abort))
            {
                tracing::error!(%error, "failed to abort countdown before adding lobby player");
                self.report_classic_lobby_error(format!(
                    "Unable to abort the countdown before adding the player: {error}"
                ));
                return;
            }
            if let Some(controller) = self.visible_classic_lobby_controller_mut() {
                let _ = controller.apply_countdown_packet(
                    clonk_frontend::game_lobby::LobbyCountdownPacket::Abort,
                );
            }
        }
        if let Err(error) = self.submit_runtime_network_player_path(source_path, wire_filename) {
            tracing::error!(path = %source_path.display(), %error, "failed to add lobby player");
            self.report_classic_lobby_error(format!("Unable to add player: {error}"));
        }
    }

    fn add_classic_lobby_script_player(&mut self) {
        let request = {
            let _guard = lock_unpoisoned(&CLASSIC_SAFE_RANDOM_LOCK);
            self.classic_lobby_script_player_request_with_random(classic_safe_random_unlocked)
        };
        self.submit_classic_lobby_script_player_request(request);
    }

    #[cfg(test)]
    pub(crate) fn add_classic_lobby_script_player_with_random(
        &mut self,
        next_random: impl FnMut(usize) -> usize,
    ) {
        let request = self.classic_lobby_script_player_request_with_random(next_random);
        self.submit_classic_lobby_script_player_request(request);
    }

    fn classic_lobby_script_player_request_with_random(
        &self,
        mut next_random: impl FnMut(usize) -> usize,
    ) -> Option<clonk_engine::PlayerInfoUpdateRequest> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return None;
        }
        let metadata = self.network_team_assignment.as_ref()?.teams();
        let (_, packets) = self.control_player_infos.retained_rows_snapshot();
        let active = packets
            .iter()
            .flat_map(|(_, _, players)| players)
            .filter(|player| player.flags & clonk_engine::PLAYER_INFO_FLAG_REMOVED == 0)
            .collect::<Vec<_>>();
        let active_script_players = active
            .iter()
            .filter(|player| player.is_script_player())
            .count();
        if metadata.max_script_players <= i32::try_from(active_script_players).unwrap_or(i32::MAX) {
            return None;
        }
        let active_names = active
            .iter()
            .map(|player| {
                if !player.league_account.is_empty() {
                    player.league_account.as_bytes()
                } else if !player.forced_name.is_empty() {
                    player.forced_name.as_bytes()
                } else {
                    player.name.as_bytes()
                }
            })
            .collect::<Vec<_>>();
        let name = classic_script_player_name(
            &metadata.script_player_names,
            &active_names,
            &mut next_random,
        );
        let color = classic_script_player_color(&mut next_random);
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        Some(clonk_engine::PlayerInfoUpdateRequest {
            client_id: local_client_id,
            flags: clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS,
            players: vec![clonk_engine::ControlPlayerInfoEntry {
                name,
                player_type: clonk_engine::PLAYER_INFO_TYPE_SCRIPT,
                color,
                original_color: color,
                ..clonk_engine::ControlPlayerInfoEntry::default()
            }],
        })
    }

    fn submit_classic_lobby_script_player_request(
        &self,
        request: Option<clonk_engine::PlayerInfoUpdateRequest>,
    ) {
        let Some(request) = request else {
            return;
        };
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_player_info_update(request))
        {
            tracing::error!(%error, "failed to submit lobby script player");
        }
    }

    fn open_classic_lobby_team_combo(&mut self, player_id: i32) -> Result<bool, EngineError> {
        let dismissed_player = self.context_menu_pointer_dismissed_lobby_team_player.take();
        if dismissed_player == Some(player_id) {
            // Screen::MouseInput already aborted this combo's menu on the
            // same left-down. ComboBox::MouseInput rechecks the last menu ID
            // and deliberately does not reopen it.
            return Ok(false);
        }
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::NetworkLobby
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }

        let (_, roster) = self.visible_classic_lobby_layouts()?;
        let Some((client_id, current_team, anchor, minimum_width)) =
            self.visible_classic_lobby_controller()
                .and_then(|controller| {
                    if controller.countdown().is_locked() {
                        return None;
                    }
                    let (index, player) = controller.rows().iter().enumerate().find_map(
                        |(index, row)| match row {
                            LobbyRosterRow::Player(player) if player.id == player_id => {
                                Some((index, player))
                            }
                            _ => None,
                        },
                    )?;
                    let team = player.team.as_ref().filter(|team| team.selectable)?;
                    let team_rect = roster.rows.iter().find(|row| row.index == index)?.team?;
                    Some((
                        player.client_id,
                        team.id,
                        GuiPoint::new(team_rect.x as f32, (team_rect.y + team_rect.h) as f32),
                        team_rect.w,
                    ))
                })
        else {
            return Ok(false);
        };
        if !self.classic_lobby_team_change_is_allowed(player_id, client_id) {
            return Ok(false);
        }

        let select_template = self
            .startup_tooltip_resources
            .get("IDS_MSG_SELECT")
            .cloned();
        let metadata = self.visible_classic_lobby_team_metadata();
        let entries = metadata
            .as_ref()
            .into_iter()
            .flat_map(|metadata| metadata.teams.iter())
            .filter(|team| {
                team.id == current_team
                    || team.max_players == 0
                    || i32::try_from(team.player_ids.len()).unwrap_or(i32::MAX) < team.max_players
            })
            .map(|team| {
                let name = legacy_presentation_text(team.name.as_bytes());
                let tooltip = select_template
                    .as_deref()
                    .map(|template| template.replacen("%s", &name, 1))
                    .unwrap_or_else(|| format!("Select {name}"));
                ContextMenuEntry::new(name)
                    .with_tooltip(tooltip)
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(AppContextMenuCommand::LobbyTeam {
                        player_id,
                        team_id: team.id,
                    })
            })
            .collect();
        self.open_context_menu_at_with_minimum_width(
            entries,
            anchor,
            minimum_width,
            Some(player_id),
        )
    }

    fn classic_lobby_option_is_editable(&self, option: LobbyOptionKind) -> bool {
        self.classic_host_lobby.as_ref().is_some_and(|lobby| {
            lobby.controller.active_sheet() == LobbySheet::Options
                && lobby
                    .controller
                    .option_rows()
                    .iter()
                    .any(|row| row.kind == option && row.editable)
        })
    }

    fn classic_lobby_option_accepts_choice(&self, option: LobbyOptionKind, selected: i32) -> bool {
        self.classic_host_lobby.as_ref().is_some_and(|lobby| {
            lobby.controller.active_sheet() == LobbySheet::Options
                && lobby.controller.option_rows().iter().any(|row| {
                    row.kind == option
                        && row.editable
                        && row.choices.iter().any(|choice| choice.id == selected)
                })
        })
    }

    fn open_classic_lobby_option_combo(
        &mut self,
        option: LobbyOptionKind,
        anchor: GuiPoint,
        minimum_width: i32,
    ) -> Result<bool, EngineError> {
        if self.context_menu_pointer_dismissed_lobby_option.take() == Some(option) {
            // The outside click which closed this ComboBox is still being
            // delivered to the underlying sheet; do not reopen it.
            return Ok(false);
        }
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::NetworkLobby
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
            || option == LobbyOptionKind::ControlMode
        {
            return Ok(false);
        }
        let Some(choices) = self.classic_host_lobby.as_ref().and_then(|lobby| {
            (lobby.controller.active_sheet() == LobbySheet::Options)
                .then(|| {
                    lobby
                        .controller
                        .option_rows()
                        .iter()
                        .find(|row| row.kind == option && row.editable)
                        .map(|row| row.choices.clone())
                })
                .flatten()
        }) else {
            return Ok(false);
        };
        let entries = choices
            .into_iter()
            .map(|choice| {
                let command = match option {
                    LobbyOptionKind::ControlRate => {
                        AppContextMenuCommand::LobbyControlRate(choice.id)
                    }
                    LobbyOptionKind::RuntimeJoin => {
                        AppContextMenuCommand::LobbyRuntimeJoin(choice.id != 0)
                    }
                    LobbyOptionKind::TeamDistribution => {
                        AppContextMenuCommand::LobbyTeamDistribution(choice.id)
                    }
                    LobbyOptionKind::TeamColors => {
                        AppContextMenuCommand::LobbyTeamColors(choice.id != 0)
                    }
                    LobbyOptionKind::RandomTeamCount => {
                        AppContextMenuCommand::LobbyRandomTeamCount(choice.id)
                    }
                    LobbyOptionKind::ControlMode => unreachable!("read-only lobby option"),
                };
                ContextMenuEntry::new(choice.label)
                    .with_tooltip(choice.tooltip)
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(command)
            })
            .collect();
        let opened =
            self.open_context_menu_at_with_minimum_width(entries, anchor, minimum_width, None)?;
        if opened {
            self.set_context_menu_lobby_option(Some(option));
        }
        Ok(opened)
    }

    pub(crate) fn submit_classic_lobby_control_rate(&mut self, selected: i32) {
        if !(1..=9).contains(&selected)
            || !self.classic_lobby_option_is_editable(LobbyOptionKind::ControlRate)
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self
                .network
                .as_ref()
                .is_some_and(|network| network.local_client_id() == 0)
        {
            return;
        }
        // Native re-reads ControlRate when the menu action activates. A
        // control echoed before this click therefore changes the submitted
        // relative adjustment instead of applying a stale absolute value.
        let current = self
            .network_control_clock
            .map(NetworkControlClock::control_rate)
            .unwrap_or_else(|| self.engine.control_rate());
        if selected == current {
            return;
        }
        let result = self.network.as_ref().map(|network| {
            network.submit_control_set(clonk_network::LegacyControlSet {
                value_type: 0,
                data: selected - current,
                by_client: 0,
            })
        });
        if let Some(Err(error)) = result {
            tracing::error!(%error, "failed to submit lobby control-rate adjustment");
            self.report_classic_lobby_error(format!("Unable to change the control rate: {error}"));
        }
    }

    pub(crate) fn submit_classic_lobby_team_setting(
        &mut self,
        option: LobbyOptionKind,
        selected: i32,
    ) {
        let value_type = match option {
            LobbyOptionKind::TeamDistribution => 3,
            LobbyOptionKind::TeamColors => 4,
            _ => return,
        };
        if !self.classic_lobby_option_accepts_choice(option, selected)
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self
                .network
                .as_ref()
                .is_some_and(|network| network.local_client_id() == 0)
        {
            return;
        }
        let result = self.network.as_ref().map(|network| {
            network.submit_control_set(clonk_network::LegacyControlSet {
                value_type,
                data: selected,
                by_client: 0,
            })
        });
        if let Some(Err(error)) = result {
            tracing::error!(%error, value_type, selected, "failed to submit lobby team setting");
            self.report_classic_lobby_error(format!("Unable to change the team setting: {error}"));
        }
    }

    pub(crate) fn set_classic_lobby_random_team_count(&mut self, selected: i32) {
        if !self.classic_lobby_option_accepts_choice(LobbyOptionKind::RandomTeamCount, selected)
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self
                .network
                .as_ref()
                .is_some_and(|network| network.local_client_id() == 0)
        {
            return;
        }
        let has_or_will_have_lobby = self.has_or_will_have_network_lobby();
        let Some((metadata, updates)) = self.network_team_assignment.as_mut().map(|assignment| {
            let updates = assignment.set_random_team_count(
                &mut self.control_player_infos,
                selected,
                has_or_will_have_lobby,
            );
            (assignment.teams().clone(), updates)
        }) else {
            return;
        };

        let runtime_teams = runtime_teams_from_initial_metadata(&metadata);
        let team_snapshot = clonk_network::join_team_list_snapshot(metadata);
        self.engine.set_teams(runtime_teams.clone());
        if let Some(prepared) = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.prepared_go.as_mut())
        {
            prepared.team_registry = runtime_teams;
        }
        if let Some(join_data) = self.pending_network_join_data.as_mut() {
            join_data.parameters.teams = team_snapshot.clone();
        }
        let mut host_snapshot_changed = false;
        if let Some(snapshot) = self.host_join_snapshot.as_mut() {
            snapshot.parameters.teams = team_snapshot;
            host_snapshot_changed = true;
        }
        host_snapshot_changed |= self.refresh_current_host_player_infos();
        if let Some(network) = self.network.as_ref() {
            for update in updates {
                if let Err(error) = network.broadcast_player_info(update) {
                    tracing::error!(%error, "failed to broadcast RandomTeamCount PlayerInfo update");
                }
            }
        }
        if host_snapshot_changed {
            self.publish_updated_host_join_snapshot();
        }
        self.sync_classic_lobby_roster();
        let _ = self.refresh_classic_lobby_options(true);
    }

    pub(crate) fn set_classic_lobby_runtime_join(&mut self, allowed: bool) {
        if !self.classic_lobby_option_is_editable(LobbyOptionKind::RuntimeJoin)
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self
                .network
                .as_ref()
                .is_some_and(|network| network.local_client_id() == 0)
        {
            return;
        }
        let Some(lobby) = self.classic_host_lobby.as_mut() else {
            return;
        };
        lobby.runtime_join_allowed = allowed;
        if let Some(NetworkMode::Host(HostSettings {
            prepared: Some(prepared),
            ..
        })) = self.network_mode.as_mut()
        {
            prepared.set_runtime_join_allowed(allowed);
        }
        self.persist_game_option_value(
            "Network",
            "NoRuntimeJoin",
            if allowed { "0" } else { "1" }.to_string(),
        );
        // Lobby admission deliberately remains open. The retained prepared
        // policy is queued immediately after the Go status request when the
        // lobby exits, matching DoLobby's post-status admission update.
        let _ = self.refresh_classic_lobby_options(true);
    }

    fn classic_lobby_team_change_is_allowed(&self, player_id: i32, client_id: i32) -> bool {
        let player_is_eligible = self
            .control_player_infos
            .client_update_request(client_id)
            .and_then(|request| {
                request
                    .players
                    .into_iter()
                    .find(|player| player.id == player_id)
            })
            .is_some_and(|player| !player.is_joined() && player.savegame_player == 0);
        if !player_is_eligible {
            return false;
        }
        let Some(metadata) = self.visible_classic_lobby_team_metadata() else {
            return false;
        };
        let distribution_allows_change = match metadata.team_distribution {
            clonk_engine::InitialNetworkTeamDistribution::Free => true,
            clonk_engine::InitialNetworkTeamDistribution::Host => {
                matches!(self.network_mode, Some(NetworkMode::Host(_)))
            }
            clonk_engine::InitialNetworkTeamDistribution::None
            | clonk_engine::InitialNetworkTeamDistribution::Random
            | clonk_engine::InitialNetworkTeamDistribution::RandomInvisible => false,
        };
        if !metadata.active || !distribution_allows_change {
            return false;
        }
        if metadata.auto_generate_teams {
            return true;
        }
        let current_team = metadata
            .teams
            .iter()
            .find(|team| team.player_ids.contains(&player_id))
            .map(|team| team.id);
        metadata.teams.iter().any(|team| {
            Some(team.id) != current_team
                && (team.max_players == 0
                    || i32::try_from(team.player_ids.len()).unwrap_or(i32::MAX) < team.max_players)
        })
    }

    pub(crate) fn submit_classic_lobby_team_selection(&mut self, player_id: i32, team_id: i32) {
        let Some(client_id) = self
            .visible_classic_lobby_controller()
            .and_then(|controller| {
                if controller.countdown().is_locked() {
                    return None;
                }
                controller.rows().iter().find_map(|row| match row {
                    LobbyRosterRow::Player(player)
                        if player.id == player_id
                            && player.team.as_ref().is_some_and(|team| team.selectable) =>
                    {
                        Some(player.client_id)
                    }
                    _ => None,
                })
            })
        else {
            return;
        };
        if !self.classic_lobby_team_change_is_allowed(player_id, client_id) {
            return;
        }
        if !self
            .visible_classic_lobby_team_metadata()
            .is_some_and(|metadata| metadata.teams.iter().any(|team| team.id == team_id))
        {
            return;
        }
        let Some(mut request) = self.control_player_infos.client_update_request(client_id) else {
            return;
        };
        let Some(player) = request
            .players
            .iter_mut()
            .find(|player| player.id == player_id)
        else {
            return;
        };
        player.team = team_id;
        let Some(network) = self.network.as_ref() else {
            tracing::error!(
                player_id,
                team_id,
                "lobby team selection has no network session"
            );
            return;
        };
        if let Err(error) = network.submit_player_info_update(request) {
            tracing::error!(%error, player_id, team_id, "failed to submit lobby team selection");
        }
    }

    pub(crate) fn move_local_classic_lobby_players_into_team(&mut self, team_id: i32) {
        if self
            .visible_classic_lobby_controller()
            .is_none_or(|controller| controller.countdown().is_locked())
        {
            return;
        }
        let Some(metadata) = self.visible_classic_lobby_team_metadata() else {
            return;
        };
        if !metadata.active {
            return;
        }
        let distribution_allows_change = match metadata.team_distribution {
            clonk_engine::InitialNetworkTeamDistribution::Free => true,
            clonk_engine::InitialNetworkTeamDistribution::Host => {
                matches!(self.network_mode, Some(NetworkMode::Host(_)))
            }
            clonk_engine::InitialNetworkTeamDistribution::None
            | clonk_engine::InitialNetworkTeamDistribution::Random
            | clonk_engine::InitialNetworkTeamDistribution::RandomInvisible => false,
        };
        let has_available_team = metadata.auto_generate_teams
            || metadata.teams.iter().any(|team| {
                team.max_players == 0
                    || i32::try_from(team.player_ids.len()).unwrap_or(i32::MAX) < team.max_players
            });
        if !distribution_allows_change
            || !has_available_team
            || !metadata.teams.iter().any(|team| team.id == team_id)
        {
            return;
        }
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return;
        };
        let Some(mut request) = self
            .control_player_infos
            .client_update_request(local_client_id)
        else {
            return;
        };
        let mut changed = false;
        for player in &mut request.players {
            if player.player_type == clonk_engine::PLAYER_INFO_TYPE_USER && player.team != team_id {
                player.team = team_id;
                changed = true;
            }
        }
        if !changed {
            return;
        }
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.submit_player_info_update(request) {
            tracing::error!(%error, team_id, "failed to move local lobby players into team");
        }
    }

    fn classic_lobby_player_action_is_allowed(&self, client_id: i32, player_id: i32) -> bool {
        player_id != 0
            && self.network.is_some()
            && self.classic_lobby_player_is_owned(client_id)
            && self
                .visible_classic_lobby_controller()
                .is_some_and(|controller| {
                    controller.rows().iter().any(|row| {
                        matches!(row, LobbyRosterRow::Player(player)
                        if player.id == player_id && player.client_id == client_id)
                    })
                })
    }

    pub(crate) fn take_over_classic_lobby_savegame_player(
        &mut self,
        savegame_player_id: i32,
        player_id: i32,
    ) {
        let target_is_free = self
            .visible_classic_lobby_player_context_target(savegame_player_id)
            .is_some_and(|(client_id, free_savegame_player)| {
                client_id == -1 && free_savegame_player
            });
        if !target_is_free || player_id == 0 {
            return;
        }
        let Some(local_client_id) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return;
        };
        let Some(mut request) = self
            .control_player_infos
            .client_update_request(local_client_id)
        else {
            return;
        };
        let Some(player) = request
            .players
            .iter_mut()
            .find(|player| player.id == player_id)
        else {
            return;
        };
        player.savegame_player = savegame_player_id;
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.submit_player_info_update(request) {
            tracing::error!(
                %error,
                savegame_player_id,
                player_id,
                "failed to submit lobby savegame-player takeover"
            );
        }
    }

    pub(crate) fn remove_classic_lobby_player(&mut self, client_id: i32, player_id: i32) {
        if !self.classic_lobby_player_action_is_allowed(client_id, player_id) {
            return;
        }
        let countdown_locked = self
            .visible_classic_lobby_controller()
            .is_some_and(|controller| controller.countdown().is_locked());
        if countdown_locked {
            if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
                return;
            }
            if !self.abort_network_lobby_countdown() {
                self.submit_and_apply_lobby_countdown(clonk_network::LobbyCountdownPacket::new(
                    clonk_network::LobbyCountdownPacket::ABORT,
                ));
            }
        }

        let Some(mut request) = self.control_player_infos.client_update_request(client_id) else {
            return;
        };
        let Some(index) = request
            .players
            .iter()
            .position(|player| player.id == player_id)
        else {
            return;
        };
        request.players.swap_remove(index);
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.submit_player_info_update(request) {
            tracing::error!(%error, client_id, player_id, "failed to remove lobby player");
        }
        if self.network_is_league {
            // C++ clears this immediately after the void update request so
            // account changes require fresh authentication on the next join.
            self.clear_remembered_league_password();
        }
    }

    pub(crate) fn reset_classic_lobby_player_color(&mut self, client_id: i32, player_id: i32) {
        if !self.classic_lobby_player_action_is_allowed(client_id, player_id) {
            return;
        }
        let Some(mut request) = self.control_player_infos.client_update_request(client_id) else {
            return;
        };
        let Some(player) = request
            .players
            .iter_mut()
            .find(|player| player.id == player_id)
        else {
            return;
        };
        player.color = player.original_color;
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.submit_player_info_update(request) {
            tracing::error!(%error, client_id, player_id, "failed to reset lobby player color");
        }
    }

    fn start_network_lobby_countdown(&mut self) -> Result<(), EngineError> {
        self.start_network_lobby_countdown_with(DEFAULT_LOBBY_COUNTDOWN_SECONDS)
    }

    pub(crate) fn start_classic_command_line_lobby_timeout(&mut self) -> Result<(), EngineError> {
        let Some(seconds) = self.classic_command_line.lobby_timeout.flatten() else {
            return Ok(());
        };
        if seconds == 0 {
            return Ok(());
        }
        self.start_network_lobby_countdown_with(i32::try_from(seconds).unwrap_or(i32::MAX))
    }

    fn prepare_network_lobby_countdown(&mut self) -> Result<bool, EngineError> {
        if self.classic_host_lobby.is_some() {
            if let Some(overrides) = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| &staged.pending_global_gui_failures)
            {
                self.assets
                    .require_classic_global_gui_bootstrap_resources(overrides)
                    .map_err(report_classic_parity_boundary)
                    .map_err(classic_parity_engine_error)?;
            }
            self.assets
                .network_start_wait_resources()
                .map_err(|error| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                            detail: format!("network start-wait dialog is unavailable: {error}"),
                        }),
                    ))
                })?;
        }
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            self.network_game_start_guard_passes();
            return Ok(false);
        }
        if self.abort_network_lobby_countdown() {
            return Ok(false);
        }
        if !self.network_game_start_guard_passes() {
            return Ok(false);
        }
        Ok(true)
    }

    pub(crate) fn start_network_lobby_countdown_with(
        &mut self,
        countdown_seconds: i32,
    ) -> Result<(), EngineError> {
        if !self.prepare_network_lobby_countdown()? {
            return Ok(());
        }
        if countdown_seconds <= 0 || self.network.is_none() {
            return self.start_network_game_now();
        }
        self.host_lobby_countdown = Some(HostLobbyCountdown::with_seconds(countdown_seconds));
        let packet = clonk_network::LobbyCountdownPacket::new(countdown_seconds);
        self.submit_and_apply_lobby_countdown(packet);
        Ok(())
    }

    fn start_console_lobby_countdown_with(
        &mut self,
        countdown_seconds: i32,
    ) -> Result<(), EngineError> {
        if countdown_seconds != 0 {
            return self.start_network_lobby_countdown_with(countdown_seconds);
        }
        if !self.prepare_network_lobby_countdown()? {
            return Ok(());
        }
        if self.network.is_none() {
            return self.start_network_game_now();
        }
        self.host_lobby_countdown = Some(HostLobbyCountdown::with_seconds(0));
        self.submit_and_apply_lobby_countdown(clonk_network::LobbyCountdownPacket::new(0));
        Ok(())
    }

    pub(crate) fn abort_network_lobby_countdown(&mut self) -> bool {
        if self.host_lobby_countdown.take().is_none() {
            return false;
        }
        let packet =
            clonk_network::LobbyCountdownPacket::new(clonk_network::LobbyCountdownPacket::ABORT);
        self.submit_and_apply_lobby_countdown(packet);
        true
    }

    fn submit_and_apply_lobby_countdown(&mut self, packet: clonk_network::LobbyCountdownPacket) {
        if let Some(network) = self.network.as_ref() {
            match network.submit_lobby_countdown(packet) {
                Ok(()) => self.pending_local_lobby_countdown_echoes.push_back(packet),
                Err(error) => {
                    tracing::error!(%error, "failed to submit host lobby countdown");
                }
            }
        }
        self.apply_lobby_countdown_presentation(packet);
    }

    /// The countdown a dedicated engine has nowhere to draw goes to the log.
    ///
    /// C++ routes every countdown packet at `Game.Network.GetLobby()` and,
    /// when there is no such dialog, logs it instead — on the opening packet
    /// (src/C4GameLobby.cpp:1118-1127), on each broadcast second while the
    /// timer is still running (`:1150-1157`), and on an abort, as
    /// `IDS_PRC_STARTABORTED` (`:1183-1190`). Zero is deliberately not logged
    /// here: the round either starts or aborts, and both say so themselves.
    fn log_dialogless_lobby_countdown(
        &mut self,
        packet: clonk_network::LobbyCountdownPacket,
        initial: bool,
    ) {
        if !(self.console_mode || self.headless) {
            return;
        }
        let labels = self.classic_lobby_labels();
        let message = if packet.is_abort() {
            labels.start_aborted
        } else if packet.countdown() == 0 {
            return;
        } else {
            lobby_countdown_message(packet.countdown(), initial, &labels.countdown_template)
        };
        tracing::info!("{message}");
    }

    pub(crate) fn apply_lobby_countdown_presentation(
        &mut self,
        packet: clonk_network::LobbyCountdownPacket,
    ) {
        let frontend_packet = if packet.is_abort() {
            clonk_frontend::game_lobby::LobbyCountdownPacket::Abort
        } else {
            clonk_frontend::game_lobby::LobbyCountdownPacket::Seconds(packet.countdown())
        };
        // `MainDlg::OnCountdownPacket` passes `!fWasCountdown` as the packet's
        // "initial" flag (src/C4GameLobby.cpp:415), so it must be read before
        // the controller below consumes this packet.
        let was_counting_down = self
            .visible_classic_lobby_controller()
            .is_some_and(|controller| controller.countdown().is_locked());
        self.log_dialogless_lobby_countdown(packet, !was_counting_down);
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.apply_lobby_countdown(packet);
        }
        let actions = if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.apply_countdown_packet(frontend_packet)
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            // A joined client owns the same long-lived MainDlg as the host.
            // Initialize it before applying the packet so the controller,
            // log, sounds and focus transition all observe the event once.
            lobby.sync_classic_controller();
            lobby.controller.apply_countdown_packet(frontend_packet)
        } else {
            Vec::new()
        };
        for action in actions {
            match action {
                ClassicLobbyAction::CountdownChanged(state) => {
                    self.scenario_game_options.set_countdown(state.is_locked());
                }
                ClassicLobbyAction::FocusChanged(control) => {
                    self.set_active_lobby_chat_focus(control == LobbyControl::ChatInput);
                    self.scenario_game_options
                        .set_focused_button(match control {
                            LobbyControl::GameOption(button) => Some(button),
                            _ => None,
                        });
                }
                ClassicLobbyAction::NotifyUserIfInactive => {
                    self.request_control_message_attention();
                }
                ClassicLobbyAction::AppendLog(line) => {
                    let removed_frontend_copy = self
                        .visible_classic_lobby_controller_mut()
                        .is_some_and(|controller| {
                            let mut logs = controller.logs().to_vec();
                            if logs.last() != Some(&line) {
                                return false;
                            }
                            logs.pop();
                            controller.set_logs(logs);
                            true
                        });
                    if removed_frontend_copy {
                        self.append_control_message_log(line.text, 0x00ff_1f1f, None);
                    }
                }
                _ => unreachable!("countdown presentation emitted a non-countdown action"),
            }
        }
        self.play_classic_lobby_sounds();
    }

    pub(crate) fn request_lobby_ready_check_at(
        &mut self,
        now: Instant,
    ) -> Result<bool, EngineError> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return Ok(false);
        }
        if !self.lobby_ready_check_cooldown.try_reset_at(now) {
            let remaining = self.lobby_ready_check_cooldown.remaining_seconds_at(now);
            self.status_text = format!("Too early! Please wait {remaining} seconds.");
            return Ok(false);
        }
        self.abort_network_lobby_countdown();
        if let Some(lobby) = self.network_lobby.as_mut() {
            for (client_id, participant) in &mut lobby.participants {
                if *client_id != 0 {
                    participant.ready = false;
                }
            }
        }
        if self.control_clients.clear_nonhost_lobby_ready() {
            self.publish_updated_host_join_snapshot();
        }
        self.sync_classic_lobby_roster();
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_ready_check(clonk_network::ReadyCheckData::Request))
        {
            tracing::error!(%error, "failed to submit lobby ready check request");
        }
        Ok(true)
    }

    pub(crate) fn append_remote_lobby_ready_log(
        &mut self,
        packet: clonk_network::ReadyCheckPacket,
    ) {
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok());
        if local_client_id == Some(packet.client_id) {
            return;
        }
        let Some(client) = self.control_clients.state(packet.client_id) else {
            return;
        };
        let client_name = legacy_presentation_text(client.name.as_bytes());
        let key = if packet.data.is_ready() {
            "IDS_NET_CLIENT_READY"
        } else {
            "IDS_NET_CLIENT_UNREADY"
        };
        let fallback = if packet.data.is_ready() {
            "Client %s ready."
        } else {
            "Client %s not ready."
        };
        let template = self.runtime_resource_text(key, fallback);
        let text = template
            .split_once("%s")
            .map(|(prefix, suffix)| format!("{prefix}{client_name}{suffix}"))
            .unwrap_or(template);
        self.append_control_message_log(text, CONTROL_LOG_COLOR, None);
    }

    pub(crate) fn handle_lobby_ready_check_request(
        &mut self,
        packet: clonk_network::ReadyCheckPacket,
    ) -> Result<(), EngineError> {
        if self.message_dialogs.iter().any(|dialog| {
            matches!(
                &dialog.continuation,
                MessageDialogContinuation::LobbyReadyCheck { .. }
            )
        }) {
            return Ok(());
        }
        if !matches!(self.network_mode, Some(NetworkMode::Client(_))) || packet.client_id != 0 {
            return Ok(());
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            for (client_id, participant) in &mut lobby.participants {
                if *client_id != 0 {
                    participant.ready = false;
                }
            }
        }
        if !self.admission_resources.lobby_ready_available() {
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_ready_check(clonk_network::ReadyCheckData::NotReady))
            {
                tracing::error!(%error, "failed to submit lobby ready check response");
            }
            return Ok(());
        }
        let remaining_seconds = LOBBY_READY_CHECK_PROMPT_SECONDS;
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                lobby_ready_check_message(remaining_seconds),
                "Are you ready?",
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(30),
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            )
            .with_centered_message()
            .without_focus(),
            MessageDialogContinuation::LobbyReadyCheck { remaining_seconds },
        )?;
        if self.ready_check_toasts_enabled && !self.window_active {
            self.pending_desktop_notifications
                .push_back(DesktopNotification::new(
                    "Are you ready?",
                    lobby_ready_check_message(remaining_seconds).replace('|', "\n"),
                    Duration::from_secs(u64::from(remaining_seconds)),
                ));
        }
        Ok(())
    }

    pub(crate) fn on_lobby_client_ready_state_change(
        &mut self,
        changed_client_id: ClientId,
    ) -> Result<(), EngineError> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return Ok(());
        }
        let player_infos = &self.control_player_infos;
        let clients = self.control_clients.snapshot();
        let first_unready_control_client = || {
            clients.iter().find_map(|client| {
                let relevant = client.client_id == 0
                    || !player_infos.client_info_ids(client.client_id).is_empty();
                (relevant && !client.lobby_ready)
                    .then(|| ClientId::try_from(client.client_id).ok())
                    .flatten()
            })
        };
        let first_unready_generic_lobby_client = || {
            self.network_lobby.as_ref().and_then(|lobby| {
                lobby
                    .participants
                    .iter()
                    .find_map(|(client_id, participant)| {
                        let relevant = *client_id == 0
                            || i32::try_from(*client_id).ok().is_some_and(|client_id| {
                                !player_infos.client_info_ids(client_id).is_empty()
                            });
                        (relevant && !participant.ready).then_some(*client_id)
                    })
            })
        };
        let first_relevant_unready = if self.classic_host_lobby_active() {
            first_unready_control_client()
        } else if self.network_lobby.is_some() {
            first_unready_generic_lobby_client()
        } else {
            first_unready_control_client()
        };
        if let Some(unready_client_id) = first_relevant_unready {
            if unready_client_id == changed_client_id {
                self.abort_network_lobby_countdown();
            }
            return Ok(());
        }
        if self.host_lobby_countdown.is_none() {
            let countdown_seconds = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| {
                    let configured = if staged.lobby.countdown_seconds < 0 {
                        DEFAULT_LOBBY_COUNTDOWN_SECONDS
                    } else {
                        staged.lobby.countdown_seconds
                    };
                    if self.network_is_league {
                        configured.max(5)
                    } else {
                        configured
                    }
                })
                .unwrap_or(DEFAULT_LOBBY_COUNTDOWN_SECONDS);
            if self.classic_host_lobby_active() {
                self.request_classic_lobby_start(
                    countdown_seconds,
                    true,
                    self.classic_lobby_has_unassociated_savegame_players(),
                )?;
            } else {
                self.start_network_lobby_countdown_with(countdown_seconds)?;
            }
        }
        Ok(())
    }

    pub(crate) fn paste_network_lobby_chat_text(&mut self, text: &str) -> Result<(), EngineError> {
        if self.network_lobby.is_none() {
            return Ok(());
        }
        let (layout, fonts) = self.active_lobby_chat_scroll_context()?;
        let (mut view, local_client_id) = {
            let lobby = self
                .network_lobby
                .as_mut()
                .expect("joined lobby was checked above");
            (std::mem::take(&mut lobby.chat_edit), lobby.local_client_id)
        };
        if self.lobby_chat_drag_anchor.is_some() && lobby_chat_paste_attempts_insertion(text) {
            if let Some((anchor, caret)) = view.selection {
                self.lobby_chat_drag_anchor = Some(anchor.min(caret));
            }
        }
        let result = lobby_chat_paste_text(
            &mut view,
            text,
            LobbyChatPasteMode::Lobby,
            |view| lobby_chat_scroll_caret_in_view(view, &layout, &fonts.text),
            |submission| {
                self.process_lobby_action(LobbyAction::SubmitMessage(submission))?;
                Ok(self.startup_view == StartupView::NetworkLobby
                    && self
                        .network_lobby
                        .as_ref()
                        .is_some_and(|lobby| lobby.local_client_id == local_client_id))
            },
        );
        let completed_lines = result
            .as_ref()
            .is_ok_and(|outcome| outcome.completed_lines > 0);
        if completed_lines && self.lobby_chat_drag_anchor.is_some() {
            self.lobby_chat_drag_anchor = Some(0);
        }
        let still_active = self.startup_view == StartupView::NetworkLobby
            && self
                .network_lobby
                .as_ref()
                .is_some_and(|lobby| lobby.local_client_id == local_client_id);
        if still_active {
            self.install_active_lobby_chat_view(view);
        }
        if completed_lines {
            if let Some(lobby) = self
                .network_lobby
                .as_mut()
                .filter(|lobby| lobby.local_client_id == local_client_id)
            {
                lobby.chat_history_index = -1;
            }
        }
        result.map(|_| ())
    }

    pub(crate) fn handle_joined_lobby_hotkey(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::NetworkLobby
            || self.network_lobby.is_none()
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if modifiers != ModifiersState::ALT
            && modifiers != (ModifiersState::ALT | ModifiersState::SHIFT)
        {
            return Ok(false);
        }
        let Some(hotkey) = startup_dialog_hotkey(key) else {
            return Ok(false);
        };
        let exit_hotkey = self
            .network_lobby
            .as_ref()
            .and_then(NetworkLobbyState::exit_hotkey);
        if Some(hotkey) == exit_hotkey {
            if state == ElementState::Pressed {
                self.process_lobby_action(LobbyAction::ExitRequested)?;
            }
            return Ok(true);
        }
        // The chat mnemonic plus the option-strip mnemonics the controller
        // dispatches as GameOptions hotkeys; the Players/Options/Start/Ready
        // mnemonics of Dialog::KeyHotkey are not routed on the joined path
        // yet (their focused confirm-key activation is).
        if hotkey != 'T' && !matches!(hotkey, 'I' | 'L' | 'M' | 'F' | 'R') {
            return Ok(false);
        }
        if state == ElementState::Pressed {
            let actions = self
                .network_lobby
                .as_mut()
                .expect("joined lobby was checked above")
                .classic_hotkey(hotkey);
            self.process_joined_lobby_controller_actions(actions)?;
        }
        Ok(true)
    }

    /// Dialog-level keys of the reconstructed joined lobby: Tab traverses the
    /// controller focus order, Escape aborts from any non-chat focus, and
    /// every mapped key reaches the focused controller-owned stop — an
    /// option-strip button, a chrome button or the Ready checkbox, whose
    /// Space/Return bindings live at control priority
    /// (src/C4GuiButton.cpp:33-47, src/C4GuiCheckBox.cpp:43-52) — with
    /// `Dialog::KeyFocusDefault` rerouting unhandled ones. Chat-focused
    /// Escape stays with the adapter's direct Exit route, and chat and the
    /// roster family keep their dedicated handlers.
    pub(crate) fn handle_joined_lobby_controller_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !self.joined_network_lobby_active() {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if !c4_modifiers.is_empty() && c4_modifiers != ModifiersState::SHIFT {
            return Ok(false);
        }
        let Some(gui_key) = map_key_code(key) else {
            return Ok(false);
        };
        if gui_key == KeyCode::Escape {
            // Dialog::KeyEscape aborts from any focus at PRIO_Dlg
            // (src/C4GuiDialogs.cpp:371-378); the chat-focused default keeps
            // the adapter's own silent Exit route below.
            let non_chat_focus = self.network_lobby.as_mut().is_some_and(|lobby| {
                lobby.sync_classic_controller();
                lobby.controller.focus() != LobbyControl::ChatInput
            });
            if !non_chat_focus {
                return Ok(false);
            }
            if state == ElementState::Pressed {
                self.process_joined_lobby_controller_actions(vec![
                    ClassicLobbyAction::ExitRequested,
                ])?;
            }
            return Ok(true);
        }
        let controller_focused = self.network_lobby.as_mut().is_some_and(|lobby| {
            lobby.sync_classic_controller();
            matches!(
                lobby.controller.focus(),
                LobbyControl::GameOption(_)
                    | LobbyControl::TeamsTab
                    | LobbyControl::PlayersTab
                    | LobbyControl::ResourcesTab
                    | LobbyControl::OptionsTab
                    | LobbyControl::ScenarioTab
                    | LobbyControl::ChatDialog
                    | LobbyControl::Exit
                    | LobbyControl::Run
                    | LobbyControl::Preload
                    | LobbyControl::Ready
            )
        });
        if gui_key != KeyCode::Tab && !controller_focused {
            return Ok(false);
        }
        let shift = self.keyboard_modifiers.shift_key();
        let assets = Arc::clone(&self.assets);
        let actions = {
            let lobby = self
                .network_lobby
                .as_mut()
                .expect("joined lobby was checked above");
            match state {
                ElementState::Pressed => lobby
                    .with_classic_controller_input(
                        self.graphics.surface(),
                        assets.as_ref(),
                        &self.scenario_game_options,
                        |controller, layout, roster| {
                            controller.key_down(gui_key, shift, layout, roster, Instant::now())
                        },
                    )
                    .map_err(Self::joined_lobby_input_error)?,
                ElementState::Released => {
                    lobby.sync_classic_controller();
                    lobby.controller.key_up(gui_key)
                }
            }
        };
        self.process_joined_lobby_controller_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn handle_network_lobby_chat_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.startup_view != StartupView::NetworkLobby
            || self.classic_host_lobby_active()
            || self.network_lobby.is_none()
        {
            return Ok(false);
        }
        let chat_focused = self.network_lobby.as_mut().is_some_and(|lobby| {
            lobby.sync_classic_controller();
            lobby.controller.focus() == LobbyControl::ChatInput
        });
        if !chat_focused {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let modifiers = LobbyChatKeyModifiers {
            shift: c4_modifiers.contains(ModifiersState::SHIFT),
            control: c4_modifiers.contains(ModifiersState::CONTROL),
        };
        let clipboard = if c4_modifiers == ModifiersState::CONTROL {
            match key {
                VirtualKeyCode::KeyC => Some(LobbyChatClipboardShortcut::Copy),
                VirtualKeyCode::KeyX => Some(LobbyChatClipboardShortcut::Cut),
                VirtualKeyCode::KeyV => Some(LobbyChatClipboardShortcut::Paste),
                VirtualKeyCode::KeyA => Some(LobbyChatClipboardShortcut::SelectAll),
                _ => None,
            }
        } else {
            None
        };
        let edit = if c4_modifiers.contains(ModifiersState::ALT) {
            None
        } else {
            match key {
                VirtualKeyCode::ArrowLeft => Some(LobbyChatEditKey::Left),
                VirtualKeyCode::ArrowRight => Some(LobbyChatEditKey::Right),
                VirtualKeyCode::Home => Some(LobbyChatEditKey::Home),
                VirtualKeyCode::End => Some(LobbyChatEditKey::End),
                VirtualKeyCode::Backspace => Some(LobbyChatEditKey::Backspace),
                VirtualKeyCode::Delete => Some(LobbyChatEditKey::Delete),
                _ => None,
            }
        };
        let plain_command = c4_modifiers.is_empty()
            && matches!(
                key,
                VirtualKeyCode::Enter
                    | VirtualKeyCode::NumpadEnter
                    | VirtualKeyCode::ArrowUp
                    | VirtualKeyCode::ArrowDown
            );
        let recognized = clipboard.is_some() || edit.is_some() || plain_command;
        if !recognized {
            return Ok(matches!(
                key,
                VirtualKeyCode::Enter
                    | VirtualKeyCode::NumpadEnter
                    | VirtualKeyCode::ArrowUp
                    | VirtualKeyCode::ArrowDown
                    | VirtualKeyCode::ArrowLeft
                    | VirtualKeyCode::ArrowRight
                    | VirtualKeyCode::Home
                    | VirtualKeyCode::End
                    | VirtualKeyCode::Backspace
                    | VirtualKeyCode::Delete
            ));
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        if let Some(shortcut) = clipboard {
            if shortcut == LobbyChatClipboardShortcut::Paste {
                self.chat_paste_consumed_keys.insert(key);
            }
            self.process_classic_lobby_chat_request(LobbyChatRequest::Clipboard { shortcut })?;
            return Ok(true);
        }
        if let Some(edit) = edit {
            self.process_classic_lobby_chat_request(LobbyChatRequest::EditKey {
                key: edit,
                modifiers,
            })?;
            return Ok(true);
        }
        match key {
            VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter => {
                let text = self
                    .active_lobby_chat_view()
                    .map(|view| view.text)
                    .unwrap_or_default();
                self.process_classic_lobby_chat_request(LobbyChatRequest::Submit(text))?;
            }
            VirtualKeyCode::ArrowUp | VirtualKeyCode::ArrowDown => {
                self.process_classic_lobby_chat_request(LobbyChatRequest::History {
                    older: key == VirtualKeyCode::ArrowUp,
                })?;
            }
            _ => {}
        }
        Ok(true)
    }

    pub(crate) fn process_lobby_action(&mut self, action: LobbyAction) -> Result<(), EngineError> {
        match action {
            LobbyAction::ExitRequested => {
                self.exit_startup_lobby_to_main();
                return Ok(());
            }
            LobbyAction::ToggleReady => {
                if !self.admission_resources.lobby_ready_available() {
                    return Ok(());
                }
                if let Some((changed_client_id, ready)) =
                    self.network_lobby.as_mut().and_then(|lobby| {
                        let client_id = lobby.local_client_id;
                        lobby
                            .participants
                            .contains_key(&client_id)
                            .then(|| (client_id, lobby.toggle_local_ready()))
                    })
                {
                    self.submit_joined_lobby_ready_state(changed_client_id, ready)?;
                }
            }
            LobbyAction::SelectSheet(sheet) => {
                let selected = self.network_lobby.as_mut().is_some_and(|lobby| {
                    let supported = matches!(
                        sheet,
                        LobbySheet::Players
                            | LobbySheet::Resources
                            | LobbySheet::Options
                            | LobbySheet::Scenario
                    ) || sheet == LobbySheet::Teams && lobby.has_teams;
                    if supported {
                        lobby.active_sheet = sheet;
                        lobby.last_roster_click = None;
                        lobby.controller.set_active_sheet(sheet);
                        if sheet == LobbySheet::Resources {
                            lobby
                                .controller
                                .set_resource_rows(lobby.resource_rows.values().cloned().collect());
                        }
                    }
                    supported
                });
                if !selected {
                    return Err(classic_game_lobby_child_error(
                        ClassicGameLobbyChild::Sheet(sheet),
                    ));
                }
                if sheet == LobbySheet::Options {
                    // C4GameOptionsList::Activate forces one Update before the
                    // Sec1 timer takes over (src/C4GameOptions.cpp:302-308).
                    let _ = self.refresh_classic_lobby_options(true);
                }
                if sheet == LobbySheet::Scenario {
                    let _ = self.refresh_lobby_scenario_description();
                }
                if sheet.is_roster() {
                    self.sync_classic_lobby_roster();
                }
            }
            LobbyAction::StartGame => self.start_network_lobby_countdown()?,
            LobbyAction::SaveResource(resource_id) => {
                self.request_lobby_resource_save(resource_id, false)?;
            }
            LobbyAction::Preload => self.request_lobby_preload(),
            LobbyAction::OpenExternalIrcChat => self.show_external_irc_dialog()?,
            LobbyAction::SubmitMessage(text) => {
                if let Some(lobby) = self.network_lobby.as_mut() {
                    lobby.chat_history_index = -1;
                    lobby_chat_clear_preserving_scroll(&mut lobby.chat_edit);
                    lobby.controller.set_chat_edit_view(lobby.chat_edit.clone());
                }
                if text.is_empty() {
                    self.play_ui_sound("Error");
                    return Ok(());
                }
                self.store_message_input_history(&text);
                if self.process_classic_lobby_command(&text)? {
                    return Ok(());
                }
                if self.process_control_message_local_command(&text) {
                    return Ok(());
                }
                if is_team_message_syntax(&text) && self.engine.team_distribution() == 4 {
                    self.append_control_message_log(
                        "Can't send team message: Teams not known.".to_string(),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    return Ok(());
                }
                let control = match parse_lobby_message_control(&text) {
                    Ok(control) => control,
                    Err(error) => {
                        tracing::warn!(%error, "classic lobby chat command is not implemented");
                        self.append_unknown_lobby_command(&text);
                        return Ok(());
                    }
                };
                if let Some(control) = control {
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_message(control))
                    {
                        tracing::error!(%error, "failed to submit classic lobby message");
                    }
                }
            }
            LobbyAction::ChatEdited => {}
        }
        Ok(())
    }

    /// Publishes an already-applied local ready value the way
    /// `MainDlg::OnReadyCheck` broadcasts `PID_ReadyCheck` and updates the
    /// local client (src/C4GameLobby.cpp:329-344).
    fn submit_joined_lobby_ready_state(
        &mut self,
        changed_client_id: ClientId,
        ready: bool,
    ) -> Result<(), EngineError> {
        let data = if ready {
            clonk_network::ReadyCheckData::Ready
        } else {
            clonk_network::ReadyCheckData::NotReady
        };
        if i32::try_from(changed_client_id)
            .ok()
            .is_some_and(|client_id| self.control_clients.set_lobby_ready(client_id, ready))
        {
            self.publish_updated_host_join_snapshot();
        }
        self.sync_classic_lobby_roster();
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_ready_check(data))
        {
            tracing::error!(%error, "failed to submit lobby ready state");
        }
        // MainDlg::OnReadyCheck broadcasts, mutates the local core and
        // refreshes its row without presenting a status overlay
        // (src/C4GameLobby.cpp:329-344).
        self.on_lobby_client_ready_state_change(changed_client_id)
    }

    pub(crate) fn classic_host_lobby_layouts(
        &mut self,
    ) -> std::result::Result<(LobbyLayout, LobbyRosterLayout), EngineError> {
        let fonts = self.assets.clonk_fonts.clone().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                    detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
                }),
            ))
        })?;
        let surface = self.graphics.surface();
        let (width, height) = (surface.width() as i32, surface.height() as i32);
        let state = self.classic_host_lobby.as_mut().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Model {
                    detail: "exact host lobby state is absent".to_string(),
                }),
            ))
        })?;
        let layout = state.controller.layout(width, height, &fonts);
        let _ = state.controller.chat_scroll_metrics(&layout, &fonts.text);
        let roster = state.controller.right_list_layout(&layout, &fonts);
        Ok((layout, roster))
    }

    pub(crate) fn joined_lobby_layouts(
        &mut self,
    ) -> std::result::Result<(LobbyLayout, LobbyRosterLayout), EngineError> {
        let fonts = self.assets.clonk_fonts.clone().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                    detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
                }),
            ))
        })?;
        let surface = self.graphics.surface();
        let (width, height) = (surface.width() as i32, surface.height() as i32);
        let state = self.network_lobby.as_mut().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Model {
                    detail: "exact joined lobby state is absent".to_string(),
                }),
            ))
        })?;
        let (layout, roster, _) = state.synchronize_classic_controller(
            width,
            height,
            &fonts,
            &self.scenario_game_options,
        );
        Ok((layout, roster))
    }

    fn visible_classic_lobby_layouts(
        &mut self,
    ) -> std::result::Result<(LobbyLayout, LobbyRosterLayout), EngineError> {
        if self.classic_host_lobby.is_some() {
            self.classic_host_lobby_layouts()
        } else {
            self.joined_lobby_layouts()
        }
    }

    fn play_lobby_sound_events(&mut self, sounds: Vec<LobbySound>) {
        for sound in sounds {
            match sound {
                LobbySound::StartElevatorLoop => {
                    if let Some(audio) = self.audio.as_ref() {
                        let mut audio = audio.borrow_mut();
                        audio.start_lobby_elevator(&self.snapshot);
                    }
                }
                LobbySound::StopElevatorLoop => {
                    if let Some(audio) = self.audio.as_ref() {
                        let mut audio = audio.borrow_mut();
                        audio.stop_lobby_elevator();
                    }
                }
                LobbySound::ArrowHit => self.play_ui_sound("ArrowHit"),
                LobbySound::Click => self.play_ui_sound("Click"),
                LobbySound::Command => self.play_ui_sound("Command"),
                LobbySound::CountdownCommand => self.play_global_sound_effect("Command"),
                LobbySound::Fuse => self.play_global_sound_effect("Fuse"),
                LobbySound::Pshshsh => self.play_global_sound_effect("Pshshsh"),
                LobbySound::Blast3 => self.play_global_sound_effect("Blast3"),
            }
        }
    }

    fn play_classic_lobby_sounds(&mut self) {
        let sounds = self
            .visible_classic_lobby_controller_mut()
            .map(ClassicGameLobby::take_sounds)
            .unwrap_or_default();
        self.play_lobby_sound_events(sounds);
        let option_sounds = self.scenario_game_options.take_sound_events();
        self.play_game_option_sound_events(option_sounds);
    }

    pub(crate) fn cancel_classic_lobby_interaction(&mut self) -> bool {
        if self.mode != AppMode::Menu || self.startup_view != StartupView::NetworkLobby {
            return false;
        }
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.pointer = None;
            lobby.last_roster_click = None;
            lobby.controller.cancel_interaction();
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.pointer = None;
            lobby.last_roster_click = None;
            lobby.sync_classic_controller();
            lobby.controller.cancel_interaction();
        } else {
            return false;
        }
        self.lobby_chat_drag_anchor = None;
        self.scenario_game_options.cancel_interaction();
        self.play_classic_lobby_sounds();
        true
    }

    pub(crate) fn classic_lobby_pointer_left(&mut self) -> bool {
        if self.mode != AppMode::Menu || self.startup_view != StartupView::NetworkLobby {
            return false;
        }
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.pointer = None;
            lobby.last_roster_click = None;
            lobby.controller.pointer_left();
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.pointer_left();
        } else {
            return false;
        }
        self.lobby_chat_drag_anchor = None;
        self.menu_state.set_pointer_position(None);
        self.scenario_game_options.pointer_left();
        self.play_classic_lobby_sounds();
        true
    }

    fn route_classic_lobby_game_option_input(
        &mut self,
        input: LobbyGameOptionInput,
    ) -> Result<Vec<ClassicLobbyAction>, EngineError> {
        self.route_lobby_game_option_input(input, false)
    }

    /// Routes a strip input of the reconstructed joined lobby, using its
    /// retained classic controller for the enclosing-dialog focus
    /// bookkeeping.
    fn route_joined_lobby_game_option_input(
        &mut self,
        input: LobbyGameOptionInput,
    ) -> Result<Vec<ClassicLobbyAction>, EngineError> {
        self.route_lobby_game_option_input(input, true)
    }

    /// The `ClassicGameLobby` that encloses the retained option strip: the
    /// exact host lobby's, or the joined adapter's when `joined`.
    fn lobby_game_option_enclosing_controller(
        &mut self,
        joined: bool,
    ) -> Option<&mut ClassicGameLobby> {
        if joined {
            self.network_lobby
                .as_mut()
                .map(|lobby| &mut lobby.controller)
        } else {
            self.classic_host_lobby
                .as_mut()
                .map(|state| &mut state.controller)
        }
    }

    fn route_lobby_game_option_input(
        &mut self,
        input: LobbyGameOptionInput,
        joined: bool,
    ) -> Result<Vec<ClassicLobbyAction>, EngineError> {
        let previous_focus = self.scenario_game_options.focused_button();
        let mut unhandled = false;
        let option_actions = match input {
            LobbyGameOptionInput::PointerMove(point) => {
                self.scenario_game_options.handle_pointer_move(point)
            }
            LobbyGameOptionInput::PointerDown(point) => {
                self.scenario_game_options.handle_pointer_down(point)
            }
            LobbyGameOptionInput::PointerUp(point) => {
                self.scenario_game_options.handle_pointer_up(point)
            }
            LobbyGameOptionInput::MouseLeave => {
                self.scenario_game_options.pointer_left();
                Vec::new()
            }
            LobbyGameOptionInput::TouchCancel => {
                self.scenario_game_options.handle_touch_cancel();
                Vec::new()
            }
            LobbyGameOptionInput::Focus(button) => {
                self.scenario_game_options.set_focused_button(Some(button));
                Vec::new()
            }
            LobbyGameOptionInput::ClearFocus => {
                self.scenario_game_options.set_focused_button(None);
                Vec::new()
            }
            LobbyGameOptionInput::KeyDown { key, shift } => {
                let outcome = self
                    .scenario_game_options
                    .handle_key_down_with_tab_direction(key, shift);
                unhandled = !outcome.captured;
                outcome.actions
            }
            LobbyGameOptionInput::KeyUp(key) => {
                // Dialog::KeyFocusDefault binds only key-down events
                // (C4GuiDialogs.cpp:380-383): an unmatched release never
                // refocuses the default control.
                self.scenario_game_options.handle_key_up(key).actions
            }
            LobbyGameOptionInput::Hotkey(hotkey) => {
                self.scenario_game_options.handle_hotkey(hotkey)
            }
            LobbyGameOptionInput::GamepadLowDown => {
                let outcome = self.scenario_game_options.handle_gamepad_low_down();
                unhandled = !outcome.captured;
                outcome.actions
            }
            LobbyGameOptionInput::GamepadLowUp => {
                // Releases share the key-down-only KeyFocusDefault rule.
                self.scenario_game_options.handle_gamepad_low_up().actions
            }
            LobbyGameOptionInput::GamepadDirection {
                horizontal,
                vertical,
            } => {
                let direction = if horizontal < 0 {
                    GameOptionGamepadDirection::Left
                } else if horizontal > 0 {
                    GameOptionGamepadDirection::Right
                } else if vertical < 0 {
                    GameOptionGamepadDirection::Up
                } else {
                    GameOptionGamepadDirection::Down
                };
                let outcome = self
                    .scenario_game_options
                    .handle_gamepad_direction(direction);
                unhandled = !outcome.captured;
                outcome.actions
            }
        };

        let mut lobby_actions = Vec::new();
        for action in option_actions {
            match action {
                GameOptionAction::FocusTraversalRequested { backwards } => {
                    lobby_actions.extend(
                        self.lobby_game_option_enclosing_controller(joined)
                            .map(|controller| {
                                controller.game_option_focus_traversal_requested(backwards)
                            })
                            .unwrap_or_default(),
                    );
                }
                action => self.process_lobby_game_option_action(action)?,
            }
        }
        let focused = self.scenario_game_options.focused_button();
        if focused != previous_focus {
            if let Some(button) = focused {
                let actions = self
                    .lobby_game_option_enclosing_controller(joined)
                    .ok_or_else(|| {
                        classic_game_lobby_child_error(ClassicGameLobbyChild::GameOptionSideEffect(
                            "recursive focus",
                        ))
                    })?
                    .game_option_focus_changed(button)
                    .map_err(|error| {
                        classic_game_lobby_child_error(ClassicGameLobbyChild::GameOptionSideEffect(
                            if error.to_string().is_empty() {
                                "recursive focus"
                            } else {
                                "invalid recursive focus"
                            },
                        ))
                    })?;
                lobby_actions.extend(actions);
            }
        }
        if unhandled {
            lobby_actions.extend(
                self.lobby_game_option_enclosing_controller(joined)
                    .map(|controller| controller.game_option_input_unhandled())
                    .unwrap_or_default(),
            );
        }
        Ok(lobby_actions)
    }

    pub(crate) fn process_classic_lobby_actions(
        &mut self,
        actions: Vec<ClassicLobbyAction>,
    ) -> Result<(), EngineError> {
        self.close_stale_classic_lobby_team_combo();
        self.guard_classic_global_gui_bootstrap()?;
        if actions
            .iter()
            .any(|action| matches!(action, ClassicLobbyAction::StartRequested { .. }))
        {
            if let Some(overrides) = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| &staged.pending_global_gui_failures)
            {
                self.assets
                    .require_classic_global_gui_bootstrap_resources(overrides)
                    .map_err(report_classic_parity_boundary)
                    .map_err(classic_parity_engine_error)?;
            }
            self.assets
                .network_start_wait_resources()
                .map_err(|error| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                            detail: format!("network start-wait dialog is unavailable: {error}"),
                        }),
                    ))
                })?;
        }
        let mut pending: VecDeque<ClassicLobbyAction> = actions.into();
        while let Some(action) = pending.pop_front() {
            match action {
                ClassicLobbyAction::FocusChanged(control) => {
                    self.set_active_lobby_chat_focus(control == LobbyControl::ChatInput);
                    let focused = match control {
                        LobbyControl::GameOption(button) => Some(button),
                        _ => None,
                    };
                    self.scenario_game_options.set_focused_button(focused);
                }
                ClassicLobbyAction::RosterSelectionChanged(_) => {}
                ClassicLobbyAction::SheetRequested(sheet) => {
                    if !self.select_classic_lobby_sheet(sheet) {
                        return Err(classic_game_lobby_child_error(
                            ClassicGameLobbyChild::Sheet(sheet),
                        ));
                    }
                }
                ClassicLobbyAction::GameOptions(input) => {
                    pending.extend(self.route_classic_lobby_game_option_input(input)?);
                }
                ClassicLobbyAction::ExitRequested => {
                    // C4GUI::Button raises its click sound before invoking
                    // MainDlg::OnExitBtn. Drain the controller queue while
                    // the lobby still exists so pointer/key activation keeps
                    // that ordering; Escape and Alt+X enqueue no click.
                    self.play_classic_lobby_sounds();
                    self.exit_startup_lobby_to_main();
                    return Ok(());
                }
                ClassicLobbyAction::StartRequested {
                    countdown_seconds,
                    check_league_rules,
                    confirm_unassociated_savegame_players,
                } => {
                    // C4GUI::Button queues Click before MainDlg::OnRunBtn;
                    // preserve that ordering before Start tears the lobby down.
                    self.play_classic_lobby_sounds();
                    self.request_classic_lobby_start(
                        countdown_seconds,
                        check_league_rules,
                        confirm_unassociated_savegame_players,
                    )?;
                    if !self.classic_host_lobby_active() {
                        return Ok(());
                    }
                }
                ClassicLobbyAction::AbortCountdownRequested => {
                    self.abort_network_lobby_countdown();
                }
                ClassicLobbyAction::PreloadRequested => self.request_lobby_preload(),
                ClassicLobbyAction::ReadyChanged(ready) => {
                    self.apply_classic_lobby_ready_change(ready)?;
                }
                ClassicLobbyAction::TabContextRequested { position } => {
                    self.open_lobby_tab_context(position)?;
                }
                ClassicLobbyAction::RosterContextRequested { row, position } => {
                    self.open_classic_lobby_roster_context(row, position)?;
                }
                ClassicLobbyAction::AddPlayerRequested { client_id } => {
                    self.open_classic_lobby_player_selector(client_id)?;
                }
                ClassicLobbyAction::AddScriptPlayerRequested => {
                    self.add_classic_lobby_script_player();
                }
                ClassicLobbyAction::TeamSelectionRequested { player_id } => {
                    self.open_classic_lobby_team_combo(player_id)?;
                }
                ClassicLobbyAction::MoveLocalPlayersIntoTeamRequested { team_id } => {
                    self.move_local_classic_lobby_players_into_team(team_id);
                }
                ClassicLobbyAction::OptionSelectionRequested {
                    option,
                    anchor,
                    minimum_width,
                } => {
                    self.open_classic_lobby_option_combo(option, anchor, minimum_width)?;
                }
                ClassicLobbyAction::SaveResourceRequested { resource_id } => {
                    self.request_lobby_resource_save(resource_id, false)?;
                }
                ClassicLobbyAction::Chat(request) => {
                    self.process_classic_lobby_chat_request(request)?;
                }
                ClassicLobbyAction::CountdownChanged(state) => {
                    self.scenario_game_options.set_countdown(state.is_locked());
                }
                ClassicLobbyAction::NotifyUserIfInactive => {
                    self.request_control_message_attention();
                }
                ClassicLobbyAction::AppendLog(_) => {
                    // The frontend owns and has already appended this line.
                }
            }
        }
        self.play_classic_lobby_sounds();
        Ok(())
    }

    fn request_classic_lobby_start(
        &mut self,
        countdown_seconds: i32,
        check_league_rules: bool,
        confirm_unassociated_savegame_players: bool,
    ) -> Result<(), EngineError> {
        if check_league_rules && !self.check_classic_lobby_league_rules_start()? {
            return Ok(());
        }
        let has_unassociated_savegame_players = self
            .classic_lobby_authoritative_has_unassociated_savegame_players()
            .unwrap_or(confirm_unassociated_savegame_players);
        if has_unassociated_savegame_players && !self.startup_message_hidden("HideMsgPlrNoTakeOver")
        {
            let message = self.runtime_resource_text(
                "IDS_MSG_NOTALLSAVEGAMEPLAYERSHAVE",
                "Not all savegame players have been associated with a local player!|Any unassociated savegame players will be removed from the game. Unassociated local players will join as new players.|Start anyway?",
            );
            let caption =
                self.runtime_resource_text("IDS_MSG_FREESAVEGAMEPLRS", "Player assignment");
            let checkbox = self.runtime_resource_text(
                "IDS_MSG_DONTSHOW",
                "&Don't display this message in the future.",
            );
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::new(
                    message,
                    caption,
                    clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                    clonk_frontend::message_dialog::MessageDialogIcon::Standard(12),
                    clonk_frontend::message_dialog::MessageDialogSize::Regular,
                    false,
                )
                .with_checkbox(checkbox, false),
                MessageDialogContinuation::ClassicLobbyStart { countdown_seconds },
            )?;
            return Ok(());
        }
        self.start_network_lobby_countdown_with(countdown_seconds)
    }

    fn check_classic_lobby_league_rules_start(&mut self) -> Result<bool, EngineError> {
        if !self.network_is_league {
            return Ok(true);
        }
        let teams_custom = self
            .network_team_assignment
            .as_ref()
            .map(NetworkTeamAssignmentState::teams)
            .map(|teams| teams.custom)
            .ok_or_else(|| {
                classic_game_lobby_model_engine_error(
                    "league start validation has no retained pregame team list",
                )
            })?;
        let melee = if teams_custom {
            false
        } else {
            let scenario = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| &staged.scenario)
                .ok_or_else(|| {
                    classic_game_lobby_model_engine_error(
                        "league start validation has no retained staged scenario",
                    )
                })?;
            let metadata = scenario
                .initial_network_scenario_metadata()
                .map_err(|error| {
                    classic_game_lobby_model_engine_error(format!(
                        "league start validation cannot read scenario goals: {error}"
                    ))
                })?;
            ["MELE", "MEL2"].into_iter().any(|wanted| {
                metadata
                    .goals
                    .iter()
                    .find(|entry| entry.id == wanted)
                    .is_some_and(|entry| entry.count != 0)
            })
        };
        let fallback = "Players %s and %s would be playing against each other in split-screen. This is disallowed in league games!";
        let template = self.runtime_resource_text("IDS_MSG_NOSPLITSCREENINLEAGUE", fallback);
        let template_bytes =
            self.runtime_resource_bytes_with_fallback("IDS_MSG_NOSPLITSCREENINLEAGUE", fallback);
        let caption = self.runtime_resource_text("IDS_NET_ERR_LEAGUE", "League error");
        let (_, clients) = self.control_player_infos.retained_rows_snapshot();
        let mut removals = Vec::new();
        let mut blocking_reason = None;
        for (client_id, _, players) in clients {
            let mut users = players
                .iter()
                .filter(|player| player.player_type == clonk_engine::PLAYER_INFO_TYPE_USER);
            let Some(first) = users.next() else {
                continue;
            };
            for player in users {
                if !((!teams_custom && melee) || player.team != first.team) {
                    continue;
                }
                let first_name = legacy_presentation_text(first.name.as_bytes());
                let second_name = legacy_presentation_text(player.name.as_bytes());
                let mut pieces = template.splitn(3, "%s");
                let prefix = pieces.next().unwrap_or_default();
                let middle = pieces.next().unwrap_or_default();
                let suffix = pieces.next().unwrap_or_default();
                let reason = format!("{prefix}{first_name}{middle}{second_name}{suffix}");
                let reason_bytes = format_two_legacy_string_arguments(
                    &template_bytes,
                    first.name.as_bytes(),
                    player.name.as_bytes(),
                )
                .ok_or_else(|| {
                    classic_game_lobby_model_engine_error(
                        "league split-screen template has fewer than two %s arguments",
                    )
                })?;
                let reason_wire = LegacyCString::from_bytes(reason_bytes).ok_or_else(|| {
                    classic_game_lobby_model_engine_error(
                        "league split-screen reason contains an embedded NUL",
                    )
                })?;
                let known_nonhost = client_id != 0 && self.control_clients.contains(client_id);
                if known_nonhost {
                    removals.push(clonk_engine::ClientRemoveControlData {
                        client_id,
                        reason: reason_wire,
                        by_client: 0,
                    });
                } else {
                    blocking_reason = Some(reason);
                }
            }
        }
        for remove in removals {
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_client_remove(remove))
            {
                tracing::error!(%error, "failed to remove split-screen league client");
            }
        }
        let Some(reason) = blocking_reason else {
            return Ok(true);
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                reason,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(46),
            ),
            MessageDialogContinuation::None,
        )?;
        Ok(false)
    }

    fn apply_classic_lobby_ready_change(&mut self, ready: bool) -> Result<(), EngineError> {
        let Some(local_client_id) = self.network.as_ref().map(NetworkManager::local_client_id)
        else {
            self.status_text = "Network lobby ready state is unavailable".to_string();
            return Ok(());
        };
        let Ok(local_client_id_i32) = i32::try_from(local_client_id) else {
            self.status_text = "Local client ID exceeds the ready-check wire field".to_string();
            return Ok(());
        };
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_ready(ready);
        }
        if self
            .control_clients
            .set_lobby_ready(local_client_id_i32, ready)
        {
            self.publish_updated_host_join_snapshot();
        }
        if let Some(Err(error)) = self.network.as_ref().map(|network| {
            network.submit_ready_check(if ready {
                clonk_network::ReadyCheckData::Ready
            } else {
                clonk_network::ReadyCheckData::NotReady
            })
        }) {
            tracing::error!(%error, "failed to submit classic lobby ready state");
        }
        self.sync_classic_lobby_roster();
        self.on_lobby_client_ready_state_change(local_client_id)?;
        Ok(())
    }

    pub(crate) fn select_network_lobby_scenario(&mut self, identifier: &str, title: &str) -> bool {
        let Some(current_identifier) = self
            .network_lobby
            .as_ref()
            .map(|lobby| lobby.selected_identifier().map(str::to_owned))
        else {
            return false;
        };
        let changed = current_identifier.as_deref() != Some(identifier);
        if changed {
            self.clear_lobby_preload();
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.preload.reset_for_context();
            }
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.select_scenario(identifier, title);
            self.scenario_label = lobby.scenario_label();
        }
        if changed {
            self.sync_classic_lobby_resource_ready();
        }
        true
    }

    pub(crate) fn sync_classic_lobby_resource_ready(&mut self) {
        let ready = self.admission_resources.lobby_ready_available();
        let context_ready = self.staged_network_host_scenario.is_some()
            || self.pending_network_join_data.is_some()
            || self.catalog_host_preload_scenario().is_some();
        let mut automatic_preload = false;
        let actions = if let Some(lobby) = self.classic_host_lobby.as_mut() {
            let actions = lobby.controller.set_resources_loaded(ready);
            automatic_preload = lobby.preload.synchronize(ready, context_ready);
            lobby.controller.set_preload_button_state(
                lobby.preload.manual_button_present,
                lobby.preload.eligible,
            );
            actions
        } else {
            Vec::new()
        };
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.resources_loaded = ready;
            automatic_preload |= lobby.preload.synchronize(ready, context_ready);
        }
        for action in actions {
            if let ClassicLobbyAction::FocusChanged(control) = action {
                self.scenario_game_options
                    .set_focused_button(match control {
                        LobbyControl::GameOption(button) => Some(button),
                        _ => None,
                    });
            }
        }
        if automatic_preload {
            self.request_lobby_preload();
        }
    }

    fn active_lobby_preload_state(&self) -> Option<&LobbyPreloadState> {
        self.classic_host_lobby
            .as_ref()
            .map(|lobby| &lobby.preload)
            .or_else(|| self.network_lobby.as_ref().map(|lobby| &lobby.preload))
    }

    fn record_lobby_preload_result(&mut self, succeeded: bool) {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.preload.record_result(succeeded);
            lobby.controller.set_preload_button_state(
                lobby.preload.manual_button_present,
                lobby.preload.eligible,
            );
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.preload.record_result(succeeded);
        }
    }

    pub(crate) fn request_lobby_preload(&mut self) {
        let launched = (|| {
            if !self
                .active_lobby_preload_state()
                .is_some_and(|preload| preload.eligible)
            {
                return Err("Game.CanPreload() is false".to_string());
            }
            if self.lobby_preload_task.is_some() || self.lobby_preload_artifact.is_some() {
                return Err("a lobby preload has already been launched".to_string());
            }
            let job = self.prepare_lobby_preload_job()?;
            let (sender, receiver) = mpsc::channel();
            let worker = thread::Builder::new()
                .name("LobbyPreload".to_string())
                .spawn(move || {
                    let result = Self::run_lobby_preload_job(job);
                    if let Err(error) = sender.send(result) {
                        if let Ok(artifact) = error.0 {
                            Self::discard_lobby_preload_artifact(artifact);
                        }
                    }
                })
                .map_err(|error| format!("failed to launch lobby preload worker: {error}"))?;
            self.lobby_preload_task = Some(LobbyPreloadTask {
                state: LobbyPreloadTaskState::Loading(receiver),
                start_host_when_ready: false,
                worker: LobbyPreloadWorker::new(worker),
            });
            Ok(())
        })();
        match launched {
            Ok(()) => {
                // Native treats successful thread creation as Preload success;
                // the manual button disappears before the worker completes.
                self.record_lobby_preload_result(true);
            }
            Err(error) => {
                tracing::info!(%error, "lobby preload failed");
                self.record_lobby_preload_result(false);
                let message = self.runtime_resource_text("IDS_ERR_PRELOADING", "Preloading error.");
                self.append_control_message_log(message, 0x00ff_1f1f, None);
            }
        }
    }

    fn prepare_lobby_preload_job(&self) -> Result<LobbyPreloadJob, String> {
        let graphics = LobbyPreloadGraphicsContext {
            app_paths: self.app_paths.clone(),
            fallback: self.startup_game_graphics_resources(),
            liquid_animation_enabled: self.assets.liquid_animation_enabled(),
        };
        if let Some(staged) = self.staged_network_host_scenario.as_ref() {
            let frontend = staged.frontend.clone();
            let scenario_path = frontend
                .path
                .clone()
                .ok_or_else(|| "staged host scenario has no filesystem path".to_string())?;
            let definition_paths = staged
                .scenario
                .definition_resource_paths()
                .iter()
                .map(|path| path_as_legacy_text(path))
                .collect::<Vec<_>>();
            return Ok(LobbyPreloadJob {
                graphics,
                source: LobbyPreloadJobSource::Host {
                    frontend,
                    scenario_path,
                    definition_paths,
                },
            });
        }

        if let Some(frontend) = self.catalog_host_preload_scenario().cloned() {
            let key = self
                .catalog_host_preload_key()
                .ok_or_else(|| "selected host scenario has no preload key".to_string())?;
            return Ok(LobbyPreloadJob {
                graphics,
                source: LobbyPreloadJobSource::CatalogHost { frontend, key },
            });
        }

        let join_data = self
            .pending_network_join_data
            .clone()
            .ok_or_else(|| "client JoinData is unavailable".to_string())?;
        let (resource_directory, maker) = match self.network_mode.as_ref() {
            Some(NetworkMode::Client(settings)) => (
                settings.resource_directory.clone(),
                settings.group_maker.clone(),
            ),
            _ => return Err("lobby preload has no staged host or client scenario".to_string()),
        };
        let filename = format!("Combined{}.c4s", join_data.client_id);
        let scenario_path = self
            .client_combined_scenario_path
            .clone()
            .unwrap_or_else(|| resource_directory.join(&filename));
        let (scenario_resources, staging_path) = if self.client_combined_scenario_path.is_some() {
            (None, None)
        } else {
            let resources = resolve_client_scenario_resources(&join_data, |core| {
                self.admission_resources
                    .complete_path(core.id)
                    .map(Path::to_path_buf)
            })
            .map_err(|error| error.to_string())?;
            let serial = LOBBY_PRELOAD_SERIAL.fetch_add(1, AtomicOrdering::Relaxed);
            let staging_path = resource_directory.join(format!(
                ".{filename}.preload-{}-{serial}.tmp",
                std::process::id()
            ));
            (Some(resources), Some(staging_path))
        };
        let game_resources = resolve_client_game_resources(&join_data, |core| {
            self.admission_resources
                .complete_path(core.id)
                .map(Path::to_path_buf)
        })
        .map_err(|error| error.to_string())?;
        Ok(LobbyPreloadJob {
            graphics,
            source: LobbyPreloadJobSource::Client {
                join_data,
                scenario_resources,
                game_resources,
                resource_directory,
                maker,
                scenario_path,
                staging_path,
            },
        })
    }

    pub(crate) fn run_lobby_preload_job(
        job: LobbyPreloadJob,
    ) -> std::result::Result<LobbyPreloadArtifact, String> {
        let LobbyPreloadJob { graphics, source } = job;
        match source {
            LobbyPreloadJobSource::Host {
                frontend,
                scenario_path,
                definition_paths,
            } => {
                let definition_load = ScenarioDefinitionLoad::Fixed {
                    modules: definition_paths.clone(),
                    definition_root: None,
                };
                let game_graphics = load_game_graphics_resources(
                    graphics.app_paths.as_ref(),
                    graphics.fallback,
                    graphics.liquid_animation_enabled,
                    &frontend,
                    Some(&definition_load),
                )
                .map_err(|error| format!("failed to preload host graphics: {error:#}"))?;
                Ok(LobbyPreloadArtifact {
                    scenario_path: scenario_path.clone(),
                    definition_paths,
                    game_graphics,
                    material_texture_images: Arc::new(load_scenario_material_textures_with_paths(
                        &scenario_path,
                        None,
                        graphics.app_paths.as_ref(),
                    )),
                    material_render_info: Arc::new(load_material_render_info_with_paths(
                        &scenario_path,
                        None,
                        graphics.app_paths.as_ref(),
                    )),
                    catalog_host: None,
                    client: None,
                })
            }
            LobbyPreloadJobSource::CatalogHost { frontend, key } => {
                let resolver =
                    InstallDefinitionResolver::new(graphics.app_paths.clone().map(Arc::new));
                let scenario = load_scenario_with_definition_load(
                    &key.scenario_path,
                    &resolver,
                    &key.languages,
                    &key.definition_load,
                )
                .map_err(|error| format!("failed to preload host scenario: {error}"))?;
                let definition_paths = scenario
                    .definition_resource_paths()
                    .iter()
                    .map(|path| path_as_legacy_text(path))
                    .collect::<Vec<_>>();
                let effective_definition_load = ScenarioDefinitionLoad::Fixed {
                    modules: definition_paths.clone(),
                    definition_root: None,
                };
                let game_graphics = load_game_graphics_resources(
                    graphics.app_paths.as_ref(),
                    graphics.fallback,
                    graphics.liquid_animation_enabled,
                    &frontend,
                    Some(&effective_definition_load),
                )
                .map_err(|error| format!("failed to preload host graphics: {error:#}"))?;
                Ok(LobbyPreloadArtifact {
                    scenario_path: key.scenario_path.clone(),
                    definition_paths,
                    game_graphics,
                    material_texture_images: Arc::new(load_scenario_material_textures_with_paths(
                        &key.scenario_path,
                        None,
                        graphics.app_paths.as_ref(),
                    )),
                    material_render_info: Arc::new(load_material_render_info_with_paths(
                        &key.scenario_path,
                        None,
                        graphics.app_paths.as_ref(),
                    )),
                    catalog_host: Some(CatalogHostLobbyPreloadArtifact {
                        key,
                        scenario: Some(scenario),
                    }),
                    client: None,
                })
            }
            LobbyPreloadJobSource::Client {
                join_data,
                scenario_resources,
                game_resources,
                resource_directory,
                maker,
                scenario_path,
                staging_path,
            } => {
                let working_path = staging_path.as_ref().unwrap_or(&scenario_path).clone();
                let result = (|| {
                    if let Some(resources) = scenario_resources.as_ref() {
                        let filename = scenario_path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| format!("Combined{}.c4s", join_data.client_id));
                        let packed = compose_client_network_scenario_with_maker_bytes(
                            resources,
                            &filename,
                            maker.as_bytes(),
                        )
                        .map_err(|error| error.to_string())?;
                        fs::create_dir_all(&resource_directory).map_err(|error| {
                            format!("failed to create {}: {error}", resource_directory.display())
                        })?;
                        fs::write(&working_path, packed).map_err(|error| {
                            format!("failed to write {}: {error}", working_path.display())
                        })?;
                    }

                    let definition_paths = game_resources
                        .iter()
                        .filter(|resource| {
                            resource.core.resource_type
                                == clonk_network::HostResourceType::Definitions as u8
                        })
                        .map(|resource| path_as_legacy_text(&resource.path))
                        .collect::<Vec<_>>();
                    let mut definition_groups = Vec::new();
                    let mut material_groups = Vec::new();
                    for resource in &game_resources {
                        let target = match resource.core.resource_type {
                            value
                                if value == clonk_network::HostResourceType::Definitions as u8 =>
                            {
                                &mut definition_groups
                            }
                            value if value == clonk_network::HostResourceType::Material as u8 => {
                                &mut material_groups
                            }
                            _ => continue,
                        };
                        target.push(Group::open(&resource.path).map_err(|error| {
                            format!(
                                "failed to open synchronized game resource {} at {}: {error}",
                                resource.core.id,
                                resource.path.display()
                            )
                        })?);
                    }
                    let scenario_group = Group::open(&working_path).map_err(|error| {
                        format!(
                            "failed to open combined scenario {}: {error}",
                            working_path.display()
                        )
                    })?;
                    let resolver_paths = graphics.app_paths.clone().map(Arc::new);
                    let graphics_groups = InstallDefinitionResolver::new(resolver_paths.clone())
                        .resolve_graphics_groups_with_definition_roots(
                            &scenario_group,
                            &definition_groups,
                        )
                        .map_err(|error| {
                            format!("failed to resolve client graphics resources: {error}")
                        })?;
                    let languages = startup_language_sequence(resolver_paths.as_deref());
                    let language_packs = resolver_paths
                        .as_deref()
                        .map(classic_language_packs)
                        .unwrap_or_default();
                    let random_seed = u64::from(join_data.parameters.random_seed as u32);
                    let scenario =
                        Scenario::load_network_from_path_with_languages_and_seed_and_packs(
                            &working_path,
                            &definition_groups,
                            &material_groups,
                            &graphics_groups,
                            &languages,
                            random_seed,
                            &language_packs,
                        )
                        .map_err(|error| error.to_string())?;
                    validate_client_network_scenario(&scenario)?;

                    let title = legacy_presentation_text(join_data.parameters.title.as_bytes());
                    let frontend = FrontendScenario {
                        identifier: working_path
                            .file_name()
                            .map(|name| name.to_string_lossy().into_owned())
                            .unwrap_or_else(|| format!("Combined{}.c4s", join_data.client_id)),
                        title: if title.is_empty() {
                            "Network game".to_string()
                        } else {
                            title
                        },
                        description: None,
                        kind: ScenarioKind::Scenario,
                        is_editable: false,
                        is_playable: true,
                        mission_access: None,
                        path: Some(working_path.clone()),
                        source_paths: vec![working_path.clone()],
                        root_label: None,
                        preview: None,
                        title_picture: None,
                        children: Vec::new(),
                        folder_index: None,
                        icon_index: None,
                        difficulty: None,
                        author: None,
                        version: None,
                        local_only: None,
                        allow_user_change: None,
                        definition_modules: Vec::new(),
                    };
                    let definition_load = ScenarioDefinitionLoad::Fixed {
                        modules: definition_paths.clone(),
                        definition_root: None,
                    };
                    let game_graphics = load_game_graphics_resources(
                        graphics.app_paths.as_ref(),
                        graphics.fallback,
                        graphics.liquid_animation_enabled,
                        &frontend,
                        Some(&definition_load),
                    )
                    .map_err(|error| format!("failed to preload client graphics: {error:#}"))?;
                    Ok(LobbyPreloadArtifact {
                        scenario_path: scenario_path.clone(),
                        definition_paths,
                        game_graphics,
                        material_texture_images: Arc::new(
                            load_scenario_material_textures_with_paths(
                                &working_path,
                                Some(&material_groups),
                                graphics.app_paths.as_ref(),
                            ),
                        ),
                        material_render_info: Arc::new(load_material_render_info_with_paths(
                            &working_path,
                            Some(&material_groups),
                            graphics.app_paths.as_ref(),
                        )),
                        catalog_host: None,
                        client: Some(ClientLobbyPreloadArtifact {
                            client_id: join_data.client_id,
                            dynamic_resource_id: join_data.dynamic.id,
                            random_seed,
                            scenario: Some(scenario),
                            material_groups,
                            staging_path: staging_path.clone(),
                        }),
                    })
                })();
                if result.is_err() {
                    if let Some(path) = staging_path {
                        let _ = fs::remove_file(path);
                    }
                }
                result
            }
        }
    }

    fn discard_lobby_preload_artifact(mut artifact: LobbyPreloadArtifact) {
        if let Some(client) = artifact.client.as_mut() {
            if let Some(path) = client.staging_path.take() {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn join_lobby_preload_worker(worker: &mut LobbyPreloadWorker) {
        worker.join();
    }

    pub(crate) fn clear_lobby_preload(&mut self) {
        if let Some(mut task) = self.lobby_preload_task.take() {
            Self::join_lobby_preload_worker(&mut task.worker);
            if let LobbyPreloadTaskState::RemovingClientResource { artifact, .. } = task.state {
                Self::discard_lobby_preload_artifact(artifact);
            }
        }
        if let Some(artifact) = self.lobby_preload_artifact.take() {
            Self::discard_lobby_preload_artifact(artifact);
        }
        self.clear_client_preload_projection();
    }

    fn install_lobby_preload_artifact(
        &mut self,
        mut artifact: LobbyPreloadArtifact,
    ) -> std::result::Result<(), String> {
        if let Some(client) = artifact.client.as_mut() {
            let current = self
                .pending_network_join_data
                .as_ref()
                .is_some_and(|join_data| {
                    join_data.client_id == client.client_id
                        && join_data.dynamic.id == client.dynamic_resource_id
                        && u64::from(join_data.parameters.random_seed as u32) == client.random_seed
                });
            if !current {
                if self.client_combined_scenario_path.as_ref() == Some(&artifact.scenario_path) {
                    self.clear_client_preload_projection();
                }
                Self::discard_lobby_preload_artifact(artifact);
                return Err("client preload completed for stale JoinData".to_string());
            }
            self.client_combined_scenario_path = Some(artifact.scenario_path.clone());
            self.network_material_resource_groups =
                Some(std::mem::take(&mut client.material_groups));
        } else if let Some(catalog_host) = artifact.catalog_host.as_ref() {
            let current = self.catalog_host_preload_key().as_ref() == Some(&catalog_host.key);
            if !current {
                Self::discard_lobby_preload_artifact(artifact);
                return Err("host preload completed for a stale catalog scenario".to_string());
            }
        } else {
            let current = self
                .staged_network_host_scenario
                .as_ref()
                .is_some_and(|staged| {
                    let definition_paths = staged
                        .scenario
                        .definition_resource_paths()
                        .iter()
                        .map(|path| path_as_legacy_text(path))
                        .collect::<Vec<_>>();
                    staged.frontend.path.as_ref() == Some(&artifact.scenario_path)
                        && definition_paths == artifact.definition_paths
                });
            if !current {
                Self::discard_lobby_preload_artifact(artifact);
                return Err("host preload completed for a stale scenario".to_string());
            }
        }
        self.lobby_preload_artifact = Some(artifact);
        Ok(())
    }

    pub(crate) fn poll_lobby_preload(&mut self) -> Result<(), EngineError> {
        let Some(task) = self.lobby_preload_task.take() else {
            return Ok(());
        };
        let start_host_when_ready = task.start_host_when_ready;
        let mut worker = task.worker;
        let mut finished = false;
        match task.state {
            LobbyPreloadTaskState::Loading(receiver) => match receiver.try_recv() {
                Ok(Ok(mut artifact)) => {
                    Self::join_lobby_preload_worker(&mut worker);
                    let staging_path = artifact
                        .client
                        .as_mut()
                        .and_then(|client| client.staging_path.take());
                    if let Some(staging_path) = staging_path {
                        let current = {
                            let client = artifact.client.as_ref().expect("client artifact");
                            self.pending_network_join_data
                                .as_ref()
                                .is_some_and(|join_data| {
                                    join_data.client_id == client.client_id
                                        && join_data.dynamic.id == client.dynamic_resource_id
                                        && u64::from(join_data.parameters.random_seed as u32)
                                            == client.random_seed
                                })
                        };
                        let mut committed = false;
                        let commit = if !current {
                            Err("client preload completed for stale JoinData".to_string())
                        } else {
                            self.client_combined_preload_file.clear();
                            if artifact.scenario_path.exists() {
                                let _ = fs::remove_file(&artifact.scenario_path);
                            }
                            fs::rename(&staging_path, &artifact.scenario_path)
                                .map(|()| {
                                    committed = true;
                                    self.client_combined_scenario_path =
                                        Some(artifact.scenario_path.clone());
                                    self.client_combined_preload_file
                                        .replace(artifact.scenario_path.clone());
                                })
                                .map_err(|error| {
                                    format!(
                                        "failed to commit {} to {}: {error}",
                                        staging_path.display(),
                                        artifact.scenario_path.display()
                                    )
                                })
                        };
                        match commit.and_then(|()| {
                            let resource_id = artifact
                                .client
                                .as_ref()
                                .expect("client artifact")
                                .dynamic_resource_id;
                            self.network
                                .as_ref()
                                .ok_or_else(|| {
                                    "client network disappeared during preload commit".to_string()
                                })?
                                .remove_client_resource_async(resource_id)
                                .map_err(|error| error.to_string())
                        }) {
                            Ok(receiver) => {
                                self.lobby_preload_task = Some(LobbyPreloadTask {
                                    state: LobbyPreloadTaskState::RemovingClientResource {
                                        artifact,
                                        receiver,
                                    },
                                    start_host_when_ready,
                                    worker,
                                });
                            }
                            Err(error) => {
                                tracing::error!(%error, "lobby preload client commit failed");
                                let _ = fs::remove_file(staging_path);
                                if committed {
                                    self.clear_client_preload_projection();
                                }
                                Self::discard_lobby_preload_artifact(artifact);
                                finished = true;
                            }
                        }
                    } else {
                        if let Err(error) = self.install_lobby_preload_artifact(artifact) {
                            tracing::error!(%error, "discarding stale lobby preload");
                        }
                        finished = true;
                    }
                }
                Ok(Err(error)) => {
                    Self::join_lobby_preload_worker(&mut worker);
                    tracing::error!(%error, "lobby preload worker failed");
                    finished = true;
                }
                Err(TryRecvError::Empty) => {
                    self.lobby_preload_task = Some(LobbyPreloadTask {
                        state: LobbyPreloadTaskState::Loading(receiver),
                        start_host_when_ready,
                        worker,
                    });
                }
                Err(TryRecvError::Disconnected) => {
                    Self::join_lobby_preload_worker(&mut worker);
                    tracing::error!("lobby preload worker disconnected");
                    finished = true;
                }
            },
            LobbyPreloadTaskState::RemovingClientResource { artifact, receiver } => match receiver
                .try_recv()
            {
                Ok(Ok(())) => {
                    if let Err(error) = self.install_lobby_preload_artifact(artifact) {
                        tracing::error!(%error, "discarding stale lobby preload");
                    }
                    finished = true;
                }
                Ok(Err(error)) => {
                    tracing::error!(%error, "failed to retire preloaded dynamic resource");
                    self.clear_client_preload_projection();
                    Self::discard_lobby_preload_artifact(artifact);
                    finished = true;
                }
                Err(TryRecvError::Empty) => {
                    self.lobby_preload_task = Some(LobbyPreloadTask {
                        state: LobbyPreloadTaskState::RemovingClientResource { artifact, receiver },
                        start_host_when_ready,
                        worker,
                    });
                }
                Err(TryRecvError::Disconnected) => {
                    tracing::error!("client resource-removal worker disconnected");
                    self.clear_client_preload_projection();
                    Self::discard_lobby_preload_artifact(artifact);
                    finished = true;
                }
            },
        }
        if finished {
            if start_host_when_ready {
                self.start_network_game_now()?;
            } else if self.pending_client_start_status.is_some() {
                self.prepare_client_network_scenario_if_ready()?;
            }
        }
        Ok(())
    }

    fn classic_lobby_authoritative_has_unassociated_savegame_players(&self) -> Option<bool> {
        let save_game = self
            .staged_network_host_scenario
            .as_ref()
            .and_then(|staged| staged.scenario.lobby_metadata())
            .map(|metadata| metadata.head().is_save_game());
        if save_game == Some(false) {
            return Some(false);
        }
        let restore_snapshot = self.host_join_snapshot.as_ref().or_else(|| {
            self.network_mode.as_ref().and_then(|mode| match mode {
                NetworkMode::Host(HostSettings {
                    prepared: Some(prepared),
                    ..
                }) => prepared.host_config().initial_join_snapshot.as_ref(),
                NetworkMode::Host(_) | NetworkMode::Client(_) => None,
            })
        })?;
        let restore_infos = host_restore_player_info_entries(Some(restore_snapshot));
        (!restore_infos.is_empty()).then(|| {
            self.control_player_infos
                .has_unassociated_restore_info(&restore_infos)
        })
    }

    fn classic_lobby_has_unassociated_savegame_players(&self) -> bool {
        if let Some(has_unassociated) =
            self.classic_lobby_authoritative_has_unassociated_savegame_players()
        {
            return has_unassociated;
        }

        // A synthetic/fallback lobby has no retained restore packet. Keep the
        // visible-header projection as its best available approximation.
        self.classic_host_lobby.as_ref().is_some_and(|lobby| {
            lobby.controller.rows().iter().any(|row| {
                matches!(
                    row,
                    LobbyRosterRow::Header(clonk_frontend::game_lobby::LobbyHeaderRow {
                        kind: clonk_frontend::game_lobby::LobbyRosterHeader::UnassignedSavegamePlayers,
                        ..
                    })
                )
            })
        })
    }

    pub(crate) fn append_lobby_command_error(&mut self, message: String) {
        self.play_global_sound_effect("Error");
        self.append_lobby_command_log(message);
    }

    fn append_lobby_command_log(&mut self, message: String) {
        let color = readable_lobby_rgba(0x00ff_1f1f);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.push_log(LobbyLogLine {
                text: message.clone(),
                color,
            });
        } else if self.startup_view == StartupView::NetworkLobby {
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.push_log(LobbyLogLine {
                    text: message,
                    color,
                });
            }
        }
    }

    fn append_unknown_lobby_command(&mut self, text: &str) {
        let raw = clonk_script::c4_string_bytes(text);
        let name = raw
            .get(1..)
            .unwrap_or_default()
            .split(|byte| *byte == b' ')
            .next()
            .unwrap_or_default();
        let name = legacy_presentation_text(&name[..name.len().min(30)]);
        let template = self.classic_lobby_resource_text(
            "IDS_ERR_UNKNOWNCMD",
            "Unknown command: \"%s\" - type /help to get a list of valid commands",
        );
        self.append_lobby_command_error(format_resource_string(template, &[&name]));
    }

    pub(crate) fn clear_lobby_log(&mut self) {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_logs(Vec::new());
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.logs.clear();
            lobby.controller.set_logs(Vec::new());
        }
    }

    fn append_lobby_command_help(&mut self) {
        let header = self.classic_lobby_resource_text(
            "IDS_TEXT_COMMANDSAVAILABLEDURINGLO",
            "Commands available during lobby:",
        );
        self.append_control_message_log(header, CONTROL_LOG_COLOR, None);
        for line in [
            "/start [time]",
            "/abort",
            "/alert",
            "/joinplr [filename]",
            "/kick [client]",
            "/observer [client]",
            "/me [action]",
            "/sound [sound]",
            "/mute [client]",
            "/unmute [client]",
            "/team [message]",
            "/plrclr [player] [RGB]",
            "/plrclr [RGB]",
            "/set comment [comment]",
            "/set password [password]",
            "/set faircrew [on/off]",
            "/set maxplayer [number]",
            "/clear",
            "/readycheck",
        ] {
            self.append_control_message_log(line.to_string(), CONTROL_LOG_COLOR, None);
        }
    }

    fn classic_lobby_resource_text(&self, key: &str, fallback: &str) -> String {
        self.runtime_resource_text(key, fallback)
    }

    fn process_classic_lobby_command(&mut self, text: &str) -> Result<bool, EngineError> {
        let raw = clonk_script::c4_string_bytes(text);
        if raw.first() != Some(&b'/') {
            return Ok(false);
        }
        let (command, parameter) = raw
            .iter()
            .position(|byte| *byte == b' ')
            .map_or((raw.as_slice(), &[][..]), |space| {
                (&raw[..space], &raw[space + 1..])
            });
        let host = matches!(self.runtime_network_role(), RuntimeNetworkRole::Host);
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        if command.eq_ignore_ascii_case(b"/joinplr") {
            let filename = clonk_script::c4_string_from_bytes(parameter);
            let config = self
                .app_paths
                .as_ref()
                .and_then(|paths| Config::load(paths.config_file()).ok())
                .unwrap_or_default();
            let player_path = startup_player_path(&config);
            let wire_path = player_path.join(&filename);
            let source_path = self
                .app_paths
                .as_ref()
                .map(|paths| startup_player_search_paths(paths, &config))
                .unwrap_or_else(|| vec![player_path])
                .into_iter()
                .map(|root| root.join(&filename))
                .find(|path| path.exists());
            let Some(source_path) = source_path else {
                let displayed_path = wire_path.to_string_lossy();
                let template = self.classic_lobby_resource_text(
                    "IDS_MSG_CMD_JOINPLR_NOFILE",
                    "Cannot join player %s: File not found!",
                );
                self.append_lobby_command_error(format_resource_string(
                    template,
                    &[&displayed_path],
                ));
                return Ok(true);
            };
            let wire_path = wire_path.to_string_lossy().into_owned();
            if let Err(error) = self.submit_lobby_network_player(source_path, &wire_path) {
                self.append_lobby_command_error(error);
            }
            return Ok(true);
        }
        if command.eq_ignore_ascii_case(b"/plrclr") {
            let named_player = parameter
                .iter()
                .position(|byte| *byte == b' ')
                .filter(|position| *position > 0);
            let target = if let Some(separator) = named_player {
                let pattern = &parameter[..separator];
                self.control_player_infos
                    .retained_rows_snapshot()
                    .1
                    .into_iter()
                    .flat_map(|(client_id, _, players)| {
                        players.into_iter().map(move |player| (client_id, player))
                    })
                    .filter(|(_, player)| player.id > 0)
                    .filter(|(_, player)| {
                        classic_raw_wildcard_match(pattern, control_player_effective_name(player))
                    })
                    .min_by_key(|(_, player)| player.id)
                    .map(|(client_id, player)| (client_id, player.id))
            } else {
                self.control_player_infos
                    .retained_rows_snapshot()
                    .1
                    .into_iter()
                    .find(|(client_id, _, _)| *client_id == local_client_id)
                    .and_then(|(client_id, _, players)| {
                        players.first().map(|player| (client_id, player.id))
                    })
            };
            let Some((client_id, player_id)) = target else {
                let message = self.classic_lobby_resource_text(
                    "IDS_MSG_CMD_PLRCLR_NOPLAYER",
                    "Player not found!",
                );
                self.append_lobby_command_error(message);
                return Ok(true);
            };
            if client_id != local_client_id && !host {
                let message = self
                    .classic_lobby_resource_text("IDS_MSG_CMD_PLRCLR_NOACCESS", "Access denied");
                self.append_lobby_command_error(message);
                return Ok(true);
            }
            let color_parameter = named_player
                .map(|separator| &parameter[separator + 1..])
                .unwrap_or(parameter);
            let Some(mut color) = legacy_sscanf_hex_prefix(color_parameter) else {
                let message = self.classic_lobby_resource_text(
                    "IDS_MSG_CMD_PLRCLR_USAGE",
                    "Usage: /plrclr [Johnny] ff0000",
                );
                self.append_lobby_command_error(message);
                return Ok(true);
            };
            color &= 0x00ff_ffff;
            if color == 0 {
                color = 1;
            }
            if let Some(mut update) = self.control_player_infos.client_update_request(client_id) {
                if let Some(player) = update
                    .players
                    .iter_mut()
                    .find(|player| player.id == player_id)
                {
                    player.original_color = color;
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_player_info_update(update))
                    {
                        tracing::error!(%error, "failed to submit lobby player-color update");
                    }
                }
            }
            return Ok(true);
        }
        if command.eq_ignore_ascii_case(b"/start") {
            if !host {
                let message =
                    self.classic_lobby_resource_text("IDS_MSG_CMD_HOSTONLY", "Host only!");
                self.append_lobby_command_error(message);
                return Ok(true);
            }
            let configured = self
                .staged_network_host_scenario
                .as_ref()
                .map(|staged| staged.lobby.countdown_seconds)
                .unwrap_or(DEFAULT_LOBBY_COUNTDOWN_SECONDS);
            let requested = if parameter.is_empty() {
                configured
            } else if let Some(seconds) =
                legacy_sscanf_decimal_prefix(parameter).filter(|seconds| *seconds >= 0)
            {
                seconds
            } else {
                let message = self.classic_lobby_resource_text(
                    "IDS_MSG_CMD_START_USAGE",
                    "Usage: /start [timer]",
                );
                self.append_lobby_command_error(message);
                return Ok(true);
            };
            let countdown_seconds = if requested < 0 {
                DEFAULT_LOBBY_COUNTDOWN_SECONDS
            } else if self.network_is_league {
                requested.max(5)
            } else {
                requested
            };
            self.abort_network_lobby_countdown();
            self.request_classic_lobby_start(
                countdown_seconds,
                true,
                self.classic_lobby_has_unassociated_savegame_players(),
            )?;
            return Ok(true);
        }
        if command.eq_ignore_ascii_case(b"/abort") {
            if !host {
                let message =
                    self.classic_lobby_resource_text("IDS_MSG_CMD_HOSTONLY", "Host only!");
                self.append_lobby_command_error(message);
            } else if !self.abort_network_lobby_countdown() {
                let message = self.classic_lobby_resource_text(
                    "IDS_MSG_CMD_ABORT_NOCOUNTDOWN",
                    "Not in countdown!",
                );
                self.append_lobby_command_error(message);
            }
            return Ok(true);
        }
        if command.eq_ignore_ascii_case(b"/readycheck") {
            if !host {
                let message =
                    self.classic_lobby_resource_text("IDS_MSG_CMD_HOSTONLY", "Host only!");
                self.append_lobby_command_error(message);
            } else if !self.request_lobby_ready_check_at(Instant::now())? {
                let message = std::mem::take(&mut self.status_text);
                self.append_lobby_command_error(message);
            }
            return Ok(true);
        }
        if command.eq_ignore_ascii_case(b"/help") {
            self.append_lobby_command_help();
            return Ok(true);
        }
        if command == b"/clear" {
            self.clear_lobby_log();
            return Ok(true);
        }
        if command == b"/kick" {
            if host {
                let target = self
                    .control_clients
                    .snapshot()
                    .into_iter()
                    .find(|client| client.name.as_bytes() == parameter);
                let Some(target) = target else {
                    let name = legacy_presentation_text(parameter);
                    let template = self.classic_lobby_resource_text(
                        "IDS_MSG_CMD_NOCLIENT",
                        "Client %s not found!",
                    );
                    self.append_control_message_log(
                        format_resource_string(template, &[&name]),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    return Ok(true);
                };
                let league_vote = self.network_is_league
                    && self
                        .engine
                        .players()
                        .any(|player| player.at_client().get() == target.client_id);
                if league_vote {
                    self.submit_own_league_vote(
                        LeagueVoteSubject {
                            vote_type: clonk_engine::VOTE_TYPE_KICK,
                            data: target.client_id,
                        },
                        true,
                    );
                } else {
                    let reason = self.classic_lobby_resource_text(
                        "IDS_MSG_KICKFROMMSGBOARD",
                        "kicked from messageboard",
                    );
                    if let Some(Err(error)) = self.network.as_ref().map(|network| {
                        network.submit_client_remove(clonk_engine::ClientRemoveControlData {
                            client_id: target.client_id,
                            reason: clonk_engine::LegacyCString::from_bytes(
                                clonk_script::c4_string_bytes(&reason),
                            )
                            .unwrap_or_default(),
                            by_client: 0,
                        })
                    }) {
                        tracing::error!(%error, "failed to submit lobby kick command");
                    }
                }
            }
            return Ok(true);
        }
        if command == b"/observer" {
            if !host {
                let message =
                    self.classic_lobby_resource_text("IDS_MSG_CMD_HOSTONLY", "Host only!");
                self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
                return Ok(true);
            }
            let target = self
                .control_clients
                .snapshot()
                .into_iter()
                .find(|client| client.name.as_bytes() == parameter);
            let Some(target) = target else {
                let name = legacy_presentation_text(parameter);
                let template = self
                    .classic_lobby_resource_text("IDS_MSG_CMD_NOCLIENT", "Client %s not found!");
                self.append_control_message_log(
                    format_resource_string(template, &[&name]),
                    CONTROL_LOG_COLOR,
                    None,
                );
                return Ok(true);
            };
            if self.network_is_league {
                let message = self.classic_lobby_resource_text(
                    "IDS_LOG_COMMANDNOTALLOWEDINLEAGUE",
                    "Command not allowed in league games!",
                );
                self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
            } else if let Some(Err(error)) = self.network.as_ref().map(|network| {
                network.submit_client_update(clonk_engine::ClientUpdateControlData::new(
                    clonk_engine::CLIENT_UPDATE_SET_OBSERVER,
                    target.client_id,
                    0,
                    0,
                ))
            }) {
                tracing::error!(%error, "failed to submit lobby observer command");
            }
            return Ok(true);
        }
        if command == b"/set" {
            if host && parameter.starts_with(b"maxplayer ") {
                let value = &parameter[b"maxplayer ".len()..];
                let maximum = legacy_sscanf_decimal_prefix(value).unwrap_or(0);
                if maximum == 0 && value != b"0" {
                    self.append_control_message_log(
                        "Syntax: /set maxplayer count".to_string(),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                } else if let Some(Err(error)) = self.network.as_ref().map(|network| {
                    network.submit_control_set(clonk_network::LegacyControlSet {
                        value_type: 2,
                        data: maximum,
                        by_client: local_client_id,
                    })
                }) {
                    tracing::error!(%error, "failed to submit lobby maximum-player update");
                }
            }
            if host && self.network.is_some() {
                if parameter == b"comment" || parameter.starts_with(b"comment ") {
                    let value = parameter.strip_prefix(b"comment ").unwrap_or_default();
                    let value = &value[..value
                        .len()
                        .min(clonk_frontend::game_option_buttons::COMMENT_MAX_TEXT)];
                    self.finish_game_option_input(vec![GameOptionAction::CommentChanged(
                        clonk_script::c4_string_from_bytes(value),
                    )])?;
                    return Ok(true);
                }
                if parameter == b"password" || parameter.starts_with(b"password ") {
                    let value = parameter.strip_prefix(b"password ").unwrap_or_default();
                    self.finish_game_option_input(vec![GameOptionAction::PasswordChanged {
                        password: clonk_script::c4_string_from_bytes(value),
                        remember_for_next_round: None,
                    }])?;
                    return Ok(true);
                }
            }
            if host && !self.network_is_league {
                if let Some(value) = parameter.strip_prefix(b"faircrew ") {
                    let value = if value == b"on" {
                        Some(configured_fair_crew_strength(&load_native_config_bytes(
                            self.app_paths.as_ref(),
                        )))
                    } else if value == b"off" {
                        Some(-1)
                    } else if value.first().is_some_and(u8::is_ascii_digit) {
                        Some(legacy_sscanf_decimal_prefix(value).unwrap_or(0))
                    } else {
                        None
                    };
                    if let Some(value) = value {
                        self.process_lobby_game_option_action(
                            GameOptionAction::SendLobbyFairCrewControl { value },
                        )?;
                    }
                }
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn active_lobby_chat_view(&self) -> Option<LobbyChatEditView> {
        self.classic_host_lobby
            .as_ref()
            .map(|lobby| lobby.controller.chat_edit_view().clone())
            .or_else(|| {
                (self.startup_view == StartupView::NetworkLobby)
                    .then(|| {
                        self.network_lobby
                            .as_ref()
                            .map(|lobby| lobby.chat_edit.clone())
                    })
                    .flatten()
            })
    }

    pub(crate) fn install_active_lobby_chat_view(&mut self, view: LobbyChatEditView) {
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.set_chat_edit_view(view);
        } else if self.startup_view == StartupView::NetworkLobby {
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.chat_edit = view.clone();
                lobby.controller.set_chat_edit_view(view);
            }
        }
    }

    fn set_active_lobby_chat_focus(&mut self, focused: bool) {
        let Some(mut view) = self.active_lobby_chat_view() else {
            return;
        };
        if self.lobby_chat_drag_anchor.is_some() {
            self.lobby_chat_drag_anchor = Some(0);
        }
        if focused {
            view.caret = view.text.len();
            view.selection = (!view.text.is_empty()).then_some((0, view.caret));
            view.cursor_visible = true;
        } else {
            view.selection = None;
        }
        self.install_active_lobby_chat_view(view);
    }

    fn active_lobby_chat_layout(&mut self) -> Result<LobbyLayout, EngineError> {
        if self.classic_host_lobby_active() {
            return self.classic_host_lobby_layouts().map(|(layout, _)| layout);
        }
        let assets = Arc::clone(&self.assets);
        let surface = self.graphics.surface();
        let lobby = self.network_lobby.as_mut().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Model {
                    detail: "joined lobby state is absent".to_string(),
                }),
            ))
        })?;
        lobby
            .with_classic_controller_input(
                surface,
                assets.as_ref(),
                &self.scenario_game_options,
                |_, layout, _| layout.clone(),
            )
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                        detail: error.to_string(),
                    }),
                ))
            })
    }

    fn scroll_active_lobby_chat_caret_in_view(
        &mut self,
        view: &mut LobbyChatEditView,
    ) -> Result<(), EngineError> {
        let (layout, fonts) = self.active_lobby_chat_scroll_context()?;
        lobby_chat_scroll_caret_in_view(view, &layout, &fonts.text);
        Ok(())
    }

    fn active_lobby_chat_scroll_context(
        &mut self,
    ) -> Result<(LobbyLayout, Arc<clonk_frontend::ClonkFontSet>), EngineError> {
        let layout = self.active_lobby_chat_layout()?;
        let fonts = self.assets.clonk_fonts.clone().ok_or_else(|| {
            classic_parity_engine_error(report_classic_parity_boundary(
                ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                    detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
                }),
            ))
        })?;
        Ok((layout, fonts))
    }

    fn apply_active_lobby_chat_pointer_selection(
        &mut self,
        point: GuiPoint,
        begin: bool,
        release: bool,
    ) -> Result<(), EngineError> {
        let retained_anchor = if begin {
            None
        } else {
            self.lobby_chat_drag_anchor
        };
        let (layout, fonts) = self.active_lobby_chat_scroll_context()?;
        let mut view = self.active_lobby_chat_view().unwrap_or_default();
        let (position, anchor) = lobby_chat_apply_pointer_selection(
            &mut view,
            point,
            &layout,
            &fonts.text,
            begin,
            retained_anchor,
        );
        if begin {
            self.lobby_chat_drag_anchor = Some(position);
        } else if release {
            self.lobby_chat_drag_anchor = None;
        } else {
            self.lobby_chat_drag_anchor = Some(anchor);
        }
        self.install_active_lobby_chat_view(view);
        Ok(())
    }

    pub(crate) fn paste_classic_lobby_chat_text(&mut self, text: &str) -> Result<(), EngineError> {
        let Some(mut view) = self.active_lobby_chat_view() else {
            return Ok(());
        };
        let (layout, fonts) = self.active_lobby_chat_scroll_context()?;
        if self.lobby_chat_drag_anchor.is_some() && lobby_chat_paste_attempts_insertion(text) {
            if let Some((anchor, caret)) = view.selection {
                self.lobby_chat_drag_anchor = Some(anchor.min(caret));
            }
        }
        let host = self.classic_host_lobby_active();
        let joined_client = self
            .network_lobby
            .as_ref()
            .map(|lobby| lobby.local_client_id);
        let result = lobby_chat_paste_text(
            &mut view,
            text,
            LobbyChatPasteMode::Lobby,
            |view| lobby_chat_scroll_caret_in_view(view, &layout, &fonts.text),
            |submission| {
                self.process_classic_lobby_chat_request(LobbyChatRequest::Submit(submission))?;
                Ok(if host {
                    self.classic_host_lobby_active()
                } else {
                    self.startup_view == StartupView::NetworkLobby
                        && self
                            .network_lobby
                            .as_ref()
                            .map(|lobby| lobby.local_client_id)
                            == joined_client
                })
            },
        );
        if result
            .as_ref()
            .is_ok_and(|outcome| outcome.completed_lines > 0)
            && self.lobby_chat_drag_anchor.is_some()
        {
            self.lobby_chat_drag_anchor = Some(0);
        }
        let still_active = if host {
            self.classic_host_lobby_active()
        } else {
            self.startup_view == StartupView::NetworkLobby
                && self
                    .network_lobby
                    .as_ref()
                    .map(|lobby| lobby.local_client_id)
                    == joined_client
        };
        if still_active {
            self.install_active_lobby_chat_view(view);
        }
        result.map(|_| ())
    }

    pub(crate) fn process_classic_lobby_chat_request(
        &mut self,
        request: LobbyChatRequest,
    ) -> Result<(), EngineError> {
        match request {
            LobbyChatRequest::FocusInput => {
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = Some(0);
                }
                view.caret = view.text.len();
                view.selection = (!view.text.is_empty()).then_some((0, view.caret));
                view.cursor_visible = true;
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::InsertText(text) => {
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                if self.lobby_chat_drag_anchor.is_some() {
                    if let Some((anchor, caret)) = view.selection {
                        self.lobby_chat_drag_anchor = Some(anchor.min(caret));
                    }
                }
                if lobby_chat_insert_text(&mut view, &text) {
                    self.scroll_active_lobby_chat_caret_in_view(&mut view)?;
                }
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::RefocusAndInsert(text) => {
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = Some(0);
                }
                view.caret = view.text.len();
                view.selection = (!view.text.is_empty()).then_some((0, view.caret));
                if lobby_chat_insert_text(&mut view, &text) {
                    self.scroll_active_lobby_chat_caret_in_view(&mut view)?;
                }
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::EditKey { key, modifiers } => {
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                let old_caret = view.caret;
                let old_selection = view.selection;
                if lobby_chat_apply_edit_key(&mut view, key, modifiers) {
                    self.scroll_active_lobby_chat_caret_in_view(&mut view)?;
                }
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = match (old_selection, key, modifiers.shift) {
                        (
                            Some((anchor, caret)),
                            LobbyChatEditKey::Backspace | LobbyChatEditKey::Delete,
                            _,
                        ) => Some(anchor.min(caret)),
                        (Some(_), _, false) => Some(0),
                        (Some((anchor, _)), _, true) => Some(anchor),
                        (None, _, true) if view.caret != old_caret => Some(old_caret),
                        _ => self.lobby_chat_drag_anchor,
                    };
                }
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::Clipboard { shortcut } => {
                if shortcut == LobbyChatClipboardShortcut::Paste {
                    if let Ok(text) =
                        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text())
                    {
                        self.paste_classic_lobby_chat_text(&text)?;
                    }
                    return Ok(());
                }
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                match shortcut {
                    LobbyChatClipboardShortcut::Copy | LobbyChatClipboardShortcut::Cut => {
                        if let Some(range) = lobby_chat_selection(&view) {
                            let selection_start = range.start;
                            let selected = view.text[range].to_string();
                            let copied = arboard::Clipboard::new()
                                .and_then(|mut clipboard| clipboard.set_text(selected))
                                .is_ok();
                            if copied && shortcut == LobbyChatClipboardShortcut::Cut {
                                if self.lobby_chat_drag_anchor.is_some() {
                                    self.lobby_chat_drag_anchor = Some(selection_start);
                                }
                                lobby_chat_delete_selection(&mut view);
                            }
                        }
                    }
                    LobbyChatClipboardShortcut::Paste => unreachable!("paste handled above"),
                    LobbyChatClipboardShortcut::SelectAll => {
                        if self.lobby_chat_drag_anchor.is_some() {
                            self.lobby_chat_drag_anchor = Some(0);
                        }
                        view.caret = view.text.len();
                        view.selection = (!view.text.is_empty()).then_some((0, view.caret));
                    }
                }
                view.cursor_visible = true;
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::ContextCommand(command) => {
                let shortcut = match command {
                    LobbyChatContextCommand::Cut => Some(LobbyChatClipboardShortcut::Cut),
                    LobbyChatContextCommand::Copy => Some(LobbyChatClipboardShortcut::Copy),
                    LobbyChatContextCommand::Paste => Some(LobbyChatClipboardShortcut::Paste),
                    LobbyChatContextCommand::SelectAll => {
                        Some(LobbyChatClipboardShortcut::SelectAll)
                    }
                    LobbyChatContextCommand::Clear => None,
                };
                if let Some(shortcut) = shortcut {
                    return self.process_classic_lobby_chat_request(LobbyChatRequest::Clipboard {
                        shortcut,
                    });
                }
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                if self.lobby_chat_drag_anchor.is_some() {
                    if let Some(range) = lobby_chat_selection(&view) {
                        self.lobby_chat_drag_anchor = Some(range.start);
                    }
                }
                lobby_chat_delete_selection(&mut view);
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::Submit(text) => {
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = Some(0);
                }
                if !self.classic_host_lobby_active()
                    && self.startup_view == StartupView::NetworkLobby
                    && self.network_lobby.is_some()
                {
                    return self.process_lobby_action(LobbyAction::SubmitMessage(text));
                }
                if let Some(lobby) = self.classic_host_lobby.as_mut() {
                    lobby.chat_history_index = -1;
                    let mut view = lobby.controller.chat_edit_view().clone();
                    lobby_chat_clear_preserving_scroll(&mut view);
                    lobby.controller.set_chat_edit_view(view);
                }
                if text.is_empty() {
                    // C4GameLobby::MainDlg::OnChatInput uses GUISound here;
                    // unlike OnError, this remains subject to FESamples.
                    self.play_ui_sound("Error");
                    return Ok(());
                }
                self.store_message_input_history(&text);
                if self.process_classic_lobby_command(&text)? {
                    return Ok(());
                }
                if self.process_control_message_local_command(&text) {
                    return Ok(());
                }
                if is_team_message_syntax(&text) && self.engine.team_distribution() == 4 {
                    self.append_control_message_log(
                        "Can't send team message: Teams not known.".to_string(),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    return Ok(());
                }
                let control = match parse_lobby_message_control(&text) {
                    Ok(control) => control,
                    Err(error) => {
                        tracing::warn!(%error, "classic lobby chat command is not implemented");
                        self.append_unknown_lobby_command(&text);
                        return Ok(());
                    }
                };
                if let Some(control) = control {
                    if let Some(Err(error)) = self
                        .network
                        .as_ref()
                        .map(|network| network.submit_message(control))
                    {
                        tracing::error!(%error, "failed to submit classic lobby message");
                    }
                }
            }
            LobbyChatRequest::History { older } => {
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = Some(0);
                }
                if !self.classic_host_lobby_active() {
                    let history = self.message_input_history.clone();
                    let view = self.network_lobby.as_mut().map(|lobby| {
                        let inserted = lobby.browse_chat_history(older, &history);
                        (lobby.chat_edit.clone(), inserted)
                    });
                    if let Some((mut view, inserted)) = view {
                        if inserted {
                            self.scroll_active_lobby_chat_caret_in_view(&mut view)?;
                        }
                        self.install_active_lobby_chat_view(view);
                    }
                    return Ok(());
                }
                let (mut view, inserted) = {
                    let Some(lobby) = self.classic_host_lobby.as_mut() else {
                        return Ok(());
                    };
                    lobby.chat_history_index += if older { 1 } else { -1 };
                    let horizontal_scroll = lobby.controller.chat_edit_view().horizontal_scroll;
                    let text = usize::try_from(lobby.chat_history_index)
                        .ok()
                        .and_then(|index| self.message_input_history.get(index))
                        .filter(|text| !text.is_empty())
                        .cloned();
                    let (view, inserted) = match text {
                        Some(text) => {
                            let view = LobbyChatEditView {
                                caret: text.len(),
                                selection: Some((0, text.len())),
                                text,
                                horizontal_scroll,
                                cursor_visible: true,
                            };
                            (view, true)
                        }
                        None => {
                            lobby.chat_history_index = -1;
                            let mut view = lobby.controller.chat_edit_view().clone();
                            lobby_chat_clear_preserving_scroll(&mut view);
                            (view, false)
                        }
                    };
                    lobby.controller.set_chat_edit_view(view.clone());
                    (view, inserted)
                };
                if inserted {
                    self.scroll_active_lobby_chat_caret_in_view(&mut view)?;
                }
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::PointerMiddleDown(point) => {
                let layout = self.active_lobby_chat_layout()?;
                let fonts = self.assets.clonk_fonts.clone().ok_or_else(|| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                            detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
                        }),
                    ))
                })?;
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                let (position, _) = lobby_chat_apply_pointer_selection(
                    &mut view,
                    point,
                    &layout,
                    &fonts.text,
                    true,
                    None,
                );
                if self.lobby_chat_drag_anchor.is_some() {
                    self.lobby_chat_drag_anchor = Some(position);
                }
                if let Some(text) = primary_clipboard_text() {
                    if lobby_chat_insert_primary_text(&mut view, &text) {
                        lobby_chat_scroll_caret_in_view(&mut view, &layout, &fonts.text);
                    }
                }
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::OpenContextMenu { anchor } => {
                let entries = lobby_chat_context_entries(
                    &self.active_lobby_chat_view().unwrap_or_default(),
                    clipboard_text_available(),
                );
                self.open_context_menu_at(entries, anchor)?;
            }
            LobbyChatRequest::PointerDown(point) => {
                self.apply_active_lobby_chat_pointer_selection(point, true, false)?;
            }
            LobbyChatRequest::PointerMove(point) => {
                self.apply_active_lobby_chat_pointer_selection(point, false, false)?;
            }
            LobbyChatRequest::PointerUp(point) => {
                self.apply_active_lobby_chat_pointer_selection(point, false, true)?;
            }
            LobbyChatRequest::PointerDoubleClick(point) => {
                let layout = self.active_lobby_chat_layout()?;
                let fonts = self.assets.clonk_fonts.clone().ok_or_else(|| {
                    classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                            detail: "CStdFont-faithful lobby fonts are unavailable".to_string(),
                        }),
                    ))
                })?;
                let mut view = self.active_lobby_chat_view().unwrap_or_default();
                lobby_chat_apply_double_click(&mut view, point, &layout, &fonts.text);
                self.lobby_chat_drag_anchor = None;
                self.install_active_lobby_chat_view(view);
            }
            LobbyChatRequest::TouchCancel => {
                self.lobby_chat_drag_anchor = None;
            }
            LobbyChatRequest::OpenExternalDialog => {
                self.show_external_irc_dialog()?;
            }
        }
        Ok(())
    }

    pub(crate) fn handle_classic_lobby_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if state == ElementState::Pressed {
            let chat_focused = self
                .classic_host_lobby
                .as_ref()
                .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::ChatInput);
            let chat_command_key = matches!(
                key,
                VirtualKeyCode::Enter
                    | VirtualKeyCode::NumpadEnter
                    | VirtualKeyCode::ArrowUp
                    | VirtualKeyCode::ArrowDown
            );
            let chat_actions = if key == VirtualKeyCode::ContextMenu && c4_modifiers.is_empty() {
                self.classic_host_lobby
                    .as_ref()
                    .map(|lobby| {
                        let actions = lobby.controller.chat_context_from_key(&layout);
                        if !actions.is_empty() {
                            return actions;
                        }
                        let anchor = lobby
                            .controller
                            .selected_roster_id()
                            .and_then(|selected| {
                                roster.rows.iter().find(|row_layout| {
                                    lobby
                                        .controller
                                        .rows()
                                        .get(row_layout.index)
                                        .is_some_and(|row| &row.id() == selected)
                                })
                            })
                            .map(|row| {
                                GuiPoint::new(
                                    (row.rect.x + row.rect.w / 2) as f32,
                                    (row.rect.y + row.rect.h / 2) as f32,
                                )
                            })
                            .unwrap_or_else(|| {
                                GuiPoint::new(
                                    (layout.roster.x + layout.roster.w / 2) as f32,
                                    (layout.roster.y + layout.roster.h / 2) as f32,
                                )
                            });
                        lobby.controller.request_focused_context(anchor)
                    })
                    .unwrap_or_default()
            } else if chat_focused && chat_command_key {
                if c4_modifiers.is_empty() {
                    map_key_code(key)
                        .and_then(|key| {
                            self.classic_host_lobby.as_mut().map(|lobby| {
                                lobby.controller.key_down(
                                    key,
                                    false,
                                    &layout,
                                    &roster,
                                    Instant::now(),
                                )
                            })
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            } else {
                let shortcut = if c4_modifiers == ModifiersState::CONTROL {
                    match key {
                        VirtualKeyCode::KeyC => Some(LobbyChatClipboardShortcut::Copy),
                        VirtualKeyCode::KeyX => Some(LobbyChatClipboardShortcut::Cut),
                        VirtualKeyCode::KeyV => Some(LobbyChatClipboardShortcut::Paste),
                        VirtualKeyCode::KeyA => Some(LobbyChatClipboardShortcut::SelectAll),
                        _ => None,
                    }
                } else {
                    None
                };
                if let Some(shortcut) = shortcut {
                    let actions = self
                        .classic_host_lobby
                        .as_ref()
                        .map(|lobby| lobby.controller.chat_clipboard(shortcut))
                        .unwrap_or_default();
                    if shortcut == LobbyChatClipboardShortcut::Paste && !actions.is_empty() {
                        self.chat_paste_consumed_keys.insert(key);
                    }
                    actions
                } else {
                    let edit_key = if c4_modifiers.contains(ModifiersState::ALT) {
                        None
                    } else {
                        match key {
                            VirtualKeyCode::ArrowLeft => Some(LobbyChatEditKey::Left),
                            VirtualKeyCode::ArrowRight => Some(LobbyChatEditKey::Right),
                            VirtualKeyCode::Home => Some(LobbyChatEditKey::Home),
                            VirtualKeyCode::End => Some(LobbyChatEditKey::End),
                            VirtualKeyCode::Backspace => Some(LobbyChatEditKey::Backspace),
                            VirtualKeyCode::Delete => Some(LobbyChatEditKey::Delete),
                            _ => None,
                        }
                    };
                    edit_key
                        .and_then(|edit_key| {
                            self.classic_host_lobby.as_ref().map(|lobby| {
                                lobby.controller.chat_edit_key(
                                    edit_key,
                                    LobbyChatKeyModifiers {
                                        shift: c4_modifiers.contains(ModifiersState::SHIFT),
                                        control: c4_modifiers.contains(ModifiersState::CONTROL),
                                    },
                                )
                            })
                        })
                        .unwrap_or_default()
                }
            };
            if !chat_actions.is_empty() {
                return self.process_classic_lobby_actions(chat_actions);
            }
            if chat_focused && chat_command_key {
                return Ok(());
            }
        }
        let alt_combo_open = state == ElementState::Pressed
            && self.keyboard_modifiers.alt_key()
            && matches!(key, VirtualKeyCode::ArrowDown | VirtualKeyCode::Space)
            && self
                .classic_host_lobby
                .as_ref()
                .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::RosterTeam);
        let actions = if alt_combo_open {
            map_key_code(key)
                .and_then(|key| {
                    self.classic_host_lobby.as_mut().map(|lobby| {
                        lobby.controller.key_down(
                            key,
                            self.keyboard_modifiers.shift_key(),
                            &layout,
                            &roster,
                            Instant::now(),
                        )
                    })
                })
                .unwrap_or_default()
        } else if state == ElementState::Pressed
            && (c4_modifiers == ModifiersState::ALT
                || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT))
        {
            startup_dialog_hotkey(key)
                .and_then(|hotkey| {
                    self.classic_host_lobby
                        .as_mut()
                        .map(|lobby| lobby.controller.hotkey(hotkey, Instant::now()))
                })
                .unwrap_or_default()
        } else if state == ElementState::Pressed
            && c4_modifiers.contains(ModifiersState::ALT)
            && matches!(
                key,
                VirtualKeyCode::ArrowLeft
                    | VirtualKeyCode::ArrowRight
                    | VirtualKeyCode::Home
                    | VirtualKeyCode::End
                    | VirtualKeyCode::Backspace
                    | VirtualKeyCode::Delete
            )
            && self
                .classic_host_lobby
                .as_ref()
                .is_some_and(|lobby| lobby.controller.focus() == LobbyControl::ChatInput)
        {
            Vec::new()
        } else if let Some(key) = map_key_code(key) {
            match state {
                ElementState::Pressed => self
                    .classic_host_lobby
                    .as_mut()
                    .map(|lobby| {
                        lobby.controller.key_down(
                            key,
                            self.keyboard_modifiers.shift_key(),
                            &layout,
                            &roster,
                            Instant::now(),
                        )
                    })
                    .unwrap_or_default(),
                ElementState::Released => self
                    .classic_host_lobby
                    .as_mut()
                    .map(|lobby| lobby.controller.key_up(key))
                    .unwrap_or_default(),
            }
        } else {
            Vec::new()
        };
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn process_joined_lobby_controller_actions(
        &mut self,
        actions: Vec<ClassicLobbyAction>,
    ) -> Result<(), EngineError> {
        let mut pending: VecDeque<ClassicLobbyAction> = actions.into();
        while let Some(action) = pending.pop_front() {
            match action {
                ClassicLobbyAction::FocusChanged(control) => {
                    self.set_active_lobby_chat_focus(control == LobbyControl::ChatInput);
                    self.scenario_game_options
                        .set_focused_button(match control {
                            LobbyControl::GameOption(button) => Some(button),
                            _ => None,
                        });
                }
                ClassicLobbyAction::GameOptions(input) => {
                    pending.extend(self.route_joined_lobby_game_option_input(input)?);
                }
                ClassicLobbyAction::Chat(request) => {
                    self.process_classic_lobby_chat_request(request)?;
                }
                ClassicLobbyAction::RosterContextRequested { row, position } => {
                    self.open_classic_lobby_roster_context(row, position)?;
                }
                ClassicLobbyAction::TabContextRequested { position } => {
                    self.open_lobby_tab_context(position)?;
                }
                // The retained roster routes its locally authorized actions
                // through the same packet-backed handlers as the persistent
                // host controller.
                action @ (ClassicLobbyAction::RosterSelectionChanged(_)
                | ClassicLobbyAction::AddPlayerRequested { .. }
                | ClassicLobbyAction::AddScriptPlayerRequested
                | ClassicLobbyAction::TeamSelectionRequested { .. }
                | ClassicLobbyAction::MoveLocalPlayersIntoTeamRequested { .. }) => {
                    self.process_classic_lobby_actions(vec![action])?;
                }
                ClassicLobbyAction::SheetRequested(sheet) => {
                    self.process_lobby_action(LobbyAction::SelectSheet(sheet))?;
                }
                ClassicLobbyAction::SaveResourceRequested { resource_id } => {
                    self.request_lobby_resource_save(resource_id, false)?;
                }
                ClassicLobbyAction::PreloadRequested => self.request_lobby_preload(),
                ClassicLobbyAction::ReadyChanged(ready) => {
                    // The retained controller is the C4GUI::CheckBox: it owns
                    // the loading lock and the ready-button cooldown and only
                    // emits accepted toggles (MainDlg::OnReadyCheck,
                    // src/C4GameLobby.cpp:329-344). Mirror the accepted value
                    // onto the adapter's authoritative participant row before
                    // publishing it.
                    let changed = self.network_lobby.as_mut().and_then(|lobby| {
                        let client_id = lobby.local_client_id;
                        lobby.participants.get_mut(&client_id).map(|participant| {
                            participant.ready = ready;
                            client_id
                        })
                    });
                    if let Some(client_id) = changed {
                        self.submit_joined_lobby_ready_state(client_id, ready)?;
                    }
                }
                ClassicLobbyAction::ExitRequested => {
                    // C4GUI::Button raises its click sound before invoking
                    // MainDlg::OnExitBtn. Drain the controller queue while
                    // the lobby still exists; dialog-level Escape arrives
                    // with an empty queue and the adapter's chat-focused
                    // Escape and Alt+mnemonic exits bypass this arm, so all
                    // three stay silent.
                    self.play_classic_lobby_sounds();
                    self.exit_startup_lobby_to_main();
                    return Ok(());
                }
                ClassicLobbyAction::StartRequested { .. } => {
                    // The generic lobby keeps its own countdown entry point;
                    // the classic league/savegame start gates stay with the
                    // exact host controller.
                    self.play_classic_lobby_sounds();
                    self.start_network_lobby_countdown()?;
                }
                ClassicLobbyAction::AbortCountdownRequested => {
                    self.abort_network_lobby_countdown();
                }
                // Joined countdown, attention and log state flow in through
                // network packets rather than controller input, and every row
                // the joined Options sheet projects is a read-only ComboBox
                // that raises no selection (src/C4GameOptions.cpp:80,126,154,
                // 186,211,234), so routed input cannot produce these here.
                ClassicLobbyAction::OptionSelectionRequested { .. }
                | ClassicLobbyAction::CountdownChanged(_)
                | ClassicLobbyAction::NotifyUserIfInactive
                | ClassicLobbyAction::AppendLog(_) => {}
            }
        }
        self.play_classic_lobby_sounds();
        Ok(())
    }

    pub(crate) fn joined_lobby_input_error(error: anyhow::Error) -> EngineError {
        classic_parity_engine_error(report_classic_parity_boundary(
            ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            }),
        ))
    }

    pub(crate) fn handle_network_lobby_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<(), EngineError> {
        let assets = Arc::clone(&self.assets);
        let actions = self
            .network_lobby
            .as_mut()
            .map(|lobby| {
                lobby.handle_panel_pointer_move(point);
                lobby.classic_pointer_move(
                    point,
                    self.graphics.surface(),
                    assets.as_ref(),
                    &self.scenario_game_options,
                )
            })
            .transpose()
            .map_err(Self::joined_lobby_input_error)?
            .unwrap_or_default();
        self.menu_state.set_pointer_position(None);
        self.process_joined_lobby_controller_actions(actions)
    }

    pub(crate) fn handle_network_lobby_pointer_button(
        &mut self,
        state: ElementState,
        double_click: bool,
    ) -> Result<(), EngineError> {
        let Some(point) = self.network_lobby.as_ref().and_then(|lobby| lobby.pointer) else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let actions = {
            let lobby = self
                .network_lobby
                .as_mut()
                .expect("joined lobby pointer came from live state");
            match state {
                ElementState::Pressed => {
                    if double_click {
                        lobby.last_roster_click = None;
                    }
                    lobby
                        .classic_pointer_down(
                            point,
                            double_click,
                            self.graphics.surface(),
                            assets.as_ref(),
                            &self.scenario_game_options,
                        )
                        .map_err(Self::joined_lobby_input_error)?
                }
                ElementState::Released => {
                    // Discrete-click platforms never deliver LeftDouble, so
                    // synthesize C4MC_Button_LeftDouble from the retained
                    // semantic row exactly like the persistent host path.
                    let now = Instant::now();
                    let (mut actions, clicked) = lobby
                        .with_classic_controller_input(
                            self.graphics.surface(),
                            assets.as_ref(),
                            &self.scenario_game_options,
                            |controller, layout, roster| {
                                let clicked =
                                    controller.accepted_roster_click_id(point, layout, roster);
                                let actions = controller.pointer_up(point, layout, roster, now);
                                (actions, clicked)
                            },
                        )
                        .map_err(Self::joined_lobby_input_error)?;
                    let synthesized_double = clicked.as_ref().is_some_and(|clicked| {
                        lobby.last_roster_click.as_ref().is_some_and(|(last, at)| {
                            last == clicked
                                && now.saturating_duration_since(*at) < CPP_DOUBLE_CLICK_INTERVAL
                        })
                    });
                    lobby.last_roster_click = if synthesized_double {
                        None
                    } else {
                        clicked.clone().map(|row| (row, now))
                    };
                    if let Some(clicked) = clicked.as_ref().filter(|_| synthesized_double) {
                        actions.extend(lobby.controller.roster_double_click(clicked));
                    }
                    actions
                }
            }
        };
        self.process_joined_lobby_controller_actions(actions)
    }

    pub(crate) fn handle_network_lobby_touch(
        &mut self,
        phase: TouchPhase,
        point: GuiPoint,
        double_click: bool,
    ) -> Result<(), EngineError> {
        let assets = Arc::clone(&self.assets);
        let actions = {
            let lobby = self
                .network_lobby
                .as_mut()
                .expect("joined lobby touch requires live state");
            let actions = lobby
                .classic_touch(
                    phase,
                    point,
                    double_click,
                    self.graphics.surface(),
                    assets.as_ref(),
                    &self.scenario_game_options,
                )
                .map_err(Self::joined_lobby_input_error)?;
            if phase == TouchPhase::Cancelled {
                lobby.last_roster_click = None;
            }
            actions
        };
        self.menu_state.set_pointer_position(None);
        self.process_joined_lobby_controller_actions(actions)?;
        if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.pointer_left_unchecked();
        }
        Ok(())
    }

    pub(crate) fn handle_classic_lobby_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<(), EngineError> {
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| {
                lobby.pointer = Some(point);
                lobby.controller.pointer_move(point, &layout, &roster)
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_classic_lobby_pointer_button(
        &mut self,
        state: ElementState,
        double_click: bool,
    ) -> Result<(), EngineError> {
        let Some(point) = self
            .classic_host_lobby
            .as_ref()
            .and_then(|lobby| lobby.pointer)
        else {
            return Ok(());
        };
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let actions = match state {
            ElementState::Pressed => self
                .classic_host_lobby
                .as_mut()
                .map(|lobby| {
                    if double_click {
                        lobby.last_roster_click = None;
                        lobby
                            .controller
                            .pointer_double_click(point, &layout, &roster)
                    } else {
                        lobby.controller.pointer_down(point, &layout, &roster)
                    }
                })
                .unwrap_or_default(),
            ElementState::Released => self
                .classic_host_lobby
                .as_mut()
                .map(|lobby| {
                    let now = Instant::now();
                    let clicked = lobby
                        .controller
                        .accepted_roster_click_id(point, &layout, &roster);
                    let double_click = clicked.as_ref().is_some_and(|clicked| {
                        lobby.last_roster_click.as_ref().is_some_and(|(last, at)| {
                            last == clicked
                                && now.saturating_duration_since(*at) < CPP_DOUBLE_CLICK_INTERVAL
                        })
                    });
                    lobby.last_roster_click = if double_click {
                        None
                    } else {
                        clicked.clone().map(|row| (row, now))
                    };
                    let mut actions = lobby.controller.pointer_up(point, &layout, &roster, now);
                    if let Some(clicked) = clicked.as_ref().filter(|_| double_click) {
                        actions.extend(lobby.controller.roster_double_click(clicked));
                    }
                    actions
                })
                .unwrap_or_default(),
        };
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_joined_lobby_roster_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let Some(focus) = self
            .network_lobby
            .as_ref()
            .map(|lobby| lobby.controller.focus())
        else {
            return Ok(false);
        };
        let tab = state == ElementState::Pressed
            && key == VirtualKeyCode::Tab
            && (self.keyboard_modifiers.is_empty()
                || self.keyboard_modifiers == ModifiersState::SHIFT);
        let no_modifiers = self.keyboard_modifiers.is_empty();
        let default_focus_modifiers =
            no_modifiers || self.keyboard_modifiers == ModifiersState::SHIFT;
        let combo_open_modifiers = no_modifiers || self.keyboard_modifiers == ModifiersState::ALT;
        let roster_has_rows = self
            .network_lobby
            .as_ref()
            .is_some_and(|lobby| !lobby.controller.rows().is_empty());
        // C4GUI::Dialog advances focus for Tab regardless of which control
        // holds it; only the non-traversal keys stay focus-specific here.
        // The C4GUI listbox binds no confirm keys and Dialog::CharIn excludes
        // space from default-control refocusing (src/C4GuiDialogs.cpp:552-567),
        // so eat them here instead of leaking them to the frontend's
        // default-focus fallback or the generic menu shim.
        if focus == LobbyControl::Roster
            && no_modifiers
            && matches!(
                key,
                VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter | VirtualKeyCode::Space
            )
        {
            return Ok(true);
        }
        let accepted = tab
            || match focus {
                LobbyControl::Roster => {
                    no_modifiers
                        && matches!(
                            key,
                            VirtualKeyCode::ArrowUp
                                | VirtualKeyCode::ArrowDown
                                | VirtualKeyCode::Home
                                | VirtualKeyCode::End
                        )
                        || no_modifiers
                            && roster_has_rows
                            && matches!(key, VirtualKeyCode::PageUp | VirtualKeyCode::PageDown)
                }
                LobbyControl::RosterTeam => {
                    combo_open_modifiers
                        && matches!(key, VirtualKeyCode::ArrowDown | VirtualKeyCode::Space)
                        || default_focus_modifiers && key == VirtualKeyCode::ArrowUp
                }
                LobbyControl::RosterAddPlayer => {
                    no_modifiers
                        && matches!(
                            key,
                            VirtualKeyCode::Enter
                                | VirtualKeyCode::NumpadEnter
                                | VirtualKeyCode::Space
                        )
                }
                _ => false,
            };
        if !accepted {
            return Ok(false);
        }
        let Some(key_code) = map_key_code(key) else {
            return Ok(false);
        };
        let (layout, roster) = self.joined_lobby_layouts()?;
        let shift = self.keyboard_modifiers.shift_key();
        let actions = self
            .network_lobby
            .as_mut()
            .map(|lobby| match state {
                ElementState::Pressed => {
                    lobby
                        .controller
                        .key_down(key_code, shift, &layout, &roster, Instant::now())
                }
                ElementState::Released => lobby.controller.key_up(key_code),
            })
            .unwrap_or_default();
        self.process_joined_lobby_controller_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn handle_classic_lobby_secondary_button(
        &mut self,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let Some(point) = self
            .classic_host_lobby
            .as_ref()
            .and_then(|lobby| lobby.pointer)
        else {
            return Ok(());
        };
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby
                .controller
                .note_pointer_button(point, &layout, &roster);
        }
        self.scenario_game_options.note_pointer_button();
        if state == ElementState::Released {
            return Ok(());
        }
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| {
                lobby
                    .controller
                    .pointer_secondary_down(point, &layout, &roster)
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_network_lobby_secondary_button(
        &mut self,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let Some(point) = self.network_lobby.as_ref().and_then(|lobby| lobby.pointer) else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        self.network_lobby
            .as_mut()
            .expect("network lobby was checked above")
            .classic_note_pointer_button(
                point,
                self.graphics.surface(),
                assets.as_ref(),
                &self.scenario_game_options,
            )
            .map_err(Self::joined_lobby_input_error)?;
        self.scenario_game_options.note_pointer_button();
        if state == ElementState::Released {
            return Ok(());
        }
        let actions = self
            .network_lobby
            .as_mut()
            .expect("network lobby was checked above")
            .classic_secondary_down(
                point,
                self.graphics.surface(),
                assets.as_ref(),
                &self.scenario_game_options,
            )
            .map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                        detail: error.to_string(),
                    }),
                ))
            })?;
        self.process_joined_lobby_controller_actions(actions)
    }

    pub(crate) fn handle_network_lobby_context_key(&mut self) -> Result<(), EngineError> {
        let assets = Arc::clone(&self.assets);
        let actions = self
            .network_lobby
            .as_mut()
            .expect("network lobby context key requires live state")
            .classic_context_key(
                self.graphics.surface(),
                assets.as_ref(),
                &self.scenario_game_options,
            )
            .map_err(Self::joined_lobby_input_error)?;
        self.process_joined_lobby_controller_actions(actions)
    }

    pub(crate) fn handle_classic_lobby_middle_button(
        &mut self,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let Some(point) = self
            .classic_host_lobby
            .as_ref()
            .and_then(|lobby| lobby.pointer)
        else {
            return Ok(());
        };
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby
                .controller
                .note_pointer_button(point, &layout, &roster);
        }
        self.scenario_game_options.note_pointer_button();
        if state == ElementState::Released {
            return Ok(());
        }
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| {
                lobby
                    .controller
                    .pointer_middle_down(point, &layout, &roster)
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_network_lobby_middle_button(
        &mut self,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let Some(point) = self.network_lobby.as_ref().and_then(|lobby| lobby.pointer) else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        self.network_lobby
            .as_mut()
            .expect("network lobby was checked above")
            .classic_note_pointer_button(
                point,
                self.graphics.surface(),
                assets.as_ref(),
                &self.scenario_game_options,
            )
            .map_err(Self::joined_lobby_input_error)?;
        self.scenario_game_options.note_pointer_button();
        if state == ElementState::Released {
            return Ok(());
        }
        let actions = self
            .network_lobby
            .as_mut()
            .expect("network lobby was checked above")
            .classic_middle_down(
                point,
                self.graphics.surface(),
                assets.as_ref(),
                &self.scenario_game_options,
            )
            .map_err(Self::joined_lobby_input_error)?;
        self.process_joined_lobby_controller_actions(actions)
    }

    pub(crate) fn handle_classic_lobby_wheel(&mut self, delta: i32) -> Result<(), EngineError> {
        if delta == 0 {
            return Ok(());
        }
        let Some(point) = self
            .classic_host_lobby
            .as_ref()
            .and_then(|lobby| lobby.pointer)
        else {
            return Ok(());
        };
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let contains = |rect: clonk_frontend::classic_gui::IntRect| {
            point.x >= rect.x as f32
                && point.y >= rect.y as f32
                && point.x < (rect.x + rect.w) as f32
                && point.y < (rect.y + rect.h) as f32
        };
        let scroll_window_captured =
            contains(layout.chat_log_client) || contains(layout.roster_client);
        if let Some(lobby) = self.classic_host_lobby.as_mut() {
            lobby.controller.note_pointer_wheel();
        }
        self.scenario_game_options.note_pointer_wheel();
        let outside_scroll_window = contains(layout.chat_log) && !contains(layout.chat_log_client)
            || contains(layout.roster) && !contains(layout.roster_client);
        let _ = self.classic_host_lobby.as_mut().is_some_and(|lobby| {
            !outside_scroll_window && lobby.controller.wheel(point, delta, &layout, &roster)
        });
        if scroll_window_captured {
            // C4GUI::ScrollWindow consumes the wheel and clears the screen's
            // pMouseOver owner. Keep retained positions for integer motion
            // detection, but require a later pointer event before either
            // controller may expose another tooltip.
            self.note_classic_lobby_non_pointer_input();
        }
        Ok(())
    }

    pub(crate) fn handle_classic_lobby_touch(
        &mut self,
        phase: TouchPhase,
        point: GuiPoint,
        double_click: bool,
    ) -> Result<(), EngineError> {
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| {
                lobby.pointer = (!matches!(phase, TouchPhase::Cancelled)).then_some(point);
                match phase {
                    TouchPhase::Started if double_click => lobby
                        .controller
                        .pointer_double_click(point, &layout, &roster),
                    TouchPhase::Started => lobby.controller.touch_start(point, &layout, &roster),
                    TouchPhase::Moved => lobby.controller.touch_move(point, &layout, &roster),
                    TouchPhase::Ended => {
                        lobby
                            .controller
                            .touch_end(point, &layout, &roster, Instant::now())
                    }
                    TouchPhase::Cancelled => lobby.controller.touch_cancel(),
                }
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_classic_lobby_gamepad_direction(
        &mut self,
        button: ControlButton,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if state == ElementState::Released {
            return Ok(());
        }
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let (horizontal, vertical) = match button {
            ControlButton::Left => (-1, 0),
            ControlButton::Right => (1, 0),
            ControlButton::Up => (0, -1),
            ControlButton::Down => (0, 1),
        };
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| {
                lobby
                    .controller
                    .gamepad_direction(horizontal, vertical, &layout, &roster)
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    pub(crate) fn handle_classic_lobby_gamepad_action(
        &mut self,
        action: GamepadActionType,
        state: ElementState,
    ) -> Result<(), EngineError> {
        let (layout, roster) = self.classic_host_lobby_layouts()?;
        let actions = self
            .classic_host_lobby
            .as_mut()
            .map(|lobby| match action {
                GamepadActionType::Select => match state {
                    ElementState::Pressed => {
                        lobby
                            .controller
                            .gamepad_low_down(Instant::now(), &layout, &roster)
                    }
                    ElementState::Released => lobby.controller.gamepad_low_up(),
                },
                GamepadActionType::Cancel | GamepadActionType::MenuToggle => {
                    if state == ElementState::Pressed {
                        lobby.controller.gamepad_high_down()
                    } else {
                        Vec::new()
                    }
                }
            })
            .unwrap_or_default();
        self.process_classic_lobby_actions(actions)
    }

    /// State the operating mode in the lobby, so the profile is visible
    /// *before* anyone commits to the session.
    ///
    /// Only the host announces, because only the host's setting decides what
    /// the session actually runs: `session_control_mode` resolves the host's
    /// `initial_status.control_mode`, and every client adopts the received
    /// `status.control_mode` rather than applying its own. A joining client
    /// stating its local profile here would assert a promise about a session
    /// its configuration has no part in — the host may be running the normal
    /// profile. Surfacing the *host's* advertised profile to a client is the
    /// missing half; the reference already carries it as `CompatProfile=`
    /// (`clonk-network/src/advertise.rs:131-135`) and nothing reads it yet
    /// (clonk-org/clonk-rs#583, clonk-org/clonk-rs#588).
    ///
    /// Only a non-default profile says anything. `CompatProfile::Normal`
    /// promises nothing and is what every session runs today, so announcing it
    /// would add a line to a C++-mirrored surface for no information — the
    /// default lobby stays exactly as it was.
    fn announce_compat_profile_in_lobby(&mut self) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        if self.compat_profile == crate::settings::CompatProfile::Normal {
            return;
        }
        let text = format!(
            "Compatibility profile: {}",
            self.compat_profile.display_name()
        );
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.push_log(clonk_frontend::game_lobby::LobbyLogLine {
                text,
                color: [0xff, 0xff, 0xff, 0xff],
            });
        }
    }

    pub(crate) fn open_network_lobby(&mut self) {
        self.close_context_menu_silently();
        self.replace_startup_view(StartupView::NetworkLobby);
        self.menu_state.set_pointer_position(None);
        self.menu_state.set_include_back(true);
        self.menu_state.refresh_menu_entries();
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        self.menu_state.menu().resize(width, height);
        if let Err(err) = self.handle_menu_input(|menu| menu.select_default_entry()) {
            tracing::error!(error = %err, "failed to select default scenario entry");
        }
        let labels = self.classic_lobby_labels();
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.labels = labels;
            lobby.update_layout(width, height);
            self.scenario_label = lobby.scenario_label();
        } else {
            self.scenario_label = "Network lobby unavailable".to_string();
        }
        self.sync_network_lobby_game_option_state();
        self.announce_compat_profile_in_lobby();
        self.status_text.clear();
        self.acknowledge_initial_lobby_status_if_ready();
    }

    pub(crate) fn tick_lobby_ready_check_prompt(&mut self) -> bool {
        let Some(prompt_index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                &dialog.continuation,
                MessageDialogContinuation::LobbyReadyCheck { .. }
            )
        }) else {
            return false;
        };
        let expires = self
            .message_dialogs
            .get_mut(prompt_index)
            .is_some_and(|dialog| {
                let MessageDialogContinuation::LobbyReadyCheck { remaining_seconds } =
                    &mut dialog.continuation
                else {
                    return false;
                };
                if *remaining_seconds <= 1 {
                    return true;
                }
                *remaining_seconds -= 1;
                dialog
                    .state
                    .set_message(lobby_ready_check_message(*remaining_seconds));
                false
            });
        if expires {
            if prompt_index + 1 == self.message_dialogs.len() {
                if let Err(error) = self.finish_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogResult::Dismissed,
                ) {
                    tracing::error!(%error, "failed to close timed lobby ready check");
                }
            } else {
                self.remove_message_dialog_at(prompt_index);
                if let Err(error) = self.complete_lobby_ready_check_response(false) {
                    tracing::error!(%error, "failed to expire lobby ready check");
                }
            }
        }
        true
    }

    pub(crate) fn close_lobby_child_dialogs_silently(&mut self) {
        // Fullscreen DoLobby destroys MainDlg and then asks Screen to close
        // every remaining dialog without invoking acceptance/cancellation
        // callbacks (src/C4Network2.cpp:493-512).
        self.close_context_menu_silently();
        self.release_message_dialog_pointer_elements();
        self.message_dialogs.clear();
        self.message_dialog_active_index = None;
        self.message_dialog_pointer_capture_index = None;
        self.message_dialog_consumed_keys.clear();
        self.game_option_input_dialog = None;
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        self.league_signup_dialog = None;
        self.cancelled_league_signup_continuation = None;
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        self.definition_selector = None;
        self.pending_definition_selection = None;
        self.pending_lobby_player_selection = None;
        self.definition_selector_last_click = None;
        self.definition_selector_consumed_keys.clear();
        self.definition_selector_pointer_capture = false;
        self.startup_player_properties_dialog = None;
    }

    pub(crate) fn tick_network_lobby_countdown(&mut self) -> bool {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return false;
        }
        let Some((next, broadcast)) = self
            .host_lobby_countdown
            .as_mut()
            .map(HostLobbyCountdown::advance)
        else {
            return false;
        };
        if next == 0 {
            self.host_lobby_countdown = None;
        }
        if broadcast {
            self.submit_and_apply_lobby_countdown(clonk_network::LobbyCountdownPacket::new(next));
        }
        if next == 0 {
            if self.abort_dialogless_round_short_of_min_players() {
                return broadcast;
            }
            if let Err(error) = self.start_network_game_now() {
                tracing::error!(%error, "failed to start network game after lobby countdown");
                self.status_text = format!("Unable to start network game: {error}");
            }
        }
        broadcast
    }

    /// The dedicated-server arm of a countdown that reached zero: quit rather
    /// than start a round the scenario cannot be played with.
    ///
    /// ```cpp
    /// if (!Game.Network.GetLobby() && (Game.PlayerInfos.GetPlayerCount() < Game.C4S.GetMinPlayer()))
    /// {
    ///     Log(C4ResStrTableKey::IDS_MSG_NOTENOUGHPLAYERSFORTHISRO);
    ///     Application.Quit();
    /// }
    /// ```
    ///
    /// (src/C4GameLobby.cpp:1163-1168.) `GetLobby()` is null exactly when the
    /// lobby is not a dialog, which `fFullscreenLobby =
    /// !Console.Active && (lpDDraw->GetEngine() != GFXENGN_NOGFX)` decides
    /// (src/C4Network2.cpp:463) — so the console engine and the dedicated one
    /// both take this arm, and a windowed host never does. Returns whether the
    /// round was aborted.
    fn abort_dialogless_round_short_of_min_players(&mut self) -> bool {
        if !(self.console_mode || self.headless) {
            return false;
        }
        // An undetermined minimum never quits a running server.
        let Some(minimum) = self.network_lobby_min_players else {
            return false;
        };
        let players =
            i32::try_from(self.control_player_infos.nonremoved_player_count()).unwrap_or(i32::MAX);
        if players >= minimum {
            return false;
        }
        let message = self.runtime_resource_text(
            "IDS_MSG_NOTENOUGHPLAYERSFORTHISRO",
            "Not enough players for this round.",
        );
        // C++ leaves this one on the log deliberately — "it would also be nice
        // to send this message to all clients..." (C4GameLobby.cpp:1167).
        tracing::warn!(%message, players, minimum, "aborting the round");
        self.status_text = message;
        self.request_exit("too few players for the round at countdown zero");
        true
    }

    pub(crate) fn publish_lobby_game_option_reference(
        &mut self,
        password_needed: bool,
        comment: clonk_engine::LegacyCString,
    ) {
        let Some(reference) = self.advertised_game_reference.clone() else {
            return;
        };
        let updated = match reference.replacing_lobby_options(password_needed, comment) {
            Ok(updated) => updated,
            Err(error) => {
                tracing::error!(%error, "failed to rebuild lobby game-option reference");
                self.status_text = format!("Unable to update network game reference: {error}");
                return;
            }
        };
        if let Some(advertiser) = self.network_game_advertiser.as_ref() {
            if let Err(error) = advertiser.update_exact(&updated) {
                tracing::error!(%error, "failed to publish lobby game-option reference");
            }
        }
        self.advertised_game_reference = Some(updated);
        if let Some(network) = self.network.as_ref() {
            if let Err(error) = network.invalidate_league_reference() {
                tracing::error!(%error, "failed to invalidate lobby game-option reference");
            }
        }
    }

    pub(crate) fn process_lobby_game_option_action(
        &mut self,
        action: GameOptionAction,
    ) -> Result<(), EngineError> {
        match action {
            GameOptionAction::FocusTraversalRequested { .. } => {
                tracing::error!("lobby game-option focus traversal escaped its recursive owner");
            }
            GameOptionAction::InternetSignupChanged {
                enabled,
                live_lobby,
            } => {
                debug_assert!(live_lobby);
                if let Some(pending) = self.pending_lobby_internet_signup.as_ref() {
                    self.scenario_game_options
                        .apply_lobby_internet_result(pending.previous_enabled());
                    return Ok(());
                }
                let config = load_prepared_league_host_config(self.app_paths.as_ref(), false);
                let server_name = config.endpoint.clone();
                let reference = self.advertised_game_reference.clone();
                let result = match (self.network.as_ref(), reference) {
                    (Some(network), Some(reference)) => {
                        network.begin_masterserver_signup(enabled, config, reference)
                    }
                    _ => Err(anyhow!("live host registration state is unavailable")),
                };
                match result {
                    Ok(pending) => {
                        let previous_enabled = pending.previous_enabled();
                        self.scenario_game_options
                            .apply_lobby_internet_result(previous_enabled);
                        self.pending_lobby_internet_signup = Some(pending);
                        if enabled {
                            if let Err(error) =
                                self.open_live_masterserver_signup_dialog(&server_name)
                            {
                                self.abort_live_masterserver_signup();
                                return Err(error);
                            }
                        }
                    }
                    Err(error) => {
                        let effective = !enabled;
                        self.scenario_game_options
                            .apply_lobby_internet_result(effective);
                        self.persist_game_option_value(
                            "Network",
                            "MasterServerSignUp",
                            i32::from(effective).to_string(),
                        );
                        tracing::error!(%error, enabled, "failed to change live masterserver signup");
                        self.status_text =
                            format!("Unable to change Internet game signup: {error}");
                    }
                }
            }
            GameOptionAction::LeagueSignupChanged(enabled) => {
                self.persist_game_option_value(
                    "Network",
                    "LeagueServerSignUp",
                    i32::from(enabled).to_string(),
                );
            }
            GameOptionAction::ShowInputDialog(request) => {
                self.open_game_option_input_dialog(request)?;
            }
            GameOptionAction::PasswordChanged {
                password,
                remember_for_next_round,
            } => {
                let Some(password_bytes) = clonk_resources::encode_legacy_script_text(&password)
                else {
                    self.status_text =
                        "Network password is not representable in the classic encoding".to_string();
                    return Ok(());
                };
                let Some(network_password) =
                    clonk_engine::LegacyCString::from_bytes(password_bytes)
                else {
                    self.status_text =
                        "Network password contains an unsupported NUL byte".to_string();
                    return Ok(());
                };
                if let Some(network) = self.network.as_ref() {
                    if let Err(error) = network.set_host_password(network_password) {
                        tracing::error!(%error, "failed to update live host password");
                        self.status_text = format!("Unable to update network password: {error}");
                        return Ok(());
                    }
                } else {
                    self.status_text = "Live host password state is unavailable".to_string();
                    return Ok(());
                }
                if let Some(password) = remember_for_next_round.as_ref() {
                    self.persist_game_option_text("Network", "LastPassword", password);
                }
                let comment = self
                    .advertised_game_reference
                    .as_ref()
                    .map(|reference| reference.metadata().comment.clone())
                    .unwrap_or_default();
                self.publish_lobby_game_option_reference(!password.is_empty(), comment);
                self.scenario_game_options
                    .apply_lobby_password_result(password, remember_for_next_round);
            }
            GameOptionAction::CommentChanged(comment) => {
                let Some(comment_bytes) = clonk_resources::encode_legacy_script_text(&comment)
                else {
                    self.status_text =
                        "Network comment is not representable in the classic encoding".to_string();
                    return Ok(());
                };
                let Some(reference_comment) =
                    clonk_engine::LegacyCString::from_bytes(comment_bytes)
                else {
                    self.status_text =
                        "Network comment contains an unsupported NUL byte".to_string();
                    return Ok(());
                };
                self.persist_game_option_text("Network", "Comment", &comment);
                let password_needed = self
                    .advertised_game_reference
                    .as_ref()
                    .is_some_and(|reference| reference.summary().password_needed);
                self.publish_lobby_game_option_reference(password_needed, reference_comment);
                self.append_control_message_log(
                    clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG.to_string(),
                    CONTROL_LOG_COLOR,
                    None,
                );
                self.scenario_game_options
                    .apply_lobby_comment_result(comment);
            }
            GameOptionAction::SendLobbyFairCrewControl { value } => {
                if let Some(network) = self.network.as_ref() {
                    if let Err(error) =
                        network.submit_control_set(clonk_network::LegacyControlSet {
                            value_type: 5,
                            data: value,
                            by_client: 0,
                        })
                    {
                        tracing::error!(%error, "failed to submit lobby FairCrew update");
                        self.status_text = format!("Unable to change Fair Crew: {error}");
                    }
                }
            }
            GameOptionAction::RecordPreferenceChanged(enabled) => {
                self.startup_view_flags.record = enabled;
                self.recording_enabled = enabled && self.recordings_dir.is_some();
                self.persist_game_option_value("General", "Record", i32::from(enabled).to_string());
            }
            GameOptionAction::FairCrewPreferenceChanged(_) => {
                tracing::error!("lobby controller emitted selector-only FairCrew preference");
            }
        }
        Ok(())
    }

    pub(crate) fn complete_lobby_ready_check_response(
        &mut self,
        ready: bool,
    ) -> Result<(), EngineError> {
        // C++ checks the network status after the modal closes and returns
        // unless it is still exactly GS_Lobby. DoLobby normally deletes the
        // lobby and closes this dialog as soon as GS_Go is installed.
        let status_left_lobby = self
            .pending_client_start_status
            .is_some_and(|status| status.state != clonk_network::NETWORK_STATE_LOBBY);
        if !matches!(self.mode, AppMode::Menu) || status_left_lobby || self.network_lobby.is_none()
        {
            return Ok(());
        }
        let changed_client_id = self.network_lobby.as_mut().and_then(|lobby| {
            let local_client_id = lobby.local_client_id;
            let participant = lobby.participants.get_mut(&local_client_id)?;
            (participant.ready != ready).then(|| {
                participant.ready = ready;
                local_client_id
            })
        });
        let data = if ready {
            clonk_network::ReadyCheckData::Ready
        } else {
            clonk_network::ReadyCheckData::NotReady
        };
        if changed_client_id
            .and_then(|client_id| i32::try_from(client_id).ok())
            .is_some_and(|client_id| self.control_clients.set_lobby_ready(client_id, ready))
        {
            self.publish_updated_host_join_snapshot();
        }
        self.sync_classic_lobby_roster();
        if let Some(Err(error)) = self
            .network
            .as_ref()
            .map(|network| network.submit_ready_check(data))
        {
            tracing::error!(%error, "failed to submit lobby ready check response");
        }
        if let Some(changed_client_id) = changed_client_id {
            self.on_lobby_client_ready_state_change(changed_client_id)?;
        }
        Ok(())
    }

    pub(crate) fn render_classic_host_lobby(&mut self) -> Result<()> {
        self.close_stale_classic_lobby_team_combo();
        let gamma = self.startup_fragment_gamma();
        let config = self.loader_render_config.ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "loader render configuration is unavailable".to_string(),
            })
        })?;
        if let Some(detail) = self.loader_render_error.as_deref() {
            return Err(classic_game_lobby_error(
                ClassicGameLobbyBoundary::Resources {
                    detail: detail.to_string(),
                },
            ));
        }
        let loader = self.loader_screen.as_ref().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: "staged scenario loader is unavailable".to_string(),
            })
        })?;
        let assets = Arc::clone(&self.assets);
        let lobby_resources = assets.game_lobby_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let option_resources = assets.game_option_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let lobby = self.classic_host_lobby.as_mut().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "exact host lobby state is absent".to_string(),
            })
        })?;
        let active = self.context_menu.is_none()
            && self.definition_selector.is_none()
            && self.game_option_input_dialog.is_none()
            && self.league_signup_dialog.is_none()
            && self.message_dialogs.is_empty()
            && self.runtime_client_list.is_none()
            && !self.external_irc_dialog_visible;
        let surface = self.graphics.surface_mut();
        loader.render_background(surface, config, Some(&gamma));
        lobby.controller.render_without_tooltips(
            surface,
            &lobby_resources,
            &self.scenario_game_options,
            &option_resources,
            active,
            Some(&gamma),
        )
    }

    pub(crate) fn render_classic_host_lobby_tooltips(&mut self) -> Result<()> {
        let gamma = self.startup_fragment_gamma();
        let assets = Arc::clone(&self.assets);
        let lobby_resources = assets.game_lobby_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let option_resources = assets.game_option_resources().map_err(|error| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Resources {
                detail: error.to_string(),
            })
        })?;
        let lobby = self.classic_host_lobby.as_mut().ok_or_else(|| {
            classic_game_lobby_error(ClassicGameLobbyBoundary::Model {
                detail: "exact host lobby state is absent".to_string(),
            })
        })?;
        let active = self.context_menu.is_none()
            && self.definition_selector.is_none()
            && self.game_option_input_dialog.is_none()
            && self.league_signup_dialog.is_none()
            && self.message_dialogs.is_empty()
            && self.runtime_client_list.is_none()
            && !self.external_irc_dialog_visible;
        lobby.controller.render_tooltips(
            self.graphics.surface_mut(),
            &lobby_resources,
            &self.scenario_game_options,
            &option_resources,
            active,
            Some(&gamma),
        )
    }
}
