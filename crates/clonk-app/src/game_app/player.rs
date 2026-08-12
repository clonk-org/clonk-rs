//! `impl GameApp` — players, teams & crew methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn submit_due_input_latency_benchmark_pair(
        &mut self,
        started: Instant,
        now: Instant,
    ) {
        let due = self
            .input_latency_benchmark
            .as_mut()
            .is_some_and(|benchmark| {
                benchmark.start(started);
                benchmark.pair_due(now)
            });
        if !due {
            return;
        }
        let Some(by_client) = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
        else {
            return;
        };
        let owners = self
            .local_controls
            .owners()
            .filter(|owner| runtime_player_has_live_crew(&self.snapshot, *owner))
            .collect::<Vec<_>>();
        if owners.is_empty() {
            return;
        }
        let tick = self.local_control_submission_tick();
        // A release with no matching press takes the complete synchronized
        // PlayerControl route but C4Player::InCom drops it before DirectCom
        // or pressed-state mutation (C4Player.cpp:1541-1548). Keep the probe
        // observational so repeated benchmark runs follow the same game state.
        for owner in owners {
            let left_release = clonk_engine::PlayerControlData {
                player: owner,
                command: i32::from(clonk_engine::COM_LEFT + clonk_engine::COM_RELEASE_OFFSET),
                data: 0,
                by_client,
            };
            let right_release = clonk_engine::PlayerControlData {
                command: i32::from(clonk_engine::COM_RIGHT + clonk_engine::COM_RELEASE_OFFSET),
                ..left_release
            };
            if let Some(benchmark) = self.input_latency_benchmark.as_mut() {
                benchmark.record_submission(tick, &left_release, now);
                benchmark.record_submission(tick, &right_release, now);
            }
            if let Some(network) = self.network.as_ref() {
                network.submit_local_control(
                    owner,
                    ControlEvent::Release(ControlButton::Left),
                    tick,
                );
                network.submit_local_control(
                    owner,
                    ControlEvent::Release(ControlButton::Right),
                    tick,
                );
            }
        }
    }

    pub(crate) fn synchronized_player_profile_path(
        &self,
        info: &clonk_engine::ControlPlayerInfoEntry,
    ) -> Option<PathBuf> {
        if let Some(path) = self.local_player_profile_paths.get(&info.id) {
            return Some(path.clone());
        }
        if let Some(path) = info.resource.as_ref().and_then(|resource| {
            self.admission_resources
                .complete_path(resource.id)
                .map(Path::to_path_buf)
        }) {
            return Some(path);
        }
        if let Some(path) = self
            .startup_player_files
            .iter()
            .find(|player| {
                clonk_script::c4_string_bytes(&player.file_name)
                    .eq_ignore_ascii_case(info.filename.as_bytes())
            })
            .map(|player| player.path.clone())
        {
            return Some(path);
        }

        let configured = path_from_group_name_bytes(info.filename.as_bytes());
        if configured.as_os_str().is_empty() {
            // A filename-less info has no profile to synchronize. Native
            // C4Player::Save fails on its empty Filename at EraseItem/
            // C4Group_MoveItem (C4Player.cpp:454-456) without ever touching
            // the installation; resolving "" against the install root would
            // name the install root itself as the profile to rewrite.
            return None;
        }
        if configured.exists() {
            return Some(configured);
        }
        self.app_paths
            .as_ref()
            .map(|paths| paths.install_root().join(configured))
            .filter(|path| path.exists())
    }

    /// Persist the application-owned half of
    /// `C4PlayerList::SynchronizeLocalFiles`. The engine has already applied
    /// `C4Player::LocalSync`'s time checkpoint at this boundary.
    pub(crate) fn persist_synchronized_local_player_files(&mut self) -> bool {
        tracing::info!("synchronizing local player files");
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or_else(|| self.offline_local_client_id());
        let league = self.network_is_league;
        let max_players = self.engine.max_players();
        let candidates = self
            .engine
            .players()
            .map(|player| {
                (
                    player.id(),
                    player.player_info_id(),
                    synchronized_player_file_policy(
                        player.status(),
                        player.is_script_player(),
                        player.at_client().get(),
                        local_client_id,
                        league,
                        max_players,
                    ),
                )
            })
            .collect::<Vec<_>>();
        let maker = self.process_group_maker.as_bytes().to_vec();
        let (add_new_crew_portraits, save_default_portraits, player_rank_name_default) =
            self.developer_console_player_save_options();
        let options = clonk_engine::LiveC4PlayerSaveOptions {
            savegame: false,
            store_tiny: false,
            add_new_crew_portraits,
            save_default_portraits,
            player_rank_name_default: &player_rank_name_default,
        };
        let mut success = true;

        for (player_number, info_id, policy) in candidates {
            let local_control = match policy {
                SynchronizedPlayerFilePolicy::Skip => continue,
                SynchronizedPlayerFilePolicy::BlockedRemote => {
                    success = false;
                    continue;
                }
                SynchronizedPlayerFilePolicy::Persist { local_control } => local_control,
            };
            if !local_control {
                // Native permanently strips missing-definition crew before
                // opening its temporary remote group. Keep that mutation
                // even when Rust-only provenance lookup fails afterward.
                clonk_engine::strip_unresolved_remote_crew_for_synchronization(
                    &mut self.engine,
                    player_number,
                );
            }
            let Some(info) = self.control_player_infos.get(info_id).cloned() else {
                tracing::warn!(
                    player_number,
                    info_id,
                    "cannot save player without PlayerInfo"
                );
                success = false;
                continue;
            };
            let Some(path) = self.synchronized_player_profile_path(&info) else {
                tracing::warn!(
                    player_number,
                    info_id,
                    filename = %info.filename.to_string_lossy(),
                    "cannot resolve synchronized player profile path"
                );
                success = false;
                continue;
            };
            let saved = (|| -> Result<()> {
                // C4Player::Save ignores a failed source copy before opening
                // its temporary group with create enabled. Consequently a
                // local profile removed after admission is recreated from
                // the live core and roster instead of becoming ineligible.
                let original = if local_control && path.exists() {
                    Some(
                        open_group_path_for_folder_map(&path)
                            .with_context(|| format!("open player profile {}", path.display()))?,
                    )
                } else {
                    None
                };
                let synchronized = clonk_engine::serialize_live_c4_player_for_synchronization(
                    &mut self.engine,
                    player_number,
                    info.filename.as_bytes(),
                    &maker,
                    local_control,
                    original.as_ref(),
                    options,
                )
                .with_context(|| format!("serialize player profile {}", path.display()))?;
                let mut group = if let Some(original) = original.as_ref() {
                    developer_console_save::overlay_live_player_group_with_cleanup(
                        original,
                        &synchronized.group,
                        &synchronized.crew_cleanup,
                    )?
                } else {
                    // Remote profiles and missing local profiles both start
                    // from the freshly serialized group. Remote saves use
                    // fStoreTiny=true; local saves retain the full payload.
                    synchronized.group
                };
                if !self.process_group_maker.as_bytes().is_empty() {
                    group.set_maker_bytes_recursively(self.process_group_maker.as_bytes());
                }
                // Native snapshots fOfficial after serializing and before
                // Derive, then consults that same value after the move.
                let official_derivation = self.engine.is_control_host();
                let derivation = self.network.as_ref().and_then(|network| {
                    let resource = info.resource.as_ref()?;
                    let resource_id = self.admission_resources.derivation_target(resource.id)?;
                    let ownership = if local_control {
                        clonk_network::ResourceFileOwnership::Persistent
                    } else {
                        clonk_network::ResourceFileOwnership::Temporary
                    };
                    match network.begin_resource_derive(resource_id, path.clone(), ownership) {
                        Ok(derivation) => Some((derivation, ownership)),
                        Err(error) => {
                            // C4Player::Save proceeds when Derive returns null;
                            // the failed rescue only makes this update
                            // unavailable as an official network resource.
                            tracing::warn!(
                                player_number,
                                info_id,
                                resource_id,
                                path = %path.display(),
                                %error,
                                "failed to protect player resource before synchronization"
                            );
                            None
                        }
                    }
                });
                persist_console_save_group(&group, &path, local_control && path.is_dir())
                    .with_context(|| format!("persist player profile {}", path.display()))?;
                if official_derivation {
                    if let (Some(network), Some((derivation, ownership))) =
                        (self.network.as_ref(), derivation)
                    {
                        match network.finish_resource_derive(derivation) {
                            Ok(core) => self.admission_resources.register_finished_derivation(
                                &core,
                                path.clone(),
                                ownership,
                            ),
                            Err(error) => {
                                // FinishDerive's result is ignored by
                                // C4Player::Save; the profile itself has
                                // already been saved.
                                tracing::warn!(
                                    player_number,
                                    info_id,
                                    path = %path.display(),
                                    %error,
                                    "failed to publish synchronized player resource derivation"
                                );
                            }
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = saved {
                tracing::warn!(
                    player_number,
                    info_id,
                    path = %path.display(),
                    %error,
                    "failed to synchronize player profile"
                );
                success = false;
            }
        }
        success
    }

    pub(crate) fn ensure_local_player_registered(&mut self) -> Result<(), EngineError> {
        if self.engine.player(self.local_owner).is_some() {
            return Ok(());
        }
        let control = self.local_controls.initialize(LocalControlInit {
            owner: self.local_owner,
            preferred_set: 0,
            prefers_mouse: true,
            gamepads_enabled: self.gamepads_enabled,
            replay: false,
            disable_mouse: !self.mouse_control_allowed,
        });
        let config = PlayerConfig::new(self.local_owner, self.player_name.clone());
        if let Err(error) = self
            .engine
            .register_player_with_runtime_control(config, control.runtime_control())
        {
            self.remove_local_control_assignment(self.local_owner);
            return Err(error);
        }
        self.mouse_control = self.local_controls.mouse_owner().is_some();
        Ok(())
    }

    /// Fills the C4ObjectInfo-backed crew fields (`pObj->Info`): name, rank
    /// and rank name. The cursor label above the flashing mark draws from
    /// these (C4Game::DrawCursors, src/C4Game.cpp:1873-1887) — independent of
    /// the ShowPortraits flag gating [`Self::populate_crew_portraits`].
    pub(crate) fn populate_crew_infos(&self, players: &mut [PlayerOverlay]) {
        for player in players.iter_mut() {
            for crew in player.crew.iter_mut() {
                if let Some(info) = self.engine.crew_object_info(crew.object_id) {
                    crew.info_name = Some(c4_presentation_text(&info.name));
                    crew.rank = info.rank;
                    crew.rank_name = Some(info.rank_name.clone());
                }
            }
        }
    }

    /// Prepare the object-name half of `C4Object::DrawTopFace`. Geometry,
    /// viewport clipping and the final player-color draw stay in the frontend,
    /// while the app resolves C4ObjectInfo/definition names and the
    /// process-local invisible-player flag.
    pub(crate) fn crew_name_overlays(
        &self,
        viewports: &[ViewportInput<'_>],
    ) -> Vec<CrewNameOverlay> {
        if (!self.display_flags.player_names && !self.display_flags.clonk_names)
            || self.engine.film_replay()
        {
            return Vec::new();
        }

        let mut viewers = Vec::new();
        for viewport in viewports {
            if !viewers.contains(&viewport.owner) {
                viewers.push(viewport.owner);
            }
        }

        self.snapshot
            .objects
            .iter()
            .filter(|object| {
                object.status == clonk_engine::ObjectStatus::Normal
                    && object.ocf & clonk_engine::ocf::CREW_MEMBER != 0
                    && object.container.is_none()
            })
            .filter_map(|object| {
                let owner = self.engine.player(object.owner)?;
                let invisible = self
                    .control_player_infos
                    .get(owner.player_info_id())
                    .is_some_and(|info| info.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE != 0);
                if invisible {
                    return None;
                }

                let player_name = c4_presentation_text(owner.name());
                let clonk_name = object
                    .custom_name
                    .as_deref()
                    .filter(|name| !name.is_empty())
                    .or_else(|| {
                        self.engine
                            .crew_object_info(object.id)
                            .map(|info| info.name.as_str())
                    })
                    .or_else(|| self.engine.definition_name(&object.definition_id))
                    .unwrap_or(object.definition_id.as_str());
                let clonk_name = c4_presentation_text(clonk_name);
                let text = match (
                    self.display_flags.player_names,
                    self.display_flags.clonk_names,
                ) {
                    (true, true) => format!("{clonk_name} ({player_name})"),
                    (true, false) => player_name,
                    (false, true) => clonk_name,
                    (false, false) => unreachable!("empty display-name flags returned above"),
                };

                let visible_to = viewers
                    .iter()
                    .copied()
                    .filter(|viewer| {
                        if *viewer == object.owner {
                            return false;
                        }
                        let Some(viewer_player) = self.engine.player(*viewer) else {
                            return *viewer == OWNER_NONE;
                        };
                        !viewer_player.is_hostile_towards(object.owner)
                            && !owner.is_hostile_towards(*viewer)
                    })
                    .collect::<Vec<_>>();
                (!visible_to.is_empty()).then_some(CrewNameOverlay {
                    object_id: object.id,
                    text,
                    visible_to,
                })
            })
            .collect()
    }

    /// Fills the presentation half of the crew overlays: the selected info
    /// portrait (C4ObjectInfo::Draw, src/C4ObjectInfo.cpp:308-320), the crew
    /// name and rank (src/C4ObjectInfo.cpp:330-370) and the def rank symbols
    /// (src/C4ObjectInfo.cpp:334-341).
    pub(crate) fn populate_crew_portraits(&self, players: &mut [PlayerOverlay]) {
        // Config.Graphics.ShowPortraits from the Display menu
        // (C4MainMenu.cpp:872) gates only the portrait branch.
        let show_portraits = self.display_flags.portraits;
        let mut portrait_cache: HashMap<(String, String), Option<CursorPortraitImages>> =
            HashMap::new();
        let mut rank_cache: HashMap<String, Option<ImageData>> = HashMap::new();

        for player in players.iter_mut() {
            for crew in player.crew.iter_mut() {
                let current_portrait =
                    self.engine
                        .crew_object_info(crew.object_id)
                        .and_then(|info| {
                            crew.label = c4_presentation_text(&info.name);
                            crew.rank = info.rank;
                            info.portraits.current.clone()
                        });
                let Some(object) = self.snapshot.object(crew.object_id) else {
                    crew.portrait = None;
                    crew.portrait_owner_overlay = None;
                    crew.portrait_owner_color = u32::MAX;
                    continue;
                };

                let definition_id = object.definition_id.clone();
                let owner_color = cursor_portrait_owner_color(&self.snapshot, object.owner);
                let portrait = if show_portraits {
                    current_portrait.and_then(|portrait| {
                        let source = portrait.source?;
                        portrait_cache
                            .entry((source.clone(), portrait.name.clone()))
                            .or_insert_with(|| {
                                self.engine
                                    .definition_named_portrait_graphics_image(
                                        &source,
                                        &portrait.name,
                                    )
                                    .map(cursor_portrait_images)
                            })
                            .clone()
                    })
                } else {
                    None
                };
                crew.portrait = portrait.as_ref().map(|portrait| portrait.base.clone());
                crew.portrait_owner_overlay = portrait.and_then(|portrait| portrait.owner_overlay);
                crew.portrait_owner_color = owner_color;
                crew.rank_symbols = rank_cache
                    .entry(definition_id.clone())
                    .or_insert_with(|| {
                        self.engine
                            .definition_rank_symbols_image(&definition_id)
                            .map(|image| {
                                ImageData::from_arc(image.width(), image.height(), image.pixels())
                            })
                    })
                    .clone();
                crew.rank_symbol_count = self.engine.definition_rank_symbol_count(&definition_id);
            }
        }
    }

    pub(crate) fn dispatch_control_event_for_local_player(
        &mut self,
        owner: i32,
        event: ControlEvent,
    ) -> Result<(), EngineError> {
        let mut event = event;
        let cursor_menu_text_progressing = (self.object_menu.is_none()
            && !self.ingame_menu_belongs_to(owner)
            && self.save_browser.is_none())
        .then(|| {
            self.engine
                .cursor_object_menu(owner)
                .map(|(_, menu)| menu.text_progressing)
        })
        .flatten();
        if let Some(text_progressing) = cursor_menu_text_progressing {
            if let Some(mapped) = map_async_cursor_menu_control_event(event, text_progressing) {
                event = mapped;
            }
        }
        let local_main_menu_control = self.ingame_menu_belongs_to(owner)
            || (owner == self.local_owner && self.save_browser.is_some())
            || matches!(
                event,
                ControlEvent::Command {
                    command: ControlCommand::PlayerMenu,
                    ..
                }
            );
        if self.menu_controls_active_for(owner) {
            if let Some(mapped) = map_menu_control_event(event) {
                event = mapped;
            }
        }
        // C4Game::LocalPlayerControl handles COM_PlayerMenu and an active
        // C4MainMenu locally; only cursor/object-menu controls enter the
        // synchronized input queue (src/C4Game.cpp:3595-3624).
        if self.mode == AppMode::Running && (self.network.is_none() || local_main_menu_control) {
            let consumed = if let ControlEvent::Command { command, kind } = event {
                self.handle_menu_command_failsafe(owner, command, kind)?
            } else {
                false
            };
            if consumed {
                return Ok(());
            }
            if self.ingame_menu_belongs_to(owner)
                || (owner == self.local_owner
                    && (self.object_menu.is_some() || self.save_browser.is_some()))
            {
                return Ok(());
            }
        }
        // clonk-rs divergence: C4Game::LocalControlKeyUp only creates a
        // synchronized control for AutoStopControl players, so classic control
        // never delivers Control*Released (C4Game.cpp:3592-3605). The port
        // synchronizes the key-up in both styles instead, so scripts can act
        // on a held key in either mode; classic movement is unaffected because
        // C4Object::DirectCom's procedure switch has no release arm.
        if let Some(network) = self.network.as_ref() {
            let tick = self.local_control_submission_tick();
            network.submit_local_control(owner, event, tick);
            return Ok(());
        }
        self.dispatch_control_event_for_owner(owner, event)
    }

    pub(crate) fn execute_player_control_failsafe(
        &mut self,
        owner: i32,
        command: i32,
        data: i32,
    ) -> Result<(), EngineError> {
        if let Err(err) = self.engine.execute_player_control(owner, command, data) {
            let status = control_script_error_to_status(err)?;
            tracing::error!(status, "control script error (non-fatal like C++)");
            self.status_text = status;
        }
        Ok(())
    }

    pub(crate) fn submit_or_execute_player_command(
        &mut self,
        command: PlayerCommandControlData,
    ) -> Result<(), EngineError> {
        if self.network.is_some() {
            let tick = self.local_control_submission_tick();
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_player_command(tick, command))
            {
                tracing::warn!(player = command.player, %error, "failed to queue player command");
                self.status_text = "Failed to queue mouse command".to_string();
            }
            return Ok(());
        }

        self.record_control_batch(std::slice::from_ref(
            &clonk_engine::ControlPacket::PlayerCommand(command),
        ));
        self.execute_player_command_failsafe(command)
    }

    pub(crate) fn submit_or_execute_player_select(
        &mut self,
        selection: PlayerSelectControlData,
    ) -> Result<(), EngineError> {
        if self.network.is_some() {
            let tick = self.local_control_submission_tick();
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_player_select(tick, selection.clone()))
            {
                tracing::warn!(player = selection.player, %error, "failed to queue player selection");
                self.status_text = "Failed to queue mouse selection".to_string();
            }
            return Ok(());
        }

        self.record_control_batch(std::slice::from_ref(
            &clonk_engine::ControlPacket::PlayerSelect(selection.clone()),
        ));
        self.engine.execute_player_select(&selection).map(|_| ())
    }

    pub(crate) fn execute_player_command_failsafe(
        &mut self,
        command: PlayerCommandControlData,
    ) -> Result<(), EngineError> {
        if let Err(err) = self.engine.execute_player_command(
            command.player,
            command.command,
            command.x,
            command.y,
            command.target,
            command.target2,
            command.data,
            command.add_mode,
        ) {
            let status = control_script_error_to_status(err)?;
            tracing::error!(status, "player-command script error (non-fatal like C++)");
            self.status_text = status;
        }
        Ok(())
    }

    /// Ordered `C4MN_TeamSelection` / `C4MN_TeamSwitch` rows. Native shows
    /// every configured team and adds `TEAMID_New` only when auto-generation
    /// is enabled and no existing team is empty (C4MainMenu.cpp:175-232).
    pub(crate) fn team_selection_entries(&self) -> Vec<TeamSelectionEntry> {
        let mut add_new_team = self.engine.auto_generate_teams();
        let mut entries = self
            .engine
            .teams()
            .iter()
            .map(|team| {
                let participants = self
                    .engine
                    .players()
                    .filter(|player| player.team() == Some(team.id))
                    .map(|player| c4_presentation_text(player.name()))
                    .collect::<Vec<_>>();
                let has_participants = !team.player_ids.is_empty();
                if !has_participants {
                    add_new_team = false;
                }
                let team_name = c4_presentation_text(&team.name);
                let caption = if participants.is_empty() {
                    team_name
                } else {
                    format!("{} ({})", team_name, participants.join(", "))
                };
                TeamSelectionEntry {
                    id: team.id,
                    caption,
                    icon_spec: team.icon_spec.clone(),
                    color: team.color,
                    has_participants,
                }
            })
            .collect::<Vec<_>>();
        if add_new_team {
            entries.push(TeamSelectionEntry {
                id: -1,
                caption: "New Team".to_string(),
                icon_spec: None,
                color: 0,
                has_participants: false,
            });
        }
        entries
    }

    pub(crate) fn cache_team_selection_icons(&mut self, entries: &[TeamSelectionEntry]) {
        let team_icons = {
            let resources = self.script_text_spec_resources();
            entries
                .iter()
                .filter_map(|entry| {
                    let icon_spec = entry.icon_spec.as_deref()?;
                    resolve_script_font_image(&self.engine, icon_spec, entry.color, resources)
                        .map(|image| (entry.id, image))
                })
                .collect()
        };
        self.ensure_ingame_menu_gfx().team_icons = team_icons;
    }

    pub(crate) fn open_initial_team_selection(&mut self, owner: i32) {
        if !self
            .engine
            .player(owner)
            .is_some_and(|player| player.status() == clonk_engine::PlayerStatus::TeamSelection)
        {
            return;
        }
        let entries = self.team_selection_entries();
        let existing = self
            .ingame_menu
            .get(owner)
            .filter(|menu| menu.page() == ingame_menu::MenuPage::TeamSelection);
        let unchanged = existing.is_some_and(|menu| {
            menu.items().len() == entries.len()
                && menu.items().iter().zip(&entries).all(|(item, entry)| {
                    item.caption == entry.caption
                        && item.symbol == entry.symbol()
                        && item.action == MenuAction::SelectTeam(entry.id)
                })
        });
        let already_open = existing.is_some();
        if unchanged {
            return;
        }
        self.cache_team_selection_icons(&entries);
        // An already-open page is refilled in place. C4Menu keeps the menu
        // instance across `ClearItems(false)`, so its dragged position,
        // scroll, tooltip age and numeric selection survive; only
        // `AdjustSelection` clamps an out-of-range row (C4Menu.cpp:947-973).
        if already_open {
            let labels = self.ingame_menu_labels();
            if let Some(menu) = self.ingame_menu.get_mut(owner) {
                menu.refill_team(&entries, false, &labels);
            }
            return;
        }
        if owner == self.local_owner {
            self.close_object_menu();
        }
        self.ingame_menu.replace(
            owner,
            Some(IngameMenuState::team_selection_menu(
                &entries,
                &self.ingame_menu_labels(),
            )),
        );
    }

    /// `C4Player::Execute`'s PS_TeamSelection branch: a sole joinable team
    /// bypasses the menu and is submitted through the synchronized control
    /// path; ambiguous choices keep the selection menu open
    /// (C4Player.cpp:159-173; C4Teams.cpp:876-914).
    fn execute_local_team_selection(&mut self, owner: i32) -> Result<(), EngineError> {
        if !self
            .engine
            .player(owner)
            .is_some_and(|player| player.status() == clonk_engine::PlayerStatus::TeamSelection)
        {
            return Ok(());
        }
        let Some(team) = self.engine.forced_team_selection(owner) else {
            self.open_initial_team_selection(owner);
            return Ok(());
        };
        if self
            .ingame_menu
            .get(owner)
            .is_some_and(|menu| menu.page() == ingame_menu::MenuPage::TeamSelection)
        {
            self.close_ingame_menu_for_player(owner);
        }
        self.engine.mark_team_selection_pending(owner)?;
        if self.network.is_some() {
            let tick = self.local_control_submission_tick();
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_init_scenario_player(tick, owner, team))
            {
                tracing::warn!(player = owner, team, %error, "failed to queue forced team selection");
            }
        } else {
            self.record_control_batch(std::slice::from_ref(
                &clonk_engine::ControlPacket::InitScenarioPlayer(
                    clonk_engine::InitScenarioPlayerControlData {
                        team,
                        player: owner,
                        by_client: 0,
                    },
                ),
            ));
            self.execute_init_scenario_player_control(owner, team)?;
        }
        Ok(())
    }

    pub(crate) fn execute_local_team_selections(&mut self) -> Result<Vec<i32>, EngineError> {
        let mut owners = self.local_controls.owners().collect::<Vec<_>>();
        for &owner in &owners {
            self.execute_local_team_selection(owner)?;
        }
        owners.retain(|owner| self.engine.player(*owner).is_some());
        self.move_classic_primary_viewport_first(&mut owners);
        Ok(owners)
    }

    pub(crate) fn execute_presentation_benchmark_team_selection_controls(
        &mut self,
        controls: &[clonk_engine::InitScenarioPlayerControlData],
    ) -> Result<(), String> {
        for control in controls {
            if control.by_client != 0 {
                return Err(format!(
                    "benchmark player {} team {} is not an offline host control",
                    control.player, control.team
                ));
            }
            if !self
                .engine
                .player(control.player)
                .is_some_and(|player| player.status() == clonk_engine::PlayerStatus::TeamSelection)
            {
                return Err(format!(
                    "benchmark player {} is not awaiting team {}",
                    control.player, control.team
                ));
            }
            if !self
                .engine
                .teams()
                .iter()
                .any(|team| team.id == control.team)
            {
                return Err(format!(
                    "benchmark player {} requested unavailable team {}",
                    control.player, control.team
                ));
            }
        }
        for control in controls {
            self.engine
                .mark_team_selection_pending(control.player)
                .map_err(|error| error.to_string())?;
        }
        let packets = controls
            .iter()
            .copied()
            .map(clonk_engine::ControlPacket::InitScenarioPlayer)
            .collect::<Vec<_>>();
        self.record_control_batch(&packets);
        for control in controls {
            self.execute_init_scenario_player_control(control.player, control.team)
                .map_err(|error| error.to_string())?;
            if self.engine.player(control.player).is_some_and(|player| {
                matches!(
                    player.status(),
                    clonk_engine::PlayerStatus::TeamSelection
                        | clonk_engine::PlayerStatus::TeamSelectionPending
                )
            }) {
                return Err(format!(
                    "benchmark player {} did not initialize on team {}",
                    control.player, control.team
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn execute_init_scenario_player_control(
        &mut self,
        player: i32,
        team: i32,
    ) -> Result<(), EngineError> {
        let initialized = self.engine.initialize_scenario_player(player, team)?;
        self.snapshot = self.engine.snapshot();
        if initialized.is_some() {
            if self.refresh_current_player_info_teams() {
                self.publish_current_host_player_infos();
            } else {
                self.publish_updated_host_join_snapshot();
            }
            self.apply_focus_selection();
            self.snapshot = self.engine.snapshot();
            self.refresh_focus();
        } else if self.local_controls.owners().any(|owner| owner == player) {
            self.open_initial_team_selection(player);
        }
        Ok(())
    }

    pub(crate) fn hostility_entries_for_player(&self, player: i32) -> Option<Vec<HostilityEntry>> {
        let owner = self.engine.player(player)?;
        Some(
            self.engine
                .players()
                .filter(|opponent| {
                    opponent.id() != player
                        && !self
                            .control_player_infos
                            .get(opponent.player_info_id())
                            .is_some_and(|info| {
                                info.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE != 0
                            })
                })
                .map(|opponent| HostilityEntry {
                    opponent: opponent.id(),
                    name: c4_presentation_text(opponent.name()),
                    hostile: owner.is_hostile_towards(opponent.id()),
                    opponent_hostile: opponent.is_hostile_towards(player),
                })
                .collect(),
        )
    }

    /// Populate the app-owned counterpart of `C4Player::BigIcon` from the
    /// same local player selection or completed network resource that
    /// created the runtime player. Cache by PlayerInfo ID because runtime
    /// numbers may be recreated while loading a savegame.
    pub(crate) fn hydrate_runtime_player_big_icons(&mut self) {
        if !self.display_flags.portraits {
            return;
        }
        self.hydrate_runtime_player_big_icons_unconditionally();
    }

    /// `C4RoundResultsPlayer::EvaluatePlayer` freezes `C4Player::BigIcon`
    /// independently of the viewport-only ShowPortraits switch.
    pub(crate) fn hydrate_runtime_player_big_icons_for_evaluation(&mut self) {
        self.hydrate_runtime_player_big_icons_unconditionally();
    }

    /// `C4RoundResultsPlayer::EvaluatePlayer` copies `C4Player::BigIcon` into
    /// the frozen round result while the player is still alive
    /// (src/C4RoundResults.cpp:52-73,338-344), so an eliminated, retired or
    /// disconnected player's icon outlives its removal and its player
    /// resource. Freeze on that same event instead of only when the
    /// evaluation dialog is constructed.
    pub(crate) fn freeze_evaluated_player_big_icons(&mut self) {
        let pending = self
            .engine
            .round_results
            .players
            .iter()
            .map(|result| result.player_info_id)
            .filter(|info_id| {
                !self.runtime_player_big_icons.contains_key(info_id)
                    && !self.runtime_player_big_icon_misses.contains(info_id)
            })
            .collect::<HashSet<_>>();
        self.hydrate_player_big_icons(pending);
    }

    fn hydrate_runtime_player_big_icons_unconditionally(&mut self) {
        let pending = self
            .engine
            .players()
            .map(|player| player.player_info_id())
            .filter(|info_id| {
                !self.runtime_player_big_icons.contains_key(info_id)
                    && !self.runtime_player_big_icon_misses.contains(info_id)
            })
            .collect::<HashSet<_>>();
        self.hydrate_player_big_icons(pending);
    }

    fn hydrate_player_big_icons(&mut self, pending: HashSet<i32>) {
        for info_id in pending {
            let Some(info) = self.control_player_infos.get(info_id).cloned() else {
                continue;
            };
            if let Some(startup) = self.startup_player_files.iter().find(|startup| {
                clonk_script::c4_string_bytes(&startup.file_name) == info.filename.as_bytes()
            }) {
                if let Some(icon) = startup.render_model.big_icon.clone() {
                    self.runtime_player_big_icons.insert(info_id, icon);
                } else {
                    self.runtime_player_big_icon_misses.insert(info_id);
                }
                continue;
            }

            let has_resource = info.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE != 0;
            let complete_path = info.resource.as_ref().and_then(|resource| {
                self.admission_resources
                    .complete_path(resource.id)
                    .map(Path::to_path_buf)
            });
            if let Some(path) = complete_path {
                if let Some(icon) = load_network_player_big_icon(&path) {
                    self.runtime_player_big_icons.insert(info_id, icon);
                } else {
                    self.runtime_player_big_icon_misses.insert(info_id);
                }
            } else if !has_resource {
                // Fileless script/scenario players have no player-group
                // BigIcon to discover later.
                self.runtime_player_big_icon_misses.insert(info_id);
            }
        }
    }

    pub(crate) fn cache_joined_player_big_icon(&mut self, info_id: i32, icon: Option<&ImageData>) {
        self.runtime_player_big_icon_misses.remove(&info_id);
        if let Some(icon) = icon {
            self.runtime_player_big_icons.insert(info_id, icon.clone());
        } else {
            self.runtime_player_big_icons.remove(&info_id);
            self.runtime_player_big_icon_misses.insert(info_id);
        }
    }

    pub(crate) fn available_runtime_player_files(&self) -> Vec<NewPlayerEntry> {
        // ActivateNewPlayer walks DirectoryIterator without reordering and
        // rejects directory groups and files already used by Game.Players
        // (src/C4MainMenu.cpp:59-121; src/C4PlayerList.cpp:433-451).
        let local_client_id = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or_else(|| self.offline_local_client_id());
        let joined_player_paths = self
            .control_player_infos
            .retained_rows_snapshot()
            .1
            .into_iter()
            .filter(|(client_id, _, _)| *client_id == local_client_id)
            .flat_map(|(_, _, players)| players)
            .filter(|player| player.is_joined() && !player.filename.is_empty())
            .map(|player| PathBuf::from(player.filename.to_string_lossy().into_owned()))
            .collect::<Vec<_>>();
        self.startup_player_files
            .iter()
            .filter(|player| {
                player.path.is_file()
                    && !joined_player_paths.iter().any(|joined| {
                        match (
                            offline_player_real_path(joined),
                            offline_player_real_path(&player.path),
                        ) {
                            (Ok(joined), Ok(candidate)) => {
                                offline_player_paths_identical(&joined, &candidate)
                            }
                            _ => offline_player_paths_identical(joined, &player.path),
                        }
                    })
            })
            .map(|player| NewPlayerEntry {
                file: player.path.to_string_lossy().into_owned(),
                name: c4_presentation_text(&player.player_file.name),
            })
            .collect()
    }

    /// `C4MN_Observer` rows use live `C4PlayerList` order and omit only
    /// players whose linked `C4PlayerInfo` has `PIF_Invisible`.
    pub(crate) fn observer_player_entries(&self) -> Vec<ObserverPlayerEntry> {
        self.engine
            .players()
            .filter(|player| {
                !self
                    .control_player_infos
                    .get(player.player_info_id())
                    .is_some_and(|info| info.flags & clonk_engine::PLAYER_INFO_FLAG_INVISIBLE != 0)
            })
            .map(|player| ObserverPlayerEntry {
                id: player.id(),
                name: c4_presentation_text(player.name()),
            })
            .collect()
    }

    fn issue_reserved_joins_for_player_snapshot(
        &mut self,
        client_id: i32,
        players: &[clonk_engine::ControlPlayerInfoEntry],
    ) {
        let resources = &self.admission_resources;
        let joins =
            self.control_player_infos
                .issue_reserved_player_snapshots(client_id, players, |core| {
                    resources.complete_path(core.id).and_then(|path| {
                        clonk_engine::LegacyCString::from_bytes(path_to_legacy_bytes(path))
                    })
                });
        let tick = self.local_control_submission_tick();
        for join in joins {
            if let Some(Err(error)) = self
                .network
                .as_ref()
                .map(|network| network.submit_join_player(tick, join))
            {
                tracing::error!(%error, "failed to submit synchronized JoinPlayer");
            }
        }
    }

    pub(crate) fn handle_script_player_info_updates(&mut self) -> Result<(), EngineError> {
        // SetMaxPlayer writes Game.Parameters.MaxPlayers inside the engine's
        // synchronized script call. Mirror that value before either the
        // queued CreateScriptPlayer admission or the empty fast path.
        let mut host_snapshot_changed = false;
        if let Some(max_players) = self.engine.max_players() {
            self.network_max_players = usize::try_from(max_players).unwrap_or(0);
            if let Some(snapshot) = self.host_join_snapshot.as_mut() {
                if snapshot.parameters.max_players != max_players {
                    snapshot.parameters.max_players = max_players;
                    host_snapshot_changed = true;
                }
            }
        }
        let progress_updates = self.engine.take_player_info_league_progress_updates();
        let mut player_infos_changed = self.refresh_current_player_info_teams();
        for (player_info_id, data) in progress_updates {
            player_infos_changed |= self
                .control_player_infos
                .set_league_progress_data(player_info_id, data);
        }
        if player_infos_changed {
            host_snapshot_changed |= self.refresh_current_host_player_infos();
        }
        if host_snapshot_changed {
            self.publish_updated_host_join_snapshot();
        }
        let updates = self.engine.take_script_player_info_updates();
        if updates.is_empty() {
            return Ok(());
        }
        match self.runtime_network_role() {
            RuntimeNetworkRole::Host => {
                let Some(network) = self.network.as_ref() else {
                    return Ok(());
                };
                for update in updates {
                    if let Err(error) = network.submit_player_info_update(update) {
                        tracing::error!(%error, "failed to submit script-player PlayerInfo");
                    }
                }
            }
            RuntimeNetworkRole::Offline => {
                for update in updates {
                    let Some(info) = self
                        .control_player_infos
                        .admit_request(update, self.network_max_players)
                    else {
                        continue;
                    };
                    let client_id = info.client_id;
                    self.generate_incoming_player_info_teams(&info.players);
                    self.control_player_infos.apply(info);
                    self.recheck_team_memberships_from_player_infos();
                    seed_engine_player_info_parameters(
                        &mut self.engine,
                        &self.network_league_name,
                        &self.control_player_infos,
                    );
                    let joins = self
                        .control_player_infos
                        .issue_unjoined_local_players(client_id, |_| {
                            Some(clonk_engine::LegacyCString::default())
                        });
                    for join in joins {
                        self.apply_join_player_control(join)?;
                    }
                }
                // Offline admission applies JoinPlayer immediately, after the
                // regular tick snapshot was captured. Keep rendering and
                // recording on the post-control engine state.
                self.snapshot = self.engine.snapshot();
            }
            RuntimeNetworkRole::Client | RuntimeNetworkRole::Ambiguous => {
                tracing::debug!(
                    count = updates.len(),
                    "discarding non-host script-player requests"
                );
            }
        }
        Ok(())
    }

    pub(crate) fn apply_direct_player_info_control(
        &mut self,
        info: clonk_engine::PlayerInfoControlData,
        issue_joins: bool,
    ) -> Vec<clonk_engine::PlayerInfoControlData> {
        let client_id = info.client_id;
        let local_origin = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            == Some(info.by_client);
        let had_client_packet = self.control_player_infos.client_packet(client_id).is_some();
        let send_clean_follow_up = matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_)))
            && info.by_client == 0
            && info.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_UPDATED != 0
            && (info.flags & clonk_engine::CLIENT_PLAYER_INFO_FLAG_ADD_PLAYERS == 0
                || !had_client_packet);
        self.admission_resources
            .register_player_info_resources(&info.players);
        self.register_classic_lobby_player_resources(&info.players);
        self.generate_incoming_player_info_teams(&info.players);
        self.control_player_infos.apply(info);
        self.prune_host_local_alternate_colors();
        let rebalance_updates = self.recheck_team_memberships_from_player_infos();
        let follow_ups = if local_origin {
            let mut updated_clients = rebalance_updates
                .iter()
                .map(|update| update.client_id)
                .collect::<HashSet<_>>();
            if send_clean_follow_up {
                updated_clients.insert(client_id);
            }
            self.control_player_infos.client_packets(&updated_clients)
        } else {
            Vec::new()
        };
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.publish_current_host_player_infos();
        self.sync_classic_lobby_roster();
        self.sync_classic_lobby_resource_ready();
        let should_issue_joins = issue_joins
            && self.mode == AppMode::Running
            && matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_)))
            && self.control_clients.contains(client_id)
            && self.control_clients.is_activated(client_id);
        if should_issue_joins {
            self.issue_unjoined_joins_for_client(client_id);
        }
        follow_ups
    }

    pub(crate) fn broadcast_and_preexecute_player_info(
        &mut self,
        info: clonk_engine::PlayerInfoControlData,
        issue_joins_now: bool,
        capture_join_players_on_echo: bool,
    ) -> Result<()> {
        let mut controls = VecDeque::from([(info, issue_joins_now, capture_join_players_on_echo)]);
        while let Some((info, issue_joins_now, capture_join_players_on_echo)) = controls.pop_front()
        {
            let capture_join_players = capture_join_players_on_echo
                && self.mode == AppMode::Running
                && matches!(self.network_mode.as_ref(), Some(NetworkMode::Host(_)))
                && self.control_clients.contains(info.client_id)
                && self.control_clients.is_activated(info.client_id);
            let join_players_on_echo = if capture_join_players {
                let mut post_control = self.control_player_infos.clone();
                post_control.apply(info.clone());
                post_control
                    .client_packet(info.client_id)
                    .map(|packet| packet.players)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|player| {
                        player.flags & clonk_engine::PLAYER_INFO_FLAG_JOINED == 0
                            && player.savegame_player == 0
                    })
                    .collect()
            } else {
                Vec::new()
            };
            self.network
                .as_ref()
                .ok_or_else(|| anyhow!("network session is unavailable"))?
                .broadcast_preexecuted_player_info(info.clone(), join_players_on_echo.clone())?;
            let follow_ups = self.apply_direct_player_info_control(info, issue_joins_now);
            if capture_join_players {
                self.control_player_infos
                    .reserve_unjoined_player_snapshots(&join_players_on_echo);
            }
            for follow_up in follow_ups.into_iter().rev() {
                // HandlePlayerInfo's nested DirectExec precedes the next outer
                // admission control and inherits its join-issuance boundary.
                controls.push_front((follow_up, issue_joins_now, capture_join_players_on_echo));
            }
        }
        Ok(())
    }

    pub(crate) fn apply_preexecuted_player_info_echo(
        &mut self,
        original: clonk_engine::PlayerInfoControlData,
        info: clonk_engine::PlayerInfoControlData,
        mut join_players_on_echo: Vec<clonk_engine::ControlPlayerInfoEntry>,
    ) {
        let client_id = info.client_id;
        self.admission_resources
            .register_player_info_resources(&info.players);
        self.register_classic_lobby_player_resources(&info.players);
        for player in &mut join_players_on_echo {
            let Some(normalized) = info
                .players
                .iter()
                .find(|normalized| normalized.id == player.id)
            else {
                continue;
            };
            player.flags = (player.flags & !clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE)
                | (normalized.flags & clonk_engine::PLAYER_INFO_FLAG_HAS_RESOURCE);
            player.resource.clone_from(&normalized.resource);
        }
        if self
            .control_player_infos
            .apply_player_resource_normalization(&original, &info)
        {
            seed_engine_player_info_parameters(
                &mut self.engine,
                &self.network_league_name,
                &self.control_player_infos,
            );
            self.publish_current_host_player_infos();
        }
        if !join_players_on_echo.is_empty() {
            self.issue_reserved_joins_for_player_snapshot(client_id, &join_players_on_echo);
        }
        self.sync_classic_lobby_roster();
        self.sync_classic_lobby_resource_ready();
    }

    pub(crate) fn submit_runtime_offline_player(&mut self, file: &str) -> Result<(), String> {
        let source_path = Path::new(file);
        let player_file = PlayerFile::load_from_path(source_path)
            .map_err(|error| format!("failed to load {}: {error}", source_path.display()))?;
        let wire_name =
            clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(file))
                .ok_or_else(|| "player filename contains an interior NUL".to_string())?;
        let selected = SelectedClientPlayer::new(source_path, wire_name, player_file);
        let alternate_color = selected.alternate_color();
        let request = selected
            .offline_runtime_add_player_info_update(self.offline_local_client_id())
            .map_err(|error| error.to_string())?;
        self.refresh_current_player_info_teams();
        let next_info_id = self
            .control_player_infos
            .retained_rows_snapshot()
            .0
            .wrapping_add(1);
        let restore_players = Vec::new();
        let admission = match self.network_team_assignment.as_mut() {
            Some(team_assignment) => team_assignment
                .admit_request_with_alternate_colors(
                    &mut self.control_player_infos,
                    request,
                    self.network_max_players,
                    true,
                    false,
                    &restore_players,
                    |player| (player.id == next_info_id).then_some(alternate_color),
                )
                .map_err(|error| error.to_string())?,
            None => {
                let mut oracle = ProcessInitialHostTeamAssignmentOracle::new(
                    self.generated_team_name_template.clone(),
                );
                self.control_player_infos
                    .admit_request_with_attributes_and_alternate_colors(
                        request,
                        self.network_max_players,
                        None,
                        &restore_players,
                        &mut oracle,
                        |player| (player.id == next_info_id).then_some(alternate_color),
                    )
                    .map_err(|error| error.to_string())?
            }
        }
        .ok_or_else(|| "local player-info admission rejected the player".to_string())?;
        let clonk_engine::PlayerInfoAdmission {
            mut updated_existing,
            admitted,
            joined_player_team_updates,
        } = admission;
        for update in joined_player_team_updates {
            self.engine
                .apply_admitted_player_team_update(update.info_id, update.team, update.color)
                .map_err(|error| error.to_string())?;
        }
        let resolved_profile =
            offline_player_real_path(source_path).unwrap_or_else(|_| source_path.to_path_buf());
        for player in &admitted.players {
            self.local_player_profile_paths
                .insert(player.id, resolved_profile.clone());
        }
        updated_existing.push(admitted);
        let tick = self.local_control_submission_tick();
        let controls = updated_existing
            .into_iter()
            .map(NetworkControl::PlayerInfo)
            .collect();
        self.apply_ready_controls(tick, controls)
            .map_err(|error| error.to_string())?;
        self.snapshot = self.engine.snapshot();
        Ok(())
    }

    /// PlayerListItem's constructor restores a differing recorded team by
    /// cloning the complete owning client packet and submitting an update.
    /// Keep this side effect outside the pure row projector and run it once
    /// for each item construction, while authoritative state still waits for
    /// the network echo.
    pub(crate) fn submit_restart_restore_team_updates_for_new_roster_items(&mut self) {
        if self.restart_restore_infos.what & RESTART_RESTORE_PLAYER_TEAMS == 0
            || !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || self.network.is_none()
            || (self.classic_host_lobby.is_none() && self.network_lobby.is_none())
        {
            return;
        }

        let (_, packets) = self.control_player_infos.retained_rows_snapshot();
        let mut visible_items = HashSet::new();
        for (client_id, _, players) in &packets {
            if !self.control_clients.contains(*client_id)
                || self.control_clients.is_observer(*client_id)
            {
                continue;
            }
            visible_items.extend(players.iter().filter_map(|player| {
                (player.flags
                    & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                        | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                    == 0)
                    .then_some((*client_id, player.id))
            }));
        }
        self.restart_restore_roster_items
            .retain(|item| visible_items.contains(item));

        let mut requests = Vec::new();
        let mut restored_teams = Vec::new();
        for (client_id, flags, players) in packets {
            if !self.control_clients.contains(client_id)
                || self.control_clients.is_observer(client_id)
            {
                continue;
            }
            let mut working_players = players;
            for index in 0..working_players.len() {
                let player = &working_players[index];
                if player.flags
                    & (clonk_engine::PLAYER_INFO_FLAG_REMOVED
                        | clonk_engine::PLAYER_INFO_FLAG_INVISIBLE)
                    != 0
                    || !self
                        .restart_restore_roster_items
                        .insert((client_id, player.id))
                    || player.player_type != clonk_engine::PLAYER_INFO_TYPE_USER
                {
                    continue;
                }
                let lobby_name = restart_restore_lobby_name(player);
                let Some(restore) = self.restart_restore_infos.players.get(&lobby_name) else {
                    continue;
                };
                if restore.team == player.team {
                    continue;
                }
                let restored_team = restore.team;
                working_players[index].team = restored_team;
                restored_teams.push(restored_team);
                requests.push(clonk_network::PlayerInfoUpdateRequest {
                    client_id,
                    flags,
                    players: working_players.clone(),
                });
            }
        }

        let mut generated_team = false;
        if let Some(assignment) = self.network_team_assignment.as_mut() {
            for team in restored_teams {
                generated_team |= assignment.generate_team_for_id(team);
            }
        }
        if generated_team {
            if let (Some(assignment), Some(snapshot)) = (
                self.network_team_assignment.as_ref(),
                self.host_join_snapshot.as_mut(),
            ) {
                snapshot.parameters.teams =
                    clonk_network::join_team_list_snapshot(assignment.teams().clone());
            }
            self.publish_updated_host_join_snapshot();
        }

        let Some(network) = self.network.as_ref() else {
            return;
        };
        for request in requests {
            if let Err(error) = network.submit_player_info_update(request) {
                tracing::error!(%error, "failed to submit restart team PlayerInfo update");
            }
        }
    }

    /// Execute the host-only `CID_RemovePlr` body. Missing players are a
    /// synchronized no-op; a successful removal updates the retained
    /// C4PlayerInfo history after the engine has run the full removal cascade.
    pub(crate) fn execute_remove_player_control(
        &mut self,
        control: clonk_engine::RemovePlayerControlData,
    ) -> Result<(), EngineError> {
        if control.by_client != 0 {
            return Ok(());
        }
        let Some(info_id) = self
            .engine
            .player(control.player)
            .map(|player| player.player_info_id())
        else {
            return Ok(());
        };
        let game_part_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        self.remove_runtime_player_with_viewport_feedback(control.player)?;
        if info_id != 0
            && self.control_player_infos.mark_removed(
                info_id,
                control.disconnected,
                game_part_frame,
            )
        {
            self.prune_host_local_alternate_colors();
            self.publish_current_host_player_infos();
        }
        Ok(())
    }

    /// Move script-produced `CtrlRemove` requests into the next open host
    /// control tick. Offline control executes only on the next cadence frame;
    /// network hosts submit to the frame builder and wait for the resulting
    /// complete control packet to return.
    pub(crate) fn flush_pending_remove_player_controls(
        &mut self,
        execute_offline_control_frame: bool,
    ) -> Result<(), EngineError> {
        if self.network.is_none() {
            let control_rate = u64::try_from(self.engine.control_rate())
                .unwrap_or(1)
                .max(1);
            if !execute_offline_control_frame || !self.engine.frame().is_multiple_of(control_rate) {
                return Ok(());
            }
            let controls = self.engine.take_pending_remove_player_controls();
            for control in controls {
                self.record_control_batch(std::slice::from_ref(
                    &clonk_engine::ControlPacket::RemovePlayer(control),
                ));
                self.execute_remove_player_control(control)?;
            }
            return Ok(());
        }

        let controls = self.engine.take_pending_remove_player_controls();
        if controls.is_empty() {
            return Ok(());
        }
        let tick = self.local_control_submission_tick();
        for control in controls {
            if let Some(Err(error)) = self.network.as_ref().map(|network| {
                network.submit_remove_player(tick, control.player, control.disconnected)
            }) {
                tracing::warn!(player = control.player, %error, "failed to queue RemovePlr");
            }
        }
        Ok(())
    }

    pub(crate) fn remove_remote_runtime_players(&mut self, local_client_id: i32) {
        // ChangeToLocal calls C4ClientList::RemoveRemote in client-list order;
        // each C4Client::Remove repeatedly removes that client's first player
        // in C4PlayerList order with fNoCalls=false. This is not the silent
        // hard-abort path used by C4Game::Abort.
        let remote_clients = self
            .control_clients
            .snapshot()
            .into_iter()
            .map(|client| client.client_id)
            .filter(|client_id| *client_id != local_client_id)
            .collect::<Vec<_>>();
        let game_part_frame = i32::try_from(self.engine.frame()).unwrap_or(i32::MAX);
        for client_id in remote_clients {
            loop {
                let next = self
                    .engine
                    .players()
                    .find(|player| player.at_client().get() == client_id)
                    .map(|player| (player.id(), player.player_info_id()));
                let Some((player_id, info_id)) = next else {
                    break;
                };
                match self.remove_runtime_player_with_viewport_feedback(player_id) {
                    Ok(()) => {
                        self.control_player_infos
                            .mark_removed(info_id, true, game_part_frame);
                    }
                    Err(error) => {
                        tracing::warn!(%player_id, %info_id, %error, "failed to remove remote player");
                        break;
                    }
                }
            }
        }
        self.prune_host_local_alternate_colors();
    }

    /// `C4Network2Players::HandlePlayerInfo` immediately rechecks the live
    /// C4TeamList. Keep both the pre-activation registry and any retained
    /// JoinData projection at that same direct-control boundary.
    pub(crate) fn recheck_team_memberships_from_player_infos(
        &mut self,
    ) -> Vec<clonk_engine::PlayerInfoControlData> {
        self.reconcile_player_info_teams(true, true)
    }

    pub(crate) fn generate_incoming_player_info_teams(
        &mut self,
        players: &[clonk_engine::ControlPlayerInfoEntry],
    ) {
        let Some(assignment) = self.network_team_assignment.as_mut() else {
            return;
        };
        for team in players
            .iter()
            .map(|player| player.team)
            .filter(|team| *team != 0)
        {
            assignment.generate_team_for_id(team);
        }
    }

    pub(crate) fn recheck_team_memberships_without_random_rebalance(&mut self) {
        self.reconcile_player_info_teams(true, false);
    }

    pub(crate) fn recheck_random_teams_from_player_infos(
        &mut self,
    ) -> Vec<clonk_engine::PlayerInfoControlData> {
        self.reconcile_player_info_teams(false, true)
    }

    fn reconcile_player_info_teams(
        &mut self,
        recheck_memberships: bool,
        recheck_random_teams: bool,
    ) -> Vec<clonk_engine::PlayerInfoControlData> {
        let recheck_random_teams = recheck_random_teams
            && matches!(self.runtime_network_role(), RuntimeNetworkRole::Host)
            && self.engine.is_control_host();
        let memberships = ordered_control_player_team_memberships(&self.control_player_infos);
        let exact_metadata = self.network_team_assignment.as_mut().map(|assignment| {
            if recheck_memberships {
                self.control_player_infos
                    .recheck_team_players(assignment.teams_mut());
            }
            let updates = if recheck_random_teams {
                assignment.recheck_random_teams(&mut self.control_player_infos)
            } else {
                Vec::new()
            };
            (assignment.teams().clone(), updates)
        });

        if let Some((metadata, updates)) = exact_metadata {
            let runtime_teams = runtime_teams_from_initial_metadata(&metadata);
            let snapshot = clonk_network::join_team_list_snapshot(metadata);
            self.engine.set_teams(runtime_teams.clone());
            if let Some(prepared) = self
                .loading_state
                .as_mut()
                .and_then(|loading| loading.prepared_go.as_mut())
            {
                prepared.team_registry = runtime_teams;
            }
            if let Some(join_data) = self.pending_network_join_data.as_mut() {
                join_data.parameters.teams = snapshot.clone();
            }
            if let Some(host_snapshot) = self.host_join_snapshot.as_mut() {
                host_snapshot.parameters.teams = snapshot;
            }
            return updates;
        }

        if !recheck_memberships {
            return Vec::new();
        }

        if let Some(prepared) = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.prepared_go.as_mut())
        {
            recheck_runtime_team_memberships_from_infos(&mut prepared.team_registry, &memberships);
        }
        if let Some(join_data) = self.pending_network_join_data.as_mut() {
            recheck_join_team_memberships_from_infos(
                &mut join_data.parameters.teams.teams,
                &memberships,
            );
        }
        if matches!(self.mode, AppMode::Running) && !self.engine.teams().is_empty() {
            let mut runtime_teams = self.engine.teams().to_vec();
            recheck_runtime_team_memberships_from_infos(&mut runtime_teams, &memberships);
            self.engine.set_teams(runtime_teams);
        }
        Vec::new()
    }

    pub(crate) fn refresh_current_player_info_teams(&mut self) -> bool {
        let updates = self
            .engine
            .players()
            .filter_map(|player| {
                let info_id = player.player_info_id();
                (info_id != 0).then(|| {
                    let color = player.color().map(|color| {
                        (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b)
                    });
                    (info_id, player.team().unwrap_or(0), color)
                })
            })
            .collect::<Vec<_>>();
        if let Some(maximum_id) = updates.iter().map(|(info_id, _, _)| *info_id).max() {
            self.control_player_infos
                .reserve_player_ids_through(maximum_id);
        }
        let mut changed = false;
        for (info_id, team, color) in updates {
            changed |= self
                .control_player_infos
                .set_team_and_color(info_id, team, color);
        }
        changed
    }

    pub(crate) fn persist_fair_crew_preference(&mut self, enabled: bool) {
        let Some(paths) = self.app_paths.as_ref() else {
            return;
        };
        let section = "General";
        let key = "NoCrew";
        if let Err(error) = persist_native_config_values(
            paths,
            section,
            &[(
                key,
                clonk_app_netplay::NativeConfigValue::RawAscii(if enabled {
                    "true"
                } else {
                    "false"
                }),
            )],
        ) {
            tracing::error!(%error, section, key, "failed to persist game option");
            self.status_text = format!("Unable to save game option: {error}");
        }
    }

    pub(crate) fn player_selection_tooltip_target_at(
        &self,
        point: GuiPoint,
    ) -> Option<StartupTooltip> {
        let dialog = self.startup_player_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let layout = dialog.layout();
        let title = match dialog.mode() {
            clonk_frontend::startup_plrsel::PlrSelMode::Player => {
                self.startup_tooltip_resource_no_amp("IDS_DLG_PLAYERSELECTION")
            }
            clonk_frontend::startup_plrsel::PlrSelMode::Crew { player_name, .. } => format!(
                "{} {}",
                self.startup_tooltip_resource_no_amp("IDS_CTL_CREW"),
                player_name
            ),
        };
        let (display_title, _) = clonk_frontend::expand_hotkey_markup(&title);
        if let Some(tooltip) = clonk_frontend::centered_label_tooltip_at(
            point,
            layout.title_anchor,
            fonts.title.measure(&display_title, true),
            StartupTooltip::text(title),
        ) {
            return Some(tooltip);
        }
        if dialog.is_crew_mode() {
            dialog.tooltip_at(
                point,
                self.startup_crew_models
                    .iter()
                    .map(|crew| crew.name.as_str()),
            )
        } else {
            dialog.tooltip_at(
                point,
                self.startup_player_models
                    .iter()
                    .map(|player| player.name.as_str()),
            )
        }
    }

    /// Local `C4ScenarioListLoader::Scenario::CanOpen` player-count gate.
    /// Replays bypass the regular-game checks. Savegames lift a stale zero
    /// maximum to their effective minimum before the upper-bound comparison.
    pub(crate) fn local_scenario_player_count_error(
        &self,
        scenario: &FrontendScenario,
    ) -> std::result::Result<Option<String>, ClassicParityBoundary> {
        let Some(head) = self.scenario_loader_head_for_start(scenario)? else {
            return Ok(None);
        };
        self.local_scenario_player_count_error_from_head(&head)
    }

    pub(crate) fn local_scenario_player_count_error_from_head(
        &self,
        head: &ScenarioLoaderHead,
    ) -> std::result::Result<Option<String>, ClassicParityBoundary> {
        if head.is_replay() {
            return Ok(None);
        }
        let Some(paths) = self.app_paths.as_ref() else {
            return Ok(None);
        };
        let inspect_error = |error: &dyn fmt::Display| {
            report_classic_parity_boundary(ClassicParityBoundary::ScenarioStartInspection {
                path: PathBuf::from("Config.General.Participants"),
                detail: error.to_string(),
            })
        };
        let player_count =
            startup_participant_module_count(paths).map_err(|error| inspect_error(&error))?;
        if player_count < head.min_players() {
            return Ok(Some(format!(
                "This scenario is designed for a minimum of {} players. Please go to the Player Selection dialog and activate the participants for this round.",
                head.min_players()
            )));
        }
        let max_players = if head.is_save_game() {
            head.max_players().max(head.min_players())
        } else {
            head.max_players()
        };
        Ok((player_count > max_players).then(|| {
            format!(
                "This scenario is designed for a maximum of {} players.",
                head.max_players()
            )
        }))
    }
}
