//! `impl Engine` — scenario, team and base-rule configuration.
//!
//! Moved verbatim from the root `impl Engine` block in `lib.rs`.
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl Engine {
    pub(crate) fn resort_any_object_pending(&self) -> bool {
        self.resort_any_object
            || self.pending_object_order_commands.iter().any(|command| {
                matches!(
                    command,
                    ObjectOrderCommand::ResortObject(_) | ObjectOrderCommand::ResortUnsortedSweep
                )
            })
    }

    /// Installs the exact runtime fields compiled from a network savegame.
    /// The caller must invoke this after the static scenario landscape,
    /// physics, weather and sky setup, but before definition scripts or
    /// loaded objects can observe the game state.
    #[doc(hidden)]
    pub fn apply_initial_network_game_data(
        &mut self,
        data: &InitialNetworkGameData,
    ) -> Result<(), InitialNetworkGameApplyError> {
        data.validate_runtime_application()?;

        self.game_time = data.time;
        self.frame = data.frame as u64;
        self.control_tick = data.control_tick;
        self.sync_rate = data.sync_rate;
        self.next_object_id = self
            .next_object_id
            .max(data.object_enumeration_index as u64 + 1);

        self.set_structures_need_energy(data.rules & 1 != 0);
        self.set_construction_needs_material(data.rules & 2 != 0);
        self.flag_removeable = data.rules & 4 != 0;
        self.structures_snow_in = data.rules & 8 != 0;
        self.set_team_home_base_rule(data.rules & 16 != 0);

        let playlist = (!data.play_list.is_empty()).then(|| data.play_list.clone());
        self.audio_registry.restore_music_playlist(playlist.clone());
        self.pending_audio.push(AudioCommand::SetMusicPlaylist {
            playlist,
            restart: false,
        });
        let music_level = data.music_level.clamp(0, 100) as u8;
        let music_level = self.audio_registry.restore_music_level(music_level);
        self.pending_audio
            .push(AudioCommand::SetMusicLevel { level: music_level });

        if data.current_scenario_section.is_empty() {
            self.current_scenario_section = "main".to_string();
            self.last_scenario_section_flags = None;
        } else {
            self.current_scenario_section = data.current_scenario_section.clone();
            // The loaded section name is save-persistent even though the
            // transient section-load flag word is not part of Game.txt.
            self.last_scenario_section_flags = Some(0);
        }
        self.resort_any_object = data.resort_any_object;
        self.next_mission = data.next_mission.clone();
        self.message_board_commands = data.message_board_commands.clone();
        self.scenario_script_go = data.script_go;
        self.scenario_script_counter = data.script_counter;

        // Weather::CompileFunc owns only these live values. Scenario C4S
        // fields such as Wind.Std, season bounds and precipitation remain
        // installed so later Tick1000/season/weather behavior uses the
        // original scenario configuration just like C++.
        self.environment.season = data.environment.season;
        self.environment.year_speed = data.environment.year_speed;
        self.environment.season_delay = data.environment.season_delay;
        self.environment.wind = data.environment.wind;
        self.environment.wind_target = data.environment.wind_target;
        self.environment.temperature = data.environment.temperature;
        self.environment.temperature_range = data.environment.temperature_range;
        self.environment.climate = data.environment.climate;
        self.environment.meteorite = data.environment.meteorite;
        self.environment.volcano = data.environment.volcano;
        self.environment.earthquake = data.environment.earthquake;
        self.environment.lightning = data.environment.lightning;
        self.environment.no_gamma = data.environment.no_gamma;
        self.gamma = data.gamma;
        Ok(())
    }

    /// Install the final `C4Sky` frame after the caller has combined the
    /// earlier-compiled runtime words with C4Sky::Init's later resource and
    /// SkyScrollMode adjustments. The scenario surface/fade settings travel
    /// in `frame.settings`; the resulting fixed words remain authoritative.
    pub(crate) fn apply_initial_network_sky_frame(&mut self, frame: &SkyFrame) {
        self.sky = Some(sky::SkyState::from_frame(frame));
    }

    /// Install the compiled scoreboard before any startup callback can query
    /// it. Presentation requests remain suppressed until shared GUI mode.
    pub(crate) fn apply_initial_network_scoreboard(&mut self, scoreboard: ScoreboardState) {
        *self.scoreboard.borrow_mut() = scoreboard;
    }

    /// C++ denumerates script globals and global effects after all objects are
    /// loaded and before `InitializeDef`. Values supplied here have already
    /// resolved legacy string/object enumeration through that same live object
    /// set.
    pub(crate) fn apply_initial_network_post_object_state(
        &mut self,
        script_globals: &ScriptGlobalState,
        mut global_effects: Vec<EffectState>,
    ) {
        let object_definition_ids = self
            .objects
            .iter()
            .filter(|object| object.state.status != ObjectStatus::Deleted)
            .map(|object| (object.id.as_u64(), object.definition_id.clone()))
            .collect::<HashMap<_, _>>();
        let object_numbers = object_definition_ids
            .keys()
            .copied()
            .collect::<HashSet<_>>();
        for effect in &mut global_effects {
            denumerate_loaded_effect(effect, &object_numbers, &object_definition_ids);
        }
        self.restore_script_globals(script_globals);
        self.global_effects = global_effects;
    }

    pub fn show_scenario_intro(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let cleaned = trimmed.replace('\r', "");
        let normalized = cleaned
            .split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join("|");
        let spec = MessageSpec {
            kind: message::MessageKind::Global,
            text: normalized,
            target: None,
            player: None,
            offset: Vector2::new(0, 0),
            color: 0xffff_ffff,
            flags: message::FLAG_TOP | message::FLAG_HCENTER | message::FLAG_ALIGN_CENTER,
            width: Some(400),
            decoration: Some("Mission".to_string()),
            frame_decoration: None,
            portrait: None,
        };
        self.messages.add_message(spec);
    }

    /// Install the client-local sample filenames admitted by the active
    /// sound resource chain. Message/PlayerMessage/PlrMessage need this
    /// presentation-only inventory to know whether StartSoundEffect would
    /// suppress their text fallback; it is deliberately absent from saves
    /// and synchronization state.
    #[doc(hidden)]
    pub fn configure_sound_samples<I, S>(&mut self, samples: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.audio_registry.set_available_samples(samples);
    }

    /// Resolve client-local message fallbacks after the frontend attempts
    /// their corresponding speech instances. These messages are presentation
    /// state just like the sample/channel decision that selected them.
    #[doc(hidden)]
    pub fn apply_speech_playback_outcomes(
        &mut self,
        outcomes: Vec<SpeechPlaybackOutcome>,
    ) -> Vec<MessageSnapshot> {
        for outcome in outcomes {
            match outcome {
                SpeechPlaybackOutcome::Played(fallback) => {
                    self.messages.resolve_speech_fallback(fallback, false);
                }
                SpeechPlaybackOutcome::Rejected(fallback) => {
                    self.messages.resolve_speech_fallback(fallback, true);
                }
            }
        }
        self.messages.snapshot()
    }

    /// Install the client-local filenames represented by the active music
    /// resource chain. Script `SetPlayList` uses this presentation inventory
    /// for its immediate local match count.
    #[doc(hidden)]
    pub fn configure_music_tracks<I, S>(&mut self, tracks: I)
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.audio_registry.set_available_music(tracks);
    }

    #[doc(hidden)]
    pub fn music_playlist(&self) -> &str {
        self.audio_registry.music_playlist().unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn music_level(&self) -> u8 {
        self.audio_registry.music_level()
    }

    /// Savegame player finalization runs before C++ `PlayScenarioMusic`.
    /// Fold its deferred Music calls into `Game.IsMusicEnabled`, then discard
    /// presentation commands that the final default-playlist/play/level pass
    /// supersedes. The registry remains untouched so `Game.PlayList` and
    /// `iMusicLevel` retain their script-visible values for the next save.
    #[doc(hidden)]
    pub fn reconcile_music_after_restore(&mut self, mut enabled: bool) -> bool {
        self.pending_audio.retain(|command| match command {
            AudioCommand::PlayMusic { .. } => {
                enabled = true;
                false
            }
            AudioCommand::StopMusic => {
                enabled = false;
                false
            }
            AudioCommand::SetMusicLevel { .. } | AudioCommand::SetMusicPlaylist { .. } => false,
            _ => true,
        });
        enabled
    }

    pub fn set_construction_needs_material(&mut self, enabled: bool) {
        self.construction_needs_material = enabled;
    }

    pub fn structures_need_energy(&self) -> bool {
        self.structures_need_energy
    }

    pub fn set_structures_need_energy(&mut self, enabled: bool) {
        self.structures_need_energy = enabled;
    }

    pub fn set_base_buy_enabled(&mut self, enabled: bool) {
        self.base_buy_enabled = enabled;
    }

    pub fn set_base_sell_enabled(&mut self, enabled: bool) {
        self.base_sell_enabled = enabled;
    }

    pub fn set_base_auto_sell_enabled(&mut self, enabled: bool) {
        self.base_auto_sell_enabled = enabled;
    }

    pub fn set_base_reject_entrance_enabled(&mut self, enabled: bool) {
        self.base_reject_entrance_enabled = enabled;
    }

    pub fn set_base_regenerate_energy_enabled(&mut self, enabled: bool) {
        self.base_regenerate_energy_enabled = enabled;
    }

    pub fn set_base_extinguish_enabled(&mut self, enabled: bool) {
        self.base_extinguish_enabled = enabled;
    }

    pub fn set_base_regenerate_energy_price(&mut self, price: i32) {
        self.base_regenerate_energy_price = price;
    }

    pub fn set_landscape_insert_thrust(&mut self, enabled: bool) {
        self.landscape_insert_thrust = enabled;
    }

    /// Installs `[Head] ForcedAutoStopControl` before players join. The
    /// effective per-player value is selected in `join_player`, matching
    /// C4Player::ApplyForcedControl (C4Player.cpp:2369-2389).
    #[doc(hidden)]
    pub fn set_forced_control_style(&mut self, control_style: Option<bool>) {
        self.forced_control_style = control_style;
    }

    /// Installs `[Head] ForcedAutoContextMenu` before players join. The
    /// effective per-player value is selected in `join_player`, matching
    /// C4Player::ApplyForcedControl (C4Player.cpp:2369-2375).
    #[doc(hidden)]
    pub fn set_forced_auto_context_menu(&mut self, enabled: Option<bool>) {
        self.forced_auto_context_menu = enabled;
    }

    /// Installs the scenario's C4SPlrStart slots (set by `Scenario::apply`;
    /// C4Player::ScenarioInit reads them at join, C4Player.cpp:670-777).
    pub fn set_player_starts(&mut self, starts: Vec<scenario::PlayerStart>) {
        self.player_starts = starts;
        self.player_starts
            .resize_with(scenario::MAX_PLAYER_STARTS, Default::default);
    }

    #[doc(hidden)]
    pub fn set_teams(&mut self, teams: Vec<TeamInfo>) {
        self.team_last_team_id = self
            .team_last_team_id
            .max(teams.iter().map(|team| team.id).max().unwrap_or(0));
        self.teams = Rc::new(teams);
        self.recheck_runtime_team_memberships();
    }

    /// Installs the four non-queryable fields of C4TeamList::CompileFunc.
    /// Network/bootstrap callers must apply this together with the team
    /// registry and TeamConfiguration before the first synchronized save.
    #[doc(hidden)]
    pub fn set_initial_network_team_metadata(&mut self, metadata: &InitialNetworkTeamMetadata) {
        self.team_last_team_id = metadata
            .last_team_id
            .max(metadata.teams.iter().map(|team| team.id).max().unwrap_or(0));
        self.team_max_script_players = metadata.max_script_players;
        self.team_script_player_names = metadata.script_player_names.as_bytes().to_vec();
        self.team_random_team_count = metadata.random_team_count;
    }

    /// Reconciles the live C4Team player-info ID lists after a PlayerInfo
    /// control. Existing valid IDs retain their order; missing members are
    /// appended in the caller-supplied (C4PlayerInfo ID) order.
    #[doc(hidden)]
    pub fn recheck_team_player_info_memberships(&mut self, memberships: &[(i32, i32)]) {
        for team in Rc::make_mut(&mut self.teams) {
            team.player_ids.retain(|player_info_id| {
                memberships
                    .iter()
                    .any(|(id, assigned_team)| id == player_info_id && *assigned_team == team.id)
            });
            for &(player_info_id, assigned_team) in memberships {
                if assigned_team == team.id && !team.player_ids.contains(&player_info_id) {
                    team.player_ids.push(player_info_id);
                }
            }
        }
    }

    /// Applies the immediate live-player half of `C4Team::AddPlayer` for an
    /// already-joined PlayerInfo row. This deliberately bypasses
    /// SetPlayerTeam callbacks, hostility changes, and team-home-base sync.
    #[doc(hidden)]
    pub fn apply_admitted_player_team_update(
        &mut self,
        info_id: i32,
        team: i32,
        color: Option<u32>,
    ) -> Result<bool, EngineError> {
        let Some(player_id) = self
            .players
            .values()
            .find(|player| player.player_info_id() == info_id)
            .map(Player::id)
        else {
            return Ok(false);
        };
        self.players
            .get_mut(&player_id)
            .expect("the selected runtime player remains present")
            .set_team(Some(team));
        if let Some(color) = color {
            self.set_player_color(player_id, color)?;
        }
        self.recheck_runtime_team_memberships();
        Ok(true)
    }

    pub fn teams(&self) -> &[TeamInfo] {
        &self.teams
    }

    pub fn auto_generate_teams(&self) -> bool {
        self.team_configuration.auto_generate_teams
    }

    /// Returns the sole team this runtime player can join, or `None` when
    /// the choice is ambiguous or impossible (`C4TeamList::GetForcedTeamSelection`,
    /// C4Teams.cpp:876-914). A current team remains eligible even when full.
    pub fn forced_team_selection(&self, number: i32) -> Option<i32> {
        let mut possible = self
            .player(number)
            .and_then(Player::team)
            .filter(|team_id| self.teams.iter().any(|team| team.id == *team_id));
        for team in self.teams.iter().filter(|team| !self.team_is_full(team)) {
            if possible.is_some_and(|team_id| team_id != team.id) {
                return None;
            }
            possible = Some(team.id);
        }
        match (possible, self.team_configuration.auto_generate_teams) {
            (Some(_), true) => None,
            (Some(team), false) => Some(team),
            (None, true) => Some(-1),
            (None, false) => None,
        }
    }

    #[doc(hidden)]
    pub fn set_team_colors(&mut self, enabled: bool) {
        self.team_configuration.team_colors = enabled;
    }

    pub fn team_colors(&self) -> bool {
        self.team_configuration.team_colors
    }

    /// Assign C4TeamList::eTeamDist from a CID_Set packet. Native debug
    /// builds assert on values outside the five defined variants; release
    /// builds leave the current distribution unchanged.
    pub fn set_team_distribution(&mut self, distribution: i32) -> bool {
        if !(0..=4).contains(&distribution) {
            return false;
        }
        self.team_configuration.distribution = distribution;
        true
    }

    pub fn team_distribution(&self) -> i32 {
        self.team_configuration.distribution
    }

    #[doc(hidden)]
    pub fn team_configuration(&self) -> TeamConfiguration {
        self.team_configuration
    }

    #[doc(hidden)]
    pub fn set_auto_generate_teams(&mut self, enabled: bool) {
        self.team_configuration.auto_generate_teams = enabled;
    }

    #[doc(hidden)]
    pub fn set_team_configuration(&mut self, mut config: TeamConfiguration) {
        if self.league_game {
            config.allow_team_switch = false;
        }
        self.runtime_join_team_choice = config.custom && config.active;
        self.team_configuration = config;
    }

    #[doc(hidden)]
    pub fn set_runtime_join_team_choice(&mut self, enabled: bool) {
        self.team_configuration.custom = enabled;
        self.team_configuration.active = enabled;
        self.runtime_join_team_choice = enabled;
    }

    /// One C4SPlrStart slot; `None` past `C4S_MaxPlayer` (4). Joining
    /// players use slot `Number % C4S_MaxPlayer` (C4Player.cpp:673).
    pub fn player_start(&self, index: usize) -> Option<&scenario::PlayerStart> {
        self.player_starts.get(index)
    }

    /// `Game.Names` (C4Game.cpp:2772, 3288-3289): the standard clonk-name
    /// list crew-info creation draws from when the def has no ClonkNames.
    pub fn set_standard_names(&mut self, names: Option<String>) {
        self.standard_names = names;
        self.invalidate_host_definition_tables();
    }

    /// Attach the process-local mission-access configuration shared across
    /// fresh game engines.
    pub fn set_mission_access_store(&mut self, store: MissionAccessStore) {
        self.mission_access = store;
    }

    /// Attach the embedding app's process-local ShowCommands request latch.
    pub fn set_show_commands_request_store(&mut self, store: ShowCommandsRequestStore) {
        self.show_commands_requests = store;
    }

    /// Installs the frontend's process-local control-binding display names.
    /// Control indices use the legacy `CON_*` order (0 through 11).
    pub fn set_control_key_names(&mut self, names: HashMap<i32, Vec<ControlKeyName>>) {
        self.control_key_names = Rc::new(names);
    }

    /// `[Landscape] MapZoom` as a C4SVal — ScenarioInit evaluates it per
    /// configured start coordinate (C4Player.cpp:713-714).
    pub fn set_map_zoom(&mut self, map_zoom: scenario::LegacyC4SVal) {
        self.map_zoom = map_zoom;
    }

    pub(crate) fn set_scenario_values(&mut self, values: scenario::ScenarioValueStore) {
        self.scenario_values = Rc::new(values);
    }

    pub(crate) fn set_legacy_string_table(&mut self, strings: HashMap<i32, String>) {
        let registrations = clonk_script::new_string_registrations();
        let mut ids = strings.keys().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        for id in ids {
            if let Some(value) = strings.get(&id) {
                clonk_script::register_loaded_c4_string(&registrations, id, value);
            }
        }
        self.adopt_legacy_string_table(registrations);
    }

    /// Adopt the exact process-global C4StringTable used while compiling
    /// legacy scenario state. Pre-resolved C4Values retain identities from
    /// this ledger, so rebuilding it from text here would split pointers.
    pub(crate) fn adopt_legacy_string_table(
        &mut self,
        registrations: clonk_script::StringRegistrations,
    ) {
        self.script_string_registrations = registrations.clone();
        self.legacy_string_table = registrations.clone();

        // Scenario application normally runs before installing any new
        // definition/scenario host. Reattach surviving hosts as well so a
        // reused Engine cannot keep registering literals in the old table.
        for definition in self.definitions.values_mut() {
            Arc::make_mut(&mut definition.script)
                .set_string_registrations_deferred(registrations.clone());
        }
        if let Some(scenario) = self.scenario_script.as_mut() {
            Arc::make_mut(&mut scenario.script)
                .set_string_registrations_deferred(registrations.clone());
        }
        for source in &mut self.script_link_sources {
            if let ScriptLinkSource::Script { script, .. } = source {
                Arc::make_mut(script).set_string_registrations_deferred(registrations.clone());
            }
        }
        self.invalidate_host_definition_tables();
    }

    pub(crate) fn legacy_string_table_snapshot(&self) -> clonk_script::StringRegistrations {
        self.legacy_string_table.clone()
    }

    pub(crate) fn configure_scenario_sections(
        &mut self,
        sections: &[scenario::ScenarioSectionSpec],
    ) {
        let root_section = sections
            .first()
            .map(|section| section.name.clone())
            .unwrap_or_else(|| "main".to_string());
        self.scenario_section_order = sections
            .iter()
            .skip(1)
            .rev()
            .map(|section| section.name.to_ascii_lowercase())
            .collect();
        self.scenario_current_section_registered = false;
        self.scenario_sections = sections
            .iter()
            .enumerate()
            .map(|(index, section)| {
                (
                    section.name.to_ascii_lowercase(),
                    RuntimeScenarioSection {
                        name: section.name.clone(),
                        source_is_scenario_root: index == 0,
                        modified: false,
                        landscape_modified: false,
                        objects_modified: false,
                        frozen_group: None,
                        source_group: section.source_group.clone(),
                        landscape: section.landscape.clone(),
                        landscape_systems: section.landscape_systems.clone(),
                        exact_landscape: section.exact_landscape,
                        texmap_lookups: section.texmap_lookups.clone(),
                        resynthesize_static_map: section.resynthesize_static_map,
                        map_creator: section.map_creator.clone(),
                        s2_overload: section.s2_overload.clone(),
                        gravity: section.gravity,
                        post_init_map_callbacks: section.post_init_map_callbacks.clone(),
                        keep_map_creator: section.keep_map_creator,
                        no_initialize: section.no_initialize,
                        initial_objects: if section.source_group.is_none() {
                            section.objects.clone()
                        } else {
                            Vec::new()
                        },
                        saved_objects: None,
                        saved_object_order: Vec::new(),
                        scenario_values: section.scenario_values.clone(),
                        base_reject_entrance_enabled: section.base_reject_entrance_enabled,
                        base_extinguish_enabled: section.base_extinguish_enabled,
                        environment: section.environment,
                    },
                )
            })
            .collect();
        self.current_scenario_section = root_section;
        self.last_scenario_section_flags = None;
    }

    pub(crate) fn refresh_initial_s2_section(
        &mut self,
        landscape: &Landscape,
        creator: &map_creator_s2::MapCreatorS2State,
        callbacks: &map_creator_s2::PostInitMapCallbacks,
    ) {
        let key = self.current_scenario_section.to_ascii_lowercase();
        if let Some(section) = self.scenario_sections.get_mut(&key) {
            section.landscape = Some(landscape.clone());
            section.map_creator = Some(creator.clone());
            section.post_init_map_callbacks = callbacks.clone();
        }
    }

    #[doc(hidden)]
    pub fn debug_current_scenario_section(&self) -> &str {
        &self.current_scenario_section
    }

    #[doc(hidden)]
    pub fn debug_current_scenario_section_exists(&self) -> bool {
        self.scenario_current_section_registered
            && self
                .scenario_sections
                .contains_key(&self.current_scenario_section.to_ascii_lowercase())
    }

    #[doc(hidden)]
    pub fn debug_last_scenario_section_flags(&self) -> Option<i32> {
        self.last_scenario_section_flags
    }

    /// The C4ObjectInfo data linked to a crew object (CreateInfoObject,
    /// C4Game.cpp:1156-1170).
    pub fn crew_object_info(&self, id: ObjectId) -> Option<&CrewObjectInfo> {
        self.crew_object_infos.get(&id)
    }

    /// `C4Game::JoinPlayer` -> `C4PlayerList::Join` -> `C4Player::Init`
    /// with fScenarioInit (C4Game.cpp:3511-3534, C4PlayerList.cpp:271-318,
    /// C4Player.cpp:246-352): registers the player, broadcasts
    /// PreInitializePlayer, runs the ScenarioInit placement (crew, ready
    /// material/vehicles/base, synced RNG draws) and broadcasts
    /// InitializePlayer.
    pub fn join_player(
        &mut self,
        config: JoinPlayerConfig,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_with_runtime_control(config, PlayerRuntimeControl::NONE)
    }

    /// Local join form that carries the already resolved, process-local
    /// `InitControl` result into player registration. Supplying it here (and
    /// not after this method returns) is required because C++ exposes these
    /// fields to `PreInitializePlayer` (C4Player.cpp:323-347).
    pub fn join_player_with_runtime_control(
        &mut self,
        config: JoinPlayerConfig,
        runtime_control: PlayerRuntimeControl,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            PlayerAtClient::HOST,
            "Local".to_string(),
            ControlJoinPlayerSemantics::default(),
            runtime_control,
            None,
        )
    }

    /// Offline/control form retaining script-player flags carried by the
    /// authoritative C4PlayerInfo.
    pub fn join_player_with_info(
        &mut self,
        config: JoinPlayerConfig,
        info: &ControlPlayerInfoEntry,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_with_info_and_runtime_control(config, info, PlayerRuntimeControl::NONE)
    }

    /// Offline/control join with both the authoritative player-info flags and
    /// the final client-local input assignment.
    pub fn join_player_with_info_and_runtime_control(
        &mut self,
        config: JoinPlayerConfig,
        info: &ControlPlayerInfoEntry,
        runtime_control: PlayerRuntimeControl,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            PlayerAtClient::HOST,
            "Local".to_string(),
            info.into(),
            runtime_control,
            None,
        )
    }

    /// Network form of [`Engine::join_player`], retaining the authoritative
    /// `C4ControlJoinPlayer::AtClient` before PreInitialize/ScenarioInit.
    pub fn join_player_at_client(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            at_client,
            "Local".to_string(),
            ControlJoinPlayerSemantics::default(),
            PlayerRuntimeControl::NONE,
            None,
        )
    }

    /// Network-control form retaining C4PlayerInfo-only script flags without
    /// adding those transient control fields to every ordinary join config.
    pub fn join_player_at_client_with_info(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        info: &ControlPlayerInfoEntry,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            at_client,
            "Local".to_string(),
            info.into(),
            PlayerRuntimeControl::NONE,
            None,
        )
    }

    /// Network-control form with the join-time client name supplied by the
    /// current C4Client registry. C++ stores this snapshot in
    /// `C4Player::AtClientName`; it is not a dynamic client-name lookup.
    pub fn join_player_at_client_with_info_and_name(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: impl Into<String>,
        info: &ControlPlayerInfoEntry,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            at_client,
            at_client_name.into(),
            info.into(),
            PlayerRuntimeControl::NONE,
            None,
        )
    }

    /// Network-control join with an explicit client-local input assignment.
    /// The frontend resolves device availability and local-player conflicts
    /// before entering the engine, just as `C4Player::InitControl` runs before
    /// any player initialization callback.
    pub fn join_player_at_client_with_info_and_name_and_runtime_control(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: impl Into<String>,
        info: &ControlPlayerInfoEntry,
        runtime_control: PlayerRuntimeControl,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            at_client,
            at_client_name.into(),
            info.into(),
            runtime_control,
            None,
        )
    }

    /// Player-file join boundary. C++ loads the inherited C4PlayerInfoCore,
    /// including ExtraData, before InitControl and every initialization script
    /// callback. Keeping the core outside JoinPlayerConfig lets synthetic test
    /// joins retain their compact gameplay-only configuration while real file
    /// joins install the exact profile at the same point as C++.
    pub fn join_player_with_profile_core(
        &mut self,
        config: JoinPlayerConfig,
        at_client: PlayerAtClient,
        at_client_name: impl Into<String>,
        info: Option<&ControlPlayerInfoEntry>,
        runtime_control: PlayerRuntimeControl,
        player_info_core: PlayerInfoCoreState,
    ) -> Result<JoinPlayerOutcome, EngineError> {
        self.join_player_at_client_with_semantics(
            config,
            at_client,
            at_client_name.into(),
            info.map(ControlJoinPlayerSemantics::from)
                .unwrap_or_default(),
            runtime_control,
            Some(player_info_core),
        )
    }

}
