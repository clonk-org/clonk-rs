//! `impl Engine` — player join, registration, removal and per-player state.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn join_player_at_client_with_semantics(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: String,
        semantics: ControlJoinPlayerSemantics,
        runtime_control: PlayerRuntimeControl,
        player_info_core: Option<PlayerInfoCoreState>,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        let auto_generate_teams = self.team_state.team_configuration.auto_generate_teams;
        let generated_team_is_valid = auto_generate_teams
            && config
                .team
                .filter(|team_id| *team_id != 0)
                .is_some_and(|team_id| self.resolve_or_generate_runtime_team(team_id));
        let has_valid_team = generated_team_is_valid
            || config
                .team
                .is_some_and(|team_id| self.team_state.teams.iter().any(|team| team.id == team_id));
        if self.team_state.runtime_join_team_choice && !has_valid_team && !semantics.script_player {
            let number = self.join_player_for_team_selection_at_client_with_name(
                config,
                at_client,
                at_client_name,
                runtime_control,
                semantics.no_elimination_check,
                semantics.league_score,
                semantics.league_progress_data.as_deref(),
                player_info_core,
            )?;
            return Ok(JoinPlayerOutcome::AwaitingTeamSelection { number });
        }
        let number = self.register_joining_player(
            &config,
            at_client,
            &at_client_name,
            runtime_control,
            semantics.no_elimination_check,
            semantics.league_score,
            semantics.league_progress_data.as_deref(),
            player_info_core,
        );
        if let Some(player) = self.players.get_mut(&number) {
            player.set_script_player(semantics.script_player);
        }
        if !semantics.scenario_init {
            if config.team.is_some() {
                self.set_player_team_hostility(number);
            }
            self.sync_team_home_base_for(number);
            if let Some(extra_id) = semantics.extra_id.as_ref() {
                self.initialize_script_player_from_definition(number, config.team, extra_id)?;
            }
            self.finalize_joining_player(number, true, true)?;
            return Ok(JoinPlayerOutcome::Initialized(JoinedPlayer {
                number,
                start_x: 0,
                start_y: 0,
                first_base: None,
            }));
        }
        self.preinitialize_joining_player(number)?;
        let joined = self.scenario_init_for_player(number, &config, semantics.extra_id.as_ref())?;
        self.finalize_joining_player(number, true, true)?;
        Ok(JoinPlayerOutcome::Initialized(joined))
    }

    /// Registers a teamless user while runtime team choice is active, then
    /// stops before `ScenarioInit` until a synchronized team-selection
    /// control resumes the join (`C4Player.cpp:299-320, 344-349`).
    pub fn join_player_for_team_selection(
        &mut self,
        config: JoinPlayerConfig,
    ) -> Result<i32, EngineError> {
        self.join_player_for_team_selection_at_client(config, PlayerAtClient::HOST)
    }

    fn join_player_for_team_selection_at_client(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
    ) -> Result<i32, EngineError> {
        self.join_player_for_team_selection_at_client_with_name(
            config,
            at_client,
            "Local".to_string(),
            PlayerRuntimeControl::NONE,
            false,
            None,
            None,
            None,
        )
    }

    fn join_player_for_team_selection_at_client_with_name(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: String,
        runtime_control: PlayerRuntimeControl,
        no_elimination_check: bool,
        league_score: Option<i32>,
        league_progress_data: Option<&[u8]>,
        player_info_core: Option<PlayerInfoCoreState>,
    ) -> Result<i32, EngineError> {
        let number = self.register_joining_player(
            &config,
            at_client,
            &at_client_name,
            runtime_control,
            no_elimination_check,
            league_score,
            league_progress_data,
            player_info_core,
        );
        self.player_mut(number)?
            .set_status(PlayerStatus::TeamSelection);
        self.preinitialize_joining_player(number)?;
        // Game::JoinPlayer/Game::InitGameFinal calls FinalInit even while the
        // player is awaiting a team. ScenarioAndTeamInit performs the second,
        // non-initial FinalInit after the synchronized choice.
        self.finalize_joining_player(number, true, true)?;
        Ok(number)
    }

    /// Marks the local menu choice as waiting for its synchronized
    /// `CID_InitScenarioPlayer` control (`C4Player::DoTeamSelection`,
    /// C4Player.cpp:1774-1780).
    pub fn mark_team_selection_pending(&mut self, number: i32) -> Result<(), EngineError> {
        self.player_mut(number)?
            .set_status(PlayerStatus::TeamSelectionPending);
        Ok(())
    }

    /// Executes the synchronized `InitScenarioPlayer(player, team)` call.
    /// `C4Player::ScenarioAndTeamInit` accepts every live player with a
    /// C4PlayerInfo, including one whose initial ScenarioInit already ran.
    /// `Ok(None)` mirrors C++'s false return for a missing info/team; a
    /// pending player returns to the selection menu and may retry.
    pub fn initialize_scenario_player(
        &mut self,
        number: i32,
        team: i32,
    ) -> Result<Option<JoinedPlayer>, EngineError> {
        if !self
            .players
            .get(&number)
            .is_some_and(|player| player.player_info_id() != 0)
        {
            return Ok(None);
        }
        let Some(mut config) = self.pending_player_joins.get(&number).cloned() else {
            return Ok(None);
        };
        let selected_team_id = match team {
            -1 => self.generate_runtime_team(),
            0 => None,
            id => Some(id),
        };
        let selected_team = selected_team_id.and_then(|id| {
            self.team_state
                .teams
                .iter()
                .find(|candidate| candidate.id == id)
        });
        let previous_team = self.player(number).and_then(Player::team);
        let team_is_full = selected_team.is_some_and(|selected| {
            previous_team != Some(selected.id) && self.team_is_full(selected)
        });
        if team != 0 && (selected_team.is_none() || team_is_full) {
            if self
                .player(number)
                .is_some_and(|player| player.status() == PlayerStatus::TeamSelectionPending)
            {
                self.player_mut(number)?
                    .set_status(PlayerStatus::TeamSelection);
            }
            return Ok(None);
        }

        config.team = selected_team.map(|selected| selected.id);
        let selected_team_color = selected_team
            .filter(|selected| {
                self.team_state.team_configuration.team_colors && selected.color != 0
            })
            .map(|selected| selected.color);
        self.player_mut(number)?.set_team(config.team);
        if let Some(color) = selected_team_color {
            config.color_dw = color;
            self.set_player_color(number, color)?;
        }
        self.recheck_runtime_team_memberships();
        let joined = self.scenario_init_for_player(number, &config, None)?;
        self.finalize_joining_player(number, false, true)?;
        self.pending_player_joins.insert(number, config);
        Ok(Some(joined))
    }

    /// `C4TeamList::GetGenerateTeamByID`: resolve `TEAMID_New`, or create each
    /// sequential default team through a requested positive ID
    /// (C4Teams.cpp:386-420).
    fn resolve_or_generate_runtime_team(&mut self, team_id: i32) -> bool {
        if !self.team_state.team_configuration.active {
            return false;
        }
        let team_id = if team_id == -1 {
            let largest_team_id = self
                .team_state
                .teams
                .iter()
                .map(|team| team.id)
                .max()
                .unwrap_or(0);
            let Some(team_id) = largest_team_id.checked_add(1) else {
                return false;
            };
            team_id
        } else {
            team_id
        };
        while self.team_state.team_last_team_id < team_id {
            if self.generate_runtime_team().is_none() {
                return false;
            }
        }
        self.team_state.teams.iter().any(|team| team.id == team_id)
    }

    pub(crate) fn generate_runtime_team(&mut self) -> Option<i32> {
        if !self.team_state.team_configuration.auto_generate_teams {
            return None;
        }
        let id = self.team_state.team_last_team_id.checked_add(1)?;
        // Higher IDs require C++'s process-global SafeRandom color search.
        // Keep zero as an explicit unresolved marker rather than consuming
        // the lockstep RNG or inventing a color; callers do not apply zero
        // to the player while host-color transport remains open.
        let color = default_generated_team_color(id).unwrap_or(0);
        Rc::make_mut(&mut self.team_state.teams).push(TeamInfo::new(
            id,
            format!("Team {id}"),
            color,
        ));
        self.team_state.team_last_team_id = id;
        Some(id)
    }

    pub(crate) fn team_is_full(&self, team: &TeamInfo) -> bool {
        team.max_players != 0
            && self
                .players
                .values()
                .filter(|player| player.team() == Some(team.id))
                .count()
                >= team.max_players.max(0) as usize
    }

    /// `C4Player::SetPlayerColor`: update the runtime display color and any
    /// live owned object that still carries the old player-color RGB while
    /// preserving its alpha byte (C4Player.cpp:2263-2281).
    pub(crate) fn set_player_color(&mut self, number: i32, color: u32) -> Result<(), EngineError> {
        let player = self
            .player(number)
            .ok_or(EngineError::UnknownPlayer(number))?;
        let old_color_dw = player.color_dw();
        let old_color = player.color();
        if old_color_dw == color {
            return Ok(());
        }
        self.player_mut(number)?.set_color_dw(color);
        if let Some(old_color) = old_color {
            let old_color = (u32::from(old_color.r) << 16)
                | (u32::from(old_color.g) << 8)
                | u32::from(old_color.b);
            let new_color = color & 0x00ff_ffff;
            for object in &mut self.objects {
                if object.state.status.is_active()
                    && object.state.owner == number
                    && object.state.color & 0x00ff_ffff == old_color
                {
                    object.state.color = object.state.color & 0xff00_0000 | new_color;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn register_joining_player(
        &mut self,
        config: &JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: &str,
        runtime_control: PlayerRuntimeControl,
        no_elimination_check: bool,
        league_score: Option<i32>,
        league_progress_data: Option<&[u8]>,
        player_info_core: Option<PlayerInfoCoreState>,
    ) -> i32 {
        // C4PlayerList::GetFreeNumber: lowest unused player number.
        let number = self.next_player_number();

        let color = RgbColor::new(
            ((config.color_dw >> 16) & 0xff) as u8,
            ((config.color_dw >> 8) & 0xff) as u8,
            (config.color_dw & 0xff) as u8,
        );
        let player_info_id = self.assign_player_info_id(config.player_info_id);
        if player_info_id != 0 {
            if let Some(score) = league_score {
                let scores = Rc::make_mut(&mut self.player_info_league_scores);
                if score == 0 {
                    scores.remove(&player_info_id);
                } else {
                    scores.insert(player_info_id, score);
                }
            }
            let progress_data = Rc::make_mut(&mut self.player_info_league_progress_data);
            if let Some(bytes) = league_progress_data {
                progress_data.insert(player_info_id, Some(bytes.to_vec()));
            } else {
                progress_data.entry(player_info_id).or_insert(None);
            }
        }
        let mut player_config = PlayerConfig::new(number, config.name.clone())
            .with_player_info_id(player_info_id)
            .with_score(config.score)
            .with_rounds(config.rounds, config.rounds_won, config.rounds_lost)
            .with_total_playing_time(config.total_playing_time);
        if config.team.is_some() {
            player_config = player_config.with_team(config.team);
        }
        let mut player = player_config.with_color(Some(color)).build();
        player.set_color_dw(config.color_dw);
        player.set_at_client(at_client);
        player.set_at_client_name(at_client_name);
        player.set_no_elimination_check(no_elimination_check);
        player.set_game_join_time(self.game_time);
        player.set_runtime_control(runtime_control.control_set, runtime_control.mouse_control);
        player.set_control_preferences(
            runtime_control.preferred_control_set,
            runtime_control.prefers_mouse,
        );
        player.set_control_style_preferences(config.control_style, config.auto_context_menu);
        let mut player_info_core = player_info_core.unwrap_or_default();
        player_info_core.score = config.score;
        player_info_core.rounds = config.rounds;
        player_info_core.rounds_won = config.rounds_won;
        player_info_core.rounds_lost = config.rounds_lost;
        player_info_core.total_playing_time = config.total_playing_time;
        let preferred_control_style_value =
            if (player_info_core.pref_control_style_value != 0) == config.control_style {
                player_info_core.pref_control_style_value
            } else {
                i32::from(config.control_style)
            };
        let preferred_auto_context_menu_value =
            if (player_info_core.pref_auto_context_menu_value != 0) == config.auto_context_menu {
                player_info_core.pref_auto_context_menu_value
            } else {
                i32::from(config.auto_context_menu)
            };
        player.set_player_info_core(player_info_core);
        // C4Player::InitControl (C4Player.cpp:1747, 2371-2380): flash both
        // markers and let ForcedControlStyle override the player preference.
        player.control.select_flash = 30;
        player.control.cursor_flash = 30;
        let control_style_value = self
            .forced_control_style
            .map(i32::from)
            .unwrap_or(preferred_control_style_value);
        let control_style = control_style_value != 0;
        let activates_auto_stop = player.control.control_style != control_style && control_style;
        player.control.set_control_style_value(control_style_value);
        let auto_context_menu_value = self
            .forced_auto_context_menu
            .map(i32::from)
            .unwrap_or(preferred_auto_context_menu_value);
        player
            .control
            .set_auto_context_menu_value(auto_context_menu_value);
        // The new player's CrewInfoList owns fresh runtime-only control
        // counters even when a departed player previously used this number.
        self.crew_info_control_counts
            .retain(|link, _| link.player_id != number);
        self.players.insert(number, player);
        self.append_and_recheck_player_order(number);
        self.actualize_ownerless_fow_objects_for_new_player();
        self.recheck_runtime_team_memberships();
        if activates_auto_stop {
            self.reset_inactive_crew_command_directions(number);
        }
        self.players_registered = true;
        self.crew_rosters.insert(number, config.crew.clone());
        self.crew_info_order
            .insert(number, (0..config.crew.len()).rev().collect());
        self.bootstrap_player_crew_from_union(number);
        self.sync_player_cursor(number);
        self.pending_player_joins.insert(number, config.clone());
        number
    }

    /// Append a freshly constructed player and reproduce
    /// `C4PlayerList::RecheckPlayerSort` verbatim (C4PlayerList.cpp:597-627).
    /// The native scan is intentionally not equivalent to a full sort: when a
    /// reused lower number is appended directly after the current head, the
    /// scan reaches the new player itself and returns without moving it.
    pub(crate) fn append_and_recheck_player_order(&mut self, number: i32) {
        self.player_order.push(number);
        if self.player_order.len() == 1 {
            return;
        }

        let joining_index = self.player_order.len() - 1;
        let mut previous_index = 0;
        while previous_index + 1 < self.player_order.len()
            && self.player_order[previous_index + 1] <= number
        {
            previous_index += 1;
        }
        if previous_index == joining_index {
            return;
        }

        let joining = self
            .player_order
            .pop()
            .expect("fresh player was appended to the order ledger");
        if previous_index == 0 && self.player_order[previous_index] > number {
            self.player_order.insert(0, joining);
        } else {
            self.player_order.insert(previous_index + 1, joining);
        }
    }

    /// Project the private link ledger while tolerating legacy test fixtures
    /// that still mutate the public player map directly. Known ledger entries
    /// keep their order; otherwise-untracked live players append by number.
    pub(crate) fn player_ids_in_order(&self) -> Vec<i32> {
        let mut order = Vec::with_capacity(self.players.len());
        if self.player_order.len() == self.players.len()
            && self
                .player_order
                .iter()
                .all(|number| self.players.contains_key(number))
        {
            order.extend(self.player_order.iter().copied());
            return order;
        }

        let mut seen = HashSet::with_capacity(self.players.len());
        order.extend(
            self.player_order
                .iter()
                .copied()
                .filter(|number| self.players.contains_key(number) && seen.insert(*number)),
        );
        let missing_start = order.len();
        order.extend(
            self.players
                .keys()
                .copied()
                .filter(|number| seen.insert(*number)),
        );
        order[missing_start..].sort_unstable();
        order
    }

    /// C4PlayerList::GetFreeNumber: the lowest unused non-negative player
    /// number. Frontends use this to reserve a local input assignment before
    /// calling a join method; the single-threaded join then consumes exactly
    /// this number.
    pub fn next_player_number(&self) -> i32 {
        (0..)
            .find(|candidate| !self.players.contains_key(candidate))
            .unwrap_or_default()
    }

    /// Overwrite the process-local `Control`/`MouseControl` projection. This
    /// is also the restore hook corresponding to C++ `InitControl`, which runs
    /// after loading runtime player data.
    pub fn set_player_runtime_control(
        &mut self,
        number: i32,
        runtime_control: PlayerRuntimeControl,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(number)?;
        player.set_runtime_control(runtime_control.control_set, runtime_control.mouse_control);
        player.set_control_preferences(
            runtime_control.preferred_control_set,
            runtime_control.prefers_mouse,
        );
        Ok(())
    }

    /// Savegame recreation's post-load `InitControl` pass. AtClient/name are
    /// authoritative join parameters, Control is always recomputed,
    /// MouseControl is only ever set true (a loaded nonzero value survives a
    /// failed preference gate), forced control preferences are reapplied, and
    /// held synchronized com bits are cleared (C4Player.cpp:354-386,
    /// 1871-1918).
    pub fn reinitialize_player_after_restore(
        &mut self,
        number: i32,
        at_client: PlayerAtClient,
        at_client_name: impl Into<String>,
        player_name: impl Into<String>,
        runtime_control: PlayerRuntimeControl,
        script_player: bool,
        no_elimination_check: bool,
        pref_control_style: bool,
        pref_auto_context_menu: bool,
    ) -> Result<(), EngineError> {
        let forced_control_style = self.forced_control_style;
        let forced_auto_context_menu = self.forced_auto_context_menu;
        let clear_inactive_com_dir = {
            let player = self.player_mut(number)?;
            player.set_at_client(at_client);
            player.set_at_client_name(at_client_name);
            player.set_name(player_name);
            player.set_script_player(script_player);
            player.set_no_elimination_check(no_elimination_check);
            let mouse_control = if runtime_control.mouse_control != 0 {
                1
            } else {
                player.mouse_control()
            };
            player.set_runtime_control(runtime_control.control_set, mouse_control);
            player.set_control_preferences(
                runtime_control.preferred_control_set,
                runtime_control.prefers_mouse,
            );
            let preferred_control_style_value = player
                .player_info_core()
                .map(|core| core.pref_control_style_value)
                .filter(|raw| (*raw != 0) == pref_control_style)
                .unwrap_or_else(|| i32::from(pref_control_style));
            let preferred_auto_context_menu_value = player
                .player_info_core()
                .map(|core| core.pref_auto_context_menu_value)
                .filter(|raw| (*raw != 0) == pref_auto_context_menu)
                .unwrap_or_else(|| i32::from(pref_auto_context_menu));
            player.set_control_style_preferences(pref_control_style, pref_auto_context_menu);
            let control_style_value = forced_control_style
                .map(i32::from)
                .unwrap_or(preferred_control_style_value);
            let control_style = control_style_value != 0;
            let auto_context_menu_value = forced_auto_context_menu
                .map(i32::from)
                .unwrap_or(preferred_auto_context_menu_value);
            let changed = player.control.control_style != control_style;
            if changed {
                player.control.last_com = i32::from(crate::control::COM_NONE);
            }
            player.control.set_control_style_value(control_style_value);
            player
                .control
                .set_auto_context_menu_value(auto_context_menu_value);
            player.control.pressed_coms = 0;
            changed && control_style
        };
        if clear_inactive_com_dir {
            self.reset_inactive_crew_command_directions(number);
        }
        Ok(())
    }

    fn reset_inactive_crew_command_directions(&mut self, owner: i32) {
        let definitions = &self.definitions;
        for object in &mut self.objects {
            if object.state.status == ObjectStatus::Inactive
                && object.state.owner == owner
                && definitions
                    .get(&object.definition_id)
                    .is_some_and(Definition::is_crew)
            {
                object.state.command_direction = CommandDirection::Stop;
            }
        }
    }

    /// Apply the runtime Options-menu mouse toggle, including the automatic
    /// (non-forced) FoW and scrolling-mode transitions owned by
    /// `C4Player::ToggleMouseControl`.
    pub fn set_player_mouse_control(
        &mut self,
        number: i32,
        enabled: bool,
    ) -> Result<(), EngineError> {
        if self.player_mut(number)?.apply_mouse_control_toggle(enabled) {
            self.rebuild_fow_view_objects();
        }
        Ok(())
    }

    fn preinitialize_joining_player(&mut self, number: i32) -> Result<(), EngineError> {
        // Player preinit broadcast before ScenarioInit (C4Player.cpp:347;
        // fail-safe exec, the join continues on script errors).
        tolerate_script_error(
            self.broadcast_scenario_function("PreInitializePlayer", vec![Value::Int(number)]),
        )
        .map(|_| ())
    }

    fn initialize_script_player_from_definition(
        &mut self,
        number: i32,
        team: Option<i32>,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        let Some((script_name, script)) = self
            .definitions
            .get(definition_id)
            .filter(|definition| definition.has_function("InitializeScriptPlayer"))
            .map(|definition| (definition.id.clone(), definition.script_arc()))
        else {
            return Ok(());
        };
        let world = self.host_world_context();
        let (_value, _args, batch, audio_state, rng, script_error) =
            ScenarioScript::call_value_for_script(
                &script_name,
                &script,
                Some(definition_id.clone()),
                "InitializeScriptPlayer",
                &[Value::Int(number), Value::Int(team.unwrap_or(0))],
                world,
                self.rng.clone(),
                self.frame,
                &self.global_effects.clone(),
                self.physics,
                self.environment,
                self.audio_registry.clone(),
                self.game_over_triggered,
            );
        self.rng = rng;
        self.audio_registry = audio_state;
        self.apply_scenario_batch(batch)?;
        if let Some(error) = script_error {
            tolerate_script_error::<()>(Err(error))?;
        }
        Ok(())
    }

    fn finalize_joining_player(
        &mut self,
        number: i32,
        initial_value: bool,
        check_elimination: bool,
    ) -> Result<(), EngineError> {
        let status = self
            .players
            .get(&number)
            .map(Player::status)
            .ok_or(EngineError::UnknownPlayer(number))?;
        if status == PlayerStatus::Inactive {
            return Ok(());
        }
        self.crew_ranks = Rc::new(
            self.crew_object_infos
                .iter()
                .map(|(id, info)| (id.as_u64(), info.rank))
                .collect(),
        );
        // FinalInit(true) establishes InitialValue from one UpdateValue
        // pass; restore/team-selection finalization skips that reset.
        if initial_value {
            self.update_player_asset_value(number)?;
            if let Some(player) = self.players.get_mut(&number) {
                player.reset_initial_value();
            }
        }

        // FinalInit preserves a cursor explicitly installed by
        // InitializePlayer. Otherwise AdjustCursorCommand prefers the
        // highest-ranked already-selected crew and only then all crew.
        if self.crew_cursor(number).is_none() {
            self.player_adjust_cursor_command(number)?;
        }
        self.sync_player_cursor(number);

        self.assign_player_captain_on_final_init(number);

        // FinalInit always performs another UpdateValue, followed by one
        // immediate Player::Execute. If the global Tick35 is active that
        // Execute performs the third value pass before delays decay.
        self.update_player_asset_value(number)?;
        let _ = self.execute_one_player(number, check_elimination)?;
        Ok(())
    }

    /// C4Game::InitGameFinal restore phase: after every recreated player has
    /// rerun InitControl, FinalInit executes in `C4PlayerList` order.
    pub fn finalize_restored_players(
        &mut self,
        establish_initial_value: bool,
    ) -> Result<(), EngineError> {
        self.finalize_restored_object_links()?;
        self.finalize_restored_player_initialization(establish_initial_value)
    }

    /// The object half of InitGameFinal always runs after player recreation,
    /// even when every attempted player join failed.
    pub fn finalize_restored_object_links(&mut self) -> Result<(), EngineError> {
        self.finalize_legacy_object_links_unconditionally()
    }

    /// Run restored-player `FinalInit(!Head.SaveGame)` after the optional
    /// scenario constructor. C++ places this after `Script.Initialize` for
    /// regular restore-info scenarios (C4Game.cpp:2724-2739).
    pub fn finalize_restored_player_initialization(
        &mut self,
        establish_initial_value: bool,
    ) -> Result<(), EngineError> {
        let players = self.player_ids_in_order();
        for player in players {
            self.finalize_joining_player(player, establish_initial_value, true)?;
        }
        Ok(())
    }

    /// `C4Player::FinalInit` assigns Captain once, after ready crew and the
    /// InitializePlayer callback, but only while a live KILC rule exists
    /// (C4Player.cpp:778-803). Later selection/rank changes never re-elect.
    fn assign_player_captain_on_final_init(&mut self, number: i32) {
        let Some(player) = self.players.get(&number) else {
            return;
        };
        if player.status() == PlayerStatus::Inactive || player.captain().is_some() {
            return;
        }
        let kill_the_captain_active = self.objects.iter().any(|object| {
            !object.destroyed
                && object.state.status.is_active()
                && object.definition_id.as_str() == "KILC"
        });
        if !kill_the_captain_active {
            return;
        }
        let captain = self.player_hi_rank_active_crew(number, false);
        if let Some(player) = self.players.get_mut(&number) {
            player.set_captain(captain);
        }
    }

    /// `C4Player::ScenarioInit` (C4Player.cpp:670-777). The RNG draw order
    /// is load-bearing: Wealth.Evaluate, optional MapZoom.Evaluate per
    /// configured coordinate, the all-random start position, then the
    /// PlaceReady* draws in Base/Material/Vehicles/Crew order.
    fn scenario_init_for_player(
        &mut self,
        number: i32,
        config: &JoinPlayerConfig,
        extra_id: Option<&DefinitionId>,
    ) -> Result<JoinedPlayer, EngineError> {
        // Start index by player number, overridden by the team's one-based
        // PlrStartIndex when configured (C4Player.cpp:670-677).
        let start_index = config
            .team
            .and_then(|team_id| self.team_state.teams.iter().find(|team| team.id == team_id))
            .map(|team| team.player_start_index)
            .filter(|index| *index != 0)
            .and_then(|index| usize::try_from(index - 1).ok())
            .unwrap_or_else(|| (number.max(0) as usize) % scenario::MAX_PLAYER_STARTS);
        let start = self
            .player_starts
            .get(start_index)
            .cloned()
            .unwrap_or_default();

        // Indexed color (C4Player.cpp:678-685): take PrefColor unless
        // another player owns it; C4MaxColor = 12 (C4Constants.h:38).
        const C4_MAX_COLOR: i32 = 12;
        let mut color_index = config.pref_color.clamp(0, C4_MAX_COLOR - 1);
        while self
            .players
            .values()
            .any(|player| player.id() != number && player.color_index() == color_index)
        {
            color_index = (color_index + 1) % C4_MAX_COLOR;
            if color_index == config.pref_color {
                break;
            }
        }

        // Wealth, home base material/production, knowledge and magic
        // (C4Player.cpp:702-711); ConsolidateValids keeps known defs only.
        let wealth = start.wealth.evaluate(&mut self.rng);
        let valid_entries = |entries: &[(String, i32)]| -> Vec<(DefinitionId, i32)> {
            entries
                .iter()
                .filter(|(id, _)| {
                    self.definitions
                        .contains_key(&DefinitionId::from(id.as_str()))
                })
                .map(|(id, count)| (DefinitionId::from(id.as_str()), *count))
                .collect()
        };
        let home_base_material = valid_entries(&start.home_base_material);
        let home_base_production = valid_entries(&start.home_base_production);
        let knowledge = valid_entries(&start.build_knowledge);
        let mut magic = valid_entries(&start.magic);
        if magic.is_empty() {
            magic = self
                .runtime_definition_order
                .iter()
                .filter(|id| {
                    self.definitions
                        .get(*id)
                        .is_some_and(|definition| definition.category() & CATEGORY_MAGIC != 0)
                })
                .cloned()
                .map(|id| (id, 0))
                .collect();
        }
        magic.sort_by_key(|(id, _)| {
            self.definitions
                .get(id)
                .map(Definition::value)
                .unwrap_or_default()
        });
        {
            let player = self.player_mut(number)?;
            player.set_status(PlayerStatus::Active);
            player.set_color_index(color_index);
            player.set_wealth(wealth);
            player.set_home_base_material_entries(home_base_material);
            player.set_home_base_production_entries(home_base_production);
            player.set_knowledge_entries(knowledge);
            player.set_magic_entries(magic);
        }
        // Starting position (C4Player.cpp:713-755).
        let (world_width, world_height) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        let mut ptx = start.position[0];
        let mut pty = start.position[1];
        if ptx > -1 {
            let zoom = self.map_zoom.evaluate(&mut self.rng);
            ptx = (ptx * zoom).clamp(0, (world_width - 1).max(0));
        }
        if pty > -1 {
            let zoom = self.map_zoom.evaluate(&mut self.rng);
            pty = (pty * zoom).clamp(0, (world_height - 1).max(0));
        }
        // Standard position by PrefPosition (C4Player.cpp:717-732);
        // C4P_MaxPosition = 4 (C4Constants.h:82).
        if ptx < 0 && config.startup_player_count >= 2 {
            const C4P_MAX_POSITION: i32 = 4;
            let max_pos = config.startup_player_count;
            let start_pos =
                (config.pref_position * max_pos / C4P_MAX_POSITION).clamp(0, max_pos - 1);
            let mut position = start_pos;
            while self
                .players
                .values()
                .any(|player| player.id() != number && player.position_index() == position)
            {
                position = (position + 1) % max_pos;
                if position == start_pos {
                    break;
                }
            }
            self.player_mut(number)?.set_position_index(position);
            ptx = (16 + position * (world_width - 32) / (max_pos - 1)).clamp(0, world_width - 16);
        }
        // All-random position (C4Player.cpp:745-746) — synced draws.
        if ptx < 0 {
            ptx = 16 + self.rng.random(world_width - 32);
        }
        if pty < 0 {
            pty = 16 + self.rng.random(world_height - 32);
        }
        // Settle on solid ground, then a construction-site spot
        // (C4Player.cpp:748-755).
        if !start.enforce_position {
            if let Some((nx, ny)) = self
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.find_solid_ground(ptx, pty, 30))
            {
                ptx = nx;
                pty = ny;
            }
            if let Some((nx, ny)) =
                self.placement_con_site_spot(ptx, pty, 30, 50, CATEGORY_STRUCTURE, 400)
            {
                ptx = nx;
                pty = ny;
            }
        }

        // Place readies in C++ order (C4Player.cpp:757-760).
        let mut first_base: Option<ObjectId> = None;
        self.place_ready_base(number, &start, &mut ptx, &mut pty, &mut first_base)?;
        self.place_ready_material(number, &start, ptx - 10, ptx + 10, pty, first_base)?;
        self.place_ready_vehic(number, &start, ptx - 30, ptx + 30, pty, first_base)?;
        self.place_ready_crew(number, &start, ptx - 30, ptx + 30, pty, first_base)?;

        if config.team.is_some() {
            self.set_player_team_hostility(number);
        }

        // Mouse-controlled players automatically enable unforced FoW after
        // ready placement and before InitializePlayer. A PreInitializePlayer
        // SetFoW call has already set the force flag and wins this race.
        if self
            .players
            .get_mut(&number)
            .is_some_and(Player::initialize_mouse_fog_of_war)
        {
            self.rebuild_fow_view_objects();
        }

        // Scenario script init broadcast (C4Player.cpp:769-775): fail-safe
        // exec, the join never aborts on script errors.
        let base_value = first_base.map(object_reference_value).unwrap_or(Value::Nil);
        tolerate_script_error(self.broadcast_scenario_function(
            "InitializePlayer",
            vec![
                Value::Int(number),
                Value::Int(ptx),
                Value::Int(pty),
                base_value,
                Value::Int(config.team.unwrap_or(0)),
                extra_id
                    .map(|id| Value::C4Id(id.as_str().to_string()))
                    .unwrap_or(Value::Nil),
            ],
        ))?;
        // Team home-base state overwrites the player's ScenarioInit material
        // only after InitializePlayer returns (C4Player.cpp:349-352).
        self.sync_team_home_base_for(number);

        Ok(JoinedPlayer {
            number,
            start_x: ptx,
            start_y: pty,
            first_base,
        })
    }

    /// Resolves a PlayerStart ID token against the loaded definitions with
    /// the legacy loader's semantics (C4Id match first, then a lenient
    /// name match — mirrors find_definition_by_token). Ties resolve to the
    /// lexicographically smallest id for determinism.
    fn resolve_definition_token(&self, token: &str) -> Option<DefinitionId> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return None;
        }
        if trimmed.len() == 4
            && trimmed
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            if let Some(id) = self
                .definitions
                .keys()
                .filter(|id| id.as_str().eq_ignore_ascii_case(trimmed))
                .min_by(|a, b| a.as_str().cmp(b.as_str()))
            {
                return Some(id.clone());
            }
        }
        self.definitions
            .iter()
            .filter(|(id, definition)| {
                id.as_str().eq_ignore_ascii_case(trimmed)
                    || definition.name().eq_ignore_ascii_case(trimmed)
            })
            .map(|(id, _)| id.clone())
            .min_by(|a, b| a.as_str().cmp(b.as_str()))
    }

    /// `Game.OverlapObject` (C4Game.cpp:1298-1313): any active,
    /// uncontained object whose category intersects `category` within
    /// C4D_SortLimit and whose shape rect overlaps the given rect.
    fn placement_overlaps_object(
        objects: &[Object],
        x: i32,
        y: i32,
        wdt: i32,
        hgt: i32,
        category: i32,
    ) -> bool {
        Self::placement_overlapping_object(objects, x, y, wdt, hgt, category).is_some()
    }

    /// `Game.OverlapObject`'s returned blocker; ConstructionCheck names it
    /// in the IDS_OBJ_NOOTHER feedback (C4Landscape.cpp:2159-2163).
    pub(crate) fn placement_overlapping_object(
        objects: &[Object],
        x: i32,
        y: i32,
        wdt: i32,
        hgt: i32,
        category: i32,
    ) -> Option<ObjectId> {
        objects
            .iter()
            .find(|object| {
                if !object.state.status.is_active()
                    || object.destroyed
                    || object.state.container.is_some()
                {
                    return false;
                }
                if object.state.category & category & CATEGORY_SORT_LIMIT == 0 {
                    return false;
                }
                let position = object.state.position;
                let rect = object
                    .current_shape_rect()
                    .map(|rect| {
                        DefinitionRect::new(
                            position.x.saturating_add(rect.x),
                            position.y.saturating_add(rect.y),
                            rect.width,
                            rect.height,
                        )
                    })
                    .or_else(|| vertex_bounds_rect(position, &object.state.vertices))
                    .unwrap_or_else(|| DefinitionRect::new(position.x, position.y, 1, 1));
                x < rect.x + rect.width
                    && rect.x < x + wdt
                    && y < rect.y + rect.height
                    && rect.y < y + hgt
            })
            .map(|object| object.id)
    }

    /// `FindConSiteSpot` with the engine-side object-overlap veto
    /// (C4Landscape.cpp:1982-2043 + C4Game.cpp:1298-1313).
    fn placement_con_site_spot(
        &self,
        x: i32,
        y: i32,
        wdt: i32,
        hgt: i32,
        category: i32,
        hrange: i32,
    ) -> Option<(i32, i32)> {
        let landscape = self.landscape.as_ref()?;
        let objects = &self.objects;
        landscape.find_con_site_spot(x, y, wdt, hgt, hrange, |rx, ry, rw, rh| {
            Self::placement_overlaps_object(objects, rx, ry, rw, rh, category)
        })
    }

    fn definition_shape_size(&self, id: &DefinitionId) -> (i32, i32) {
        self.definitions
            .get(id)
            .and_then(|definition| definition.shape_rect())
            .map(|rect| (rect.width, rect.height))
            .unwrap_or((0, 0))
    }

    /// The `fTerrain` arm of `C4Game::CreateObjectConstruction` before
    /// `NewObject`: clear the structure rectangle, lift nearby ground to its
    /// bottom edge, then draw a configured granite basement
    /// (C4Game.cpp:1191-1227; C4Landscape.cpp:1064-1090).
    pub(crate) fn prepare_construction_terrain(
        &mut self,
        center_x: i32,
        bottom_y: i32,
        width: i32,
        height: i32,
        basement: i32,
    ) {
        let x = center_x.saturating_sub(width / 2);
        let y = bottom_y.saturating_sub(height);
        if width.saturating_mul(height) < 12_000 {
            self.execute_dig_rect_operation(Vector2::new(x, y), width, height, false, None);
        }
        self.raise_terrain(x, bottom_y, width);

        let Some(granite) = self.materials.id_of("Granite") else {
            return;
        };
        const BASEMENT_STRENGTH: i32 = 8;
        if basement > 1 {
            let border_width = basement.min(width);
            self.draw_material_rect(granite, x, bottom_y, border_width, BASEMENT_STRENGTH);
            self.draw_material_rect(
                granite,
                x.saturating_add(width).saturating_sub(border_width),
                bottom_y,
                border_width,
                BASEMENT_STRENGTH,
            );
        } else if basement != 0 {
            self.draw_material_rect(granite, x, bottom_y, width, BASEMENT_STRENGTH);
        }
    }

    /// `C4Landscape::RaiseTerrain`: for each column, copy the first solid
    /// pixel upward when it lies less than 20 pixels below the requested
    /// construction bottom. Vehicle pixels are deliberately never copied.
    fn raise_terrain(&mut self, x: i32, y: i32, width: i32) {
        let materials = &self.materials;
        let vehicle = materials.id_of("Vehicle");
        let Some(landscape) = self.landscape.as_mut() else {
            return;
        };

        if let Some((grid_width, grid_height)) = landscape.grid_dimensions() {
            for column in x..x.saturating_add(width) {
                let mut target_y = y;
                while target_y + 1 < grid_height
                    && landscape.density_at(column, target_y + 1, materials) < C4M_SOLID
                {
                    target_y += 1;
                }
                if target_y + 1 >= grid_height || target_y - y >= 20 {
                    continue;
                }
                let Some(pixel) = landscape.grid_byte_at(column, target_y + 1) else {
                    continue;
                };
                if vehicle.is_some_and(|vehicle| {
                    landscape.border_material_at(column, target_y + 1) == Some(vehicle)
                }) {
                    continue;
                }
                while target_y >= y {
                    landscape.grid_set_byte(column, target_y, pixel);
                    target_y -= 1;
                }
            }
            let start = x.max(0).min(grid_width) as usize;
            let end = x.saturating_add(width).max(0).min(grid_width) as usize;
            landscape.refresh_raster_columns(start..end);
            return;
        }

        // Column-only fixture fallback: the first solid pixel is the scalar
        // surface. Copying it upward is exactly a surface-height change.
        for column in x..x.saturating_add(width) {
            let Ok(column_index) = u32::try_from(column) else {
                continue;
            };
            let Some(&surface) = landscape.surface().get(column as usize) else {
                continue;
            };
            if surface.saturating_sub(1).saturating_sub(y) < 20 {
                landscape.set_height(column_index, y);
            }
        }
    }

    /// `C4Player::PlaceReadyBase` (C4Player.cpp:580-617). Power-line
    /// auto-connections (C4RULE_StructuresNeedEnergy, :608-616) are not
    /// ported yet — they need the CreateLine object machinery.
    fn place_ready_base(
        &mut self,
        number: i32,
        start: &scenario::PlayerStart,
        tx: &mut i32,
        ty: &mut i32,
        first_base: &mut Option<ObjectId>,
    ) -> Result<(), EngineError> {
        for (token, count) in &start.ready_base {
            let Some(definition_id) = self.resolve_definition_token(token) else {
                continue;
            };
            let Some(definition) = self.definitions.get(&definition_id) else {
                continue;
            };
            let (wdt, hgt) = definition
                .shape_rect()
                .map(|rect| (rect.width, rect.height))
                .unwrap_or((0, 0));
            let category = definition.category();
            let can_be_base = definition.can_be_base();
            let basement = definition.basement();
            for _ in 0..(*count).max(0) {
                let mut ctx = *tx;
                let mut cty = *ty;
                let mut placeable = start.enforce_position;
                if !placeable {
                    if let Some((nx, ny)) =
                        self.placement_con_site_spot(ctx, cty, wdt, hgt, category, 20)
                    {
                        ctx = nx;
                        cty = ny;
                        placeable = true;
                    }
                }
                if !placeable {
                    continue;
                }
                self.prepare_construction_terrain(ctx, cty, wdt, hgt, basement);
                // PlaceReadyBase uses CreateObjectConstruction(...,
                // FullCon,true), not a pre-grown raw spawn. Construction
                // therefore observes Con=0 at the unadjusted site and can
                // create basement children there before initial DoCon lifts
                // the completed structure (C4Player.cpp:594-600;
                // C4Game.cpp:1191-1230).
                let Some(id) = self.spawn_object_with_initial_lifecycle(
                    SpawnConfig::new(definition_id.as_str())
                        .with_position(Vector2::new(ctx, cty))
                        .with_owner(number),
                    None,
                )?
                else {
                    continue;
                };
                if first_base.is_none() && can_be_base {
                    *first_base = Some(id);
                    if let Some(index) = self.find_object_index(id) {
                        *tx = self.objects[index].state.position.x;
                        *ty = self.objects[index].state.position.y;
                    }
                }
            }
        }
        Ok(())
    }

    /// `C4Player::PlaceReadyMaterial` (C4Player.cpp:642-668): into the
    /// first base via CreateContentsByList, otherwise spread on the ground
    /// with one Random(tx2-tx1) draw per item.
    fn place_ready_material(
        &mut self,
        number: i32,
        start: &scenario::PlayerStart,
        tx1: i32,
        tx2: i32,
        ty: i32,
        first_base: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        if let Some(base) = first_base {
            for (token, count) in &start.ready_material {
                let Some(definition_id) = self.resolve_definition_token(token) else {
                    continue;
                };
                for _ in 0..(*count).max(0) {
                    self.spawn_object(
                        SpawnConfig::new(definition_id.as_str())
                            .with_owner(number)
                            .with_container(base),
                    )?;
                }
            }
            return Ok(());
        }
        for (token, count) in &start.ready_material {
            let Some(definition_id) = self.resolve_definition_token(token) else {
                continue;
            };
            let (wdt, _) = self.definition_shape_size(&definition_id);
            for _ in 0..(*count).max(0) {
                let mut ctx = tx1 + self.rng.random(tx2 - tx1);
                let mut cty = ty;
                if !start.enforce_position {
                    if let Some((nx, ny)) = self
                        .landscape
                        .as_ref()
                        .and_then(|landscape| landscape.find_solid_ground(ctx, cty, wdt))
                    {
                        ctx = nx;
                        cty = ny;
                    }
                }
                self.spawn_object(
                    SpawnConfig::new(definition_id.as_str())
                        .with_position(Vector2::new(ctx, cty))
                        .with_owner(number),
                )?;
            }
        }
        Ok(())
    }

    /// `C4Player::PlaceReadyVehic` (C4Player.cpp:619-640): ready vehicles
    /// enter the first base and immediately receive a replacing Exit command
    /// (:631-636).
    fn place_ready_vehic(
        &mut self,
        number: i32,
        start: &scenario::PlayerStart,
        tx1: i32,
        tx2: i32,
        ty: i32,
        first_base: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        for (token, count) in &start.ready_vehic {
            let Some(definition_id) = self.resolve_definition_token(token) else {
                continue;
            };
            let (wdt, _) = self.definition_shape_size(&definition_id);
            for _ in 0..(*count).max(0) {
                let mut ctx = tx1 + self.rng.random(tx2 - tx1);
                let mut cty = ty;
                if !start.enforce_position {
                    if let Some((nx, ny)) = self
                        .landscape
                        .as_ref()
                        .and_then(|landscape| landscape.find_level_ground(ctx, cty, wdt, 6))
                    {
                        ctx = nx;
                        cty = ny;
                    }
                }
                let mut config = SpawnConfig::new(definition_id.as_str())
                    .with_position(Vector2::new(ctx, cty))
                    .with_owner(number);
                if let Some(base) = first_base {
                    config = config.with_container(base);
                }
                let vehicle = self.spawn_object(config)?;
                if first_base.is_some() {
                    if let Some(index) = self.find_object_index(vehicle) {
                        self.set_object_command(
                            index,
                            command::CommandRequest::new(CommandId::Exit)
                                .with_mode(CommandMode::Base),
                            false,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    /// `C4Player::PlaceReadyCrew` (C4Player.cpp:481-570): old spec (no
    /// ready-crew list) evaluates the Clonks C4SVal for a count of
    /// NativeCrew members; new spec walks the ready-crew ID list. Each
    /// member is recruited from the roster (GetIdle, else New with its
    /// synced name draw), placed with one Random(tx2-tx1) draw, and gets
    /// the fail-safe Recruitment callback (PSF_OnJoinCrew, C4Script.h:107).
    #[allow(clippy::too_many_arguments)]
    fn place_ready_crew(
        &mut self,
        number: i32,
        start: &scenario::PlayerStart,
        tx1: i32,
        tx2: i32,
        ty: i32,
        first_base: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        if start.ready_crew.is_empty() {
            // Old specification (C4Player.cpp:489-526).
            let crew_count = start.crew_count.evaluate(&mut self.rng);
            let native = start.native_crew.clone().unwrap_or_default();
            for _ in 0..crew_count.max(0) {
                self.place_one_crew_member(number, &native, start, tx1, tx2, ty, first_base)?;
            }
        } else {
            // New specification (C4Player.cpp:528-570): minimum one per
            // listed id.
            for (token, count) in start.ready_crew.clone() {
                for _ in 0..count.max(1) {
                    self.place_one_crew_member(number, &token, start, tx1, tx2, ty, first_base)?;
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn place_one_crew_member(
        &mut self,
        number: i32,
        id_token: &str,
        start: &scenario::PlayerStart,
        tx1: i32,
        tx2: i32,
        ty: i32,
        first_base: Option<ObjectId>,
    ) -> Result<(), EngineError> {
        // Select from home crew, adding new info as necessary
        // (C4Player.cpp:502-506/541-545) — no position draw when no info
        // or definition is available.
        // ID tokens resolve like C4Id parsing did at compile time; an
        // unresolvable token leaves recruit unable to create infos (no
        // position draw, like C++ skipping on `!C4Id2Def`).
        let resolved = if id_token.is_empty() {
            String::new()
        } else {
            self.resolve_definition_token(id_token)
                .map(|id| id.as_str().to_string())
                .unwrap_or_else(|| id_token.to_string())
        };
        let Some((info_index, info)) = self.recruit_crew_info(number, &resolved) else {
            return Ok(());
        };
        let definition_id = DefinitionId::from(info.id.as_str());
        if !self.definitions.contains_key(&definition_id) {
            return Ok(());
        }
        let (wdt, _) = self.definition_shape_size(&definition_id);

        // Crew placement location (C4Player.cpp:507-510/547-550).
        let mut ctx = tx1 + self.rng.random(tx2 - tx1);
        let mut cty = ty;
        if !start.enforce_position {
            if let Some((nx, ny)) = self
                .landscape
                .as_ref()
                .and_then(|landscape| landscape.find_solid_ground(ctx, cty, wdt * 3))
            {
                ctx = nx;
                cty = ny;
            }
        }

        // Game.CreateInfoObject (C4Game.cpp:1156-1170): same creation path
        // as CreateObject, with the info linked.
        let mut config = SpawnConfig::new(definition_id.as_str())
            .with_position(Vector2::new(ctx, cty))
            .with_owner(number)
            .with_crew_member(true);
        if let Some(base) = first_base {
            config = config.with_container(base);
        }
        let info_physical = info.physical;
        let id = self.spawn_object_with_crew_info(
            config,
            CrewObjectInfo {
                definition_id: definition_id.clone(),
                name: info.name.clone(),
                death_message: info.death_message.clone(),
                core: info.core.clone(),
                rank: info.rank,
                rank_name: info.rank_name.clone(),
                experience: info.experience,
                participation: info.participation,
                rounds: info.rounds,
                death_count: info.death_count,
                total_playing_time: info.total_playing_time,
                birthday: info.birthday,
                age: info.age,
                in_action_time: info.in_action_time,
                extra_data: info.extra_data.clone(),
                portraits: info.portraits.clone(),
            },
            CrewInfoLink {
                player_id: number,
                roster_index: info_index,
            },
            info_physical,
        )?;

        // C4Player adds the default FoW range only after NewObject has
        // finished Construction/Completion/Initialize (C4Player.cpp:
        // 511-520,551-568).
        if let Some(index) = self.find_object_index(id) {
            self.objects[index].state.plr_view_range = 500;
        }
        self.actualize_object_fow_view_range(id);

        if first_base.is_some() {
            if let Some(index) = self.find_object_index(id) {
                // CreateInfoObject has already attached the roster info and
                // physicals. Enter(FirstBase), then SetCommand(C4CMD_Exit)
                // precedes Recruitment (C4Player.cpp:551-568).
                self.set_object_command(
                    index,
                    command::CommandRequest::new(CommandId::Exit).with_mode(CommandMode::Base),
                    false,
                )?;
            }
        }

        // Fail-safe Recruitment callback (PSF_OnJoinCrew = "~Recruitment",
        // C4Script.h:107; C4Player.cpp:520-524/565-568).
        let has_recruitment = self
            .definitions
            .get(&definition_id)
            .map(|definition| definition.has_function("Recruitment"))
            .unwrap_or(false);
        if has_recruitment {
            if let Some(index) = self.find_object_index(id) {
                if self.objects[index].state.status.is_active() {
                    tolerate_script_error(
                        self.call_object_function(index, "Recruitment", vec![Value::Int(number)])
                            .map(|_| ()),
                    )?;
                }
            }
        }
        Ok(())
    }

    /// The `while (!(pInfo = GetIdle)) if (!New) break;` recruit loop
    /// (C4Player.cpp:502-504/541-543). Returns the recruited info (marked
    /// InAction) or None when no info could be created.
    pub(crate) fn recruit_crew_info(
        &mut self,
        number: i32,
        id_token: &str,
    ) -> Option<(usize, player_file::CrewInfo)> {
        loop {
            if let Some(index) = self.idle_crew_info_index(number, id_token) {
                let (definition_id, rank, stored_rank_name) = {
                    let info = self.crew_rosters.get(&number)?.get(index)?;
                    (info.id.clone(), info.rank, info.rank_name.clone())
                };
                let rank_name = self.recruit_rank_name(
                    &DefinitionId::from(definition_id.as_str()),
                    rank,
                    &stored_rank_name,
                );
                let roster = self.crew_rosters.entry(number).or_default();
                roster[index].in_action = true; // pHiExp->Recruit()
                roster[index].was_in_action = true;
                roster[index].in_action_time = self.game_time;
                roster[index].rank_name = rank_name;
                return Some((index, roster[index].clone()));
            }
            if !self.create_crew_info(number, id_token) {
                return None;
            }
        }
    }

    pub(crate) fn recruit_rank_name(
        &self,
        definition_id: &DefinitionId,
        rank: i32,
        stored_rank_name: &str,
    ) -> String {
        self.definitions
            .get(definition_id)
            .and_then(Definition::rank_names)
            .filter(|names| !names.is_empty())
            .and_then(|names| {
                usize::try_from(rank)
                    .ok()
                    .and_then(|rank| names.get_or_last(rank))
                    .map(|name| name.into_owned())
            })
            .unwrap_or_else(|| stored_rank_name.to_string())
    }

    /// `C4ObjectInfoList::GetIdle` (C4ObjectInfoList.cpp:113-142): the
    /// highest-experience idle entry whose def is loaded; first of equal
    /// experience wins. An empty id accepts only definitions whose DefCore
    /// `NoStandardCrew` (`C4Def::NativeCrew`) value is zero, while an explicit
    /// matching id remains eligible regardless of that flag.
    pub(crate) fn idle_crew_info_index(&self, number: i32, id_token: &str) -> Option<usize> {
        let roster = self.crew_rosters.get(&number)?;
        let mut best: Option<usize> = None;
        let fallback_order;
        let order = match self.crew_info_order.get(&number) {
            Some(order) => order.as_slice(),
            None => {
                fallback_order = (0..roster.len()).collect::<Vec<_>>();
                fallback_order.as_slice()
            }
        };
        for &index in order {
            let Some(info) = roster.get(index) else {
                continue;
            };
            let Some(definition) = self.definitions.get(&DefinitionId::from(info.id.as_str()))
            else {
                continue;
            };
            if id_token.is_empty() {
                if definition.no_standard_crew != 0 {
                    continue;
                }
            } else if info.id != id_token {
                continue;
            }
            if info.participation == 0 || info.in_action || info.has_died {
                continue;
            }
            match best {
                Some(current) if roster[current].experience >= info.experience => {}
                _ => best = Some(index),
            }
        }
        best
    }

    /// `C4ObjectInfoList::New` (C4ObjectInfoList.cpp:144-185) +
    /// `C4ObjectInfoCore::Default` (C4InfoCore.cpp:372-417): fresh info
    /// with a name drawn from the def's ClonkNames (or the standard names)
    /// via ONE synced Random over the newline count, then numbered unique
    /// by MakeValidName (C4ObjectInfoList.cpp:93-101).
    fn create_crew_info(&mut self, number: i32, id_token: &str) -> bool {
        // Default type clonk if none specified (C4ObjectInfoList.cpp:150).
        let id = if id_token.is_empty() {
            "CLNK"
        } else {
            id_token
        };
        let Some(definition) = self.definitions.get(&DefinitionId::from(id)) else {
            return false;
        };
        let physical = crew_info_physical(*definition.physical(), 0);
        let names_source = definition
            .clonk_names()
            .map(str::to_owned)
            .or_else(|| self.standard_names.clone());
        const C4_MAX_NAME: usize = 30; // C4Constants.h:26
        let mut name = match names_source {
            Some(names) => {
                if names.to_ascii_lowercase().contains("names.txt") {
                    // GetAName always eats the Random so having or not
                    // having a Names.txt makes no difference
                    // (C4InfoCore.cpp:36-38); the file itself is not
                    // reachable from the engine, so the fallback name
                    // applies.
                    self.rng.random(1000);
                    "Clonk".to_string()
                } else {
                    let newline_count = names.bytes().filter(|&byte| byte == b'\n').count() as i32;
                    let segment_index = self.rng.random(newline_count) as usize;
                    let segment = names
                        .split('\n')
                        .nth(segment_index)
                        .unwrap_or_default()
                        .replace('\r', "");
                    let cleaned: String = segment.trim().chars().take(C4_MAX_NAME).collect();
                    if cleaned.is_empty() {
                        "Clonk".to_string()
                    } else {
                        cleaned
                    }
                }
            }
            None => "Clonk".to_string(),
        };

        let mut core = CrewInfoCoreFields {
            type_name: bounded_crew_type_name(definition.name()),
            ..CrewInfoCoreFields::default()
        };
        let mut rank_name = default_crew_rank_name();
        update_custom_rank_fields(
            &mut rank_name,
            &mut core,
            0,
            definition.rank_names(),
            definition.rank_base(),
        );
        let roster = self.crew_rosters.entry(number).or_default();
        // MakeValidName (C4ObjectInfoList.cpp:93-101): number duplicates
        // from 2, overwriting the tail to stay within C4MaxName.
        let base = name.clone();
        let mut next_number = 2;
        while roster
            .iter()
            .any(|info| info.name.eq_ignore_ascii_case(&name))
        {
            let digits = next_number.to_string();
            let keep = base
                .chars()
                .count()
                .min(C4_MAX_NAME.saturating_sub(digits.len()));
            name = base.chars().take(keep).collect::<String>() + &digits;
            next_number += 1;
        }

        roster.push(player_file::CrewInfo {
            id: id.to_string(),
            name,
            core,
            rank_name,
            physical,
            ..Default::default()
        });
        let index = roster.len() - 1;
        self.crew_info_order
            .entry(number)
            .or_insert_with(|| (0..index).collect())
            .insert(0, index);
        if let Some(player) = self.players.get_mut(&number) {
            player.increment_crew_created();
        }
        true
    }

    pub fn register_player(&mut self, config: PlayerConfig) -> Result<(), EngineError> {
        self.register_player_with_runtime_control(config, PlayerRuntimeControl::NONE)
    }

    /// Low-level player registration with the process-local input assignment
    /// installed before PreInitializePlayer/InitializePlayer callbacks.
    pub fn register_player_with_runtime_control(
        &mut self,
        config: PlayerConfig,
        runtime_control: PlayerRuntimeControl,
    ) -> Result<(), EngineError> {
        let id = config.id();
        if self.players.contains_key(&id) {
            return Err(EngineError::PlayerAlreadyExists(id));
        }
        // A new C4Player owns a fresh CrewInfoList runtime counter set even
        // if this in-round player number was used by an earlier participant.
        self.crew_info_control_counts
            .retain(|link, _| link.player_id != id);
        let player_info_id = self.assign_player_info_id(config.player_info_id());
        if player_info_id != 0 {
            Rc::make_mut(&mut self.player_info_league_progress_data)
                .entry(player_info_id)
                .or_insert(None);
        }
        let mut player = config.with_player_info_id(player_info_id).build();
        player.set_game_join_time(self.game_time);
        player.set_runtime_control(runtime_control.control_set, runtime_control.mouse_control);
        player.set_control_preferences(
            runtime_control.preferred_control_set,
            runtime_control.prefers_mouse,
        );
        player.set_control_style_preferences(false, false);
        let control_style = self.forced_control_style.unwrap_or(false);
        let activates_auto_stop = player.control.control_style != control_style && control_style;
        player
            .control
            .set_control_style_value(i32::from(control_style));
        player
            .control
            .set_auto_context_menu_value(i32::from(self.forced_auto_context_menu.unwrap_or(false)));
        // C4Player::InitControl flashes both markers at the join
        // (C4Player.cpp:1747).
        player.control.select_flash = 30;
        player.control.cursor_flash = 30;
        self.players.insert(id, player);
        self.append_and_recheck_player_order(id);
        self.actualize_ownerless_fow_objects_for_new_player();
        self.recheck_runtime_team_memberships();
        if activates_auto_stop {
            self.reset_inactive_crew_command_directions(id);
        }
        self.players_registered = true;
        self.crew_info_order.entry(id).or_default();
        self.bootstrap_player_crew_from_union(id);
        self.sync_player_cursor(id);

        // Player-init broadcasts run with the default fPassError=false
        // (C4Player.cpp:769, C4ScriptHost.h:91): a script error is logged by
        // the fail-safe exec and the join continues.
        tolerate_script_error(
            self.broadcast_scenario_function("PreInitializePlayer", vec![Value::Int(id)]),
        )?;

        if self
            .players
            .get_mut(&id)
            .is_some_and(Player::initialize_mouse_fog_of_war)
        {
            self.rebuild_fow_view_objects();
        }
        if self.players.get(&id).and_then(Player::team).is_some() {
            self.set_player_team_hostility(id);
        }

        let position = self
            .objects
            .iter()
            .filter(|object| object.state.owner == id && object.state.crew_member)
            .min_by_key(|object| object.id.as_u64())
            .map(|object| object.state.position);
        let (x_value, y_value) = match position {
            Some(pos) => (Value::Int(pos.x), Value::Int(pos.y)),
            None => (Value::Nil, Value::Nil),
        };
        let base_value = self
            .objects
            .iter()
            .filter(|object| {
                object.state.owner == id
                    && (object.state.category & (CATEGORY_STRUCTURE | CATEGORY_STATIC_BACK)) != 0
            })
            .min_by_key(|object| object.id.as_u64())
            .map(|object| object_reference_value(object.id))
            .unwrap_or(Value::Nil);
        let team_value = self
            .players
            .get(&id)
            .and_then(|player| player.team())
            .map(Value::Int)
            .unwrap_or(Value::Nil);
        let mut init_args = Vec::with_capacity(6);
        init_args.push(Value::Int(id));
        init_args.push(x_value);
        init_args.push(y_value);
        init_args.push(base_value);
        init_args.push(team_value);
        init_args.push(Value::Nil);

        tolerate_script_error(self.broadcast_scenario_function("InitializePlayer", init_args))?;
        self.sync_team_home_base_for(id);
        let establish_initial_value = self
            .players
            .get(&id)
            .is_some_and(|player| !player.initial_value_is_set());
        // This low-level fixture/sandbox registration has no ScenarioInit
        // ready-crew placement. Do not eliminate it before callers can attach
        // the explicitly registered crew; real joins and restores use the
        // full CheckElimination path above.
        self.finalize_joining_player(id, establish_initial_value, false)
    }

    /// Declare or revoke hostility between two players
    /// (C4Player::Hostility; queried by `C4PlayerList::Hostile`).
    pub fn set_hostility(
        &mut self,
        player: i32,
        opponent: i32,
        hostile: bool,
    ) -> Result<(), EngineError> {
        let plr = self
            .players
            .get_mut(&player)
            .ok_or(EngineError::UnknownPlayer(player))?;
        plr.set_hostile_towards(opponent, hostile);
        Ok(())
    }

    pub fn remove_player(&mut self, id: i32) -> Result<Player, EngineError> {
        self.remove_player_internal(id, true)
    }

    /// `C4PlayerList::Join` removes a provisional player with callbacks but
    /// suppresses the game-over check when `C4Player::Init` fails
    /// (C4PlayerList.cpp:302-314).
    pub(crate) fn remove_failed_recreated_player(&mut self, id: i32) -> Result<(), EngineError> {
        self.remove_player_internal(id, false).map(drop)
    }

    /// `C4Game::Abort`'s `RemoveLocal(true, true)` followed by
    /// `RemoveAtRemoteClient(true, true)`. Local-control user players are
    /// removed first in C4PlayerList order. The second pass removes every
    /// remaining player whose `AtClient` differs from the replay client,
    /// preserving non-local players at `C4ClientIDUnknown`.
    ///
    /// The native `fNoCalls` path deliberately suppresses RemovePlayer,
    /// NotifyOwnedObjects, crew-object removal, and game-over checks while
    /// still detaching object infos and validating object owners.
    #[doc(hidden)]
    pub fn abort_players_without_callbacks(
        &mut self,
        replay_client_id: i32,
    ) -> Result<Vec<Player>, EngineError> {
        let local_players = self
            .player_ids_in_order()
            .into_iter()
            .filter(|number| {
                self.local_players.as_ref().map_or_else(
                    || {
                        self.players.get(number).is_some_and(|player| {
                            player.at_client().get() == replay_client_id
                                && !player.is_script_player()
                        })
                    },
                    |local_players| local_players.contains(number),
                )
            })
            .collect::<Vec<_>>();
        let mut removed = Vec::with_capacity(self.players.len());
        for number in local_players {
            removed.push(self.remove_player_without_callbacks(number)?);
        }

        let remote_players = self
            .player_ids_in_order()
            .into_iter()
            .filter(|number| {
                self.players
                    .get(number)
                    .is_some_and(|player| player.at_client().get() != replay_client_id)
            })
            .collect::<Vec<_>>();
        for number in remote_players {
            removed.push(self.remove_player_without_callbacks(number)?);
        }
        Ok(removed)
    }

    /// `C4Player::NotifyOwnedObjects`: visit the live main object list first,
    /// then the inactive list, and run the engine's private
    /// `~OnOwnerRemoved` fallback for each object still owned by the
    /// departing player (C4Player.cpp:1807-1822).
    fn notify_owned_objects(&mut self, departing_player: i32) -> Result<(), EngineError> {
        self.notify_owned_objects_with_status(departing_player, ObjectStatus::Normal)?;
        // Build this pass only after the main-list callbacks have completed:
        // an earlier OnOwnerChanged may have moved a later object between
        // the two C++ lists.
        self.reconcile_inactive_list();
        self.notify_owned_objects_with_status(departing_player, ObjectStatus::Inactive)
    }

    fn notify_owned_objects_with_status(
        &mut self,
        departing_player: i32,
        status: ObjectStatus,
    ) -> Result<(), EngineError> {
        // Both ledgers store their C++ list order reversed. Use the distinct
        // InactiveObjects ledger for that pass: deactivation order can differ
        // from the object's former position in Game.Objects.
        let order = if status == ObjectStatus::Inactive {
            &self.inactive_exec_list
        } else {
            &self.exec_list
        };
        let object_ids = order
            .iter()
            .rev()
            .copied()
            .filter(|&object_id| {
                self.find_object_index(object_id).is_some_and(|index| {
                    let object = &self.objects[index];
                    !object.destroyed && object.state.status == status
                })
            })
            .collect::<Vec<_>>();

        for object_id in object_ids {
            // The C++ walk observes live state at every link. In particular,
            // an earlier OnOwnerChanged can reassign or remove a later object.
            let still_owned = self.find_object_index(object_id).is_some_and(|index| {
                let object = &self.objects[index];
                !object.destroyed
                    && object.state.status == status
                    && object.state.owner == departing_player
            });
            if still_owned {
                self.on_owner_removed_fallback(object_id)?;
            }
        }
        Ok(())
    }

    /// Native `FnOnOwnerRemoved`, registered by C++ under the literal private
    /// name `~OnOwnerRemoved` (C4Script.cpp:5834-5878). Ordinary definition
    /// functions named `OnOwnerRemoved` are deliberately not dispatched by
    /// this path: `GetFuncRecursive` performs exact-name lookup here.
    fn on_owner_removed_fallback(&mut self, object_id: ObjectId) -> Result<(), EngineError> {
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        let object = &self.objects[index];
        let owner = object.state.owner;
        let category = object.state.category;
        let is_flag = object.definition_id.as_str() == "FLAG";
        let Some(owner_player) = self.players.get(&owner) else {
            return Ok(());
        };
        if owner_player.crew().contains(&object_id) {
            // Stored crew is removed only after the player has been unlinked.
            return Ok(());
        }
        if category & CATEGORY_STATIC_BACK != 0 && !is_flag {
            // Internal StaticBack objects retain their owner until the final
            // ValidateOwners pass. Flags are the explicit C++ exception.
            return Ok(());
        }

        let owner_info_id = owner_player.player_info_id();
        let owner_team = owner_player.team().filter(|team| *team != 0);
        let mut new_owner = owner_team
            .and_then(|team| {
                self.runtime_team_members_in_order(team)
                    .into_iter()
                    .find(|&candidate| {
                        candidate != owner
                            && self.players.get(&candidate).is_some_and(|player| {
                                let info_id = player.player_info_id();
                                info_id != 0 && info_id != owner_info_id
                            })
                    })
            })
            .unwrap_or(OWNER_NONE);

        if new_owner == OWNER_NONE {
            // The native C4PlayerList walk has no break, so the last eligible
            // non-hostile player in exact link order becomes the fallback.
            let player_ids = self.player_ids_in_order();
            for candidate in player_ids {
                let eligible = candidate != owner
                    && self.players.get(&candidate).is_some_and(|player| {
                        !matches!(
                            player.status(),
                            PlayerStatus::Eliminated | PlayerStatus::Surrendered
                        )
                    })
                    && !self.players_hostile(candidate, owner);
                if eligible {
                    new_owner = candidate;
                }
            }
        }

        self.set_object_owner(object_id, new_owner)
    }

    /// Native `C4Object::SetOwner`: owner color, owner/controller write,
    /// FLAG base propagation, and the synchronous ordinary
    /// `OnOwnerChanged(new, old)` callback.
    pub(crate) fn set_object_owner(
        &mut self,
        object_id: ObjectId,
        new_owner: i32,
    ) -> Result<(), EngineError> {
        if new_owner != OWNER_NONE && !self.players.contains_key(&new_owner) {
            return Ok(());
        }
        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        let definition_id = self.objects[index].definition_id.clone();
        let graphics_definition_id = self.objects[index]
            .state
            .base_graphics
            .as_ref()
            .map(|graphics| graphics.definition.clone())
            .unwrap_or_else(|| definition_id.clone());
        let color_by_owner = self
            .definitions
            .get(&graphics_definition_id)
            .is_some_and(Definition::color_by_owner);
        let owner_color = (new_owner != OWNER_NONE && color_by_owner).then(|| {
            self.players
                .get(&new_owner)
                .and_then(Player::color)
                .map(|color| {
                    u32::from(color.r) << 16 | u32::from(color.g) << 8 | u32::from(color.b)
                })
                .unwrap_or(0)
        });

        let old_owner = self.objects[index].state.owner;
        if let Some(color) = owner_color {
            // C4Object::SetOwner refreshes the ColorByOwner face before its
            // same-owner early return (C4Object.cpp:5497-5506).
            self.objects[index].state.color = color;
            self.update_solid_mask(index);
        }
        if old_owner == new_owner {
            return Ok(());
        }

        let flag_base_target = {
            let object = &mut self.objects[index];
            object.state.owner = new_owner;
            object.state.controller = new_owner;
            (definition_id.as_str() == "FLAG" && object.state.action.name == "FlyBase")
                .then_some(object.state.action.target)
                .flatten()
        };

        self.actualize_object_fow_after_owner_change(object_id, new_owner);

        if let Some(target) = flag_base_target {
            if let Some(target_index) = self.find_object_index(target) {
                let target = &mut self.objects[target_index];
                if !target.destroyed
                    && target.state.status != ObjectStatus::Deleted
                    && target.state.base == old_owner
                {
                    target.state.base = new_owner;
                }
            }
        }

        let Some(index) = self.find_object_index(object_id) else {
            return Ok(());
        };
        if self.objects[index].destroyed
            || self.objects[index].state.status == ObjectStatus::Deleted
        {
            return Ok(());
        }
        let _ = tolerate_script_error(self.call_object_function(
            index,
            "OnOwnerChanged",
            vec![Value::Int(new_owner), Value::Int(old_owner)],
        ))?;
        Ok(())
    }

    fn remove_player_internal(
        &mut self,
        id: i32,
        check_game_over: bool,
    ) -> Result<Player, EngineError> {
        let team = match self.players.get(&id) {
            Some(player) => player.team(),
            None => return Err(EngineError::UnknownPlayer(id)),
        };
        let mut args = Vec::with_capacity(2);
        args.push(Value::Int(id));
        args.push(Value::Int(team.unwrap_or(0)));
        // GRBroadcast uses fail-safe execution here: partial mutations from
        // a failing RemovePlayer callback remain, but the native ownership
        // notification still follows (C4PlayerList.cpp:219-224).
        tolerate_script_error(self.broadcast_scenario_function("RemovePlayer", args))?;

        // C4PlayerList::Remove performs this native callback sweep after the
        // RemovePlayer broadcast but while the player and Crew list are still
        // live (C4PlayerList.cpp:219-261).
        self.notify_owned_objects(id)?;

        // C4PlayerList::Remove does not run C4Player::Evaluate here. For an
        // unevaluated player it only snapshots the current profile values
        // into C4RoundResultsPlayer before unlinking (C4PlayerList.cpp:228-242;
        // C4RoundResults.cpp:52-75).
        self.snapshot_player_round_results_for_removal(id)?;

        // C4PlayerList unlinks the departing player before walking its stored
        // Crew list and assigning every member removal. Preserve that roster
        // across the map removal below; deriving it from Owner afterwards is
        // not equivalent because RemovePlayer callbacks may change ownership
        // without changing C4Player::Crew (C4PlayerList.cpp:244-261;
        // C4Player.cpp:1799-1805).
        let departing_crew = self
            .players
            .get(&id)
            .map(|player| player.crew().to_vec())
            .unwrap_or_default();

        let player = self
            .players
            .remove(&id)
            .ok_or(EngineError::UnknownPlayer(id))?;
        self.player_order.retain(|number| *number != id);
        // C4PlayerList unlinks the player's viewport ownership as part of
        // the same removal. Never leave a deleted player in the app-facing
        // local viewport projection.
        if let Some(local_players) = self.local_players.as_mut() {
            local_players.remove(&id);
        }
        if self
            .active_message_board_input
            .as_ref()
            .is_some_and(|input| input.player == id)
        {
            self.active_message_board_input = None;
        }
        for crew in departing_crew {
            let _ = self.assign_object_removal_with_contents(crew, true)?;
        }
        self.discard_removed_player_runtime_state(id);
        self.validate_object_player_references();
        self.refresh_elimination_state();
        if self.team_home_base_rule {
            if let Some(team) = player.team() {
                self.sync_team_home_base_group(team);
            }
        }
        if check_game_over {
            self.check_game_over()?;
        }
        Ok(player)
    }

    /// Remove the saved FLAG and raw C4D_CrewMember objects for every joined
    /// restore row that no current player-info row takes over.
    ///
    /// `RestoreSavegameInfos` walks restore packets and their rows in storage
    /// order, and `RemoveUnjoined` walks `Game.Objects` First -> Next for each
    /// missing saved `GameNumber` (C4PlayerInfo.cpp:1422-1439,1610-1633;
    /// C4PlayerList.cpp:208-216; C4Object.cpp:6267-6291).
    pub fn remove_unassociated_savegame_player_objects(
        &mut self,
        current_player_infos: &ControlPlayerInfoRegistry,
        restore_player_infos: &[ControlPlayerInfoEntry],
    ) -> Result<(), EngineError> {
        let (_, current_packets) = current_player_infos.retained_rows_snapshot();
        let associated_restore_ids = current_packets
            .iter()
            .flat_map(|(_, _, players)| players)
            .map(|player| player.savegame_player)
            .filter(|id| *id != 0)
            .collect::<HashSet<_>>();

        for restore in restore_player_infos {
            if restore.is_joined()
                && restore.game_number != OWNER_NONE
                && !associated_restore_ids.contains(&restore.id)
            {
                self.remove_unjoined_player_objects(restore.game_number)?;
            }
        }
        Ok(())
    }

    fn remove_unjoined_player_objects(
        &mut self,
        saved_player_number: i32,
    ) -> Result<(), EngineError> {
        const C4D_CREW_MEMBER: i32 = 1 << 18;

        // Rust stores the active execution list in reverse C++ master-list
        // order. Advance from the current physical link after each callback:
        // AssignRemoval may append a new unsorted/Line object after that link,
        // and native's live `clnk = clnk->Next` visits it in this same pass.
        let mut current = self.exec_list.last().copied();
        while let Some(object_id) = current {
            let remove = self.find_object_index(object_id).is_some_and(|index| {
                let object = &self.objects[index];
                !object.destroyed
                    && object.state.status.is_active()
                    && object.state.owner == saved_player_number
                    && (object.definition_id.as_str() == "FLAG"
                        || object.state.category & C4D_CREW_MEMBER != 0)
            });
            if remove {
                let _ = self.assign_object_removal_with_contents(object_id, true)?;
            }
            current = self
                .exec_list
                .iter()
                .position(|candidate| *candidate == object_id)
                .and_then(|position| position.checked_sub(1))
                .and_then(|position| self.exec_list.get(position).copied());
        }
        Ok(())
    }

    /// The result-only evaluation performed by `C4PlayerList::Remove`.
    /// Unlike `C4Player::Evaluate`, this neither marks the player evaluated
    /// nor changes score, round counters, crew participation, or play time.
    fn snapshot_player_round_results_for_removal(&mut self, id: i32) -> Result<(), EngineError> {
        let player = self
            .players
            .get(&id)
            .ok_or(EngineError::UnknownPlayer(id))?;
        let state = player.to_state();
        if state.player_info_id == 0 || state.evaluated {
            return Ok(());
        }
        let league_progress_data = self
            .player_info_league_progress_data
            .get(&state.player_info_id)
            .cloned()
            .flatten();
        match self
            .round_results
            .players
            .iter_mut()
            .find(|result| result.player_info_id == state.player_info_id)
        {
            Some(result) => {
                result.total_playing_time = state.total_playing_time as u32;
                result.score_old = state.score;
                result.league_progress_data = league_progress_data;
            }
            None => self.round_results.players.push(RoundResultsPlayerState {
                player_info_id: state.player_info_id,
                total_playing_time: state.total_playing_time as u32,
                score_old: state.score,
                league_progress_data,
                ..RoundResultsPlayerState::default()
            }),
        }
        Ok(())
    }

    /// `C4ObjectInfoList::DetachFromObjects` plus destruction of the
    /// departing player's process-local list projections.
    fn discard_removed_player_runtime_state(&mut self, id: i32) {
        let info_objects = self
            .crew_info_links
            .iter()
            .filter_map(|(&object, link)| (link.player_id == id).then_some(object))
            .collect::<Vec<_>>();
        self.crew_info_control_counts
            .retain(|link, _| link.player_id != id);
        for object_id in info_objects {
            Rc::make_mut(&mut self.crew_info_links).remove(&object_id);
            Rc::make_mut(&mut self.crew_object_infos).remove(&object_id);
            Rc::make_mut(&mut self.crew_ranks).remove(&object_id.as_u64());
            if let Some(index) = self.find_object_index(object_id) {
                self.objects[index].state.info_physical = None;
            }
        }
        self.pending_player_joins.remove(&id);
        self.crew_rosters.remove(&id);
        self.crew_info_order.remove(&id);
        self.crew_selection.remove(&id);
        self.crew_roles.remove(&id);
        self.eliminated_crew_owners.remove(&id);
        self.known_crew_owners.remove(&id);
    }

    fn remove_player_without_callbacks(&mut self, id: i32) -> Result<Player, EngineError> {
        self.snapshot_player_round_results_for_removal(id)?;
        let player = self
            .players
            .remove(&id)
            .ok_or(EngineError::UnknownPlayer(id))?;
        self.player_order.retain(|number| *number != id);
        if let Some(local_players) = self.local_players.as_mut() {
            local_players.remove(&id);
        }
        if self
            .active_message_board_input
            .as_ref()
            .is_some_and(|input| input.player == id)
        {
            self.active_message_board_input = None;
        }
        self.discard_removed_player_runtime_state(id);
        self.validate_object_player_references();
        Ok(player)
    }

    /// `C4GameObjects::ValidateOwners` visits both the main and inactive
    /// object lists and directly orphans only invalid Owner/Base/Controller
    /// references. It neither calls SetOwner nor changes object color/state.
    pub(crate) fn validate_object_player_references(&mut self) {
        let valid_players: HashSet<i32> = self.players.keys().copied().collect();
        for object in self
            .objects
            .iter_mut()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
        {
            if !valid_players.contains(&object.state.owner) {
                object.state.owner = OWNER_NONE;
            }
            if !valid_players.contains(&object.state.base) {
                object.state.base = OWNER_NONE;
            }
            if !valid_players.contains(&object.state.controller) {
                object.state.controller = OWNER_NONE;
            }
        }
    }

    /// Retain one Objects.txt `Info=` token until players have been recreated.
    /// C++ loads objects in InitGame, but binds their C4ObjectInfo pointers in
    /// InitGameFinal after InitPlayers (C4Game.cpp:2699-2722).
    pub(crate) fn remember_legacy_object_info(
        &mut self,
        object: ObjectId,
        info_name: Option<String>,
    ) {
        self.pending_legacy_object_infos.insert(object, info_name);
    }

    /// C4GameObjects::AssignInfo + C4Object::AssignInfo. The main and
    /// inactive lists are walked Last -> Prev; both Rust ledgers already use
    /// that reverse-master order. Saved names select the first matching idle
    /// roster entry irrespective of its original definition. Without a name,
    /// or after a named miss, MakeCrewMember falls back to the highest-
    /// experience idle entry for the object's current definition.
    fn assign_legacy_object_infos(&mut self) -> Result<(), EngineError> {
        if self.pending_legacy_object_infos.is_empty() {
            return Ok(());
        }

        let mut order = self.exec_list.clone();
        order.extend(self.inactive_exec_list.iter().copied());
        let mut seen = HashSet::new();
        order.retain(|object| seen.insert(*object));

        for object_id in order {
            let Some(info_name) = self.pending_legacy_object_infos.remove(&object_id) else {
                continue;
            };
            let Some(object_index) = self.find_object_index(object_id) else {
                continue;
            };
            if self.objects[object_index].state.status == ObjectStatus::Deleted
                || self.crew_object_infos.contains_key(&object_id)
            {
                continue;
            }
            let owner = self.objects[object_index].state.owner;
            let definition_id = self.objects[object_index].definition_id.clone();
            let alive = self.objects[object_index].state.alive;
            let Some(player) = self.players.get(&owner) else {
                continue;
            };
            let already_in_crew = player.crew().contains(&object_id);
            let named_assignment =
                !already_in_crew && info_name.as_deref().is_some_and(|name| !name.is_empty());
            if !already_in_crew && !named_assignment {
                continue;
            }
            if !self
                .definitions
                .get(&definition_id)
                .is_some_and(Definition::is_crew)
            {
                if already_in_crew {
                    let crew = player
                        .crew()
                        .iter()
                        .copied()
                        .filter(|candidate| *candidate != object_id)
                        .collect();
                    if let Some(player) = self.players.get_mut(&owner) {
                        player.set_crew(crew);
                    }
                }
                continue;
            }

            let named_index = info_name.as_deref().and_then(|name| {
                let roster = self.crew_rosters.get(&owner)?;
                let fallback;
                let roster_order = match self.crew_info_order.get(&owner) {
                    Some(order) => order.as_slice(),
                    None => {
                        fallback = (0..roster.len()).collect::<Vec<_>>();
                        fallback.as_slice()
                    }
                };
                roster_order.iter().copied().find(|&index| {
                    roster.get(index).is_some_and(|info| {
                        info.name.eq_ignore_ascii_case(name)
                            && info.participation != 0
                            && !info.in_action
                            && !info.has_died
                    })
                })
            });

            let recruited = if let Some(index) = named_index {
                let (info_definition, rank, stored_rank_name) = {
                    let info = &self.crew_rosters[&owner][index];
                    (info.id.clone(), info.rank, info.rank_name.clone())
                };
                let rank_name = self.recruit_rank_name(
                    &DefinitionId::from(info_definition.as_str()),
                    rank,
                    &stored_rank_name,
                );
                let info = &mut self.crew_rosters.get_mut(&owner).unwrap()[index];
                info.in_action = true;
                info.was_in_action = true;
                info.in_action_time = self.game_time;
                info.rank_name = rank_name;
                Some((index, info.clone()))
            } else {
                self.recruit_crew_info(owner, definition_id.as_str())
            };
            let Some((roster_index, info)) = recruited else {
                if already_in_crew {
                    let crew = self.players[&owner]
                        .crew()
                        .iter()
                        .copied()
                        .filter(|candidate| *candidate != object_id)
                        .collect();
                    if let Some(player) = self.players.get_mut(&owner) {
                        player.set_crew(crew);
                    }
                }
                continue;
            };

            Rc::make_mut(&mut self.crew_object_infos).insert(
                object_id,
                CrewObjectInfo {
                    definition_id: DefinitionId::from(info.id.as_str()),
                    name: info.name.clone(),
                    death_message: info.death_message.clone(),
                    core: info.core.clone(),
                    rank: info.rank,
                    rank_name: info.rank_name.clone(),
                    experience: info.experience,
                    participation: info.participation,
                    rounds: info.rounds,
                    death_count: info.death_count,
                    total_playing_time: info.total_playing_time,
                    birthday: info.birthday,
                    age: info.age,
                    in_action_time: info.in_action_time,
                    extra_data: info.extra_data.clone(),
                    portraits: info.portraits.clone(),
                },
            );
            Rc::make_mut(&mut self.crew_info_links).insert(
                object_id,
                CrewInfoLink {
                    player_id: owner,
                    roster_index,
                },
            );
            Rc::make_mut(&mut self.crew_ranks).insert(object_id.as_u64(), info.rank);
            if let Some(index) = self.find_object_index(object_id) {
                self.objects[index].state.info_physical = Some(info.physical);
                self.objects[index].state.crew_member = true;
                if self.objects[index].state.plr_view_range == 0 {
                    self.objects[index].state.plr_view_range = 500;
                }
                self.objects[index].state.controller = owner;
            }
            if !already_in_crew {
                let mut crew = self.players[&owner].crew().to_vec();
                let position = self.crew_insert_position(&crew, object_id);
                crew.insert(position, object_id);
                if let Some(player) = self.players.get_mut(&owner) {
                    player.set_crew(crew);
                }
            }
            self.actualize_object_fow_view_range(object_id);

            // The nInfo branch rejects a dead loaded crew pointer after
            // marking the roster entry. The Info link itself remains on the
            // object; only player pointers and Crew membership are cleared.
            if named_assignment && !alive {
                if let Some(entry) = self
                    .crew_rosters
                    .get_mut(&owner)
                    .and_then(|roster| roster.get_mut(roster_index))
                {
                    entry.has_died = true;
                }
                let removed_cursor = self.crew_cursor(owner) == Some(object_id);
                if removed_cursor {
                    if let Some(selection) = self.crew_selection.get_mut(&owner) {
                        selection.set_cursor(None);
                    }
                }
                if let Some(player) = self.players.get_mut(&owner) {
                    player.clear_object_pointers(object_id);
                }
                self.remove_from_roles(owner, object_id);
                if let Some(index) = self.find_object_index(object_id) {
                    self.objects[index].state.crew_member = false;
                }
                if removed_cursor {
                    self.player_adjust_cursor_command(owner)?;
                }
            }
        }
        Ok(())
    }

    /// The object half of `C4Game::InitGameFinal`: owner validation precedes
    /// `AssignInfo`, then `AssignPlrViewRange` rebuilds transient FoW links
    /// across the complete active object list (C4Game.cpp:2719-2722).
    pub(crate) fn finalize_legacy_object_links(&mut self) -> Result<(), EngineError> {
        // Fresh offline startup runs Script.Initialize before queued player
        // joins. Keep the deferred Info names intact until players exist.
        if self.players.is_empty() {
            return Ok(());
        }
        self.finalize_legacy_object_links_unconditionally()
    }

    fn finalize_legacy_object_links_unconditionally(&mut self) -> Result<(), EngineError> {
        self.validate_object_player_references();
        self.assign_legacy_object_infos()?;
        self.rebuild_fow_view_objects();
        Ok(())
    }

    /// C4PlayerList::Retire evaluates an eliminated player exactly once
    /// before Remove broadcasts and erases the live player record
    /// (C4PlayerList.cpp:398-409; C4Player.cpp:930-970).
    pub(crate) fn retire_player(&mut self, id: i32) -> Result<Player, EngineError> {
        let average_value_gain = if self.players.is_empty() {
            0
        } else {
            let sum = self.players.values().fold(0_i32, |sum, player| {
                sum.wrapping_add(player.value_gain().max(0))
            });
            sum / i32::try_from(self.players.len()).unwrap_or(i32::MAX)
        };
        let evaluated = self.evaluate_player(id, average_value_gain)?;
        if let Some((player_info_id, total_playing_time, score_old, score_new)) = evaluated {
            let league_progress_data = self
                .player_info_league_progress_data
                .get(&player_info_id)
                .cloned()
                .flatten();
            match self
                .round_results
                .players
                .iter_mut()
                .find(|result| result.player_info_id == player_info_id)
            {
                Some(result) => {
                    result.total_playing_time = total_playing_time;
                    result.score_old = score_old;
                    result.score_new = Some(score_new);
                    result.league_progress_data = league_progress_data;
                }
                None => self.round_results.players.push(RoundResultsPlayerState {
                    player_info_id,
                    total_playing_time,
                    score_old,
                    score_new: Some(score_new),
                    league_progress_data,
                    ..RoundResultsPlayerState::default()
                }),
            }
        }
        self.remove_player_internal(id, false)
    }

    /// C4Player::Evaluate plus its owned C4ObjectInfoList::Evaluate. Delayed
    /// retirement and game-over evaluation must use this identical path.
    pub(crate) fn evaluate_player(
        &mut self,
        id: i32,
        average_value_gain: i32,
    ) -> Result<Option<(i32, u32, i32, i32)>, EngineError> {
        let melee = self.scenario_values.is_melee();
        let scenario_title = self.scenario_values.scenario_title().to_string();
        let unix_time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as u32);
        let evaluated = {
            let player = self
                .players
                .get_mut(&id)
                .ok_or(EngineError::UnknownPlayer(id))?;
            let scores = player.evaluate(
                average_value_gain,
                melee,
                self.game_time,
                scenario_title,
                unix_time,
            );
            scores.map(|(score_old, score_new)| {
                (
                    player.player_info_id(),
                    player.total_playing_time() as u32,
                    score_old,
                    score_new,
                )
            })
        };
        if evaluated.is_some() {
            self.evaluate_player_crew_infos(id);
        }
        Ok(evaluated)
    }

    /// C4ObjectInfoList::Evaluate: retire active entries, then count the
    /// round for each info whose sticky WasInAction bit was ever armed.
    fn evaluate_player_crew_infos(&mut self, player_id: i32) {
        let Some(roster) = self.crew_rosters.get_mut(&player_id) else {
            return;
        };
        for entry in roster.iter_mut() {
            if entry.in_action {
                entry.total_playing_time = entry
                    .total_playing_time
                    .wrapping_add(self.game_time.wrapping_sub(entry.in_action_time));
                entry.in_action = false;
            }
            if entry.was_in_action {
                entry.rounds = entry.rounds.wrapping_add(1);
            }
        }

        let linked = self
            .crew_info_links
            .iter()
            .filter_map(|(&object_id, &link)| {
                (link.player_id == player_id).then_some((object_id, link.roster_index))
            })
            .collect::<Vec<_>>();
        let infos = Rc::make_mut(&mut self.crew_object_infos);
        for (object_id, roster_index) in linked {
            let Some(entry) = roster.get(roster_index) else {
                continue;
            };
            if let Some(info) = infos.get_mut(&object_id) {
                info.rounds = entry.rounds;
                info.total_playing_time = entry.total_playing_time;
            }
        }
    }

    pub fn player(&self, id: i32) -> Option<&Player> {
        self.players.get(&id)
    }

    pub fn player_mut(&mut self, id: i32) -> Result<&mut Player, EngineError> {
        self.players
            .get_mut(&id)
            .ok_or(EngineError::UnknownPlayer(id))
    }

    pub fn players(&self) -> impl Iterator<Item = &Player> {
        let ledger_is_complete = self.player_order.len() == self.players.len()
            && self
                .player_order
                .iter()
                .all(|number| self.players.contains_key(number));
        let legacy_fallback = (!ledger_is_complete).then(|| {
            let mut missing = self
                .players
                .keys()
                .copied()
                .filter(|number| !self.player_order.contains(number))
                .collect::<Vec<_>>();
            missing.sort_unstable();
            missing
        });
        self.player_order
            .iter()
            .filter_map(move |number| self.players.get(number))
            .chain(
                legacy_fallback
                    .into_iter()
                    .flatten()
                    .filter_map(move |number| self.players.get(&number)),
            )
    }

    /// Drain each player's control/action counters in exact native
    /// `C4PlayerList` link order for one network-statistics control sample.
    /// Players with no input are included with zero counts.
    pub fn take_player_control_counts(&mut self) -> Vec<(i32, i32, i32)> {
        let player_ids = self.player_ids_in_order();
        player_ids
            .into_iter()
            .filter_map(|player_id| {
                self.players.get_mut(&player_id).map(|player| {
                    let (control_count, action_count) = player.take_control_counts();
                    (player_id, control_count, action_count)
                })
            })
            .collect()
    }

    /// The first live player in exact native `C4PlayerList` link order.
    pub fn first_player_id(&self) -> Option<i32> {
        self.player_order
            .iter()
            .copied()
            .find(|number| self.players.contains_key(number))
            .or_else(|| self.players.keys().copied().min())
    }

    pub fn set_player_status(&mut self, id: i32, status: PlayerStatus) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_status(status);
        Ok(())
    }

    pub fn set_player_team(&mut self, id: i32, team: Option<i32>) -> Result<(), EngineError> {
        {
            let player = self.player_mut(id)?;
            player.set_team(team);
        }
        self.recheck_runtime_team_memberships();
        self.sync_team_home_base_for(id);
        Ok(())
    }

    pub fn set_player_surrendered(
        &mut self,
        id: i32,
        surrendered: bool,
    ) -> Result<(), EngineError> {
        let eliminated = {
            let player = self.player_mut(id)?;
            player.set_surrendered(surrendered);
            matches!(
                player.status(),
                PlayerStatus::Eliminated | PlayerStatus::Surrendered
            ) || player.surrendered()
        };
        if eliminated {
            self.eliminated_crew_owners.insert(id);
        } else {
            self.eliminated_crew_owners.remove(&id);
        }
        Ok(())
    }

    /// Executes `C4ControlSurrenderPlayer`: the inherited player-control
    /// authorization requires an exact `AtClient == ByClient` match before
    /// the script surrender may run (`C4Control.cpp:1546-1578`;
    /// `C4Script.cpp:2849-2855`).
    pub fn execute_surrender_player_control(
        &mut self,
        control: SurrenderPlayerControlData,
    ) -> bool {
        let allowed = self.player(control.player).is_some_and(|player| {
            player.at_client() == PlayerAtClient::new(control.by_client)
                && !matches!(
                    player.status(),
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered
                )
                && !player.surrendered()
        });
        allowed && self.set_player_surrendered(control.player, true).is_ok()
    }

    pub fn set_player_wealth(&mut self, id: i32, wealth: i32) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_wealth(wealth);
        Ok(())
    }

    pub fn adjust_player_wealth(&mut self, id: i32, delta: i32) -> Result<i32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_wealth(delta))
    }

    pub fn player_wealth(&self, id: i32) -> Option<i32> {
        self.player(id).map(Player::wealth)
    }

    /// `C4Menu::DrawElement` arms ViewWealth whenever it renders a
    /// `C4MN_Extra_Value` footer (C4Menu.cpp:895-907). This presentation-time
    /// mutation is intentionally explicit because it is client-local.
    pub fn arm_player_view_wealth(&mut self, id: i32) -> Result<(), EngineError> {
        self.player_mut(id)?.arm_view_wealth();
        Ok(())
    }

    pub fn grant_player_knowledge(
        &mut self,
        id: i32,
        definition_id: impl Into<DefinitionId>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.grant_knowledge(definition_id.into());
        Ok(())
    }

    pub fn revoke_player_knowledge(
        &mut self,
        id: i32,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.revoke_knowledge(definition_id);
        Ok(())
    }

    pub fn grant_player_magic(
        &mut self,
        id: i32,
        definition_id: impl Into<DefinitionId>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.grant_magic(definition_id.into());
        Ok(())
    }

    pub fn revoke_player_magic(
        &mut self,
        id: i32,
        definition_id: &DefinitionId,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.revoke_magic(definition_id);
        Ok(())
    }

    pub fn player_inventory(&self, id: i32) -> Result<&HashMap<DefinitionId, u32>, EngineError> {
        self.player(id)
            .map(|player| player.inventory())
            .ok_or(EngineError::UnknownPlayer(id))
    }

    pub fn set_player_inventory_item(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        quantity: u32,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_inventory_item(definition_id, quantity);
        Ok(())
    }

    pub fn adjust_player_inventory_item(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_inventory_item(definition_id, delta))
    }

    pub fn replace_player_viewports(
        &mut self,
        id: i32,
        viewports: Vec<PlayerViewport>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.replace_viewports(viewports);
        Ok(())
    }

    /// Apply one `C4Player::ScrollView` camera step. Pointer-edge detection
    /// and the native ten-pixel cadence remain owned by the application.
    pub fn scroll_player_view(
        &mut self,
        id: i32,
        delta: Vector2,
        view_width: i32,
        view_height: i32,
        fullscreen: bool,
    ) -> Result<(), EngineError> {
        let (world_width, world_height) = self
            .landscape
            .as_ref()
            .map(|landscape| (landscape.width() as i32, landscape.estimated_height()))
            .unwrap_or((0, 0));
        self.player_mut(id)?.scroll_view(
            delta,
            view_width,
            view_height,
            world_width,
            world_height,
            fullscreen,
        );
        Ok(())
    }

    pub fn set_player_viewport(
        &mut self,
        id: i32,
        index: usize,
        viewport: PlayerViewport,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_viewport(index, viewport);
        Ok(())
    }

    pub fn set_player_home_base_material(
        &mut self,
        id: i32,
        material: HashMap<DefinitionId, u32>,
    ) -> Result<(), EngineError> {
        {
            let player = self.player_mut(id)?;
            player.set_home_base_material(material);
        }
        self.sync_team_home_base_from_player(id);
        Ok(())
    }

    pub fn set_player_home_base_production(
        &mut self,
        id: i32,
        production: HashMap<DefinitionId, u32>,
    ) -> Result<(), EngineError> {
        let player = self.player_mut(id)?;
        player.set_home_base_production(production);
        Ok(())
    }

    pub fn adjust_player_home_base_material(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let count = {
            let player = self.player_mut(id)?;
            player.adjust_home_base_material(definition_id, delta)
        };
        self.sync_team_home_base_from_player(id);
        Ok(count)
    }

    pub fn adjust_player_home_base_production(
        &mut self,
        id: i32,
        definition_id: DefinitionId,
        delta: i32,
    ) -> Result<u32, EngineError> {
        let player = self.player_mut(id)?;
        Ok(player.adjust_home_base_production(definition_id, delta))
    }
}
