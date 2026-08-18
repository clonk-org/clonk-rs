//! `impl GameApp` — developer console, recording & replay methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    fn console_game_initialization_active(&self) -> bool {
        self.loading_state.is_some()
            || self.auto_start_classic_command_line_scenario
            || self.classic_direct_reference_query.is_some()
            || self.startup_network_connection.is_some()
            || self.pending_network_join.is_some()
            || self.staged_network_host_scenario.is_some()
            || self.lobby_preload_task.is_some()
            || self.lobby_preload_artifact.is_some()
            || self.network_start_wait.is_some()
            || self.pending_network_join_data.is_some()
            || self.pending_client_start_status.is_some()
    }

    pub(crate) fn console_game_active(&self) -> bool {
        self.mode == AppMode::Running
            || self.console_lobby_active()
            || self.console_game_initialization_active()
            || (self.boot_loading.is_none()
                && (self.network.is_some()
                    || self.network_mode.is_some()
                    || self.network_lobby.is_some()
                    || self.classic_host_lobby.is_some()))
    }

    /// `Application.UseStartupDialog`: whether this session has a startup
    /// generation for `QuitGame` to return to (C4Application.cpp:373-405)
    /// rather than falling through to `Quit()`.
    ///
    /// `ParseCommandLine` computes it from the launch parameters
    /// (C4Game.cpp:3299) — which is what
    /// [`Self::failed_open_game_returns_to_startup`] already reports — and a
    /// console `/open` or `/close` puts it back afterwards
    /// (C4Application.cpp:598-612,617-624).
    pub(crate) fn startup_dialog_in_use(&self) -> bool {
        self.console_restored_startup_dialog || self.failed_open_game_returns_to_startup()
    }

    /// Return a finished console round to the state its next command is
    /// accepted from, the way `QuitGame` reaches `C4AS_PreInit` and `PreInit`
    /// then settles on `C4AS_Startup` (C4Application.cpp:373-405,239-293).
    pub(crate) fn park_console_session_after_round(&mut self) {
        // The scenario that just ended must not be re-launched by the boot
        // worker or leak into the next round's save.
        self.classic_command_line.scenario = None;
        self.classic_command_line.record_stream = None;
        self.classic_command_line.direct_join = None;
        self.classic_record_stream_activation_pending = false;
        self.console_restored_startup_dialog = true;
        self.close_console_game();
    }

    fn close_console_game(&mut self) {
        let boot_still_loading = self.boot_loading.is_some();
        let network_game_active = self.network.is_some()
            || self.network_mode.is_some()
            || self.network_lobby.is_some()
            || self.classic_host_lobby.is_some()
            || self.startup_network_connection.is_some()
            || self.classic_direct_reference_query.is_some()
            || self.pending_network_join.is_some()
            || self.staged_network_host_scenario.is_some()
            || self.lobby_preload_task.is_some()
            || self.lobby_preload_artifact.is_some()
            || self.network_start_wait.is_some()
            || self.pending_network_join_data.is_some()
            || self.pending_client_start_status.is_some();
        self.auto_start_sandbox = false;
        self.auto_start_classic_command_line_scenario = false;
        self.classic_direct_reference_query = None;
        if network_game_active {
            // `show_main_menu` owns the complete native network teardown for
            // a lobby. Async console startup can be between lobby views, so
            // select that teardown path before clearing the round.
            self.startup_view = StartupView::NetworkLobby;
        }
        // Components belong to the scenario that was open. C++ never has to
        // clear them — `Game.Script`/`Title`/`Info` are cleared with the whole
        // `C4Game` — but here they are `GameApp` fields, and an edit left
        // behind would be written into the *next* scenario's save.
        self.developer_component_editor = None;
        self.developer_component_hosts.clear();
        self.developer_object_list_open = false;
        self.return_to_menu();
        if boot_still_loading {
            self.mode = AppMode::Loading;
        }
    }

    pub(crate) fn developer_console_editing(&self) -> bool {
        self.console_mode && self.developer_console_editing_enabled
    }

    fn developer_console_strings(&self) -> ConsoleStrings {
        let mut strings = ConsoleStrings::default();
        for (target, key, fallback) in [
            (&mut strings.default_caption, "IDS_CNS_CONSOLE", "Console"),
            (&mut strings.menu_file, "IDS_MNU_FILE", "File"),
            (
                &mut strings.menu_components,
                "IDS_MNU_COMPONENTS",
                "Components",
            ),
            (&mut strings.menu_player, "IDS_MNU_PLAYER", "Player"),
            (&mut strings.menu_viewport, "IDS_MNU_VIEWPORT", "Viewport"),
            (&mut strings.menu_net, "IDS_MNU_NET", "Host"),
            (&mut strings.file_open, "IDS_MNU_OPEN", "Open..."),
            (
                &mut strings.file_open_with_players,
                "IDS_MNU_OPENWPLRS",
                "Open with players...",
            ),
            (
                &mut strings.file_save_scenario,
                "IDS_MNU_SAVESCENARIO",
                "Save scenario",
            ),
            (
                &mut strings.file_save_scenario_as,
                "IDS_MNU_SAVESCENARIOAS",
                "Save scenario as...",
            ),
            (&mut strings.file_save_game, "IDS_MNU_SAVEGAME", "Save game"),
            (
                &mut strings.file_save_game_as,
                "IDS_MNU_SAVEGAMEAS",
                "Save game as...",
            ),
            (&mut strings.file_record, "IDS_MNU_RECORD", "Record"),
            (&mut strings.file_close, "IDS_MNU_CLOSE", "Close"),
            (&mut strings.file_quit, "IDS_MNU_QUIT", "Quit"),
            (&mut strings.component_objects, "IDS_BTN_OBJECTS", "Objects"),
            (&mut strings.component_script, "IDS_MNU_SCRIPT", "Script"),
            (&mut strings.component_title, "IDS_MNU_TITLE", "Title"),
            (&mut strings.component_info, "IDS_MNU_INFO", "Info"),
            (&mut strings.player_join, "IDS_MNU_JOIN", "Join"),
            (&mut strings.viewport_new, "IDS_MNU_NEW", "New"),
            (&mut strings.help_about, "IDS_MENU_ABOUT", "About..."),
        ] {
            *target = self.runtime_resource_text(key, fallback);
        }
        strings
    }

    pub(crate) fn developer_console_view_model(&self) -> ConsoleViewModel {
        let strings = self.developer_console_strings();
        // Native C4Console sets fGameOpen only after Game.Init returns. Keep
        // save/close/component controls disabled throughout Rust's async load.
        let game_open = self.mode == AppMode::Running;
        let network_enabled = self.network.is_some();
        let network_host =
            network_enabled && matches!(self.network_mode, Some(NetworkMode::Host(_)));
        let editing = self.developer_console_editing();
        let players = self.developer_console_player_menu_entries(editing);
        let clients = self.developer_console_net_menu_entries();
        let completions = developer_console_completion_entries(
            &self.engine.console_script_completion_catalog(),
            DeveloperConsoleCompletionStyle::Gtk,
        )
        .into_iter()
        .filter_map(|entry| match entry {
            DeveloperConsoleCompletionEntry::Function(function) => Some(function),
            DeveloperConsoleCompletionEntry::Separator => None,
        })
        .collect();
        let current_scenario_path = self
            .active_scenario
            .as_ref()
            .and_then(|scenario| scenario.path.clone())
            .or_else(|| {
                self.loading_state
                    .as_ref()
                    .and_then(|loading| loading.scenario.path.clone())
            });
        let caption = current_scenario_path
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| strings.default_caption.clone());
        ConsoleViewModel {
            strings,
            caption,
            current_scenario_path,
            game_open,
            lobby_active: self.console_lobby_active(),
            editing,
            halted: !matches!(self.mode, AppMode::Running)
                || if self.network.is_some() {
                    self.runtime_network_is_paused()
                } else {
                    self.runtime_halt_active()
                },
            runtime_record_possible: self.developer_console_runtime_record_possible(),
            network_enabled,
            network_host,
            players,
            clients,
            completions,
            edit_mode: self.developer_console_edit_mode,
            cursor_text: self.status_text.clone(),
            frame: self.engine.frame(),
            script_counter: self.engine.scenario_script_counter(),
            time_seconds: self.engine.game_time(),
            frames_per_second: self.frames_per_second,
        }
    }

    pub(crate) fn sync_developer_console_view(&mut self) -> bool {
        if !self.console_mode {
            return false;
        }
        if self.control_playback.is_some() {
            self.developer_console_editing_enabled = false;
        }
        let view = self.developer_console_view_model();
        self.developer_console.set_view_model(view)
    }

    pub(crate) fn drain_console_log_capture(&mut self) {
        let Some(capture) = self.console_log_capture.as_ref() else {
            return;
        };
        let output = capture.take();
        if !output.is_empty() {
            self.developer_console.out(&output);
        }
    }

    pub(crate) fn show_developer_console_message(
        &mut self,
        message: String,
        follow_up: Option<String>,
    ) -> std::result::Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                "Clonk Rust",
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(44),
            ),
            MessageDialogContinuation::DeveloperConsoleNotice { follow_up },
        )?;
        Ok(())
    }

    fn open_developer_console_game(
        &mut self,
        scenario: PathBuf,
        player_files: Vec<PathBuf>,
    ) -> Result<()> {
        // C4Console::OpenGame calls Console::Default before any game
        // initialization and therefore resets only the edit cursor mode.
        self.developer_console_edit_mode = ConsoleEditMode::Play;
        if self.console_game_active() {
            self.close_console_game();
        }
        let mut arguments = Vec::with_capacity(1 + player_files.len());
        arguments.push(scenario.into_os_string());
        arguments.extend(player_files.into_iter().map(PathBuf::into_os_string));
        let mut classic = parse_classic_command_line(&arguments);
        classic.console = true;
        self.apply_classic_command_line(&classic)?;
        self.launch_classic_command_line_join()?;
        self.launch_classic_command_line_scenario()?;
        Ok(())
    }

    fn choose_developer_console_paths(request: &ConsolePathRequest) -> Vec<PathBuf> {
        let mut dialog = rfd::FileDialog::new().set_title(&request.title);
        if let Some(path) = request.suggested_path.as_deref() {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = path.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        }
        if !request.extensions.is_empty() {
            dialog = dialog.add_filter(request.filter_label.as_str(), &request.extensions);
        }
        if request.save {
            dialog.save_file().into_iter().collect()
        } else if request.allow_multiple {
            dialog.pick_files().unwrap_or_default()
        } else {
            dialog.pick_file().into_iter().collect()
        }
    }

    pub(crate) fn dispatch_developer_console_actions(
        &mut self,
        mut actions: Vec<DeveloperConsoleAction>,
    ) -> Result<()> {
        while let Some(action) = actions.pop() {
            match action {
                DeveloperConsoleAction::RequestPath(request) => {
                    let paths = Self::choose_developer_console_paths(&request);
                    let follow_up = self
                        .developer_console
                        .respond_path_request(request.token, paths);
                    actions.extend(follow_up.into_iter().rev());
                }
                DeveloperConsoleAction::OpenGame {
                    scenario,
                    player_files,
                } => {
                    if let Err(error) = self.open_developer_console_game(scenario, player_files) {
                        tracing::error!(%error, "developer-console game open failed");
                        // FileOpen intentionally ignores OpenGame(false).
                        // Clear partial async/bootstrap state and leave the
                        // persistent developer window alive.
                        self.close_console_game();
                    }
                }
                DeveloperConsoleAction::Save { kind, target } => {
                    let result = self.save_developer_console_game(kind, target.as_deref());
                    let attempted = !matches!(&result, Ok(false));
                    let save_error = result.err().map(|error| {
                        tracing::error!(%error, ?kind, target = ?target, "developer-console save failed");
                        if let Some(target) = target.as_deref() {
                            let target = target.to_string_lossy();
                            format_resource_string(
                                self.runtime_resource_text(
                                    "IDS_CNS_SAVEASERROR",
                                    "Error while saving the scenario to %s.",
                                ),
                                &[&target],
                            )
                        } else {
                            self.runtime_resource_text(
                                "IDS_CNS_SAVERROR",
                                "Error while saving the scenario.",
                            )
                        }
                    });
                    if kind == ConsoleSaveKind::Scenario && attempted && self.script_created_objects
                    {
                        let warning = format!(
                            "{}{}",
                            self.runtime_resource_text(
                                "IDS_CNS_SCRIPTCREATEDOBJECTS",
                                "This scenario's script has created objects on initialization. "
                            ),
                            self.runtime_resource_text(
                                "IDS_CNS_WARNDOUBLE",
                                "In order to avoid double creation, the script's 'Initialize' function should be modified."
                            )
                        );
                        self.script_created_objects = false;
                        self.show_developer_console_message(warning, save_error)?;
                    } else if let Some(message) = save_error {
                        self.show_developer_console_message(message, None)?;
                    }
                }
                DeveloperConsoleAction::RequestRuntimeRecord => {
                    if let Err(error) = self.developer_console_request_runtime_record() {
                        self.developer_console.out(&error);
                    }
                }
                DeveloperConsoleAction::CloseGame => self.close_console_game(),
                DeveloperConsoleAction::QuitApplication => {
                    self.request_exit("the developer console quit")
                }
                DeveloperConsoleAction::Play => self.set_runtime_pause(false),
                DeveloperConsoleAction::Halt => self.set_runtime_pause(true),
                DeveloperConsoleAction::TogglePause => self.toggle_runtime_pause(),
                DeveloperConsoleAction::SetEditMode(mode) => {
                    let previous = self.console_cursor_mode();
                    self.developer_console_edit_mode = mode;
                    self.apply_developer_cursor_mode_change(previous);
                }
                DeveloperConsoleAction::SubmitInput(input) => {
                    let editing = self.developer_console_editing();
                    self.process_developer_console_input(&input, editing)?;
                }
                DeveloperConsoleAction::JoinPlayers(paths) => {
                    let editing = self.developer_console_editing();
                    if let Err(error) = self.developer_console_join_players(&paths, editing) {
                        self.developer_console.out(&error);
                    }
                }
                DeveloperConsoleAction::EliminatePlayer(player) => {
                    let editing = self.developer_console_editing();
                    if let Err(error) = self.developer_console_quit_player(player, editing) {
                        self.developer_console.out(&error);
                    }
                }
                DeveloperConsoleAction::KickClient(client) => {
                    if let Err(error) = self.developer_console_kick_client(client) {
                        self.developer_console.out(&error);
                    }
                }
                DeveloperConsoleAction::NewViewport(player) => {
                    let owner = player.unwrap_or(OWNER_NONE);
                    // `C4Console::ViewportNew` is `Game.CreateViewport(NO_OWNER)`
                    // (C4Console.cpp:1205) and the per-player rows are
                    // `Game.CreateViewport(<player>)` (:223, :1828) — all three
                    // take `fSilent`'s `false` default (C4Game.h:222). Only
                    // `C4FullScreen::ViewportCheck` silences an ownerless
                    // creation (C4FullScreen.cpp:517), and this is not it.
                    let _ = self.create_physical_viewport(
                        owner,
                        false,
                        self.mode == AppMode::Running,
                        true,
                    );
                }
                // `C4Console::EditObjects` is one line: `ObjectListDlg.Open()`
                // (`C4Console.cpp:1353-1356`). Unlike its three siblings it
                // has no network refusal — the list only reads.
                DeveloperConsoleAction::EditObjects => self.open_developer_object_list(),
                // The three `C4Console::Edit*` entries, which share a network
                // refusal and differ only in the component and in whether
                // they relink (`C4Console.cpp:1328-1351`).
                DeveloperConsoleAction::EditScript => self.open_developer_component_editor(
                    clonk_engine::developer_components::EditableComponent::Script,
                ),
                DeveloperConsoleAction::EditTitle => self.open_developer_component_editor(
                    clonk_engine::developer_components::EditableComponent::Title,
                ),
                DeveloperConsoleAction::EditInfo => self.open_developer_component_editor(
                    clonk_engine::developer_components::EditableComponent::Info,
                ),
            };
        }
        Ok(())
    }

    /// `C4Record::Start` prepares the configured record root through
    /// `CreateSaveFolder` (C4Record.cpp:118-145), which also writes the
    /// language-prefixed `Title.txt` naming the folder (C4Config.cpp:1397-1412).
    fn prepare_recording_root(&self, directory: &std::path::Path) -> std::io::Result<()> {
        let language = classic_save_folder_language(self.app_paths.as_ref());
        crate::output_folders::create_save_folder(
            directory,
            &self.runtime_resource_string("IDS_GAME_RECORDSTITLE"),
            &String::from_utf8_lossy(&language),
        )
    }

    pub(crate) fn finish_console_shutdown(&mut self) {
        self.finish_recording();
        self.finalize_pending_league_end_for_teardown();
        // `C4Application::Quit` ends in `if (Config.fConfigLoaded) Config.Save();`
        // with no `USE_CONSOLE` guard (C4Application.cpp:367), so a dedicated
        // server writes its accumulated Config on a clean quit exactly like a
        // fullscreen run. `run_headless_server` returns before the winit loop
        // exists, so it never reaches that loop's own exiting flush.
        self.flush_deferred_config();
    }

    pub(crate) fn process_console_command(&mut self, line: &str) -> Result<()> {
        if line == "/quit" {
            self.request_exit("the console /quit command");
            return Ok(());
        }

        let lobby_active = self.console_lobby_active();
        if line == "/close" && self.console_game_active() {
            // `/close` clears the round and sets `UseStartupDialog`
            // (C4Application.cpp:617-624), so the session has a startup
            // generation again whatever it was launched with.
            self.console_restored_startup_dialog = true;
            self.close_console_game();
            return Ok(());
        }

        if lobby_active && line.starts_with("/start") {
            return self.process_console_lobby_start(line);
        }

        if self.console_startup_active() {
            if let Some(parameters) = line.strip_prefix("/open ") {
                let classic =
                    parse_classic_command_line(&parse_classic_console_parameters(parameters));
                self.apply_classic_command_line(&classic)?;
                // `/open` re-parses a command line and then sets
                // `UseStartupDialog` back (C4Application.cpp:598-612), even
                // though the parse just filled in a scenario filename that
                // would otherwise clear it (C4Game.cpp:3299). That is what
                // returns the console to `C4AS_Startup` when this round ends.
                self.console_restored_startup_dialog = true;
                self.launch_classic_command_line_join()?;
                self.launch_classic_command_line_scenario()?;
            }
            return Ok(());
        }

        if self.console_game_active() {
            self.process_console_message_input(line);
        }
        Ok(())
    }

    pub(crate) fn replay_pending_native_presentation(
        &mut self,
        composer: &mut clonk_scaling::OrderedFrameComposer<'_>,
    ) -> Result<()> {
        let Some(plan) = self.pending_native_presentation.take() else {
            return Ok(());
        };
        let _renderer_config = clonk_frontend::activate_advanced_renderer_config(
            self.graphics.advanced_renderer_config(),
        );
        let NativePresentationPlan {
            batches,
            monitor_gamma,
        } = plan;
        let default_fonts = self
            .native_startup_fonts
            .as_deref()
            .context("scale-native font bundle disappeared before presentation")?;
        let startup_gamma = self.startup_fragment_gamma();
        for batch in batches {
            if let Some(layer) = batch.logical_layer {
                let target = composer.begin_layer();
                anyhow::ensure!(
                    layer.len() == target.len(),
                    "native presentation layer has {} bytes, expected {}",
                    layer.len(),
                    target.len()
                );
                target.copy_from_slice(&layer);
                if let Some(clip) = batch.clip {
                    composer.composite_layer_with_clip(clip);
                } else {
                    composer.composite_layer();
                }
            }
            if batch.native_loader_text {
                let loader = self.loader_screen.as_ref().ok_or_else(|| {
                    self.loader_boundary("selected classic loader disappeared before presentation")
                })?;
                composer
                    .draw_native(|physical, geometry| -> Result<()> {
                        let (width, height) = geometry.physical_size();
                        let mut surface = RgbaSurfaceViewMut::new(width, height, physical)?;
                        let (logical_width, logical_height) = geometry.logical_size();
                        loader.render_native_text_to(
                            &mut surface,
                            default_fonts,
                            logical_width,
                            logical_height,
                            Some(&startup_gamma),
                        )?;
                        Ok(())
                    })
                    .map_err(|error| self.loader_boundary(error.to_string()))?;
            }
            if batch.text.is_empty() {
                continue;
            }
            let fonts = batch.fonts.as_deref().unwrap_or(default_fonts);
            composer.draw_native(|physical, geometry| -> Result<()> {
                let (width, height) = geometry.physical_size();
                let mut surface = RgbaSurfaceViewMut::new(width, height, physical)?;
                fonts.draw_captured_text_to(&mut surface, &batch.text, geometry.logical_size());
                Ok(())
            })?;
        }
        if let Some(gamma) = monitor_gamma {
            composer.draw_native(|physical, _| gamma.apply_to_rgba_bytes(physical));
        }
        Ok(())
    }

    pub(crate) fn record_network_stats_control_frame(&mut self) {
        let control_rate = self.engine.control_rate();
        if !matches!(self.mode, AppMode::Running)
            || self.network_stats.is_none()
            || control_rate <= 0
            || !self.engine.frame().is_multiple_of(control_rate as u64)
        {
            return;
        }
        self.reconcile_network_stats_series();
        let samples = self
            .engine
            .take_player_control_counts()
            .into_iter()
            .map(|(player_id, controls, actions)| PlayerControlSample {
                player_id,
                controls,
                actions,
            })
            .collect::<Vec<_>>();
        if let Some(stats) = self.network_stats.as_mut() {
            stats.record_control_frame(control_rate, samples);
        }
    }

    pub(crate) fn record_network_stats_frame(&mut self) {
        if !matches!(self.mode, AppMode::Running) || self.network_stats.is_none() {
            return;
        }
        let object_count = self
            .snapshot
            .objects
            .iter()
            // C4ObjectList::ObjectCount checks the raw nonzero Status field:
            // both Normal (1) and Inactive (2) objects are represented.
            .filter(|object| object.status != clonk_engine::ObjectStatus::Deleted)
            .count();
        if let Some(stats) = self.network_stats.as_mut() {
            stats.record_frame(i32::try_from(object_count).unwrap_or(i32::MAX));
        }
    }

    pub(crate) fn record_network_stats_second(&mut self) {
        if !matches!(self.mode, AppMode::Running) || self.network_stats.is_none() {
            return;
        }
        self.reconcile_network_stats_series();
        let mut message_pings = BTreeMap::<ClientId, i32>::new();
        if let Some(connections) = self
            .network
            .as_ref()
            .and_then(|network| network.runtime_connections().ok())
        {
            for connection in connections {
                if matches!(connection.usage.as_str(), "Msg" | "Data/Msg") {
                    // C4Network2Stats samples getMsgConn()->getLag()
                    // (src/C4Network2Stats.cpp:336-343).
                    message_pings
                        .entry(connection.client_id)
                        .or_insert(connection.lag_ms);
                }
            }
        }
        let pings = message_pings
            .into_iter()
            .map(|(client_id, lag_ms)| ClientPingSample {
                client_id,
                lag_ms: Some(lag_ms),
            })
            .collect::<Vec<_>>();
        if let Some(stats) = self.network_stats.as_mut() {
            let (input, output) = self.network.as_ref().map_or(
                (ProtocolRateSample::new(0, 0), ProtocolRateSample::new(0, 0)),
                NetworkManager::protocol_rate_samples,
            );
            stats.record_second(self.frames_per_second, input, output, pings);
        }
    }

    /// Reconcile legacy/direct-fixture ownership, then apply the ordered
    /// physical viewport request stream. An absent primary stays absent.
    pub(crate) fn sync_film_view_presentation(&mut self) {
        self.refresh_non_authoritative_physical_viewports();
        self.apply_direct_film_view_projection();
        let _ = self.apply_pending_viewport_presentation_requests();
    }

    pub(crate) fn handle_film_view_key_for_mode(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
        film_replay: bool,
    ) -> bool {
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if !film_replay
            || !self.runtime_keyboard_binding_matches(
                "FilmNextPlayer",
                key,
                key == VirtualKeyCode::ArrowRight && c4_modifiers.is_empty(),
            )
            || state != ElementState::Pressed
            || !self.viewport_cycle_scope_available()
            || self.physical_viewports.is_empty()
        {
            return false;
        }
        self.cycle_primary_viewport_player(true);
        true
    }

    pub(crate) fn running_console_script_strictness(&self) -> clonk_engine::ScriptStrictness {
        configured_console_script_strictness(&load_native_config_bytes(self.app_paths.as_ref()))
    }

    fn process_console_message_input(&mut self, text: &str) {
        self.process_message_input_text(text, false);
    }

    /// Command-only half of `C4MessageInput::ProcessCommand`. In particular,
    /// `/sound` is unknown here; `#/sound` reaches `ProcessInput` below and
    /// produces the private message control used by the developer console.
    fn process_developer_console_command(&mut self, text: &str) -> Result<(), EngineError> {
        if text == "/clear" {
            self.developer_console.clear_log();
            return Ok(());
        }
        if self.process_control_message_local_command(text) {
            return Ok(());
        }
        match self.process_running_chat_command(text)? {
            true => {}
            false => self.append_unknown_running_command(text),
        }
        Ok(())
    }

    /// Dynamic Player-menu rows from `C4Console::UpdatePlayerMenu`. The
    /// player list itself is already retained in native C4PlayerList order;
    /// network captions use the player's join-time AtClientName snapshot.
    pub(crate) fn developer_console_player_menu_entries(
        &self,
        editing: bool,
    ) -> Vec<ConsolePlayerRow> {
        let network_enabled = self.network.is_some();
        let enabled = editing
            && (!network_enabled || matches!(self.network_mode, Some(NetworkMode::Host(_))));
        self.engine
            .players()
            .map(|player| {
                let name = c4_presentation_text(player.name());
                let quit_label = if network_enabled {
                    let at_client = c4_presentation_text(player.at_client_name());
                    format_resource_string(
                        self.runtime_resource_text("IDS_CNS_PLRQUITNET", "Remove %s (%s) "),
                        &[&name, &at_client],
                    )
                } else {
                    format_resource_string(
                        self.runtime_resource_text("IDS_CNS_PLRQUIT", "Remove %s"),
                        &[&name],
                    )
                };
                ConsolePlayerRow {
                    number: player.id(),
                    quit_label,
                    quit_enabled: enabled,
                    viewport_label: format_resource_string(
                        self.runtime_resource_text("IDS_CNS_NEWPLRVIEWPORT", "New for %s"),
                        &[&name],
                    ),
                }
            })
            .collect()
    }

    /// File-picker result backend for `C4Console::PlayerJoin`. Every selected
    /// path is attempted even when an earlier player fails, as in the native
    /// semicolon-list loop. Legacy path bytes survive the Rust Path boundary.
    pub(crate) fn developer_console_join_players(
        &mut self,
        paths: &[PathBuf],
        editing: bool,
    ) -> std::result::Result<usize, String> {
        if !editing || self.mode != AppMode::Running || self.control_playback.is_some() {
            return Ok(0);
        }
        let network_enabled = self.network.is_some();
        let mut joined = 0usize;
        let mut errors = Vec::new();
        for path in paths {
            let wire_filename = clonk_script::c4_string_from_bytes(&path_to_legacy_bytes(path));
            let result = if network_enabled {
                self.submit_runtime_network_player_path(path, &wire_filename)
            } else {
                self.submit_runtime_offline_player(&wire_filename)
            };
            match result {
                Ok(()) => joined += 1,
                Err(error) => errors.push(format!("{}: {error}", path.display())),
            }
        }
        if errors.is_empty() {
            Ok(joined)
        } else {
            Err(errors.join("; "))
        }
    }

    /// Queue the Player-menu `CID_EliminatePlayer`. Network games use the
    /// ordinary authenticated control queue; local games run the same packet
    /// through the app's complete-control executor instead of removing the
    /// player directly.
    pub(crate) fn developer_console_quit_player(
        &mut self,
        player: i32,
        editing: bool,
    ) -> std::result::Result<bool, String> {
        if !editing
            || self.mode != AppMode::Running
            || self.control_playback.is_some()
            || self.engine.player(player).is_none()
        {
            return Ok(false);
        }
        let tick = self.local_control_submission_tick();
        if let Some(network) = self.network.as_ref() {
            if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
                return Ok(false);
            }
            network
                .submit_eliminate_player(tick, player)
                .map_err(|error| error.to_string())?;
        } else {
            self.offline_control_input
                .push(NetworkControl::EliminatePlayer(
                    clonk_engine::EliminatePlayerControlData {
                        player,
                        by_client: 0,
                    },
                ));
        }
        Ok(true)
    }

    /// Host and remote-client rows from `C4Console::UpdateNetMenu`. The local
    /// host is always first; remaining synchronized clients keep client-ID
    /// order and expose their activated/deactivated native captions.
    pub(crate) fn developer_console_net_menu_entries(&self) -> Vec<ConsoleClientRow> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return Vec::new();
        }
        let Some(network) = self.network.as_ref() else {
            return Vec::new();
        };
        let Ok(local_client_id) = i32::try_from(network.local_client_id()) else {
            return Vec::new();
        };
        let clients = self.control_clients.snapshot();
        let mut entries = Vec::with_capacity(clients.len());
        if let Some(host) = clients
            .iter()
            .find(|client| client.client_id == local_client_id)
        {
            let name = legacy_presentation_text(host.name.as_bytes());
            let id = host.client_id.to_string();
            entries.push(ConsoleClientRow {
                id: host.client_id,
                menu_label: format_resource_string(
                    self.runtime_resource_text("IDS_MNU_NETHOST", "Host %s (%i)"),
                    &[&name, &id],
                ),
                // Native leaves every row sensitive and rejects activation
                // in OnNetClient when this process is not control host.
                menu_enabled: true,
            });
        }
        entries.extend(
            clients
                .into_iter()
                .filter(|client| client.client_id != local_client_id)
                .map(|client| {
                    let name = legacy_presentation_text(client.name.as_bytes());
                    let id = client.client_id.to_string();
                    let (key, fallback) = if client.activated {
                        ("IDS_MNU_NETCLIENT", "Client %s (%i)")
                    } else {
                        ("IDS_MNU_NETCLIENTDE", "Client %s (%i) deactivated")
                    };
                    ConsoleClientRow {
                        id: client.client_id,
                        menu_label: format_resource_string(
                            self.runtime_resource_text(key, fallback),
                            &[&name, &id],
                        ),
                        menu_enabled: true,
                    }
                }),
        );
        entries
    }

    /// `C4ClientList::CtrlRemove` for the developer Net menu. Unlike the
    /// in-game menu this is not a league-vote shortcut: native submits the
    /// synchronized ClientRemove directly, but only on the control host.
    pub(crate) fn developer_console_kick_client(
        &mut self,
        client_id: i32,
    ) -> std::result::Result<bool, String> {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_)))
            || !self.engine.is_control_host()
            || !self.control_clients.contains(client_id)
        {
            return Ok(false);
        }
        let Some(network) = self.network.as_ref() else {
            return Ok(false);
        };
        let reason = self.runtime_resource_text("IDS_MSG_KICKBYMENU", "kicked from host menu");
        network
            .submit_client_remove(clonk_engine::ClientRemoveControlData {
                client_id,
                reason: clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(
                    &reason,
                ))
                .unwrap_or_default(),
                by_client: 0,
            })
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub(crate) fn developer_console_runtime_record_possible(&self) -> bool {
        self.mode == AppMode::Running
            && !self.runtime_record_requested
            && self.control_playback.is_none()
            && self.recording.is_none()
    }

    /// `C4GameControl::RequestRuntimeRecord`: disable the item immediately,
    /// then let the next ordinary queued Synchronize start the recorder with
    /// that complete executing control list as its first chunk.
    pub(crate) fn developer_console_request_runtime_record(
        &mut self,
    ) -> std::result::Result<bool, String> {
        if !self.developer_console_runtime_record_possible() {
            return Ok(false);
        }
        self.runtime_record_requested = true;
        let tick = self.local_control_submission_tick();
        let result = if let Some(network) = self.network.as_ref() {
            network
                .submit_queued_synchronize(tick, false, true)
                .map_err(|error| error.to_string())
        } else {
            self.apply_ready_controls(
                tick,
                vec![NetworkControl::Synchronize(
                    clonk_engine::SynchronizeControlData {
                        save_player_files: false,
                        sync_clearance: true,
                        by_client: 0,
                    },
                )],
            )
            .map_err(|error| error.to_string())
        };
        if let Err(error) = result {
            self.runtime_record_requested = false;
            return Err(error);
        }
        Ok(true)
    }

    /// Backend for `C4Console::In`. This is deliberately separate from the
    /// `/console` stdin `C4Application::OnCommand` path: the blocked native
    /// console shell will supply its own active/editing state and widget.
    pub(crate) fn process_developer_console_input(
        &mut self,
        text: &str,
        editing: bool,
    ) -> Result<bool, EngineError> {
        if text.starts_with('/') {
            self.process_developer_console_command(text)?;
            return Ok(true);
        }
        if let Some(message) = text.strip_prefix('#') {
            self.process_console_message_input(message);
            return Ok(true);
        }
        if !editing {
            let message =
                self.runtime_resource_text("IDS_CNS_NONETEDIT", "No editing while replaying.");
            tracing::warn!(%message, "developer console script rejected");
            self.status_text = message;
            return Ok(false);
        }
        let Some(script) =
            clonk_engine::LegacyCString::from_bytes(clonk_script::c4_string_bytes(text))
        else {
            tracing::warn!("developer console script contained an embedded NUL");
            return Ok(true);
        };
        self.submit_or_execute_running_script(clonk_engine::ScriptControlData {
            target_object: clonk_engine::SCRIPT_SCOPE_CONSOLE,
            strictness: self.running_console_script_strictness(),
            script,
            by_client: if self.control_playback.is_some() {
                -1
            } else {
                0
            },
        })?;
        Ok(true)
    }

    pub(crate) fn update_film_viewport_availability(&mut self) {
        self.engine.set_physical_viewport_players(
            self.physical_viewports
                .iter()
                .map(|viewport| viewport.displayed_player),
        );
        self.engine
            .set_film_viewport_available(!self.physical_viewports.is_empty());
    }

    /// `FnSetFilmView`: mutate the first physical viewport in place and keep
    /// both its stable camera identity and `fIsNoOwnerViewport` bit.
    pub(crate) fn set_physical_film_view(&mut self, player: i32) -> bool {
        self.set_physical_view_target(0, player)
    }

    /// Compatibility for focused tests that set the former scalar projection
    /// directly. Production requests always flow through the ordered sink.
    pub(crate) fn apply_direct_film_view_projection(&mut self) {
        if self.physical_viewports_authoritative {
            return;
        }
        let Some(player) = self.film_view_player else {
            return;
        };
        if player != OWNER_NONE && self.engine.player(player).is_none() {
            self.film_view_player = None;
            return;
        }
        if self
            .physical_viewports
            .first()
            .is_some_and(|viewport| viewport.displayed_player != player)
        {
            let _ = self.set_physical_film_view(player);
        }
    }

    pub(crate) fn tick_league_record_stream_at(&self, now: i64) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        let Some(network) = self.network.as_ref() else {
            return;
        };
        if let Err(error) = network.pump_league_record_stream(now) {
            tracing::error!(%error, "failed to queue league record stream pump");
        }
    }

    pub(crate) fn league_record_stream_status(&self) -> LeagueRecordStreamStatus {
        self.network
            .as_ref()
            .and_then(NetworkManager::league_record_stream_status)
            .unwrap_or_default()
    }

    pub(crate) fn record_league_surrender_round_result(&mut self) {
        let message = self.runtime_resource_bytes("IDS_ERR_YOUSURRENDEREDTHELEAGUEGA");
        self.engine.evaluate_network_round_results(
            clonk_engine::RoundResultsNetworkResult::NetworkError,
            Some(message),
        );
        self.snapshot.round_results = self.engine.snapshot().round_results;
    }

    pub(crate) fn recording_player_info_snapshot(&self) -> clonk_network::PlayerInfoListSnapshot {
        let (last_player_id, clients) = self.control_player_infos.retained_rows_snapshot();
        clonk_network::PlayerInfoListSnapshot {
            last_player_id,
            clients: clients
                .into_iter()
                .map(
                    |(client_id, flags, players)| clonk_network::ClientPlayerInfosSnapshot {
                        client_id,
                        flags,
                        players,
                    },
                )
                .collect(),
        }
    }

    fn recording_parameters(
        &self,
        scenario: &FrontendScenario,
        scenario_data: &Scenario,
    ) -> std::result::Result<
        (
            clonk_network::JoinGameParametersEnvelope,
            clonk_network::InitialNetworkScenarioDefaults,
        ),
        String,
    > {
        let scenario_metadata = scenario_data
            .initial_network_scenario_metadata()
            .map_err(|error| error.to_string())?;
        let defaults = clonk_network::initial_network_scenario_defaults(&scenario_metadata)
            .map_err(|error| error.to_string())?;
        let teams = scenario_data
            .initial_network_team_metadata()
            .map(clonk_network::join_team_list_snapshot)
            .map_err(|error| error.to_string())?;
        let empty_players = clonk_network::PlayerInfoListSnapshot {
            last_player_id: 0,
            clients: Vec::new(),
        };
        let legacy_text = |text: &[u8]| {
            LegacyCString::from_bytes(text.iter().copied().take_while(|byte| *byte != 0).collect())
                .unwrap_or_default()
        };
        let mut parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .unwrap_or_else(|| clonk_network::JoinGameParametersEnvelope {
                random_seed: (self.engine.random_seed() as u32) as i32,
                startup_player_count: 0,
                max_players: self.engine.max_players().unwrap_or(defaults.max_players),
                use_fair_crew: self.engine.use_fair_crew(),
                fair_crew_forced: self.engine.fair_crew_forced(),
                fair_crew_strength: self.engine.fair_crew_strength(),
                allow_debug: self.engine.allow_debug(),
                is_network_game: self.network.is_some(),
                control_rate: self.engine.control_rate(),
                auto_frame_skip: self.auto_frame_skip,
                rules: defaults.rules.clone(),
                goals: defaults.goals.clone(),
                league: legacy_text(&self.network_league_name),
                league_address: LegacyCString::default(),
                title: legacy_text(scenario.title.as_bytes()),
                scenario: clonk_engine::NetworkResourceCore::default(),
                game_resources: Vec::new(),
                player_infos: empty_players.clone(),
                restore_player_infos: empty_players,
                teams,
                clients: clonk_network::JoinClientRegistrySnapshot::new(Vec::new()),
            });
        parameters.random_seed = (self.engine.random_seed() as u32) as i32;
        parameters.startup_player_count = self.engine.startup_player_count().unwrap_or_else(|| {
            i32::try_from(self.control_player_infos.nonremoved_player_count()).unwrap_or(i32::MAX)
        });
        parameters.max_players = self.engine.max_players().unwrap_or(defaults.max_players);
        parameters.use_fair_crew = self.engine.use_fair_crew();
        parameters.fair_crew_forced = self.engine.fair_crew_forced();
        parameters.fair_crew_strength = self.engine.fair_crew_strength();
        parameters.allow_debug = self.engine.allow_debug();
        parameters.is_network_game = self.network.is_some();
        parameters.control_rate = self.engine.control_rate();
        parameters.auto_frame_skip = self.auto_frame_skip;
        parameters.player_infos = self.recording_player_info_snapshot();
        parameters.clients =
            clonk_network::JoinClientRegistrySnapshot::new(self.control_clients.snapshot());
        Ok((parameters, defaults))
    }

    pub(crate) fn prepare_recording_for(
        &mut self,
        scenario: &FrontendScenario,
        scenario_data: &Scenario,
        initial_source: Option<InitialRecordingSource<'_>>,
        retained_definition_modules: Option<&[String]>,
        retained_definition_save_paths: Option<(&str, &str)>,
    ) -> std::result::Result<(), String> {
        self.runtime_record_requested = false;
        self.live_save_seed = None;
        self.recording_template = None;
        let Some(scenario_path) = scenario.path.as_deref() else {
            return if self.recordings_dir.is_none() {
                Ok(())
            } else {
                Err("recording requires a filesystem-backed scenario".to_string())
            };
        };
        let definition_modules =
            recording_definition_modules(scenario_data, retained_definition_modules);
        let description_definition_modules =
            recording_description_definition_modules(scenario_data, retained_definition_modules);
        let (definition_executable_path, definition_path) = retained_definition_save_paths
            .map_or_else(
                || {
                    let native_config = load_native_config_bytes(self.app_paths.as_ref());
                    game_save_definition_paths(self.app_paths.as_ref(), &native_config)
                },
                |(executable, definitions)| (executable.to_owned(), definitions.to_owned()),
            );
        let scenario_origin =
            record_scenario_origin(scenario_path, self.app_paths.as_ref(), &scenario.identifier);
        let (recording_parameters, scenario_defaults) =
            self.recording_parameters(scenario, scenario_data)?;
        let runtime_seed = RuntimeRecordingSeed {
            scenario_path: scenario_path.to_path_buf(),
            scenario_source_path: scenario_path.to_path_buf(),
            scenario_identifier: scenario.identifier.clone(),
            scenario_title: recording_parameters.title.clone(),
            definition_modules: definition_modules.clone(),
            description_definition_modules: description_definition_modules.clone(),
            definition_executable_path: definition_executable_path.clone(),
            definition_path: definition_path.clone(),
            scenario_origin: scenario_origin.clone(),
            parameters: recording_parameters.clone(),
            scenario_defaults: scenario_defaults.clone(),
        };
        self.live_save_seed = Some(runtime_seed.clone());
        // Runtime recording still needs the seed above. The initial SaveData
        // projection and its pointer denumeration, however, exist only when
        // Config.General.Record (or league recording) actually starts a
        // C4Record from InitControl.
        let Some(initial_source) = initial_source else {
            return Ok(());
        };
        let reconstruct_loaded_runtime =
            matches!(initial_source, InitialRecordingSource::Loaded { .. });
        let Some(dir) = self.recordings_dir.as_ref() else {
            return Ok(());
        };
        self.prepare_recording_root(dir)
            .map_err(|error| error.to_string())?;
        let index = next_recording_index(dir).map_err(|error| error.to_string())?;
        let raw_base_name = scenario_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(scenario.identifier.as_str());
        let output_path = dir.join(format!(
            "{index:03}-{}.c4s",
            sanitize_record_name(raw_base_name)
        ));
        let synchronized_title = native_bytes_as_legacy_text(recording_parameters.title.as_bytes());
        let mut record_title = clonk_script::c4_string_bytes(&format!(
            "{index:03} {synchronized_title} [{CLASSIC_ENGINE_BUILD}]"
        ));
        record_title.truncate(512);
        let record_title = clonk_script::c4_string_from_bytes(&record_title);
        let source =
            open_group_path_for_folder_map(scenario_path).map_err(|error| error.to_string())?;
        let mut group = MutableGroup::from_group(&source).map_err(|error| error.to_string())?;
        let record_maker = self.process_group_maker.as_bytes().to_vec();
        let parameters = match clonk_network::serialize_initial_network_parameters(
            &recording_parameters,
            &scenario_defaults,
        ) {
            Ok(parameters) => parameters,
            Err(error) => {
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    error.to_string(),
                ));
            }
        };
        if let Err(error) = group.add_file("Parameters.txt", parameters.clone()) {
            return Err(partial_recording_failure(
                &group,
                &output_path,
                &record_maker,
                format!("write initial record Parameters.txt: {error}"),
            ));
        }
        let scenario_core = if reconstruct_loaded_runtime {
            self.engine
                .serialize_initial_record_scenario_from_runtime_savegame(
                    &record_title,
                    &definition_modules,
                    &definition_executable_path,
                    &definition_path,
                    &scenario_origin,
                )
        } else {
            match scenario_data.serialize_initial_record_scenario(
                &record_title,
                &definition_modules,
                &definition_executable_path,
                &definition_path,
                &scenario_origin,
            ) {
                Ok(scenario_core) => scenario_core,
                Err(error) => {
                    return Err(partial_recording_failure(
                        &group,
                        &output_path,
                        &record_maker,
                        error.to_string(),
                    ));
                }
            }
        };
        let copied_material_group_is_file = matches!(
            group.entry_kind("Material.c4g"),
            Some(MutableGroupEntryKind::File | MutableGroupEntryKind::UnopenableChildGroup)
        );
        let reconstructed_save = if let InitialRecordingSource::Loaded { music_enabled, .. } =
            initial_source
        {
            // Native initial recording copies the currently loaded savegame
            // group, whose exact runtime components already exist. Rust's
            // JSON save points back to the original scenario, so reconstruct
            // that copied image from the restored pre-Recreate state first.
            // SaveData's enumeration and pointer-denumeration side effects
            // deliberately remain live, just as they do in C++.
            let landscape_is_static = self
                .engine
                .landscape()
                .is_some_and(|landscape| landscape.mode() == clonk_engine::LANDSCAPE_MODE_STATIC);
            let reconstruction_target = output_path.to_string_lossy();
            let save = match self.engine.serialize_live_c4_save_with_policy(
                clonk_engine::LiveC4SaveSpec {
                    title: &record_title,
                    definition_modules: &definition_modules,
                    definition_executable_path: &definition_executable_path,
                    definition_path: &definition_path,
                    origin: &scenario_origin,
                    music_enabled,
                    copied_material_group_is_file,
                    title_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                    info_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                    script_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                },
                clonk_engine::LiveC4SavePolicy::Savegame {
                    target_group_name: &reconstruction_target,
                },
            ) {
                Ok(save) => save,
                Err(error) => {
                    let mut failure =
                        format!("reconstruct loaded save for initial record: {error}");
                    if let Some(partial) = error.pre_landscape_components() {
                        let policy = clonk_engine::LiveC4SavePolicy::Savegame {
                            target_group_name: &reconstruction_target,
                        };
                        if let Err(apply_error) =
                            developer_console_save::apply_live_save_pre_landscape_to_group(
                                &mut group, policy, partial,
                            )
                        {
                            failure.push_str(&format!(
                                "; additionally failed to apply partial loaded-save reconstruction: {apply_error}"
                            ));
                        }
                    }
                    return Err(partial_recording_failure(
                        &group,
                        &output_path,
                        &record_maker,
                        failure,
                    ));
                }
            };
            Some((save, landscape_is_static))
        } else {
            None
        };

        if let Some((save, landscape_is_static)) = reconstructed_save.as_ref() {
            let reconstruction_target = output_path.to_string_lossy();
            let reconstruction_policy = clonk_engine::LiveC4SavePolicy::Savegame {
                target_group_name: &reconstruction_target,
            };
            let reconstruction_result = (|| -> std::result::Result<(), String> {
                developer_console_save::apply_live_save_runtime_components_to_group(
                    &mut group,
                    reconstruction_policy,
                    save,
                    *landscape_is_static,
                )
                .map_err(|error| error.to_string())?;

                // C4PlayerInfoList::Save deletes the old component before it
                // compiles the replacement. Preserve that deletion if the
                // legacy compiler rejects the fallback roster.
                group.remove_entry("SavePlayerInfos.txt");
                let persisted_restore_infos = match initial_source {
                    InitialRecordingSource::Loaded {
                        source_save_player_infos,
                        ..
                    } => source_save_player_infos,
                    InitialRecordingSource::Fresh(_) => None,
                };
                let restore_infos = if let Some(bytes) = persisted_restore_infos {
                    bytes.to_vec()
                } else {
                    // Backward compatibility for JSON saves written before
                    // the source component was retained.
                    let restore_plan = runtime_join_save::set_as_live_save_restore_infos(
                        &recording_parameters.clients.clients,
                        &recording_parameters.player_infos,
                        self.network.is_some(),
                        reconstruction_policy.player_policy(),
                    );
                    if restore_plan.restore_infos.clients.is_empty() {
                        Vec::new()
                    } else {
                        clonk_network::encode_player_info_list_ini(&restore_plan.restore_infos)
                            .map_err(|error| error.to_string())?
                    }
                };
                if !restore_infos.is_empty() {
                    developer_console_save::apply_live_save_player_infos_to_group(
                        &mut group,
                        &restore_infos,
                    )
                    .map_err(|error| error.to_string())?;
                }
                Ok(())
            })();
            if let Err(error) = reconstruction_result {
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    format!("apply loaded-save reconstruction: {error}"),
                ));
            }
            if self.engine.frame() != 0 {
                let source_title_png = match initial_source {
                    InitialRecordingSource::Loaded {
                        source_title_png, ..
                    } => source_title_png,
                    InitialRecordingSource::Fresh(_) => None,
                };
                if let Some(source_title_png) = source_title_png {
                    if let Err(error) = group.add_file("Title.png", source_title_png.to_vec()) {
                        tracing::warn!(%error, "failed to install loaded savegame title image");
                    }
                }
            }
        }

        // C4GameSaveRecord::SaveCore writes Scenario after Parameters, then
        // the base Save method removes stale player/title components before
        // C4Game::SaveData writes Game.txt.
        if let Err(error) = group.add_file("Scenario.txt", scenario_core.clone()) {
            return Err(partial_recording_failure(
                &group,
                &output_path,
                &record_maker,
                format!("write initial record Scenario.txt: {error}"),
            ));
        }
        clean_initial_record_group(&mut group);

        let loaded_initial_game_data;
        let initial_game_data = match initial_source {
            InitialRecordingSource::Fresh(game_data) => game_data,
            InitialRecordingSource::Loaded { music_enabled, .. } => {
                // InitControl creates fInitial only after the copied savegame
                // source has run EnumStrings and pointer enumeration.
                loaded_initial_game_data =
                    match self.engine.capture_initial_record_game_data(music_enabled) {
                        Ok(game_data) => game_data,
                        Err(error) => {
                            return Err(partial_recording_failure(
                                &group,
                                &output_path,
                                &record_maker,
                                error.to_string(),
                            ));
                        }
                    };
                &loaded_initial_game_data
            }
        };
        let original_game = source.read_file("Game.txt").ok();
        let original_game = reconstructed_save
            .as_ref()
            .map(|(save, _)| save.game_txt.as_slice())
            .or(original_game.as_deref());
        let game =
            match clonk_engine::serialize_initial_network_game(initial_game_data, original_game) {
                Ok(game) => game,
                Err(error) => {
                    return Err(partial_recording_failure(
                        &group,
                        &output_path,
                        &record_maker,
                        error.to_string(),
                    ));
                }
            };
        if let Some(game) = game.as_ref() {
            if let Err(error) = group.add_file("Game.txt", game.clone()) {
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    format!("write initial record Game.txt: {error}"),
                ));
            }
        } else {
            group.remove_entry("Game.txt");
        }
        let player_info_snapshot = self.recording_player_info_snapshot();
        let player_infos = if player_info_snapshot.clients.is_empty() {
            Ok(None)
        } else {
            clonk_network::encode_player_info_list_ini(&player_info_snapshot).map(Some)
        };

        // SaveComponents runs after the initial core/Game writes. It ignores
        // both compiler and group-add failure, but always deletes a copied
        // PlayerInfos.txt before installing the current roster.
        group.remove_entry("PlayerInfos.txt");
        match player_infos.as_ref() {
            Ok(Some(player_infos)) => {
                if let Err(error) = group.add_file("PlayerInfos.txt", player_infos.clone()) {
                    tracing::warn!(%error, "failed to install initial record player infos");
                }
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "failed to serialize initial record player infos");
            }
        }

        let initial_stream_chunk_result = (|| -> std::result::Result<Vec<u8>, String> {
            if !self
                .network
                .as_ref()
                .is_some_and(NetworkManager::league_record_stream_available)
            {
                return Ok(Vec::new());
            }
            // C4Record::StartStreaming saves a second, no-copy record group
            // and inserts that packed image as the leading RCT_File. The
            // original scenario is recovered from Scenario.Head.Origin.
            let stream_group_name = output_path
                .file_name()
                .map(|name| path_to_legacy_bytes(Path::new(name)))
                .unwrap_or_else(|| b"Record.c4s".to_vec());
            let mut stream_initial_group = MutableGroup::new_bytes(stream_group_name);
            if !self.process_group_maker.as_bytes().is_empty() {
                stream_initial_group.set_maker_bytes(self.process_group_maker.as_bytes());
            }
            stream_initial_group
                .add_file("Parameters.txt", parameters.clone())
                .map_err(|error| error.to_string())?;
            stream_initial_group
                .add_file("Scenario.txt", scenario_core.clone())
                .map_err(|error| error.to_string())?;
            if let Some(game) = game.as_ref() {
                stream_initial_group
                    .add_file("Game.txt", game.clone())
                    .map_err(|error| error.to_string())?;
            }
            match player_infos.as_ref() {
                Ok(Some(player_infos)) => {
                    if let Err(error) =
                        stream_initial_group.add_file("PlayerInfos.txt", player_infos.clone())
                    {
                        tracing::warn!(%error, "failed to install initial streamed player infos");
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    // C4GameSaveRecord::SaveComponents ignores the result of
                    // C4PlayerInfoList::Save for an initial recording.
                    tracing::warn!(%error, "failed to serialize initial streamed player infos");
                }
            }
            let stream_initial_file = stream_initial_group
                .pack()
                .map_err(|error| error.to_string())?;
            let stream_record_name = league_record_name(&output_path)
                .ok_or_else(|| "record stream filename contains an interior NUL".to_string())?;
            clonk_network::encode_league_stream_file_chunk(
                &stream_record_name,
                &stream_initial_file,
            )
            .map_err(|error| error.to_string())
        })();
        let initial_stream_chunk = match initial_stream_chunk_result {
            Ok(chunk) => chunk,
            Err(error) => {
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    format!("prepare initial record stream: {error}"),
                ));
            }
        };

        // A source replay may already contain a stream. A fresh record owns
        // a fresh binary CtrlRec, while native leaves copied CtrlRec.txt and
        // RecPlayerInfos.txt alone until playback/Stop handles them.
        group.remove_entry("CtrlRec.c4b");
        self.recording_template = Some(RecordingTemplate {
            group,
            output_path,
            initial_stream_chunk,
            runtime_seed: Some(runtime_seed),
            description_title: recording_parameters.title.as_bytes().to_vec(),
            description_definition_modules,
        });
        Ok(())
    }

    /// Build the non-initial `C4GameSaveRecord` image at the exact
    /// `C4Game::Synchronize` boundary that consumes `fRecordNeeded`.
    pub(crate) fn prepare_runtime_recording_at_synchronize(
        &mut self,
    ) -> std::result::Result<(), String> {
        let Some(seed) = self.live_save_seed.clone().or_else(|| {
            self.recording_template
                .as_ref()
                .and_then(|template| template.runtime_seed.clone())
        }) else {
            // State-only tests and embedders may install an already-composed
            // template. Production templates always retain this seed.
            return Ok(());
        };
        let dir = self
            .recordings_dir
            .as_ref()
            .ok_or_else(|| "runtime recording has no record directory".to_string())?;
        self.prepare_recording_root(dir)
            .map_err(|error| error.to_string())?;
        let index = next_recording_index(dir).map_err(|error| error.to_string())?;
        let raw_base_name = seed
            .scenario_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(&seed.scenario_identifier);
        let output_path = dir.join(format!(
            "{index:03}-{}.c4s",
            sanitize_record_name(raw_base_name)
        ));

        let scenario_title = native_bytes_as_legacy_text(seed.scenario_title.as_bytes());
        let mut record_title = clonk_script::c4_string_bytes(&format!(
            "{index:03} {scenario_title} [{CLASSIC_ENGINE_BUILD}]"
        ));
        record_title.truncate(512);
        let record_title = clonk_script::c4_string_from_bytes(&record_title);
        let source = open_group_path_for_folder_map(&seed.scenario_source_path)
            .map_err(|error| error.to_string())?;
        let mut group = MutableGroup::from_group(&source).map_err(|error| error.to_string())?;
        let record_maker = self.process_group_maker.as_bytes().to_vec();
        let mut parameters = seed.parameters;
        parameters.random_seed = (self.engine.random_seed() as u32) as i32;
        parameters.startup_player_count = self.engine.startup_player_count().unwrap_or_else(|| {
            i32::try_from(self.control_player_infos.nonremoved_player_count()).unwrap_or(i32::MAX)
        });
        parameters.max_players = self
            .engine
            .max_players()
            .unwrap_or(seed.scenario_defaults.max_players);
        parameters.use_fair_crew = self.engine.use_fair_crew();
        parameters.fair_crew_forced = self.engine.fair_crew_forced();
        parameters.fair_crew_strength = self.engine.fair_crew_strength();
        parameters.allow_debug = self.engine.allow_debug();
        parameters.is_network_game = self.network.is_some();
        parameters.control_rate = self.engine.control_rate();
        parameters.auto_frame_skip = self.auto_frame_skip;
        parameters.player_infos = self.recording_player_info_snapshot();
        parameters.clients =
            clonk_network::JoinClientRegistrySnapshot::new(self.control_clients.snapshot());
        let restore_plan = runtime_join_save::set_as_live_save_restore_infos(
            &parameters.clients.clients,
            &parameters.player_infos,
            self.network.is_some(),
            clonk_engine::LiveC4SavePolicy::Record.player_policy(),
        );
        restore_plan
            .validate_for_live_save(
                clonk_engine::LiveC4SavePolicy::Record,
                self.engine.players().map(|player| player.player_info_id()),
            )
            .map_err(|error| error.to_string())?;
        let parameter_bytes = match clonk_network::serialize_initial_network_parameters(
            &parameters,
            &seed.scenario_defaults,
        ) {
            Ok(bytes) => bytes,
            Err(error) => {
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    error.to_string(),
                ));
            }
        };
        if let Err(error) = group.add_file("Parameters.txt", parameter_bytes) {
            return Err(partial_recording_failure(
                &group,
                &output_path,
                &record_maker,
                format!("write runtime record Parameters.txt: {error}"),
            ));
        }

        let landscape_is_static = self
            .engine
            .landscape()
            .is_some_and(|landscape| landscape.mode() == clonk_engine::LANDSCAPE_MODE_STATIC);
        let copied_material_group_is_file = matches!(
            group.entry_kind("Material.c4g"),
            Some(MutableGroupEntryKind::File | MutableGroupEntryKind::UnopenableChildGroup)
        );
        let save = match self.engine.serialize_live_c4_save_with_policy(
            clonk_engine::LiveC4SaveSpec {
                title: &record_title,
                definition_modules: &seed.definition_modules,
                definition_executable_path: &seed.definition_executable_path,
                definition_path: &seed.definition_path,
                origin: &seed.scenario_origin,
                music_enabled: self.runtime_music_enabled,
                copied_material_group_is_file,
                title_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                info_component: clonk_engine::LiveC4ComponentHost::Unmodified,
                script_component: clonk_engine::LiveC4ComponentHost::Unmodified,
            },
            clonk_engine::LiveC4SavePolicy::Record,
        ) {
            Ok(save) => save,
            Err(error) => {
                let mut failure = format!("serialize runtime record: {error}");
                if let Some(partial) = error.pre_landscape_components() {
                    if let Err(apply_error) =
                        developer_console_save::apply_live_save_pre_landscape_to_group(
                            &mut group,
                            clonk_engine::LiveC4SavePolicy::Record,
                            partial,
                        )
                    {
                        failure.push_str(&format!(
                            "; additionally failed to apply partial runtime record: {apply_error}"
                        ));
                    }
                }
                return Err(partial_recording_failure(
                    &group,
                    &output_path,
                    &record_maker,
                    failure,
                ));
            }
        };

        let mutation_result = (|| -> std::result::Result<(), String> {
            developer_console_save::apply_live_save_runtime_components_to_group(
                &mut group,
                clonk_engine::LiveC4SavePolicy::Record,
                &save,
                landscape_is_static,
            )
            .map_err(|error| error.to_string())?;

            // SaveRuntimeData writes SavePlayerInfos before it begins walking
            // live players. Keep both the delete-before-compile behavior and
            // every already-added player child visible on a later failure.
            group.remove_entry("SavePlayerInfos.txt");
            if !restore_plan.restore_infos.clients.is_empty() {
                let restore_info_bytes =
                    clonk_network::encode_player_info_list_ini(&restore_plan.restore_infos)
                        .map_err(|error| error.to_string())?;
                developer_console_save::apply_live_save_player_infos_to_group(
                    &mut group,
                    &restore_info_bytes,
                )
                .map_err(|error| error.to_string())?;
            }

            let (add_new_crew_portraits, save_default_portraits, player_rank_name_default) =
                self.developer_console_player_save_options();
            let player_save_options = clonk_engine::LiveC4PlayerSaveOptions {
                savegame: true,
                store_tiny: false,
                add_new_crew_portraits,
                save_default_portraits,
                player_rank_name_default: &player_rank_name_default,
            };
            let runtime_players = self
                .engine
                .players()
                .map(|player| (player.id(), player.player_info_id()))
                .collect::<Vec<_>>();
            let mut remaining_targets = restore_plan.player_groups;
            for (game_number, player_info_id) in runtime_players {
                let Some(target_index) = remaining_targets
                    .iter()
                    .position(|target| target.player_info_id == player_info_id)
                else {
                    continue;
                };
                let target = remaining_targets.remove(target_index);
                let player_group =
                    clonk_engine::serialize_live_c4_player_with_options_and_enumeration(
                        &self.engine,
                        game_number,
                        target.filename.as_bytes(),
                        &record_maker,
                        player_save_options,
                        &save.value_enumeration,
                    )
                    .map_err(|error| {
                        format!(
                            "serialize runtime record player info {} (game player {}): {error}",
                            target.player_info_id, game_number
                        )
                    })?;
                developer_console_save::add_live_save_player_group(
                    &mut group,
                    runtime_join_save::SerializedRuntimeJoinPlayerGroup {
                        filename: target.filename,
                        group: player_group,
                    },
                )
                .map_err(|error| error.to_string())?;
            }
            // C4PlayerList::Save ignores stale restore rows without a live
            // player. No later player can roll back an earlier child.
            Ok(())
        })();
        if let Err(error) = mutation_result {
            return Err(partial_recording_failure(
                &group,
                &output_path,
                &record_maker,
                error,
            ));
        }
        group.remove_entry("CtrlRec.c4b");
        self.recording_template = Some(RecordingTemplate {
            group,
            output_path,
            initial_stream_chunk: Vec::new(),
            runtime_seed: None,
            description_title: seed.scenario_title.as_bytes().to_vec(),
            description_definition_modules: seed.description_definition_modules,
        });
        Ok(())
    }

    pub(crate) fn start_recording(&mut self, force: bool) -> std::result::Result<bool, String> {
        self.engine.set_recording_active(false);
        if !force && !self.recording_enabled {
            self.recording = None;
            return Ok(false);
        }
        let Some(mut template) = self.recording_template.take() else {
            self.recording = None;
            return Err("recording storage was not prepared".to_string());
        };
        // C++ creates and unpacks the record group before it opens CtrlRec.
        // Persist the initial group now so a league start cannot succeed with
        // an unwritable destination and a crash still leaves the initial save.
        if !self.process_group_maker.as_bytes().is_empty() {
            template
                .group
                .set_maker_bytes_recursively(self.process_group_maker.as_bytes());
        }
        let initial_group = template.group.pack().map_err(|error| error.to_string())?;
        replace_file_from_same_directory(&template.output_path, &initial_group).map_err(
            |error| {
                format!(
                    "failed to create {}: {error}",
                    template.output_path.display()
                )
            },
        )?;
        let packed = Group::open(&template.output_path).map_err(|error| error.to_string())?;
        unpack_recording_group(&packed, &template.output_path)
            .map_err(|error| format!("failed to unpack record group: {error}"))?;
        let ctrl_rec_path = template.output_path.join("CtrlRec.c4b");
        let ctrl_rec = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&ctrl_rec_path)
            .map_err(|error| format!("failed to create {}: {error}", ctrl_rec_path.display()))?;
        // Only initial league records call C4GameControl::StartRecord with
        // streaming enabled. FileRecord's non-initial StartRecord(false,
        // false) remains a local record even during a league session.
        let league_streaming = !template.initial_stream_chunk.is_empty()
            && self
                .network
                .as_ref()
                .is_some_and(NetworkManager::league_record_stream_available);
        if league_streaming {
            let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
            let network = self
                .network
                .as_ref()
                .expect("stream availability requires a network manager");
            network
                .start_league_record_stream(now)
                .map_err(|error| error.to_string())?;
            network
                .append_league_record_bytes(&template.initial_stream_chunk)
                .map_err(|error| error.to_string())?;
        }
        self.recording = Some(RecordingSession::new(template, league_streaming, ctrl_rec));
        self.engine.set_recording_active(true);
        Ok(true)
    }

    pub(crate) fn record_control_packet(&mut self, packet: &clonk_engine::ControlPacket) {
        self.record_control_resource_file(packet);
        let frame = u32::try_from(self.engine.frame()).unwrap_or(u32::MAX);
        let stream_delta = if let Some(session) = self.recording.as_mut() {
            if let Err(error) = session.writer.record_packet(frame, packet) {
                tracing::warn!(%error, "failed to append immediate CtrlRec packet");
                None
            } else {
                if let Err(error) = session.flush_control_delta() {
                    tracing::warn!(%error, "failed to flush immediate CtrlRec packet");
                }
                session.take_stream_delta()
            }
        } else {
            None
        };
        self.append_league_record_stream_bytes(stream_delta);
    }

    pub(crate) fn record_control_batch(&mut self, packets: &[clonk_engine::ControlPacket]) {
        for packet in packets {
            self.record_control_resource_file(packet);
        }
        let frame = u32::try_from(self.engine.frame()).unwrap_or(u32::MAX);
        let stream_delta = if let Some(session) = self.recording.as_mut() {
            if let Err(error) = session.writer.record_controls(frame, packets) {
                tracing::warn!(%error, "failed to append CtrlRec control list");
                None
            } else {
                if let Err(error) = session.flush_control_delta() {
                    tracing::warn!(%error, "failed to flush CtrlRec control list");
                }
                session.take_stream_delta()
            }
        } else {
            None
        };
        self.append_league_record_stream_bytes(stream_delta);
    }

    fn append_league_record_stream_bytes(&self, bytes: Option<Vec<u8>>) {
        let (Some(bytes), Some(network)) = (bytes, self.network.as_ref()) else {
            return;
        };
        if let Err(error) = network.append_league_record_bytes(&bytes) {
            tracing::error!(%error, "failed to queue league record bytes");
        }
    }

    /// `C4ControlJoinPlayer::PreRec` copies resource-backed player groups into
    /// the record as `<resource-id>-<basename>` before serializing the control.
    /// Embedded player data remains entirely inside the control payload.
    fn record_control_resource_file(&mut self, packet: &clonk_engine::ControlPacket) {
        let clonk_engine::ControlPacket::JoinPlayer(clonk_engine::JoinPlayerControlData {
            source: clonk_engine::JoinPlayerSource::Resource(core),
            ..
        }) = packet
        else {
            return;
        };
        let Some(path) = self
            .admission_resources
            .complete_path(core.id)
            .map(Path::to_path_buf)
        else {
            return;
        };
        self.record_player_group_file(&path, recorded_player_resource_name(core), None);
    }

    /// `C4PlayerInfoList::RecreatePlayers` records directly recreated player
    /// groups separately because no `JoinPlayer` control exists to run
    /// `C4ControlJoinPlayer::PreRec` (C4PlayerInfo.cpp:1594-1598).
    pub(crate) fn record_recreated_player_file(&mut self, info_id: i32, path: &Path) {
        self.record_recreated_player_file_with_fallback(info_id, path, None);
    }

    /// Capture bytes staged before `RecreatePlayers` joins its source. This is
    /// the local-record equivalent of `C4Record::AddFile` copying the source
    /// before `C4PlayerList::Join` can reject a malformed profile
    /// (C4PlayerInfo.cpp:1594-1603).
    pub(crate) fn record_recreated_player_file_with_fallback(
        &mut self,
        info_id: i32,
        path: &Path,
        prejoin_bytes: Option<&[u8]>,
    ) {
        self.record_player_group_file(
            path,
            format!("Recreate-{info_id}.c4p").into_bytes(),
            prejoin_bytes,
        );
    }

    fn record_player_group_file(
        &mut self,
        path: &Path,
        target: Vec<u8>,
        prejoin_bytes: Option<&[u8]>,
    ) {
        let Some(league_streaming) = self
            .recording
            .as_ref()
            .map(|session| session.league_streaming)
        else {
            return;
        };
        let prepared = match (league_streaming, prejoin_bytes) {
            (false, Some(bytes)) => Ok((bytes.to_vec(), None)),
            (false, None) => match open_group_path_for_folder_map(path) {
                Ok(_group) => (|| {
                    let local_file = packed_group_bytes(path, self.process_group_maker.as_bytes())?;
                    Ok((local_file, None))
                })(),
                Err(_error) if path.is_file() => {
                    // C4Record::AddFile copies a local non-streaming source
                    // as-is; Players.Join then reports a malformed/non-group
                    // player file (C4PlayerInfo.cpp:1594-1603).
                    packed_group_bytes(path, self.process_group_maker.as_bytes())
                        .map(|local_file| (local_file, None))
                }
                Err(error) => Err(error.to_string()),
            },
            (true, _) => match open_group_path_for_folder_map(path) {
                Ok(group) => (|| {
                    let local_file = packed_group_bytes(path, self.process_group_maker.as_bytes())?;
                    let packed = if has_player_group_extension(&target) {
                        self.pack_stripped_stream_player(&group, &target)?
                    } else {
                        local_file.clone()
                    };
                    let stream_name =
                        LegacyCString::from_bytes(target.clone()).ok_or_else(|| {
                            "streamed player filename contains an interior NUL".to_string()
                        })?;
                    let stream_chunk = Some(
                        clonk_network::encode_league_stream_file_chunk(&stream_name, &packed)
                            .map_err(|error| error.to_string())?,
                    );
                    Ok((local_file, stream_chunk))
                })(),
                Err(error) => Err(error.to_string()),
            },
        };
        let (local_file, stream_chunk) = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                tracing::warn!(path = %path.display(), ?target, %error, "failed to prepare player group for record");
                return;
            }
        };
        // C4Record::AddFile streams first. A stream failure prevents the local
        // add, while a later local-disk failure cannot retract streamed data.
        if let Some(stream_chunk) = stream_chunk {
            let Some(network) = self.network.as_ref() else {
                tracing::warn!(
                    ?target,
                    "league record stream disappeared before player-group append"
                );
                return;
            };
            if let Err(error) = network.append_league_record_bytes(&stream_chunk) {
                tracing::warn!(?target, %error, "failed to stream player group into record");
                return;
            }
        }
        let Some(session) = self.recording.as_mut() else {
            return;
        };
        if let Err(error) = write_folder_save_entry(&session.output_path, &target, &local_file) {
            tracing::warn!(path = %path.display(), ?target, %error, "failed to copy player group into record");
        }
    }

    pub(crate) fn replay_record_player_group(
        &self,
        core: &clonk_engine::NetworkResourceCore,
    ) -> std::result::Result<Group, String> {
        let record_path = self
            .active_scenario
            .as_ref()
            .and_then(|scenario| scenario.path.as_deref())
            .ok_or_else(|| "active replay has no record-group path".to_string())?;
        let target = recorded_player_resource_name(core);
        let record =
            open_group_path_for_folder_map(record_path).map_err(|error| error.to_string())?;
        record
            .open_child(path_from_group_name_bytes(&target))
            .map_err(|error| error.to_string())
    }

    pub(crate) fn replay_record_player_file(
        &self,
        core: &clonk_engine::NetworkResourceCore,
    ) -> std::result::Result<PlayerFile, String> {
        let target = recorded_player_resource_name(core);
        let player_group = self.replay_record_player_group(core)?;
        let bytes = match player_group.raw_image() {
            Ok(bytes) => clonk_resources::compress_c4group_image(&bytes)
                .map_err(|error| error.to_string())?,
            Err(_) if player_group.is_directory() => MutableGroup::from_group(&player_group)
                .map_err(|error| error.to_string())?
                .pack()
                .map_err(|error| error.to_string())?,
            Err(error) => return Err(error.to_string()),
        };
        PlayerFile::load_from_bytes(path_from_group_name_bytes(&target), bytes)
            .map_err(|error| error.to_string())
    }

    pub(crate) fn finish_recording(&mut self) -> Option<LeagueEndRecord> {
        self.runtime_record_requested = false;
        self.engine.set_recording_active(false);
        self.recording.as_ref()?;
        let (league_streaming, stream_delta) = {
            let session = self.recording.as_mut().expect("recording checked above");
            (session.league_streaming, session.take_stream_delta())
        };
        self.append_league_record_stream_bytes(stream_delta);
        if league_streaming {
            let now = i64::try_from(current_unix_timestamp()).unwrap_or(i64::MAX);
            if let Some(network) = self.network.as_ref() {
                if let Err(error) = network.finish_league_record_stream(now) {
                    tracing::error!(%error, "failed to finish league record stream");
                }
            }
        }
        let (description_title, description_definition_modules) = self
            .recording
            .as_ref()
            .map(|session| {
                (
                    session.description_title.clone(),
                    session.description_definition_modules.clone(),
                )
            })
            .expect("recording checked above");
        let (description_name, description) = self.classic_save_description(
            &description_title,
            &description_definition_modules,
            ClassicSaveDescriptionKind::Record,
        );
        let final_player_info_snapshot = self.recording_player_info_snapshot();
        let session = self.recording.take().expect("recording checked above");
        let RecordingSession {
            writer,
            mut ctrl_rec,
            output_path,
            disk_writer_pos,
            ..
        } = session;
        if let Err(error) = write_folder_save_entry(&output_path, &description_name, &description) {
            // C4Record::Stop deliberately ignores SaveDesc's return value.
            tracing::warn!(%error, "failed to install final record description");
        }
        // C4PlayerInfoList::Save deletes the prior entry before it tests for
        // an empty list or invokes the compiler. A compiler failure must not
        // leave stale copied final-player data behind.
        if let Err(error) = delete_folder_save_entry(&output_path, b"RecPlayerInfos.txt") {
            tracing::warn!(%error, "failed to remove stale final player infos");
        }
        if !final_player_info_snapshot.clients.is_empty() {
            match clonk_network::encode_player_info_list_ini(&final_player_info_snapshot) {
                Ok(final_player_infos) => {
                    if let Err(error) = write_folder_save_entry(
                        &output_path,
                        b"RecPlayerInfos.txt",
                        &final_player_infos,
                    ) {
                        tracing::warn!(%error, "failed to install final player infos in record group");
                    }
                }
                Err(error) => {
                    // Native ignores this failure and still closes/packs the
                    // recording directory.
                    tracing::warn!(%error, "failed to serialize final record player infos");
                }
            }
        }
        let stream = writer.finish(u32::try_from(self.engine.frame()).unwrap_or(u32::MAX));
        if let Err(error) = ctrl_rec
            .write_all(&stream[disk_writer_pos.min(stream.len())..])
            .and_then(|()| ctrl_rec.flush())
        {
            tracing::warn!(%error, "failed to append final CtrlRec marker");
        }
        drop(ctrl_rec);
        let group = match Group::open(&output_path) {
            Ok(group) => group,
            Err(error) => {
                tracing::warn!(%error, "failed to reopen recording directory");
                return None;
            }
        };
        let mut group = match MutableGroup::from_group(&group) {
            Ok(group) => group,
            Err(error) => {
                tracing::warn!(%error, "failed to read recording directory");
                return None;
            }
        };
        if !self.process_group_maker.as_bytes().is_empty() {
            group.set_maker_bytes_recursively(self.process_group_maker.as_bytes());
        }
        let packed = match group.pack() {
            Ok(packed) => packed,
            Err(error) => {
                tracing::warn!(%error, "failed to pack scenario recording");
                return None;
            }
        };
        if let Err(error) = replace_file_from_same_directory(&output_path, &packed) {
            tracing::warn!(%error, path = %output_path.display(), "failed to write scenario recording");
            return None;
        }
        tracing::info!(path = %output_path.display(), "saved scenario recording");
        if !self.network_is_league {
            // C4Game::Evaluate passes a null SHA destination outside league
            // play, so C4Record::Stop never rereads or hashes the packed file.
            return None;
        }
        let on_disk = match fs::read(&output_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, path = %output_path.display(), "failed to read closed scenario recording");
                return None;
            }
        };
        let name = league_record_name(&output_path)?;
        Some(LeagueEndRecord {
            name,
            sha1: Sha1::digest(&on_disk).into(),
        })
    }
}
