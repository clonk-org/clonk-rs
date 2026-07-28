//! `impl GameApp` — scenario & sections methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn launch_classic_command_line_scenario(&mut self) -> Result<()> {
        if self.classic_command_line.direct_join.is_some() {
            return Ok(());
        }
        let scenario = self
            .classic_command_line
            .scenario
            .clone()
            .filter(|path| !path.as_os_str().is_empty());
        let record_stream = self
            .classic_command_line
            .record_stream
            .clone()
            .filter(|path| !path.as_os_str().is_empty());
        if scenario.is_none() && record_stream.is_none() {
            return Ok(());
        }
        if record_stream.is_some() {
            self.classic_record_stream_activation_pending = true;
        }
        if self.boot_loading.is_some() {
            self.auto_start_classic_command_line_scenario = true;
            return Ok(());
        }
        let path = if let Some(stream_path) = record_stream {
            let converted = classic_record_stream::convert_classic_record_stream(
                &stream_path,
                self.process_group_maker.as_bytes(),
            )
            .with_context(|| {
                format!(
                    "Could not process record stream data {}",
                    stream_path.display()
                )
            })?;
            self.classic_command_line.scenario = Some(converted.clone());
            converted
        } else {
            scenario.expect("a non-stream command-line scenario was checked above")
        };
        let scenario = FrontendScenario::from_command_line(&path);
        let definition_load = self.classic_command_line_definition_load();
        let replay_disables_network = if self.classic_command_line.network_active == Some(true) {
            self.scenario_loader_head_for_start(&scenario)
                .map_err(|error| anyhow!(error.to_string()))?
                .is_some_and(|head| head.is_replay())
        } else {
            false
        };
        if replay_disables_network {
            tracing::error!(
                "{}",
                self.runtime_resource_text(
                    "IDS_PRC_NONETREPLAY",
                    "Cannot play back records while in network mode."
                )
            );
        }
        if self.classic_command_line.network_active == Some(true) && !replay_disables_network {
            self.stage_network_host_scenario(scenario, definition_load);
            Ok(())
        } else {
            self.start_scenario_with_definition_load(scenario, definition_load)?;
            Ok(())
        }
    }

    pub(crate) fn abort_scenario_rename(&mut self) -> bool {
        let had_rename = self.menu_state.rename_edit.is_some();
        if let Some(previous_focus) = self.menu_state.abort_renaming() {
            self.restore_scensel_focus(previous_focus);
        }
        had_rename
    }

    pub(crate) fn submit_scenario_search(&mut self) -> Result<(), EngineError> {
        // Both search paths destroy and recreate the visible row elements.
        // The player-facing path uses the enhanced in-memory index; the
        // isolated `MenuState::submit_search` method retains C++ behavior for
        // parity coverage.
        self.startup_tooltip.pointer_left();
        self.menu_frame_cache = None;
        // Rebuilding the list necessarily recreates and reselects a row.
        // That programmatic selection must stay silent while the user types;
        // dependent controls are synchronized explicitly below.
        let _ = self.menu_state.apply_enhanced_search();
        // An empty result emits no SelectionChanged action. Explicitly clear
        // selection-derived checkbox/ForcedNoCrew state in that case rather
        // than retaining the previously selected scenario's constraint.
        self.menu_state.sync_definition_checkbox_to_selection();
        self.sync_scenario_game_option_constraint();
        Ok(())
    }

    pub(crate) fn execute_scenario_search_context_command(
        &mut self,
        command: ScenselSearchContextCommand,
    ) -> Result<(), EngineError> {
        if self.mode != AppMode::Menu || self.startup_view != StartupView::ScenarioBrowser {
            tracing::error!(?command, "stale scenario search context command");
            return Ok(());
        }
        match command {
            ScenselSearchContextCommand::Cut => {
                if self.copy_search_edit_selection(true) {
                    self.submit_scenario_search()?;
                }
            }
            ScenselSearchContextCommand::Copy => {
                let _ = self.copy_search_edit_selection(false);
            }
            ScenselSearchContextCommand::Paste => self.paste_search_edit_clipboard()?,
            ScenselSearchContextCommand::Clear => {
                if self.menu_state.search_edit.delete_selection() {
                    self.submit_scenario_search()?;
                }
            }
            ScenselSearchContextCommand::SelectAll => {
                self.menu_state.search_edit.select_all();
            }
        }
        self.mark_menu_dirty();
        Ok(())
    }

    fn retain_renamed_scenario_title(
        &mut self,
        identifier: &str,
        title: &str,
    ) -> Result<(), EngineError> {
        if let Some(state) = self.scenario_selector_discovery.as_mut() {
            state.retained_title = Some((identifier.to_string(), title.to_string()));
            return Ok(());
        }
        let mut entries = self
            .menu_state
            .stack
            .first()
            .map(|layer| layer.entries.clone())
            .unwrap_or_default();
        let alphabetical_sorting = load_startup_alphabetical_sorting(self.app_paths.as_ref());
        if !override_frontend_scenario_title(&mut entries, identifier, title, alphabetical_sorting)
        {
            return Ok(());
        }
        self.scenario_catalog = build_scenario_catalog(&entries);
        self.refresh_scenario_entry_enabled();
        let identifier = identifier.to_string();
        self.handle_menu_input(move |menu| {
            menu.replace_discovered_entries(entries, Some(&identifier), true, false)
        })?;
        self.configure_current_folder_map();
        self.menu_state.sync_definition_checkbox_to_selection();
        self.sync_scenario_game_option_constraint();
        Ok(())
    }

    pub(crate) fn apply_scenario_mission_access(&mut self, input: &str) -> Result<(), EngineError> {
        let (remove, modules) = input
            .strip_prefix('-')
            .map_or((false, input), |modules| (true, modules));
        if modules.is_empty() {
            return Ok(());
        }
        let selected = self
            .menu_state
            .selected_scenario()
            .map(|entry| entry.identifier.clone());
        let value = self.mission_access.update_modules(modules, remove);
        if let Some(paths) = self.app_paths.as_ref() {
            if let Err(error) = persist_config_value(paths, "General", "MissionAccess", value) {
                tracing::warn!(%error, "failed to persist General.MissionAccess");
                self.status_text = format!("Unable to save mission access: {error}");
            }
        }
        // C4StartupScenSelDlg::UpdateList begins with AbortRenaming. Empty or
        // cancelled input never reaches this accepted/rebuild path.
        self.abort_scenario_rename();
        self.reload_scenario_selector(selected.as_deref(), true, true)
    }

    pub(crate) fn delete_scenario_and_refresh(
        &mut self,
        path: &Path,
        next_identifier: Option<&str>,
    ) -> Result<(), EngineError> {
        if let Err(error) = delete_scenario_storage(path) {
            tracing::warn!(%error, path = %path.display(), "failed to delete scenario entry");
            self.push_message_dialog(
                clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                    "Delete failure.",
                    "Delete",
                    clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                ),
                MessageDialogContinuation::None,
            )?;
            return Ok(());
        }
        self.reload_scenario_selector(next_identifier, false, false)
    }

    pub(crate) fn commit_scenario_rename(&mut self, focus_lost: bool) -> Result<(), EngineError> {
        let Some((identifier, original_title, action)) =
            self.menu_state.rename_edit.as_mut().map(|rename| {
                (
                    rename.identifier.clone(),
                    rename.edit.label_text().to_string(),
                    if focus_lost {
                        rename.edit.focus_lost()
                    } else {
                        rename.edit.finish_input()
                    },
                )
            })
        else {
            return Ok(());
        };
        let RenameEditAction::Submit(title) = action else {
            self.abort_scenario_rename();
            self.mark_menu_dirty();
            return Ok(());
        };
        if title == original_title {
            self.menu_state.resolve_renaming(RenameEditResult::Deleted);
            self.set_scensel_dialog_focus(ScenselDialogFocus::List);
            self.mark_menu_dirty();
            return Ok(());
        }
        let scenario = self.scenario_catalog.get(&identifier).cloned().or_else(|| {
            self.menu_state
                .visible_entries()
                .iter()
                .find(|entry| entry.identifier == identifier)
                .cloned()
        });
        let result = (|| -> Result<String> {
            let scenario = scenario.ok_or_else(|| anyhow!("selected scenario is stale"))?;
            let path = scenario
                .path
                .as_deref()
                .ok_or_else(|| anyhow!("selected scenario has no storage path"))?;
            let distinct_sources = scenario
                .source_paths
                .iter()
                .map(|source| scenario_root_key(source))
                .collect::<HashSet<_>>();
            anyhow::ensure!(
                distinct_sources.len() <= 1,
                "merged scenario entries cannot be renamed safely"
            );
            let language = scenario_title_language(self.app_paths.as_ref());
            let destination = rename_scenario_storage(path, scenario.kind, &title, &language)?;
            let filename = destination
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| anyhow!("renamed scenario has no UTF-8 filename"))?;
            let parent = identifier.rsplit_once('/').map(|(parent, _)| parent);
            Ok(parent.map_or_else(
                || filename.to_string(),
                |parent| format!("{parent}/{filename}"),
            ))
        })();
        match result {
            Ok(identifier) => {
                self.menu_state.resolve_renaming(RenameEditResult::Deleted);
                let refresh_result = self
                    .reload_scenario_selector(Some(&identifier), true, true)
                    .and_then(|()| self.retain_renamed_scenario_title(&identifier, &title));
                self.set_scensel_dialog_focus(ScenselDialogFocus::List);
                refresh_result?;
            }
            Err(error) => {
                tracing::warn!(%error, "failed to rename scenario entry");
                self.menu_state.resolve_renaming(RenameEditResult::Invalid);
                self.push_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                        error.to_string(),
                        "Rename failure",
                        clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
                    ),
                    MessageDialogContinuation::None,
                )?;
            }
        }
        self.mark_menu_dirty();
        Ok(())
    }

    pub(crate) fn restart_current_scenario(&mut self) -> Result<(), EngineError> {
        // QuitGame snapshots the raw NetworkActive flag before Game.Clear and
        // restores it for any scheduled NextMission. Even a Film2 client
        // therefore re-enters the network-host/lobby path on Restart.
        if self.network.is_some() {
            self.restart_current_network_scenario();
            return Ok(());
        }
        let Some(scenario) = self.active_scenario.clone() else {
            self.return_to_menu();
            return Ok(());
        };
        let definition_load = self.active_definition_load.clone();
        self.retain_restart_restore_mask_for_restart();
        self.return_to_menu_for_relaunch();
        let start_result = match definition_load {
            Some(definition_load) => {
                self.start_scenario_with_definition_load(scenario, definition_load)
            }
            None => self.start_scenario(scenario),
        };
        if let Err(err) = start_result {
            tracing::error!(error = ?err, "failed to restart scenario");
            self.status_text = format!("Restart failed: {err:#}");
        }
        Ok(())
    }

    pub(crate) fn enter_scenario_folder(&mut self, identifier: &str) {
        self.menu_state.enter_folder(identifier);
        self.configure_current_folder_map();
        self.mark_menu_dirty();
    }

    pub(crate) fn close_scenario_browser(&mut self) {
        self.menu_state.abort_renaming();
        self.scensel_rename_pointer_focus = None;
        match self.startup_scenario_back_dialog.take() {
            Some(StartupDialog::MainMenu) => {
                // `SDID_Back` reuses the retained Main dialog, so the
                // remembered ID remains the selector until a later explicit
                // switch replaces it.
                let remembered = self.last_startup_dialog;
                self.begin_startup_dialog_fade(StartupDialog::MainMenu);
                self.show_main_menu();
                self.last_startup_dialog = remembered;
            }
            Some(StartupDialog::NetworkGame) => {
                self.begin_startup_dialog_fade(StartupDialog::NetworkGame);
                self.close_context_menu_silently();
                self.game_option_input_dialog = None;
                self.game_option_input_consumed_keys.clear();
                self.game_option_input_pointer_capture = None;
                self.game_option_consumed_keys.clear();
                self.scenario_game_options.cancel_interaction();
                self.refresh_retained_network_dialog_internet();
                self.replace_startup_view(StartupView::NetworkGame);
                if let Some(dialog) = self.startup_network_dialog.as_mut() {
                    dialog.pointer_left();
                }
                self.status_text.clear();
                self.mark_menu_dirty();
            }
            _ => {
                // `DoStartup` recreated the selector without `pLastDlg`.
                // Native normalizes Back in that state to an explicit Main.
                self.begin_startup_dialog_fade(StartupDialog::MainMenu);
                self.show_main_menu();
            }
        }
    }

    pub(crate) fn start_selected_map_scenario_from_ui(&mut self) -> Result<(), EngineError> {
        let action = self.menu_state.start_selected_map_scenario();
        self.handle_menu_input(move |_| action.into_iter().collect())
    }

    pub(crate) fn open_scenario_browser(&mut self) {
        self.open_scenario_browser_with_mode(ScenarioSelectorMode::Local);
    }

    pub(crate) fn open_scenario_browser_with_mode(&mut self, selector_mode: ScenarioSelectorMode) {
        self.cancel_scenario_selector_discovery();
        self.menu_state.abort_renaming();
        self.close_context_menu_silently();
        self.startup_player_properties_dialog = None;
        self.game_option_input_dialog = None;
        self.league_signup_dialog = None;
        self.cancelled_league_signup_continuation = None;
        self.league_signup_consumed_keys.clear();
        self.league_signup_pointer_capture = false;
        self.league_signup_pointer_position = None;
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_pointer_capture = false;
        self.game_option_consumed_keys.clear();
        self.scenario_selector_mode = selector_mode;
        self.startup_scenario_back_dialog = Some(match selector_mode {
            ScenarioSelectorMode::Local => StartupDialog::MainMenu,
            ScenarioSelectorMode::NetworkHost => StartupDialog::NetworkGame,
        });
        let values = load_scenario_game_option_values(self.app_paths.as_ref());
        self.startup_view_flags.fair_crew = values.fair_crew;
        self.startup_view_flags.record = values.record;
        self.recording_enabled = values.record && self.recordings_dir.is_some();
        self.scenario_game_options =
            GameOptionButtons::new(selector_mode.game_option_context(), values);
        self.apply_classic_game_option_overrides();
        self.replace_startup_dialog(
            StartupView::ScenarioBrowser,
            StartupDialog::ScenarioBrowser(selector_mode),
        );
        self.menu_state.set_pointer_position(None);
        self.menu_state.scensel_title_present = true;
        self.menu_state.scensel_title_topmost = false;
        // The C++ dialog reloads from the root folder every time it is
        // shown (OnShown -> pScenLoader->Load(ExePath), cpp:1431-1443).
        self.menu_state.stack.truncate(1);
        self.menu_state.clear_search();
        self.menu_state.scenario_list_scroll = 0;
        self.menu_state.selection_info_scroll = 0;
        self.menu_state.scrollbar_interaction = None;
        self.menu_state.set_dialog_focus(ScenselDialogFocus::List);
        // The C++ book has no Back list entry — Back is a button/K_LEFT
        // (C4StartupScenSelDlg.cpp:1367-1369,1388-1389).
        self.menu_state.set_include_back(false);
        self.refresh_scenario_entry_enabled();
        self.menu_state.refresh_menu_entries();
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        self.menu_state.menu().resize(width, height);
        if let Err(err) = self.handle_menu_input(|menu| menu.select_default_entry()) {
            tracing::error!(error = %err, "failed to select default scenario entry");
        }
        self.sync_scenario_game_option_bounds();
        self.sync_scenario_game_option_constraint();
        self.scenario_label = self.menu_state.label_path();
        self.status_text.clear();
    }

    pub(crate) fn finish_scenario_loading_failure(
        &mut self,
        message: String,
        prepared_go: bool,
    ) -> Result<(), EngineError> {
        let returns_to_startup = self.failed_open_game_returns_to_startup();
        if prepared_go && returns_to_startup {
            // A failed InitGame after DoLobby returned still unwinds through
            // C4Application::QuitGame. Clear the partially initialized
            // network round before reconstructing the remembered host/join
            // startup dialog (src/C4Application.cpp:373-400,442-451;
            // src/C4Game.cpp:452-477).
            let purpose = if matches!(self.runtime_network_role(), RuntimeNetworkRole::Host)
                || self.scenario_selector_mode == ScenarioSelectorMode::NetworkHost
            {
                StartupNetworkPurpose::StagedHost
            } else {
                StartupNetworkPurpose::Join
            };
            return self.finish_startup_network_failure(purpose, message);
        }
        if prepared_go && self.network.is_some() {
            // Explicit/direct command-line starts have no startup generation,
            // but QuitGame still tears down the failed network before the
            // process exits.
            let local_client_id = self
                .network
                .as_ref()
                .and_then(|network| i32::try_from(network.local_client_id()).ok())
                .unwrap_or_else(|| self.offline_local_client_id());
            self.change_network_control_to_local(local_client_id);
        }
        if !prepared_go && self.network.is_none() && returns_to_startup {
            // C4Application::OpenGame marks a failed ordinary fullscreen
            // local start, clears the partial game, enters PreInit, restores
            // the remembered startup dialog and only then presents its log.
            self.startup_restart_diagnostics.mark_quit_with_error();
            self.startup_restart_diagnostics.add_fatal_error(message);
            // PreInit re-initializes the loader screen after Game.Clear has
            // released the partial game, so it takes the restored startup
            // GraphicsResource fonts rather than the failed scenario's.
            self.return_to_menu();
            self.reinitialize_startup_loader_screen();
            return self.present_startup_restart_diagnostics();
        }

        // Explicit command-line/developer-console starts do not enter another
        // startup generation.
        self.restore_startup_gui_sheets();
        self.active_global_gui_failures.clear();
        self.status_text = message;
        self.loading_state = None;
        self.network_start_wait = None;
        self.mode = AppMode::Menu;
        self.restore_startup_fonts();
        if prepared_go || self.failed_record_stream_exits() {
            self.request_exit();
        } else if returns_to_startup {
            if let Some(audio) = self.audio.as_mut() {
                audio.configure_scenario(None);
            }
            self.reconstruct_music_system_at_preinit();
            self.begin_frontend_music_entry();
        }
        Ok(())
    }

    pub(crate) fn scenario_browser_tooltip_target_at(
        &self,
        point: GuiPoint,
    ) -> Option<StartupTooltip> {
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let book = self.assets.book_fonts.as_deref()?;
        let surface = self.graphics.surface();
        let (width, height) = (surface.width(), surface.height());
        let layout =
            clonk_frontend::startup_scensel::scen_sel_layout(width as i32, height as i32, fonts);
        let title_key = if self.scenario_selector_mode == ScenarioSelectorMode::NetworkHost {
            "IDS_DLG_NETSTART"
        } else {
            "IDS_DLG_STARTGAME"
        };
        let title = self.startup_tooltip_resource_no_amp(title_key);
        if self.menu_state.scensel_title_present && self.menu_state.scensel_title_topmost {
            if let Some(tooltip) = clonk_frontend::centered_label_tooltip_at(
                point,
                layout.title_anchor,
                fonts.title.measure(&title, true),
                StartupTooltip::text(title.clone()),
            ) {
                return Some(tooltip);
            }
        }
        if let Some(key) = self.scenario_game_options.tooltip_resource_key_at(point) {
            return Some(StartupTooltip::resource(key));
        }
        let map = self.menu_state.current_map();
        let sheet_tooltip = if let Some(map) = map {
            let transform = MapFolderTransform::for_map(map, &layout, width, height);
            let mut picture_bounds = Vec::new();
            if !map.fullscreen_background {
                picture_bounds.push(transform.background);
            }
            picture_bounds.extend(
                map.access_overlays
                    .iter()
                    .map(|overlay| transform.rect(overlay.area)),
            );
            let buttons = map.scenarios.iter().rev().map(|button| {
                clonk_frontend::startup_scensel::ScenSelMapScenarioTooltip {
                    bounds: transform.rect(button.area),
                    scenario_name: button.entry.as_ref().map(|entry| entry.title.as_str()),
                }
            });
            clonk_frontend::startup_scensel::scen_sel_map_tooltip_at(
                &layout,
                point,
                [transform.rect(map.scenario_info_area)],
                picture_bounds,
                buttons,
            )
        } else {
            let caption = self
                .menu_state
                .current_folder()
                .map(|folder| folder.title.clone())
                .unwrap_or_else(|| self.startup_tooltip_resource_string("IDS_DLG_SCENARIOS"));
            let (display_caption, _) = clonk_frontend::expand_hotkey_markup(&caption);
            clonk_frontend::startup_scensel::scen_sel_book_tooltip_at(
                &layout,
                point,
                book.title.measure(&display_caption, true),
                self.menu_state.scenario_list_scroll(),
                clonk_frontend::startup_scensel::scen_list_item_height(&book.text),
                self.menu_state
                    .visible_entries()
                    .iter()
                    .map(|entry| entry.title.as_str()),
            )
        };
        if sheet_tooltip.is_some() {
            return sheet_tooltip;
        }

        // FullscreenDialog creates the title before ScenarioBrowser adds its
        // Tabular. The active sheet therefore occludes the lower part of the
        // title label even where the sheet itself has no tooltip.
        let in_active_sheet = point.x >= layout.map_sheet.x as f32
            && point.x < (layout.map_sheet.x + layout.map_sheet.w) as f32
            && point.y >= layout.map_sheet.y as f32
            && point.y < (layout.map_sheet.y + layout.map_sheet.h) as f32;
        if in_active_sheet || !self.menu_state.scensel_title_present {
            return None;
        }
        clonk_frontend::centered_label_tooltip_at(
            point,
            layout.title_anchor,
            fonts.title.measure(&title, true),
            StartupTooltip::text(title),
        )
    }

    pub(crate) fn start_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        let definition_load = self.scenario_seed_definition_load();
        self.start_scenario_with_definition_load(scenario, definition_load)
    }

    /// Rebuilds the label-color cache when the selector is opened or local
    /// MissionAccess/player configuration changes. A failed inspection is a
    /// fail-closed disabled row, while activation still reports the boundary.
    pub(crate) fn refresh_scenario_entry_enabled(&mut self) {
        let selector_mode = self.scenario_selector_mode;
        self.scenario_entry_enabled = self
            .scenario_catalog
            .iter()
            .map(|(identifier, scenario)| {
                let enabled = match self.scenario_selector_open_error(scenario, selector_mode) {
                    Ok(error) => error.is_none(),
                    Err(error) => {
                        tracing::error!(scenario = %identifier, %error, "failed to inspect scenario CanOpen state");
                        false
                    }
                };
                (identifier.clone(), enabled)
            })
            .collect();
    }

    pub(crate) fn scenario_seed_definition_load(&self) -> ScenarioDefinitionLoad {
        let mut modules = self.initial_definition_seed.clone().unwrap_or_default();
        // The unchecked scenario-selector branch appends Objects.c4d to the
        // vector ParseCommandLine seeded once for the first game init.
        modules.push("Objects.c4d".to_string());
        let definition_root = self
            .app_paths
            .as_ref()
            .and_then(|paths| match startup_definition_paths(paths) {
                Ok(paths) => paths.active_custom_root,
                Err(error) => {
                    tracing::error!(
                        %error,
                        "failed to read General.DefinitionPath; starting without custom definition root"
                    );
                    None
                }
            });
        ScenarioDefinitionLoad::Seed {
            modules,
            definition_root,
        }
    }

    pub(crate) fn take_scenario_seed_definition_load(&mut self) -> ScenarioDefinitionLoad {
        let definition_load = self.scenario_seed_definition_load();
        self.initial_definition_seed = None;
        definition_load
    }

    pub(crate) fn start_scenario_with_definition_modules(
        &mut self,
        scenario: FrontendScenario,
        modules: Vec<String>,
        definition_root: Option<PathBuf>,
    ) -> Result<(), EngineError> {
        // This convenience route accepts a physical directory. The loader's
        // retained field is the literal C++ DefinitionPath prefix, whose
        // separator must remain present before module concatenation.
        let definition_root = definition_root
            .as_deref()
            .map(path_with_trailing_native_separator);
        self.start_scenario_with_definition_load(
            scenario,
            ScenarioDefinitionLoad::Fixed {
                modules,
                definition_root,
            },
        )
    }

    pub(crate) fn start_scenario_with_definition_load(
        &mut self,
        scenario: FrontendScenario,
        definition_load: ScenarioDefinitionLoad,
    ) -> Result<(), EngineError> {
        self.initial_definition_seed = None;
        self.startup_restart_diagnostics.begin_game_init();
        self.close_context_menu_silently();
        self.definition_selector = None;
        self.pending_definition_selection = None;
        self.pending_lobby_player_selection = None;
        self.definition_selector_last_click = None;
        self.game_over_dialog = None;
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::GameOver);
        if scenario.path.is_none() {
            return self.start_sandbox_scenario(scenario);
        }
        self.begin_loading_scenario(scenario, definition_load)
    }

    pub(crate) fn begin_loading_scenario(
        &mut self,
        mut scenario: FrontendScenario,
        definition_load: ScenarioDefinitionLoad,
    ) -> Result<(), EngineError> {
        let path = scenario
            .path
            .clone()
            .expect("scenario path must be present when starting load");
        tracing::info!(
            scenario = %scenario.title,
            path = %path.display(),
            "starting asynchronous scenario load"
        );

        let loader_setup = self
            .app_paths
            .as_ref()
            .ok_or_else(|| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::LoaderScreen {
                        context: "scenario initialization",
                        detail: "application paths are unavailable".to_string(),
                    },
                ))
            })
            .and_then(|paths| {
                build_scenario_loader(&scenario, &definition_load, paths, self.assets.as_ref())
                    .map_err(|error| {
                        classic_parity_engine_error(report_classic_parity_boundary(
                            ClassicParityBoundary::LoaderScreen {
                                context: "scenario initialization",
                                detail: error.to_string(),
                            },
                        ))
                    })
            })?;
        // `C4Game::OpenScenario` resolves the running Parameters title through
        // the same pack-aware Title component used by the selected loader.
        // Retain that exact result instead of re-reading a local-only
        // `Title.txt` after the worker finishes.
        retain_selected_scenario_title(&mut scenario, loader_setup.scenario_title.as_deref());
        let initial_fonts = loader_setup.screen.resources().fonts().clone();
        let initial_tooltip_font = loader_setup
            .initial_tooltip_font
            .clone()
            .expect("scenario loader resolves a pre-definition tooltip font");
        self.install_active_classic_fonts(
            initial_fonts,
            Some(initial_tooltip_font),
            loader_setup.initial_native_font_source.clone(),
        );
        self.loader_screen = Some(loader_setup.screen);
        self.loader_error = None;

        let resolver_paths = cached_app_paths().ok();
        let languages = startup_language_sequence(resolver_paths.as_deref());
        let catalog_preload_key = CatalogHostLobbyPreloadKey {
            identifier: scenario.identifier.clone(),
            scenario_path: path.clone(),
            definition_load: definition_load.clone(),
            languages: languages.clone(),
        };
        let scenario_title = scenario.title.clone();
        let (sender, receiver) = mpsc::channel();
        let path_for_thread = path.clone();
        // C4Game freezes the raw configured module list before OpenScenario,
        // then admits the successfully loaded player cores against the
        // scenario/Parameters capacity before landscape creation (pristine
        // 9ffa0a5d src/C4Game.cpp:361-364,231-248,2394-2431;
        // src/C4PlayerInfo.cpp:357-395,1273-1290).
        let replay_startup = if self.network.is_none() {
            open_group_path_for_folder_map(&path)
                .map_err(|error| error.to_string())
                .and_then(|group| {
                    Scenario::preflight_replay_startup_from_group(&group)
                        .map_err(|error| error.to_string())
                })
        } else {
            Ok(None)
        };
        let offline_startup = if self.network.is_none() {
            self.app_paths.as_ref().map_or(Ok(None), |paths| {
                let selection =
                    snapshot_effective_client_player_selection(paths, &self.classic_command_line)
                        .map_err(|error| error.to_string())?;
                match preflight_offline_startup(&path) {
                    Ok(preflight) => {
                        let configured = load_snapshotted_client_players(paths, &selection);
                        if preflight.save_game {
                            let language_packs = classic_language_packs(paths);
                            let (startup, savegame) = prepare_offline_savegame_startup(
                                &path,
                                configured,
                                preflight.max_players,
                                &languages,
                                &language_packs,
                            )?;
                            Ok(Some((startup, preflight.random_seed, Some(savegame))))
                        } else {
                            Ok(Some((
                                OfflineStartupPlayers::new(configured, preflight.max_players),
                                preflight.random_seed,
                                None,
                            )))
                        }
                    }
                    // Scenario.json is a Rust-only fixture format. Keep its
                    // existing synthetic single-player path isolated from the
                    // legacy C++ startup pipeline.
                    Err(ScenarioError::OfflineStartupJsonUnsupported) => Ok(None),
                    // Replay player state is supplied by PlayerInfos/CtrlRec,
                    // never by the process-local participant selection.
                    Err(ScenarioError::OfflineStartupReplayUnsupported) => Ok(None),
                    Err(error) => Err(error.to_string()),
                }
            })
        } else {
            Ok(None)
        };
        let startup_player_count = offline_startup
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|(startup, _, _)| startup.startup_player_count());
        let offline_parameter_seed = offline_startup
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .and_then(|(_, seed, _)| *seed);
        let offline_startup_error = offline_startup.as_ref().err().cloned();
        let replay_startup_error = replay_startup.as_ref().err().cloned();
        let (offline_startup_players, offline_savegame) = offline_startup
            .ok()
            .flatten()
            .map(|(startup, _, savegame)| (Some(startup), savegame))
            .unwrap_or((None, None));
        let replay_startup = replay_startup.ok().flatten();
        // C4GameParameters::Load freezes this before InitGameSecondPart calls
        // FixRandom and Landscape.Init. Parameters.txt wins when present;
        // only a fresh missing-Parameters round consults time/LC_PIN_SEED.
        let offline_random_seed = (self.network.is_none()
            && replay_startup_error.is_none()
            && offline_startup_error.is_none()
            && replay_startup.is_none())
        .then(|| current_offline_round_random_seed(offline_parameter_seed));
        let fresh_authority_may_retry = offline_random_seed.is_some() && offline_savegame.is_none();
        let preloaded_scenario = self
            .lobby_preload_artifact
            .as_mut()
            .and_then(|artifact| artifact.catalog_host.as_mut())
            .and_then(|catalog_host| catalog_host.take_matching_scenario(&catalog_preload_key));
        let retry_generated_landscape = fresh_authority_may_retry
            && preloaded_scenario
                .as_ref()
                .is_none_or(Scenario::generated_landscape_seed_retry_applies);
        let preloaded_scenario = (!retry_generated_landscape)
            .then_some(preloaded_scenario)
            .flatten();

        thread::spawn(move || {
            let mut reporter = ScenarioLoadingReporter::new(sender);
            let resolver = InstallDefinitionResolver::new(resolver_paths);
            let scenario_data = if let Some(preloaded_scenario) = preloaded_scenario {
                reporter.report(93, "Scenario preload ready");
                Ok((preloaded_scenario, None))
            } else {
                match offline_startup_error.or(replay_startup_error) {
                    Some(error) => Err(error),
                    None => match (replay_startup, startup_player_count) {
                        (Some(replay), _) => {
                            load_scenario_with_definition_load_and_seed_and_startup_player_count_and_progress(
                                &path_for_thread,
                                &resolver,
                                &languages,
                                &definition_load,
                                u64::from(replay.random_seed as u32),
                                replay.startup_player_count,
                                |progress, line| reporter.report(progress, line),
                            )
                            .map(|scenario| (scenario, None))
                            .map_err(|error| error.to_string())
                        }
                        (None, Some(startup_player_count)) if retry_generated_landscape => {
                            load_fresh_scenario_with_valid_generated_landscape(
                                &path_for_thread,
                                &resolver,
                                &languages,
                                &definition_load,
                                offline_random_seed
                                    .expect("fresh offline loading freezes a random seed"),
                                startup_player_count,
                                |progress, line| reporter.report(progress, line),
                            )
                            .map(|(scenario, random_seed)| (scenario, Some(random_seed)))
                        }
                        (None, Some(startup_player_count)) => {
                            load_scenario_with_definition_load_and_seed_and_startup_player_count_and_progress(
                                &path_for_thread,
                                &resolver,
                                &languages,
                                &definition_load,
                                offline_random_seed.unwrap_or(0),
                                startup_player_count,
                                |progress, line| reporter.report(progress, line),
                            )
                            .map(|scenario| (scenario, None))
                            .map_err(|error| error.to_string())
                        }
                        (None, None) => load_scenario_with_definition_load_and_progress(
                            &path_for_thread,
                            &resolver,
                            &languages,
                            &definition_load,
                            |progress, line| reporter.report(progress, line),
                        )
                        .map(|scenario| (scenario, None))
                        .map_err(|error| error.to_string()),
                    },
                }
            };

            match scenario_data {
                Ok((data, accepted_random_seed)) => {
                    if let Some(random_seed) = accepted_random_seed {
                        reporter.send(ScenarioLoadingEvent::AcceptedRandomSeed(random_seed));
                    }
                    reporter.send(ScenarioLoadingEvent::RefreshResources);
                    reporter.send(ScenarioLoadingEvent::Finished(Ok(data)));
                }
                Err(err) => {
                    let message = format!("Failed to load {}: {}", scenario_title, err);
                    reporter.send(ScenarioLoadingEvent::Finished(Err(message)));
                }
            }
        });

        self.fade_out_game_music();
        self.status_text.clear();
        let mut loading_state = ScenarioLoadingState::new(
            scenario,
            loader_setup.refreshed_resources,
            loader_setup.refreshed_global_gui_failures,
            loader_setup.refreshed_gui_sheet_overrides,
            receiver,
        );
        loading_state.refreshed_tooltip_font = loader_setup.refreshed_tooltip_font;
        loading_state.refreshed_native_font_source = loader_setup.refreshed_native_font_source;
        loading_state.offline_startup_players = offline_startup_players;
        loading_state.offline_savegame = offline_savegame;
        loading_state.offline_random_seed = offline_random_seed;
        self.loading_state = Some(loading_state);
        self.mode = AppMode::Loading;
        Ok(())
    }

    pub(crate) fn activate_loaded_scenario(
        &mut self,
        scenario: FrontendScenario,
        scenario_data: &Scenario,
    ) -> std::result::Result<(), ScenarioActivationError> {
        self.finish_recording();
        self.live_save_seed = None;
        self.recording_template = None;
        self.control_playback = None;
        self.local_player_profile_paths.clear();
        self.deferred_network_savegame_recreation.clear();
        let prepared_go = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .is_some();
        let retained_definition_modules = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .and_then(|prepared| prepared.definition_modules.clone());
        let retained_definition_save_paths =
            self.network_mode.as_ref().and_then(|mode| match mode {
                NetworkMode::Host(HostSettings {
                    prepared: Some(prepared),
                    ..
                }) => {
                    let (executable, definitions) = prepared.definition_save_paths();
                    Some((executable.to_owned(), definitions.to_owned()))
                }
                NetworkMode::Host(_) | NetworkMode::Client(_) => None,
            });
        let path = scenario
            .path
            .clone()
            .ok_or_else(|| format!("Scenario `{}` is missing a filesystem path", scenario.title))?;
        self.finalize_client_network_scenario_loading(scenario_data, &path)
            .map_err(ScenarioActivationError::Recoverable)?;
        let effective_definition_paths = scenario_data
            .definition_resource_paths()
            .iter()
            .map(|path| path_as_legacy_text(path))
            .collect::<Vec<_>>();
        let effective_description_definition_modules = recording_description_definition_modules(
            scenario_data,
            retained_definition_modules.as_deref(),
        );
        let effective_definition_load = ScenarioDefinitionLoad::Fixed {
            modules: effective_definition_paths.clone(),
            // The vector already contains the rooted and original blocks
            // selected during this load. C++ backs up that effective vector.
            definition_root: None,
        };
        let matching_preload = self.lobby_preload_artifact.take().filter(|artifact| {
            artifact.scenario_path == path
                && artifact.definition_paths == effective_definition_paths
        });
        let (active_game_graphics, preloaded_materials) = match matching_preload {
            Some(artifact) => (
                artifact.game_graphics,
                Some((
                    artifact.material_texture_images,
                    artifact.material_render_info,
                )),
            ),
            None => (
                self.loaded_game_graphics_resources(&scenario, Some(&effective_definition_load))
                    .map_err(|error| {
                        ScenarioActivationError::Recoverable(format!(
                            "Failed to load {} graphics: {error:#}",
                            scenario.title
                        ))
                    })?,
                None,
            ),
        };

        tracing::info!(
            scenario = %scenario.title,
            path = %path.display(),
            "applying loaded scenario"
        );

        let mut prepared_random_seed = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.random_seed);
        let offline_random_seed = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.offline_random_seed);
        let prepared_team_configuration = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.team_configuration);
        let prepared_team_registry = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.team_registry.clone());
        let prepared_initial_game_data = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .and_then(|prepared| prepared.initial_game_data.clone());
        let prepared_fair_crew = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| {
                (
                    prepared.use_fair_crew,
                    prepared.fair_crew_strength,
                    prepared.fair_crew_forced,
                    prepared.allow_debug,
                )
            });
        let synchronized_auto_frame_skip = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| prepared.auto_frame_skip);
        let embedded_auto_frame_skip = scenario_data
            .lobby_metadata()
            .and_then(ScenarioLobbyMetadata::embedded_game_parameter_values)
            .map(|parameters| parameters.auto_frame_skip());
        let auto_frame_skip = frozen_auto_frame_skip(
            configured_auto_frame_skip(&load_native_config_bytes(self.app_paths.as_ref())),
            embedded_auto_frame_skip,
            synchronized_auto_frame_skip,
        );
        let offline_startup_players = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.offline_startup_players.take());
        let offline_savegame = self
            .loading_state
            .as_mut()
            .and_then(|loading| loading.offline_savegame.take());
        let initial_game_data = prepared_initial_game_data.as_ref().or_else(|| {
            offline_savegame
                .as_ref()
                .map(|save| &save.initial_game_data)
        });
        let network_game = self.network.is_some();
        let replay = scenario_data
            .lobby_metadata()
            .is_some_and(|metadata| metadata.head().is_replay());
        let replay_parameters = replay
            .then(|| {
                scenario_data
                    .lobby_metadata()
                    .and_then(ScenarioLobbyMetadata::embedded_game_parameter_values)
            })
            .flatten();
        let serialized_startup_player_count = scenario_data.lobby_metadata().map(|metadata| {
            metadata.embedded_game_parameter_values().map_or_else(
                || metadata.game_parameter_defaults().startup_player_count(),
                |parameters| parameters.startup_player_count(),
            )
        });
        if prepared_random_seed.is_none() {
            prepared_random_seed = replay_parameters
                .as_ref()
                .map(|parameters| u64::from(parameters.random_seed() as u32));
        }
        if prepared_random_seed.is_none() {
            prepared_random_seed = offline_random_seed;
        }
        let replay_parameter_clients = replay_parameters
            .as_ref()
            .map(|parameters| {
                parameters
                    .clients()
                    .iter()
                    .map(|client| clonk_engine::ClientCoreControlData {
                        client_id: client.id(),
                        activated: client.is_activated(),
                        observer: client.is_observer(),
                        name: LegacyCString::from_bytes(clonk_script::c4_string_bytes(
                            client.name(),
                        ))
                        .unwrap_or_default(),
                        nick: LegacyCString::from_bytes(clonk_script::c4_string_bytes(
                            client.nick(),
                        ))
                        .unwrap_or_default(),
                        lobby_ready: client.is_lobby_ready(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let (control_playback, replay_player_infos, replay_startup_player_count) = if replay {
            let group = open_group_path_for_folder_map(&path).map_err(|error| {
                ScenarioActivationError::Recoverable(format!(
                    "Failed to open replay {}: {error}",
                    scenario.title
                ))
            })?;
            let startup_player_count = if group.exists("Scenario.json") {
                None
            } else {
                Scenario::preflight_replay_startup_from_group(&group)
                    .map_err(|error| {
                        ScenarioActivationError::Recoverable(format!(
                            "Replay {} has invalid startup parameters: {error}",
                            scenario.title
                        ))
                    })?
                    .map(|startup| startup.startup_player_count)
            };
            let chunks = replay_control_record_chunks(&group).map_err(|error| {
                ScenarioActivationError::Recoverable(format!("Replay {} {error}", scenario.title))
            })?;
            if let Some(destination) = self
                .classic_command_line
                .record_dump
                .as_deref()
                .filter(|destination| !destination.is_empty())
                .map(Path::new)
            {
                write_classic_record_dump(&chunks, destination).map_err(|error| {
                    ScenarioActivationError::Recoverable(format!(
                        "Replay {} could not write /recdump {}: {error:#}",
                        scenario.title,
                        destination.display()
                    ))
                })?;
            }
            let playback = ControlRecordPlayback::from_chunks(chunks);
            let player_infos = if group.exists("PlayerInfos.txt") {
                let bytes = group.read_file("PlayerInfos.txt").map_err(|error| {
                    ScenarioActivationError::Recoverable(format!(
                        "Replay {} has unreadable PlayerInfos.txt: {error}",
                        scenario.title
                    ))
                })?;
                Some(
                    clonk_network::decode_player_info_list_ini(&bytes).map_err(|error| {
                        ScenarioActivationError::Recoverable(format!(
                            "Replay {} has invalid PlayerInfos.txt: {error}",
                            scenario.title
                        ))
                    })?,
                )
            } else {
                None
            };
            (Some(playback), player_infos, startup_player_count)
        } else {
            (None, None, None)
        };
        let mut offline_team_metadata = if network_game || replay {
            None
        } else {
            match scenario_data.initial_network_team_metadata() {
                Ok(metadata) => Some(metadata),
                Err(error) => {
                    tracing::debug!(%error, "offline exact team metadata is unavailable");
                    None
                }
            }
        };
        if !network_game {
            self.network_league_name = scenario_data
                .lobby_metadata()
                .map(|metadata| {
                    metadata.embedded_game_parameter_values().map_or_else(
                        || {
                            metadata
                                .game_parameter_defaults()
                                .league()
                                .as_bytes()
                                .to_vec()
                        },
                        |parameters| parameters.league().as_bytes().to_vec(),
                    )
                })
                .unwrap_or_default();
            if let Some(metadata) = scenario_data.lobby_metadata() {
                let max_players = metadata.embedded_game_parameter_values().map_or_else(
                    || metadata.game_parameter_defaults().max_players(),
                    |parameters| parameters.max_players(),
                );
                self.network_max_players = usize::try_from(max_players).unwrap_or(0);
            }
            if let Some(startup) = offline_startup_players.as_ref() {
                self.network_max_players = self
                    .network_max_players
                    .max(usize::try_from(startup.max_players()).unwrap_or(0));
            }
        }
        let mut engine = prepared_random_seed.map_or_else(Engine::new, Engine::with_seed);
        engine.set_smoke_level(self.graphics_smoke_level);
        let frozen_startup_player_count = if replay {
            replay_startup_player_count
        } else {
            let serialized = self
                .host_join_snapshot
                .as_ref()
                .map(|snapshot| snapshot.parameters.startup_player_count)
                .or(serialized_startup_player_count);
            let frame_zero_player_count = offline_startup_players
                .as_ref()
                .map(OfflineStartupPlayers::startup_player_count)
                .or_else(|| {
                    network_game.then(|| {
                        i32::try_from(self.control_player_infos.nonremoved_player_count())
                            .unwrap_or(i32::MAX)
                    })
                });
            startup_player_count_for_init(
                initial_game_data.map_or(0, |game_data| game_data.frame),
                serialized,
                frame_zero_player_count,
            )
        };
        if let Some(startup_player_count) = frozen_startup_player_count {
            engine.freeze_startup_player_count(startup_player_count);
        }
        if let Some(parameters) = replay_parameters.as_ref() {
            engine.set_control_rate(parameters.control_rate());
        }
        let (use_fair_crew, fair_crew_strength, fair_crew_forced, allow_debug) = prepared_fair_crew
            .unwrap_or_else(|| {
                if matches!(self.network_mode, Some(NetworkMode::Client(_))) {
                    // A client reaches activation only after JoinData installed
                    // prepared_go above. Keep standalone defaults for malformed
                    // transitional state rather than consulting local options.
                    (true, 1_000, false, true)
                } else {
                    let options = self.scenario_game_options.values();
                    scenario_data.lobby_metadata().map_or(
                        (options.fair_crew, options.fair_crew_strength, false, true),
                        |metadata| {
                            let (use_fair_crew, fair_crew_strength) =
                                resolve_scenario_fair_crew_parameters(metadata, options);
                            let embedded = metadata.embedded_game_parameter_values();
                            let parameters = embedded
                                .as_ref()
                                .unwrap_or_else(|| metadata.game_parameter_defaults());
                            (
                                use_fair_crew,
                                fair_crew_strength,
                                parameters.fair_crew_forced(),
                                parameters.allow_debug(),
                            )
                        },
                    )
                }
            });
        engine.set_use_fair_crew(use_fair_crew);
        engine.set_fair_crew_strength(fair_crew_strength);
        engine.set_fair_crew_forced(fair_crew_forced);
        engine.set_allow_debug(allow_debug);
        arm_configured_engine_debug_mode(&mut engine, self.app_paths.as_ref(), self.console_mode);
        engine.set_local_players([self.local_owner]);
        engine.set_max_players(i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        if let Some(timing) = self
            .network_control_clock
            .filter(|_| network_game)
            .map(NetworkControlClock::engine_timing)
            .transpose()
            .map_err(|error| format!("Invalid network control timing: {error}"))?
        {
            engine.initialize_network_control_timing(timing);
        }
        if !network_game {
            if let Some(startup) = offline_startup_players.as_ref() {
                engine.set_local_players([]);
                self.control_player_infos = ControlPlayerInfoRegistry::default();
                self.control_player_infos.apply(startup.player_info.clone());
            }
        }
        engine.set_network_game(network_game);
        engine.set_network_control_mode(network_game);
        engine.set_recording_active(false);
        engine.set_replay_control(replay);
        engine.set_league_game(self.network_is_league);
        seed_engine_player_info_parameters(
            &mut engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        // Full C4Game::InitGame clears the consumed restart handoff and
        // snapshots the authoritative PlayerInfos before this round's script
        // selects which fields a later Restart should restore.
        self.restart_restore_infos
            .capture_player_infos(&self.control_player_infos);
        self.restart_restore_roster_items.clear();
        self.apply_material_library_to(&mut engine);
        if replay {
            // C4GameControl::InitReplay sets fHost=false; replayed Set
            // packets change the synchronized list flags but do not execute
            // the host-only reassignment/attribute-update half.
            engine.set_control_host(false);
        }

        let sound_samples =
            configure_scenario_sound_samples(self.audio.as_mut(), scenario_data, &path);
        let music_tracks = self
            .audio
            .as_ref()
            .map(AudioContext::available_music_tracks)
            .unwrap_or_default();
        engine.configure_sound_samples(sound_samples);
        engine.configure_music_tracks(music_tracks);

        // C4Game::InitRules/InitGoals read the synchronized Game.Parameters
        // lists after the lobby, not the source Scenario.txt lists
        // (C4Game.cpp:4056-4076). PreparedGo retains those exact host-snapshot
        // or client-JoinData lists across asynchronous scenario activation.
        let synchronized_rule_goal_lists = self
            .loading_state
            .as_ref()
            .and_then(|loading| loading.prepared_go.as_ref())
            .map(|prepared| &prepared.synchronized_rule_goal_lists);
        let initial_record_music_enabled = (!replay
            && (self.recording_enabled || self.network_is_league))
            .then_some(initial_game_data.is_some_and(|game_data| game_data.music_enabled));
        let initial_record_game_data = match scenario_data.apply_before_players_for_game_start(
            &mut engine,
            network_game,
            initial_game_data,
            prepared_team_configuration,
            prepared_team_registry,
            synchronized_rule_goal_lists,
            initial_record_music_enabled,
        ) {
            Ok((_, initial_record)) => {
                initial_record.map(|result| result.map_err(|error| error.to_string()))
            }
            Err(err) => {
                tracing::error!(
                    scenario = %scenario.title,
                    path = %path.display(),
                    error = %err,
                    error_debug = ?err,
                    "failed to apply scenario"
                );
                return Err(scenario_activation_scenario_error(&scenario.title, err));
            }
        };
        self.advance_scenario_loader(94, "Definitions, scripts, landscape, and objects activated");
        let restored_music_enabled = initial_game_data
            .map(|game_data| engine.reconcile_music_after_restore(game_data.music_enabled));

        let pending_offline_joins = if !network_game {
            if offline_startup_players.is_some() {
                self.control_player_infos
                    .issue_unjoined_local_players(0, |info| Some(info.filename.clone()))
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        if offline_startup_players
            .as_ref()
            .is_some_and(|startup| startup.startup_player_count() == 0)
        {
            // Ordinary graphical C++ startup permits a zero count through
            // landscape creation, then fails after issuing local joins and
            // before Script.Initialize (pristine 9ffa0a5d
            // src/C4Game.cpp:2828-2852).
            return Err(ScenarioActivationError::Recoverable(format!(
                "Failed to start {}: Fullscreen mode requires at least one participating player.",
                scenario.title
            )));
        }

        let mut script_created_objects = false;
        if !network_game && offline_savegame.is_none() {
            let objects_before_initialize = engine.active_object_count();
            if let Err(err) = engine.initialize_scenario_script() {
                tracing::error!(
                    scenario = %scenario.title,
                    path = %path.display(),
                    error = %err,
                    "failed to initialize scenario script"
                );
                return Err(scenario_activation_engine_error(&scenario.title, err));
            }
            script_created_objects = engine.active_object_count() != objects_before_initialize;
        }

        if let Some(description) = scenario_data.description() {
            engine.show_scenario_intro(description);
        }
        self.advance_scenario_loader(95, "Scenario activation state prepared");

        self.auto_frame_skip = auto_frame_skip;
        self.engine = engine;
        self.script_created_objects = script_created_objects;
        self.runtime_player_big_icons.clear();
        self.runtime_player_big_icon_misses.clear();
        if !replay {
            let recording_definition_save_paths = retained_definition_save_paths
                .as_ref()
                .map(|(executable, definitions)| (executable.as_str(), definitions.as_str()));
            let prepare_result = match initial_record_game_data.as_ref() {
                Some(Ok(game_data)) => self.prepare_recording_for(
                    &scenario,
                    scenario_data,
                    Some(InitialRecordingSource::Fresh(game_data)),
                    retained_definition_modules.as_deref(),
                    recording_definition_save_paths,
                ),
                Some(Err(error)) => Err(error.clone()),
                None => self.prepare_recording_for(
                    &scenario,
                    scenario_data,
                    None,
                    retained_definition_modules.as_deref(),
                    recording_definition_save_paths,
                ),
            };
            if let Err(error) = prepare_result {
                let league_host = self.network_is_league
                    && matches!(self.runtime_network_role(), RuntimeNetworkRole::Host);
                if league_host {
                    return Err(ScenarioActivationError::Recoverable(format!(
                        "League recording could not start: {error}"
                    )));
                }
                tracing::warn!(%error, "failed to prepare C++-compatible recording");
            }
            if let Err(error) = self.start_recording(self.network_is_league) {
                let league_host = self.network_is_league
                    && matches!(self.runtime_network_role(), RuntimeNetworkRole::Host);
                if league_host {
                    return Err(ScenarioActivationError::Recoverable(format!(
                        "League recording could not start: {error}"
                    )));
                }
                tracing::warn!(%error, "failed to start C++-compatible recording");
            }
        }
        self.advance_scenario_loader(96, "Game runtime installed");
        self.film_view_player = None;
        self.clear_physical_viewport_states();
        self.physical_viewports_authoritative = false;
        self.input = InputDispatcher::new();
        self.local_controls = LocalControlRegistry::default();
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.ingame_gui_pointer = None;
        self.ingame_pointer = None;
        self.ingame_mouse_init_centered = false;
        self.ingame_viewport_mouse = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.mouse_control_allowed = !scenario_data.disables_mouse();
        self.mouse_control = self.mouse_control_allowed;
        if let Some(audio) = self.audio.as_mut() {
            audio.clear_object_sound_instances();
        }
        self.advance_scenario_loader(97, "Input and audio runtime initialized");
        if !network_game && !replay {
            if let Some(startup) = offline_startup_players.as_ref() {
                let startup_player_count = self
                    .engine
                    .startup_player_count()
                    .unwrap_or_else(|| startup.startup_player_count());
                let (mut local_players, mut joined_player_files) = if let Some(savegame) =
                    offline_savegame.as_ref()
                {
                    self.recreate_offline_savegame_players(&path, savegame)
                        .map_err(|error| scenario_activation_engine_error(&scenario.title, error))?
                } else {
                    (Vec::new(), Vec::new())
                };
                let mut team_selection_players = Vec::new();
                for join in pending_offline_joins {
                    let Some(info) = self.control_player_infos.get(join.info_id).cloned() else {
                        tracing::warn!(info_id = join.info_id, "offline join lost its player info");
                        continue;
                    };
                    let Some(selected) = startup.selected(join.info_id) else {
                        tracing::warn!(info_id = join.info_id, "offline join lost its player file");
                        continue;
                    };
                    let real_path = match offline_player_real_path(selected.source_path()) {
                        Ok(real_path) => real_path,
                        Err(error) => {
                            tracing::warn!(
                                info_id = join.info_id,
                                path = %selected.source_path().display(),
                                %error,
                                "failed to resolve offline player file"
                            );
                            continue;
                        }
                    };
                    if joined_player_files
                        .iter()
                        .any(|joined| offline_player_paths_identical(joined, &real_path))
                    {
                        // C4PlayerList::Join rejects a filename already owned
                        // by a runtime player, after the info was admitted and
                        // its join marked issued (pristine 9ffa0a5d
                        // src/C4PlayerList.cpp:271-302,433-453).
                        tracing::warn!(
                            info_id = join.info_id,
                            path = %selected.source_path().display(),
                            "offline player file is already in use"
                        );
                        continue;
                    }
                    let player_file = match PlayerFile::load_from_path(selected.source_path()) {
                        Ok(player_file) => player_file,
                        Err(error) => {
                            tracing::warn!(
                                info_id = join.info_id,
                                path = %selected.source_path().display(),
                                %error,
                                "failed to reload offline player file for join"
                            );
                            continue;
                        }
                    };
                    let player_big_icon = load_local_player_big_icon(selected.source_path());
                    let retained_player_info_core = player_file.exact_info_core();
                    let config = match clonk_engine::prepare_join_player_config(
                        clonk_engine::JoinPlayerPreparation {
                            join: &join,
                            info: &info,
                            player_file: Some(&player_file),
                            startup_player_count,
                        },
                    ) {
                        Ok(config) => config,
                        Err(error) => {
                            tracing::warn!(
                                info_id = join.info_id,
                                %error,
                                "failed to prepare offline player join"
                            );
                            continue;
                        }
                    };
                    if self.recording.is_some() {
                        match packed_group_bytes(
                            selected.source_path(),
                            self.process_group_maker.as_bytes(),
                        ) {
                            Ok(player_data) => {
                                let mut recorded_join = join.clone();
                                recorded_join.source =
                                    clonk_engine::JoinPlayerSource::Embedded(player_data);
                                self.record_control_batch(std::slice::from_ref(
                                    &clonk_engine::ControlPacket::JoinPlayer(recorded_join),
                                ));
                            }
                            Err(error) => {
                                tracing::warn!(
                                    info_id = join.info_id,
                                    path = %selected.source_path().display(),
                                    %error,
                                    "failed to embed initial player in recording"
                                );
                            }
                        }
                    }
                    let predicted_owner = self.engine.next_player_number();
                    let control = self.local_controls.initialize(LocalControlInit {
                        owner: predicted_owner,
                        preferred_set: player_file.pref_control,
                        prefers_mouse: player_file.pref_mouse,
                        gamepads_enabled: self.gamepads_enabled,
                        replay: false,
                        disable_mouse: !self.mouse_control_allowed,
                    });
                    match self.engine.join_player_with_profile_core(
                        config,
                        clonk_engine::PlayerAtClient::HOST,
                        "Local",
                        Some(&info),
                        control.runtime_control(),
                        retained_player_info_core,
                    ) {
                        Ok(joined) => {
                            debug_assert_eq!(joined.number(), predicted_owner);
                            self.cache_joined_player_big_icon(
                                join.info_id,
                                player_big_icon.as_ref(),
                            );
                            self.control_player_infos.mark_joined(
                                join.info_id,
                                joined.number(),
                                i32::try_from(self.engine.frame()).unwrap_or(i32::MAX),
                            );
                            local_players.push(joined.number());
                            if matches!(
                                joined,
                                clonk_engine::JoinPlayerOutcome::AwaitingTeamSelection { .. }
                            ) {
                                team_selection_players.push(joined.number());
                            }
                            self.local_player_profile_paths
                                .insert(join.info_id, real_path.clone());
                            joined_player_files.push(real_path);
                        }
                        Err(error) => {
                            self.remove_local_control_assignment(predicted_owner);
                            tracing::warn!(
                                info_id = join.info_id,
                                %error,
                                "offline player join failed"
                            );
                        }
                    }
                }
                self.mouse_control = self.local_controls.mouse_owner().is_some();
                if let Some(first) = local_players.first().copied() {
                    self.local_owner = first;
                }
                self.engine.set_local_players(local_players);
                if team_selection_players.contains(&self.local_owner) {
                    self.open_initial_team_selection(self.local_owner);
                }
            } else if let Err(err) = self.join_local_player() {
                tracing::error!(
                    scenario = %scenario.title,
                    path = %path.display(),
                    error = %err,
                    "failed to join local player"
                );
                return Err(scenario_activation_engine_error(&scenario.title, err));
            }
            // Scenario state application may replace the engine's local-player
            // projection. C++ derives LocalControl at the actual player join, so
            // restore the authoritative local set after that join completes.
            if offline_startup_players.is_none() {
                self.engine.set_local_players([self.local_owner]);
            }
        }
        if !prepared_go {
            self.advance_scenario_loader(98, "Players initialized");
        }

        self.sky = scenario_data.sky().map(sky_render_state_from_config);
        self.snapshot = self.engine.snapshot();
        self.rebuild_definition_sprites();
        {
            // Network peers consume the final published GameRes groups in wire
            // order. Some([]) is authoritative and suppresses local fallback.
            // Both paths retain C++'s independent material/texture overloads.
            let (authoritative_external_groups, reuse_preloaded_materials) =
                network_material_load_plan(
                    self.network_mode.as_ref(),
                    self.network_material_resource_groups.as_deref(),
                );
            if let Some((texture_images, render_info)) =
                preloaded_materials.filter(|_| reuse_preloaded_materials)
            {
                self.material_texture_images = texture_images;
                self.material_render_info = render_info;
            } else {
                self.material_texture_images = Arc::new(load_scenario_material_textures(
                    &path,
                    authoritative_external_groups,
                ));
                self.material_render_info = Arc::new(load_material_render_info(
                    &path,
                    authoritative_external_groups,
                ));
            }
            self.graphics
                .set_material_texture_surfaces(Arc::clone(&self.material_texture_images));
            self.graphics
                .set_material_render_info(Arc::clone(&self.material_render_info));
        }

        // `begin_loading_scenario` stores the pack-aware Title component (or
        // C4S.Head.Title fallback) selected while opening the scenario.
        let label = scenario.title.clone();
        let ground = match scenario_data.ground_height_hint() {
            Some(hint) => hint.max(0),
            None => Self::derive_ground_height(&self.engine, DEFAULT_GROUND_HEIGHT),
        };

        let offline_player_infos = offline_startup_players
            .is_some()
            .then(|| std::mem::take(&mut self.control_player_infos));
        self.active_game_graphics = Some(active_game_graphics);
        self.ingame_menu_gfx = None;
        self.configure_running_state(label, ground);
        // PlayScenarioMusic one-way enables Game.IsMusicEnabled when the
        // local RXMusic option is on, while configured-off clients retain a
        // restored or callback-enabled true value.
        self.runtime_music_enabled |= restored_music_enabled.unwrap_or(false);
        if !replay_parameter_clients.is_empty() {
            self.control_clients
                .replace_snapshot(replay_parameter_clients);
        }
        if let Some(player_infos) = replay_player_infos {
            let mut clients = self.control_clients.snapshot();
            for client in &player_infos.clients {
                if !clients
                    .iter()
                    .any(|known| known.client_id == client.client_id)
                {
                    clients.push(clonk_engine::ClientCoreControlData {
                        client_id: client.client_id,
                        activated: true,
                        observer: false,
                        name: LegacyCString::default(),
                        nick: LegacyCString::default(),
                        lobby_ready: false,
                    });
                }
            }
            self.control_clients.replace_snapshot(clients);
            self.control_player_infos.replace_snapshot(
                player_infos.last_player_id,
                player_infos.clients.into_iter().map(|client| {
                    clonk_engine::PlayerInfoControlData {
                        client_id: client.client_id,
                        flags: client.flags,
                        players: client.players,
                        by_client: 0,
                    }
                }),
            );
            seed_engine_player_info_parameters(
                &mut self.engine,
                &self.network_league_name,
                &self.control_player_infos,
            );
        }
        if let Some(player_infos) = offline_player_infos {
            self.control_player_infos = player_infos;
        }
        if !network_game && !replay {
            self.refresh_current_player_info_teams();
            self.network_team_assignment = offline_team_metadata.take().map(|mut metadata| {
                project_runtime_memberships_into_initial_metadata(
                    &mut metadata,
                    self.engine.teams(),
                );
                NetworkTeamAssignmentState::from_prepared_host_with_team_name_template(
                    metadata,
                    self.generated_team_name_template.clone(),
                )
            });
        } else if replay {
            self.network_team_assignment = None;
        }
        self.open_initial_team_selection(self.local_owner);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        if !network_game {
            // InitGameFinal creates owned/replay-film viewports before
            // Game.IsRunning becomes true, so C++ uses FESamples here.
            self.initialize_physical_viewports(false);
        }
        self.arm_initial_scoreboard_reconcile();
        // C4Game::InitGame applies the scenario gamma before its first frame
        // (C4Game.cpp:487-490).
        self.graphics
            .apply_gamma_now(&self.snapshot.environment.gamma);
        self.refresh_object_menu();
        self.refresh_focus();
        self.active_scenario = Some(scenario.clone());
        self.active_definition_load = Some(activated_definition_load(
            retained_definition_modules,
            effective_definition_load,
        ));
        self.active_description_definition_modules = effective_description_definition_modules;
        self.control_playback = control_playback;
        self.play_scenario_audio(&path);
        if initial_game_data.is_some() {
            let restored_music_level = self.engine.music_level();
            if let Some(audio) = self.audio.as_mut() {
                audio.set_scenario_music_level(Some(restored_music_level));
            }
        }
        if !prepared_go {
            self.advance_scenario_loader(99, "Final game initialization complete");
        }
        self.status_text.clear();
        Ok(())
    }

    pub(crate) fn start_sandbox_scenario(
        &mut self,
        scenario: FrontendScenario,
    ) -> Result<(), EngineError> {
        let catalog_paths = self.app_paths.clone();
        let crew_paths = self.sandbox_crew_definition_paths.clone();
        let definition_load = match (catalog_paths.as_ref(), crew_paths.as_ref()) {
            (Some(paths), _) => SandboxDefinitionLoad::InstallCatalog(paths),
            (None, Some(paths)) => SandboxDefinitionLoad::InstallCrew(paths),
            (None, None) => SandboxDefinitionLoad::None,
        };
        self.start_sandbox_scenario_with_definitions(scenario, definition_load)
    }

    pub(crate) fn start_sandbox_scenario_with_definitions(
        &mut self,
        scenario: FrontendScenario,
        definition_load: SandboxDefinitionLoad<'_>,
    ) -> Result<(), EngineError> {
        self.sandbox_crew_definition_paths = match definition_load {
            SandboxDefinitionLoad::InstallCrew(paths) => Some(paths.clone()),
            SandboxDefinitionLoad::None | SandboxDefinitionLoad::InstallCatalog(_) => None,
        };
        tracing::info!(
            scenario = %scenario.title,
            "starting sandbox fallback scenario"
        );
        self.auto_frame_skip =
            configured_auto_frame_skip(&load_native_config_bytes(self.app_paths.as_ref()));

        self.active_game_graphics = None;
        self.ingame_menu_gfx = None;
        self.runtime_player_big_icons.clear();
        self.runtime_player_big_icon_misses.clear();
        self.restore_startup_gui_sheets();
        self.active_global_gui_failures.clear();
        self.finish_recording();
        self.live_save_seed = None;
        self.recording_template = None;
        self.control_playback = None;
        self.deferred_network_savegame_recreation.clear();
        self.loading_state = None;
        self.engine = Engine::new();
        self.film_view_player = None;
        self.clear_physical_viewport_states();
        self.physical_viewports_authoritative = false;
        self.engine.set_smoke_level(self.graphics_smoke_level);
        self.engine.set_local_players([self.local_owner]);
        self.engine.set_network_game(self.network.is_some());
        self.engine.set_network_control_mode(self.network.is_some());
        self.engine.set_league_game(self.network_is_league);
        seed_engine_player_info_parameters(
            &mut self.engine,
            &self.network_league_name,
            &self.control_player_infos,
        );
        self.engine
            .set_max_players(i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.local_controls = LocalControlRegistry::default();
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.ingame_gui_pointer = None;
        self.ingame_pointer = None;
        self.ingame_mouse_init_centered = false;
        self.ingame_viewport_mouse = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.construction_menu_drag = None;
        self.ingame_dragged_objects.clear();
        self.mouse_control_allowed = true;
        self.mouse_control = true;
        self.active_definition_load = None;
        self.active_description_definition_modules.clear();
        self.sky = None;

        arm_configured_engine_debug_mode(
            &mut self.engine,
            self.app_paths.as_ref(),
            self.console_mode,
        );
        let spawn_definition =
            configure_sandbox_engine(&mut self.engine, definition_load, self.audio.as_mut())?;

        self.ensure_local_player_registered()?;

        let spawn = SpawnConfig::new(spawn_definition)
            .with_owner(self.local_owner)
            .with_position(Vector2::new(240, 180))
            .with_energy(100)
            .with_action(ActionState::new("Walk"))
            .with_crew_member(true);
        self.engine.spawn_object(spawn)?;

        self.snapshot = self.engine.snapshot();
        self.rebuild_definition_sprites();
        let fallback_ground = Self::derive_ground_height(&self.engine, DEFAULT_GROUND_HEIGHT);
        self.configure_running_state(scenario.title.clone(), fallback_ground);
        if matches!(self.runtime_network_role(), RuntimeNetworkRole::Offline)
            && self.engine.is_control_host()
        {
            self.network_team_assignment = initial_team_metadata_from_runtime(
                self.engine.team_configuration(),
                self.engine.teams(),
            )
            .map(|metadata| {
                NetworkTeamAssignmentState::from_prepared_host_with_team_name_template(
                    metadata,
                    self.generated_team_name_template.clone(),
                )
            });
        }
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.initialize_physical_viewports(false);
        self.arm_initial_scoreboard_reconcile();
        self.refresh_object_menu();
        self.refresh_focus();
        self.active_scenario = Some(scenario);
        self.play_sandbox_audio();
        Ok(())
    }
}
