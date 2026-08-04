//! `impl GameApp` — pointer, keyboard, gamepad & touch input methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

/// Whether the windowing backend tells the engine that a keydown is an
/// operating-system key-repeat, i.e. whether `C4KeyCodeEx::IsRepeated()` can
/// ever be set.
///
/// C++ answers this per *windowing backend*, chosen at build time:
///
/// - **Win32** reads the real hardware bit: `DoKeyboardInput(..., !!(lParam &
///   0x40000000), ...)` (`C4Viewport.cpp:89,100`, `C4FullScreen.cpp:59,64`,
///   `C4GuiDialogs.cpp:231,240`). Repeats are repeats.
/// - **X11** passes `false` and `C4Game::DoKeyboardInput` re-derives the flag
///   from its own `PressedKeys` map — but only inside `#ifdef USE_X11`
///   (`C4Game.cpp:2143-2154`). Same answer as Win32, synthesized.
/// - **SDL** passes a literal `false` for *every* keydown and keyup
///   (`C4FullScreen.cpp:387-400`) and gets no synthesis, because `USE_X11` is
///   excluded on Apple (`CMakeLists.txt:198-200`). A macOS auto-repeat is
///   therefore a fresh press to that build.
///
/// The port answers `true` on every target, deliberately declining to model the
/// SDL branch. That branch is not a rule about repeats; it is C++ *lacking the
/// information*. The port synthesizes the flag from its own pressed-key set the
/// way the X11 branch does, and that set is just as available on macOS.
/// Modelling the absence turns a machine-local preference into gameplay:
/// `C4Game::LocalControlKey` swallows a repeat for
/// AutoStopControl players (`C4Game.cpp:3566-3570`) and `C4Player::InCom`
/// raises a second identical com to `COM_Double` (`C4Player.cpp:1532-1533`), so
/// with the flag unset a *held* direction key manufactures `Control*Double`
/// every few frames at whatever rate the host repeats — firing the ClonkMars
/// Jetbelt and the Eke Airbike's Hyperfly boost, and arming the `COM_Down_D`
/// `DFA_PUSH` ungrab, without the player ever tapping twice.
pub(crate) const BACKEND_SYNTHESIZES_KEY_REPEAT: bool = true;

/// The repeated-key flag handed to `C4KeyCodeEx`, given whether the key was
/// already down and whether this backend reports repeats at all.
pub(crate) const fn engine_key_repeated(already_pressed: bool, backend_repeats: bool) -> bool {
    backend_repeats && already_pressed
}

impl GameApp {
    /// Keep platform text composition active only while the top input owner
    /// accepts text. An always-on IME can consume physical gameplay keys;
    /// disabling it everywhere loses dead-key and composed text on macOS.
    pub(crate) fn platform_ime_allowed(&self) -> bool {
        if !self.window_active
            || self.startup_network_transition_blocks_input()
            || (!self.message_dialogs.is_empty() && !self.running_chat_active())
            || self.context_menu.is_some()
        {
            return false;
        }
        self.console_mode
            || self.mode == AppMode::Menu
            || (self.game_option_input_dialog.is_some()
                && (self.running_chat_controller().is_none()
                    || self.running_chat_keyboard_active()))
            || self.league_signup_dialog.is_some()
            || (self.external_irc_dialog_visible
                && (self.mode != AppMode::Running
                    || self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc)))
    }

    pub(crate) fn current_cursor_atlas(&self) -> Arc<CursorAtlas> {
        self.active_game_graphics
            .as_ref()
            .map(|resources| Arc::clone(&resources.cursor_atlas))
            .unwrap_or_else(|| self.assets.cursor_atlas())
    }

    pub(crate) fn handle_text_input(&mut self, character: char) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.startup_tooltip.note_non_pointer_input();
        self.note_classic_lobby_non_pointer_input();
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.note_non_pointer_input();
        }
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if !self.message_dialogs.is_empty() && !self.running_chat_active() {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            if self.context_menu.is_none() {
                let mut encoded = [0_u8; 4];
                let text = character.encode_utf8(&mut encoded);
                let layout = self.league_signup_layout();
                let fonts = self.assets.clonk_fonts.clone();
                let actions = self
                    .league_signup_dialog
                    .as_mut()
                    .map(|dialog| match (layout.as_ref(), fonts.as_deref()) {
                        (Some(layout), Some(fonts)) => dialog
                            .controller
                            .handle_text_input_with_layout(text, layout, &fonts.text),
                        _ => dialog.controller.handle_text_input(text),
                    })
                    .unwrap_or_default();
                self.process_league_signup_actions(actions)?;
            }
            return Ok(());
        }
        if self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only())
        {
            return Ok(());
        }
        if self.external_irc_dialog_visible && self.context_menu.is_some() {
            return Ok(());
        }
        if self.external_irc_dialog_visible
            && (!matches!(self.mode, AppMode::Running)
                || self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc))
        {
            let Some(fonts) = self.assets.clonk_fonts.clone() else {
                return Ok(());
            };
            let mut encoded = [0_u8; 4];
            let text = character.encode_utf8(&mut encoded);
            let actions = self
                .external_irc_dialog
                .as_mut()
                .map(|dialog| dialog.handle_text_input(text, &fonts.text))
                .unwrap_or_default();
            self.process_network_dialog_actions(actions)?;
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let mut encoded = [0_u8; 4];
            let text = character.encode_utf8(&mut encoded);
            if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                pending.controller.handle_text_input(text);
            }
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let mut encoded = [0_u8; 4];
            let text = character.encode_utf8(&mut encoded);
            let actions = self
                .startup_player_properties_dialog
                .as_mut()
                .map(|pending| pending.controller.handle_text_input(text))
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.game_option_input_dialog.is_some()
            && self.context_menu.is_none()
            && (self.running_chat_controller().is_none() || self.running_chat_keyboard_active())
        {
            let Some(layout) = self.game_option_input_layout() else {
                return Ok(());
            };
            let Some(fonts) = self.assets.clonk_fonts.clone() else {
                return Ok(());
            };
            let mut encoded = [0_u8; 4];
            let input = character.encode_utf8(&mut encoded);
            let actions = self
                .game_option_input_dialog
                .as_mut()
                .map(|dialog| {
                    dialog
                        .controller
                        .handle_text_input(input, &layout, &fonts.text)
                })
                .unwrap_or_default();
            return self.finish_game_option_input_dialog_actions(actions);
        }
        if self.definition_selector.is_some() {
            return Ok(());
        }
        if let Some(menu) = self.context_menu.as_mut() {
            menu.note_non_pointer_input();
            // C4GUI::Screen routes text to pContext before the focused
            // control. ContextMenu::CharIn itself does not dispatch hotkeys;
            // those come from the KEY_Any key binding on physical key-down.
            return Ok(());
        }
        if self.mode != AppMode::Menu || character.is_ascii_control() {
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            let mut encoded = [0_u8; 4];
            let text = character.encode_utf8(&mut encoded).to_string();
            let actions = self
                .classic_host_lobby
                .as_mut()
                .map(|lobby| lobby.controller.text_input(text))
                .unwrap_or_default();
            return self.process_classic_lobby_actions(actions);
        }
        if self.startup_view == StartupView::NetworkLobby {
            let mut encoded = [0_u8; 4];
            let text = character.encode_utf8(&mut encoded).to_string();
            let actions = self
                .network_lobby
                .as_mut()
                .map(|lobby| {
                    lobby.sync_classic_controller();
                    lobby.controller.text_input(text)
                })
                .unwrap_or_default();
            return self.process_joined_lobby_controller_actions(actions);
        }
        if self.startup_view == StartupView::ScenarioBrowser
            && self.menu_state.rename_edit.is_some()
        {
            let mut encoded = [0_u8; 4];
            if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                rename.edit.insert_text(character.encode_utf8(&mut encoded));
            }
            return Ok(());
        }
        if self.startup_view == StartupView::ScenarioBrowser && self.menu_state.search_focused() {
            let mut encoded = [0_u8; 4];
            if self
                .menu_state
                .insert_search_text(character.encode_utf8(&mut encoded))
            {
                self.submit_scenario_search()?;
            }
            return Ok(());
        }
        if self.startup_view == StartupView::ScenarioBrowser {
            if self.scenario_selector_discovery.is_some() {
                return Ok(());
            }
            self.handle_menu_input(|menu| menu.select_list_character(character))?;
            return Ok(());
        }
        if self.startup_view == StartupView::PlayerSelection && self.startup_crew_rename.is_some() {
            let mut encoded = [0_u8; 4];
            if let Some(rename) = self.startup_crew_rename.as_mut() {
                rename.edit.insert_text(character.encode_utf8(&mut encoded));
            }
            return Ok(());
        }
        if self.startup_view == StartupView::PlayerSelection {
            let actions = match self.startup_player_dialog.as_mut() {
                Some(dialog) if dialog.is_crew_mode() => dialog.handle_character(
                    character,
                    self.startup_crew_models
                        .iter()
                        .map(|crew| crew.name.as_str()),
                ),
                Some(dialog) => dialog.handle_character(
                    character,
                    self.startup_player_models
                        .iter()
                        .map(|player| player.name.as_str()),
                ),
                None => Vec::new(),
            };
            if !actions.is_empty() {
                self.process_player_dialog_actions(actions)?;
            }
            return Ok(());
        }
        if self.startup_view != StartupView::NetworkGame {
            return Ok(());
        }
        let mut encoded = [0_u8; 4];
        let text = character.encode_utf8(&mut encoded);
        let fonts = self.assets.clonk_fonts.clone();
        let actions = fonts
            .as_deref()
            .and_then(|fonts| {
                self.startup_network_dialog
                    .as_mut()
                    .map(|dialog| dialog.handle_text_input(text, &fonts.text))
            })
            .unwrap_or_default();
        self.process_network_dialog_actions(actions)
    }

    /// `C4GUI::ScrollWindow::MouseInput` scrolls by the SDL wheel delta;
    /// C4FullScreen converts one notch to 60 logical pixels
    /// (C4FullScreen.cpp:408; C4GuiContainers.cpp:612-620).
    pub(crate) fn handle_mouse_wheel(
        &mut self,
        delta: MouseScrollDelta,
        output_scale: f32,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        self.startup_tooltip.note_pointer_wheel();
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if self.network_chart_is_elevated_pointer_layer()
            && self.context_menu.is_none()
            && self
                .running_pointer_position
                .is_some_and(|point| self.network_chart_contains_point(point))
        {
            return Ok(());
        }
        let message_dialog_fallback_blocks_world = self.runtime_pointer_fallback_is_exclusive();
        let context_routed_before_running_dialogs = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && !self.running_dialog_stack.is_empty();
        if context_routed_before_running_dialogs {
            let outcome = self.context_menu.as_mut().map(|menu| {
                let point = menu.pointer_position();
                menu.handle_pointer_down(point, ContextMenuPointerButton::Other)
            });
            if let Some(outcome) = outcome {
                let captured = outcome.captured && !outcome.pass_through;
                self.process_context_menu_outcome(outcome)?;
                if captured {
                    self.occlude_running_dialog_pointer_hovers();
                    return Ok(());
                }
            }
        }
        let mut top_default_target = None;
        if self.mode == AppMode::Running {
            if let Some(point) = self.running_pointer_position {
                for dialog_kind in self
                    .runtime_default_dialog_order_snapshot()
                    .into_iter()
                    .rev()
                {
                    if self.runtime_default_dialog_contains_point(dialog_kind, point)? {
                        top_default_target = Some(dialog_kind);
                        break;
                    }
                }
            }
        }
        if self.mode == AppMode::Running {
            let mut shared_target = self
                .running_pointer_position
                .map(|point| self.top_scoreboard_message_pointer_target(point, false))
                .transpose()?
                .flatten();
            if let Some(
                entry @ (RunningDialogStackEntry::Scoreboard
                | RunningDialogStackEntry::RuntimeClientList),
            ) = shared_target
            {
                let split = self
                    .running_dialog_stack
                    .iter()
                    .position(|candidate| candidate.z_order() > 0)
                    .unwrap_or(self.running_dialog_stack.len());
                let entry_is_in_tail = self
                    .running_dialog_stack
                    .iter()
                    .rposition(|candidate| *candidate == entry)
                    .is_some_and(|position| position >= split);
                let shared_default = match entry {
                    RunningDialogStackEntry::Scoreboard => RuntimeDefaultDialog::Scoreboard,
                    RunningDialogStackEntry::RuntimeClientList => RuntimeDefaultDialog::ClientList,
                    RunningDialogStackEntry::Message(_) | RunningDialogStackEntry::Chat => {
                        unreachable!("only default-z shared dialogs reach this branch")
                    }
                };
                if !entry_is_in_tail
                    && top_default_target.is_some_and(|target| target != shared_default)
                {
                    shared_target = None;
                }
            }
            match shared_target {
                Some(RunningDialogStackEntry::RuntimeClientList) => {
                    let native_delta = match delta {
                        MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                        MouseScrollDelta::PixelDelta(position) => {
                            (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                        }
                    };
                    let geometry = self.runtime_client_list_input_geometry();
                    let point = self.running_pointer_position;
                    let _ = geometry
                        .zip(point)
                        .and_then(|((preferred, line_height), point)| {
                            self.runtime_client_list.as_mut().map(|dialog| {
                                dialog.handle_wheel(point, native_delta, preferred, line_height)
                            })
                        })
                        .unwrap_or(false);
                    self.suspend_ingame_pointer_for_gui();
                    self.cancel_ingame_mouse_gestures();
                    return Ok(());
                }
                Some(
                    RunningDialogStackEntry::Scoreboard
                    | RunningDialogStackEntry::Message(_)
                    | RunningDialogStackEntry::Chat,
                ) => {
                    self.suspend_ingame_pointer_for_gui();
                    self.cancel_ingame_mouse_gestures();
                    return Ok(());
                }
                None => {}
            }
        } else if !self.message_dialogs.is_empty() && self.running_chat_controller().is_none() {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            return Ok(());
        }
        let runtime_client_info_only = self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only());
        if runtime_client_info_only {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if let (Some((preferred, line_height)), Some(point)) = (
                self.runtime_client_list_input_geometry(),
                self.running_pointer_position,
            ) {
                if let Some(dialog) = self.runtime_client_list.as_mut() {
                    let _ = dialog.handle_wheel(point, native_delta, preferred, line_height);
                }
            }
            return Ok(());
        }
        if self.external_irc_dialog_visible && !context_routed_before_running_dialogs {
            if let Some(menu) = self.context_menu.as_mut() {
                let point = menu.pointer_position();
                let outcome = menu.handle_pointer_down(point, ContextMenuPointerButton::Other);
                let captured = outcome.captured && !outcome.pass_through;
                self.process_context_menu_outcome(outcome)?;
                if captured {
                    return Ok(());
                }
            }
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            let actions = self
                .external_irc_dialog
                .as_mut()
                .and_then(|dialog| {
                    dialog
                        .pointer_position()
                        .map(|point| dialog.handle_wheel(point, native_delta))
                })
                .unwrap_or_default();
            self.process_network_dialog_actions(actions)?;
            return Ok(());
        }
        if let Some(layout) = self.network_start_wait_layout() {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            let _ = self.network_start_wait.as_mut().is_some_and(|wait| {
                wait.pointer
                    .is_some_and(|point| wait.controller.handle_wheel(point, native_delta, &layout))
            });
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            let _ = self
                .startup_options_advanced_dialog
                .as_mut()
                .is_some_and(|pending| pending.controller.handle_wheel(native_delta));
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            let _ = self
                .startup_player_properties_dialog
                .as_mut()
                .is_some_and(|pending| pending.controller.handle_wheel(native_delta));
            return Ok(());
        }
        if self.definition_selector.is_some() {
            let amount = match delta {
                MouseScrollDelta::LineDelta(_, y) => (-y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (-position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            let layout = self.definition_selector_layout();
            let point = self
                .definition_selector
                .as_ref()
                .and_then(|controller| controller.pointer_position());
            let actions = layout
                .as_ref()
                .zip(point)
                .and_then(|(layout, point)| {
                    self.definition_selector.as_mut().map(|controller| {
                        // Controller takes the native wheel sign; `amount`
                        // is already the desired scroll-window displacement.
                        controller.handle_wheel(point, -amount, layout)
                    })
                })
                .unwrap_or_default();
            self.finish_definition_selector_input(actions)?;
            return Ok(());
        }
        if !context_routed_before_running_dialogs {
            if let Some(menu) = self.context_menu.as_mut() {
                let point = menu.pointer_position();
                let outcome = menu.handle_pointer_down(point, ContextMenuPointerButton::Other);
                let captured = outcome.captured && !outcome.pass_through;
                self.process_context_menu_outcome(outcome)?;
                if captured {
                    return Ok(());
                }
            }
        }
        if self.game_option_input_dialog.is_some()
            && self.game_option_input_owns_running_pointer_event()
            && top_default_target.is_none()
        {
            return Ok(());
        }
        if matches!(self.mode, AppMode::Running) {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if let Some(point) = self.running_pointer_position {
                for dialog_kind in self
                    .runtime_default_dialog_order_snapshot()
                    .into_iter()
                    .rev()
                {
                    if !self.runtime_default_dialog_contains_point(dialog_kind, point)? {
                        continue;
                    }
                    match dialog_kind {
                        RuntimeDefaultDialog::ExternalIrc => {
                            let actions = self
                                .external_irc_dialog
                                .as_mut()
                                .map(|dialog| dialog.handle_wheel(point, native_delta))
                                .unwrap_or_default();
                            self.process_network_dialog_actions(actions)?;
                        }
                        RuntimeDefaultDialog::GameOver => {
                            let (width, height) = {
                                let surface = self.graphics.surface();
                                (surface.width(), surface.height())
                            };
                            if let Some(dialog) = self.game_over_dialog.as_mut() {
                                dialog.handle_pointer_move(point.x, point.y, width, height);
                                dialog.handle_wheel(native_delta, width, height);
                            }
                        }
                        RuntimeDefaultDialog::ClientList => {
                            if let Some((preferred, line_height)) =
                                self.runtime_client_list_input_geometry()
                            {
                                if let Some(dialog) = self.runtime_client_list.as_mut() {
                                    dialog.handle_wheel(
                                        point,
                                        native_delta,
                                        preferred,
                                        line_height,
                                    );
                                }
                            }
                        }
                        RuntimeDefaultDialog::NetworkChart | RuntimeDefaultDialog::Scoreboard => {}
                    }
                    return Ok(());
                }
            }
        }
        if message_dialog_fallback_blocks_world {
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            let amount = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            return self.handle_classic_lobby_wheel(amount);
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.network_lobby.is_some()
        {
            let amount = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if amount == 0 {
                return Ok(());
            }
            self.scenario_game_options.note_pointer_wheel();
            let (_, scroll_window_captured) = match self.network_lobby.as_mut() {
                Some(lobby) => lobby
                    .wheel_right_sheet(
                        amount,
                        self.graphics.surface(),
                        self.assets.as_ref(),
                        &self.scenario_game_options,
                    )
                    .map_err(|error| {
                        classic_parity_engine_error(report_classic_parity_boundary(
                            ClassicParityBoundary::GameLobby(ClassicGameLobbyBoundary::Resources {
                                detail: error.to_string(),
                            }),
                        ))
                    })?,
                None => (false, false),
            };
            if scroll_window_captured {
                // C4GUI::ScrollWindow consumes the wheel and clears the
                // screen's pMouseOver owner until a later pointer event.
                self.note_classic_lobby_non_pointer_input();
            }
            return Ok(());
        }
        if self.mode == AppMode::Running {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if let Some(point) = self.ingame_gui_pointer {
                if self.handle_ingame_menu_wheel(point, native_delta.saturating_neg()) {
                    return Ok(());
                }
                if self.handle_script_menu_wheel(point, native_delta.saturating_neg())? {
                    return Ok(());
                }
            }
            self.initialize_ingame_mouse_for_wheel();
            self.advance_ingame_mouse_caption_lifetime();
            if !self.mouse_control {
                self.restore_ingame_mouse_region_caption();
                return Ok(());
            }
            let Some(owner) = self.local_controls.mouse_owner() else {
                return Ok(());
            };
            let command = if native_delta > 0 {
                Some(clonk_engine::COM_WHEEL_UP)
            } else if native_delta < 0 {
                Some(clonk_engine::COM_WHEEL_DOWN)
            } else {
                None
            };
            if let Some(command) = command {
                self.dispatch_control_event_for_local_player(
                    owner,
                    ControlEvent::RawPlayerControl { command, data: 0 },
                )?;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::PlayerSelection {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if native_delta == 0 {
                return Ok(());
            }
            let mut actions = Vec::new();
            let mut scrolled = false;
            if let Some(dialog) = self.startup_player_dialog.as_mut() {
                if let Some(point) = dialog.pointer_position() {
                    let before = dialog.list_scroll_offset();
                    actions = dialog.handle_wheel(point, native_delta);
                    scrolled = dialog.list_scroll_offset() != before;
                }
            }
            self.process_player_dialog_actions(actions)?;
            if scrolled {
                self.plrsel_last_click = None;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::NetworkGame {
            let native_delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if native_delta == 0 {
                return Ok(());
            }
            let actions = self
                .startup_network_dialog
                .as_mut()
                .and_then(|dialog| {
                    dialog
                        .pointer_position()
                        .map(|point| dialog.handle_wheel(point, native_delta))
                })
                .unwrap_or_default();
            self.process_network_dialog_actions(actions)?;
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::About {
            let delta = match delta {
                MouseScrollDelta::LineDelta(_, y) => (y * 60.0).round() as i32,
                MouseScrollDelta::PixelDelta(position) => {
                    (position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
                }
            };
            if delta == 0 {
                return Ok(());
            }
            let fonts = self.assets.clonk_fonts.clone();
            let actions = fonts
                .as_deref()
                .and_then(|fonts| {
                    self.startup_about_dialog.as_mut().and_then(|dialog| {
                        dialog
                            .pointer_position()
                            .map(|point| dialog.handle_wheel(point, delta, fonts))
                    })
                })
                .unwrap_or_default();
            self.process_about_dialog_actions(actions)?;
            return Ok(());
        }
        if self.mode != AppMode::Menu || self.startup_view != StartupView::ScenarioBrowser {
            return Ok(());
        }
        if self.scenario_selector_discovery.is_some() {
            return Ok(());
        }
        let Some(point) = self.menu_state.pointer_position() else {
            return Ok(());
        };
        let (Some(fonts), Some(book_fonts)) = (
            self.assets.clonk_fonts.as_deref(),
            self.assets.book_fonts.as_deref(),
        ) else {
            return Ok(());
        };
        let layout = clonk_frontend::startup_scensel::scen_sel_layout(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
            fonts,
        );
        let amount = match delta {
            MouseScrollDelta::LineDelta(_, y) => (-y * 60.0).round() as i32,
            MouseScrollDelta::PixelDelta(position) => {
                (-position.y / f64::from(output_scale.max(f32::EPSILON))).round() as i32
            }
        };
        if amount == 0 {
            return Ok(());
        }
        let contains = |rect: clonk_frontend::classic_gui::IntRect| {
            point.x >= rect.x as f32
                && point.x < (rect.x + rect.w) as f32
                && point.y >= rect.y as f32
                && point.y < (rect.y + rect.h) as f32
        };
        if let Some(map) = self.menu_state.current_map() {
            let transform = MapFolderTransform::for_map(
                map,
                &layout,
                self.graphics.surface().width(),
                self.graphics.surface().height(),
            );
            let info_rect = transform.rect(map.scenario_info_area);
            if !point_in_map_rect(point, &info_rect) {
                return Ok(());
            }
            self.startup_tooltip.pointer_left();
            let mut info_layout = layout;
            info_layout.selection_info = clonk_frontend::classic_gui::IntRect {
                x: info_rect.origin.x.round() as i32,
                y: info_rect.origin.y.round() as i32,
                w: info_rect.size.width.round() as i32,
                h: info_rect.size.height.round() as i32,
            };
            let metrics = {
                let info = scensel_selection_info(&self.menu_state);
                clonk_frontend::startup_scensel::selection_info_scroll_metrics(
                    &info_layout,
                    book_fonts,
                    &info,
                )
            };
            self.menu_state.scroll_selection_info_by(amount, metrics);
            return Ok(());
        }
        if contains(layout.list) {
            self.startup_tooltip.pointer_left();
            let item_height =
                clonk_frontend::startup_scensel::scen_list_item_height(&book_fonts.text);
            let viewport_height = layout.list.h - 6;
            self.menu_state
                .scroll_scenario_list_by(amount, viewport_height, item_height + 1);
            return Ok(());
        }
        if !contains(layout.selection_info) {
            return Ok(());
        }
        self.startup_tooltip.pointer_left();
        let metrics = {
            let info = scensel_selection_info(&self.menu_state);
            clonk_frontend::startup_scensel::selection_info_scroll_metrics(
                &layout, book_fonts, &info,
            )
        };
        self.menu_state.scroll_selection_info_by(amount, metrics);
        Ok(())
    }

    fn input_network_dialog(&self) -> Option<&clonk_frontend::startup_netdlg::NetDlgController> {
        if self.external_irc_dialog_visible {
            self.external_irc_dialog.as_ref()
        } else {
            self.startup_network_dialog.as_ref()
        }
    }

    pub(crate) fn input_network_dialog_mut(
        &mut self,
    ) -> Option<&mut clonk_frontend::startup_netdlg::NetDlgController> {
        if self.external_irc_dialog_visible {
            self.external_irc_dialog.as_mut()
        } else {
            self.startup_network_dialog.as_mut()
        }
    }

    pub(crate) fn handle_network_edit_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        use clonk_frontend::startup_netdlg::{
            NetDlgControl, NetDlgEditClipboardShortcut, NetDlgEditKey, NetDlgEditModifiers,
        };

        let embedded_network_chat =
            self.mode == AppMode::Menu && self.startup_view == StartupView::NetworkGame;
        if !self.external_irc_dialog_visible && !embedded_network_chat {
            return Ok(false);
        }
        if state == ElementState::Released && self.netdlg_edit_consumed_keys.remove(&key) {
            return Ok(true);
        }
        if state != ElementState::Pressed {
            return Ok(false);
        }

        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let edit_focused = self.input_network_dialog().is_some_and(|dialog| {
            matches!(
                dialog.focused_control(),
                NetDlgControl::JoinAddress | NetDlgControl::ChatInput
            )
        });
        // Both StartupNetBack and the Edit cursor bindings use exact modifier
        // masks. Modified Back/Left outside the edit, and Alt-modified ones
        // inside it, must not fall through to the modifier-blind GUI mapping.
        if matches!(key, VirtualKeyCode::Backspace | VirtualKeyCode::ArrowLeft)
            && !modifiers.is_empty()
            && (!edit_focused || modifiers.contains(ModifiersState::ALT))
        {
            self.netdlg_edit_consumed_keys.insert(key);
            return Ok(true);
        }
        if !edit_focused {
            return Ok(false);
        }
        if key == VirtualKeyCode::ContextMenu && modifiers.is_empty() {
            let outcome = self
                .input_network_dialog_mut()
                .map(|dialog| dialog.request_context_menu_from_key(clipboard_text_available()))
                .unwrap_or_default();
            if !outcome.captured {
                return Ok(false);
            }
            self.process_network_dialog_actions(outcome.actions)?;
            return Ok(true);
        }

        if modifiers.contains(ModifiersState::ALT) {
            return Ok(false);
        }
        let edit_key = match key {
            VirtualKeyCode::ArrowLeft => Some(NetDlgEditKey::Left),
            VirtualKeyCode::ArrowRight => Some(NetDlgEditKey::Right),
            VirtualKeyCode::Home => Some(NetDlgEditKey::Home),
            VirtualKeyCode::End => Some(NetDlgEditKey::End),
            VirtualKeyCode::Backspace => Some(NetDlgEditKey::Backspace),
            VirtualKeyCode::Delete => Some(NetDlgEditKey::Delete),
            _ => None,
        };
        let shortcut = if modifiers == ModifiersState::CONTROL {
            match key {
                VirtualKeyCode::KeyC => Some(NetDlgEditClipboardShortcut::Copy),
                VirtualKeyCode::KeyX => Some(NetDlgEditClipboardShortcut::Cut),
                VirtualKeyCode::KeyV => Some(NetDlgEditClipboardShortcut::Paste),
                VirtualKeyCode::KeyA => Some(NetDlgEditClipboardShortcut::SelectAll),
                _ => None,
            }
        } else {
            None
        };
        if edit_key.is_none() && shortcut.is_none() {
            return Ok(false);
        }

        let Some(fonts) = self.assets.clonk_fonts.clone() else {
            // The startup bootstrap guard reports missing fonts before this
            // route. Still consume the Edit-owned key rather than leaking it
            // to StartupNetBack if that invariant is broken.
            self.netdlg_edit_consumed_keys.insert(key);
            return Ok(true);
        };
        let clipboard = shortcut
            .filter(|shortcut| *shortcut == NetDlgEditClipboardShortcut::Paste)
            .and_then(|_| {
                arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.get_text())
                    .ok()
            });
        let outcome = self
            .input_network_dialog_mut()
            .map(|dialog| {
                if let Some(edit_key) = edit_key {
                    dialog.handle_edit_key_down(
                        edit_key,
                        NetDlgEditModifiers {
                            shift: modifiers.contains(ModifiersState::SHIFT),
                            control: modifiers.contains(ModifiersState::CONTROL),
                        },
                        &fonts.text,
                    )
                } else {
                    dialog.handle_clipboard_shortcut(
                        shortcut.expect("checked clipboard shortcut"),
                        clipboard.as_deref(),
                        &fonts.text,
                    )
                }
            })
            .unwrap_or_default();
        if !outcome.captured {
            return Ok(false);
        }
        self.netdlg_edit_consumed_keys.insert(key);
        self.process_network_dialog_actions(outcome.actions)?;
        Ok(true)
    }

    fn handle_league_signup_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.league_signup_dialog.is_none() {
            return Ok(false);
        }
        if self.context_menu.is_some() {
            return Ok(true);
        }
        use clonk_frontend::league_signup::{
            LeagueSignupEditClipboardShortcut, LeagueSignupEditKey, LeagueSignupKeyModifiers,
        };
        let layout = self.league_signup_layout();
        let fonts = self.assets.clonk_fonts.clone();
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let modifiers = LeagueSignupKeyModifiers {
            shift: c4_modifiers.shift_key(),
            control: c4_modifiers.control_key(),
        };
        let hotkey_modifiers = c4_modifiers == ModifiersState::ALT
            || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
        let actions = match state {
            ElementState::Pressed if hotkey_modifiers => context_menu_hotkey(key)
                .and_then(|character| {
                    self.league_signup_dialog
                        .as_mut()
                        .map(|dialog| dialog.controller.handle_hotkey(character))
                })
                .unwrap_or_default(),
            ElementState::Pressed
                if c4_modifiers == ModifiersState::CONTROL
                    && matches!(
                        key,
                        VirtualKeyCode::KeyA
                            | VirtualKeyCode::KeyC
                            | VirtualKeyCode::KeyX
                            | VirtualKeyCode::KeyV
                    ) =>
            {
                let shortcut = match key {
                    VirtualKeyCode::KeyA => Some(LeagueSignupEditClipboardShortcut::SelectAll),
                    VirtualKeyCode::KeyC => Some(LeagueSignupEditClipboardShortcut::Copy),
                    VirtualKeyCode::KeyX => Some(LeagueSignupEditClipboardShortcut::Cut),
                    VirtualKeyCode::KeyV => Some(LeagueSignupEditClipboardShortcut::Paste),
                    _ => None,
                };
                let clipboard = (shortcut == Some(LeagueSignupEditClipboardShortcut::Paste))
                    .then(|| {
                        arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.get_text())
                            .ok()
                    })
                    .flatten();
                shortcut
                    .and_then(|shortcut| {
                        layout
                            .as_ref()
                            .zip(fonts.as_deref())
                            .and_then(|(layout, fonts)| {
                                self.league_signup_dialog.as_mut().map(|dialog| {
                                    dialog.controller.handle_clipboard_shortcut(
                                        shortcut,
                                        clipboard.as_deref(),
                                        layout,
                                        &fonts.text,
                                    )
                                })
                            })
                    })
                    .unwrap_or_default()
            }
            ElementState::Pressed
                if c4_modifiers.is_empty() && key == VirtualKeyCode::ContextMenu =>
            {
                layout
                    .as_ref()
                    .and_then(|layout| {
                        self.league_signup_dialog.as_ref().map(|dialog| {
                            dialog
                                .controller
                                .request_context_menu_from_key(clipboard_text_available(), layout)
                        })
                    })
                    .unwrap_or_default()
            }
            ElementState::Pressed if !c4_modifiers.alt_key() => {
                let edit_key = match key {
                    VirtualKeyCode::Backspace => Some(LeagueSignupEditKey::Backspace),
                    VirtualKeyCode::Delete => Some(LeagueSignupEditKey::Delete),
                    VirtualKeyCode::Home => Some(LeagueSignupEditKey::Home),
                    VirtualKeyCode::End => Some(LeagueSignupEditKey::End),
                    VirtualKeyCode::ArrowLeft => Some(LeagueSignupEditKey::Left),
                    VirtualKeyCode::ArrowRight => Some(LeagueSignupEditKey::Right),
                    _ => None,
                };
                if let Some(edit_key) = edit_key {
                    self.league_signup_dialog
                        .as_mut()
                        .map(|dialog| match (layout.as_ref(), fonts.as_deref()) {
                            (Some(layout), Some(fonts)) => {
                                dialog.controller.handle_edit_key_with_layout(
                                    edit_key,
                                    modifiers,
                                    layout,
                                    &fonts.text,
                                )
                            }
                            _ => dialog.controller.handle_edit_key(edit_key, modifiers),
                        })
                        .unwrap_or_default()
                } else if let Some(gui_key) = league_signup_dialog_key_code(key, c4_modifiers) {
                    self.league_signup_dialog
                        .as_mut()
                        .map(|dialog| dialog.controller.handle_key_down(gui_key, modifiers.shift))
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            ElementState::Released => league_signup_dialog_key_code(key, c4_modifiers)
                .and_then(|gui_key| {
                    self.league_signup_dialog
                        .as_mut()
                        .map(|dialog| dialog.controller.handle_key_up(gui_key))
                })
                .unwrap_or_default(),
            ElementState::Pressed => Vec::new(),
        };
        if state == ElementState::Pressed {
            self.league_signup_consumed_keys.insert(key);
        }
        self.process_league_signup_actions(actions)?;
        Ok(true)
    }

    fn handle_scenario_rename_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.menu_state.rename_edit.is_none() {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if c4_modifiers.is_empty() && key == VirtualKeyCode::F5 {
            if state == ElementState::Pressed {
                self.abort_scenario_rename();
            }
            return Ok(false);
        }
        let alt_hotkey_modifiers = c4_modifiers == ModifiersState::ALT
            || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
        let matched_option_hotkey = alt_hotkey_modifiers
            && context_menu_hotkey(key).is_some_and(|hotkey| {
                hotkey == 'D'
                    || self
                        .scenario_game_options
                        .context()
                        .buttons()
                        .iter()
                        .any(|button| button.hotkey() == hotkey)
            });
        let mission_access_hotkey = key == VirtualKeyCode::KeyM
            && c4_modifiers == ModifiersState::ALT
            && self.menu_state.current_map().is_none();
        if state == ElementState::Pressed && (matched_option_hotkey || mission_access_hotkey) {
            // Dialog hotkeys invoke controls directly; they do not move focus
            // out of RenameEdit. Let the matching selector control handle the
            // key while the inline edit remains active.
            return Ok(false);
        }
        let commits_on_focus_loss = state == ElementState::Pressed
            && ((key == VirtualKeyCode::KeyF && c4_modifiers == ModifiersState::CONTROL)
                || (key == VirtualKeyCode::Tab
                    && (c4_modifiers.is_empty() || c4_modifiers == ModifiersState::SHIFT)));
        if commits_on_focus_loss {
            self.commit_scenario_rename(true)?;
            // FinishRename restores/sets focus during OnLooseFocus, which
            // makes Dialog::SetFocus cancel the original transfer.
            return Ok(true);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        let ctrl = c4_modifiers.control_key();
        let shift = c4_modifiers.shift_key();
        let cursor_modifiers = !c4_modifiers.alt_key();
        match key {
            VirtualKeyCode::F2 if c4_modifiers.is_empty() => {}
            VirtualKeyCode::Escape if c4_modifiers.is_empty() => {
                self.abort_scenario_rename();
            }
            VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter if c4_modifiers.is_empty() => {
                self.commit_scenario_rename(false)?;
            }
            VirtualKeyCode::Backspace if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename.edit.backspace(ctrl, shift);
                }
            }
            VirtualKeyCode::Delete if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename.edit.delete(ctrl, shift);
                }
            }
            VirtualKeyCode::ArrowLeft if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Left, ctrl, shift);
                }
            }
            VirtualKeyCode::ArrowRight if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Right, ctrl, shift);
                }
            }
            VirtualKeyCode::Home if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::Home, ctrl, shift);
                }
            }
            VirtualKeyCode::End if cursor_modifiers => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename
                        .edit
                        .move_cursor(RenameEditCursorOperation::End, ctrl, shift);
                }
            }
            VirtualKeyCode::KeyA if c4_modifiers == ModifiersState::CONTROL => {
                if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                    rename.edit.select_all();
                }
            }
            VirtualKeyCode::KeyC if c4_modifiers == ModifiersState::CONTROL => {
                let result = self.menu_state.rename_edit.as_mut().map(|rename| {
                    transfer_edit_selection(&mut rename.edit, false, |selected| {
                        arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
                    })
                });
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "failed to copy scenario rename text");
                }
            }
            VirtualKeyCode::KeyX if c4_modifiers == ModifiersState::CONTROL => {
                let result = self.menu_state.rename_edit.as_mut().map(|rename| {
                    transfer_edit_selection(&mut rename.edit, true, |selected| {
                        arboard::Clipboard::new()
                            .and_then(|mut clipboard| clipboard.set_text(selected.to_string()))
                    })
                });
                if let Some(Err(error)) = result {
                    tracing::warn!(%error, "failed to cut scenario rename text");
                }
            }
            VirtualKeyCode::KeyV if c4_modifiers == ModifiersState::CONTROL => {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(text) => {
                        if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                            rename.edit.insert_text(&text);
                        }
                    }
                    Err(error) => tracing::warn!(%error, "failed to paste scenario rename text"),
                }
            }
            _ => {}
        }
        Ok(true)
    }

    fn runtime_pointer_fallback_is_exclusive(&self) -> bool {
        if self.network_chart_elevated {
            return false;
        }
        if self.running_chat_controller().is_some() || self.top_message_dialog_is_exclusive() {
            return true;
        }
        self.message_dialogs.is_empty() && self.runtime_top_default_dialog_is_exclusive()
    }

    pub(crate) fn network_chart_elevated_owns_input(&self) -> bool {
        self.network_chart_elevated
            && self.message_dialog_active_index.is_none()
            && !self.running_chat_active()
    }

    pub(crate) fn network_chart_is_elevated_pointer_layer(&self) -> bool {
        self.network_chart_renders_elevated() && self.context_menu.is_none()
    }

    fn gamepad_player_button_in_scope(
        &self,
        slot: GamepadSlot,
        button: LegacyGamepadButton,
    ) -> bool {
        self.gamepad_bindings
            .control_candidates_for_button(slot.index(), button.index(), ElementState::Pressed)
            .any(|(control_set, _)| {
                i32::try_from(control_set)
                    .ok()
                    .and_then(|control_set| self.local_controls.owner_for_set(control_set))
                    .and_then(|owner| self.engine.player(owner))
                    .is_some()
            })
    }

    fn gamepad_player_axis_in_scope(&self, slot: GamepadSlot, axis: LegacyGamepadAxis) -> bool {
        self.gamepad_bindings
            .control_candidates_for_axis(
                slot.index(),
                axis.index(),
                axis.high(),
                ElementState::Pressed,
            )
            .any(|(control_set, _)| {
                i32::try_from(control_set)
                    .ok()
                    .and_then(|control_set| self.local_controls.owner_for_set(control_set))
                    .and_then(|owner| self.engine.player(owner))
                    .is_some()
            })
    }

    pub(crate) fn handle_network_chart_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        if self.network_chart_consumed_keys.contains(&key) {
            if state == ElementState::Released {
                self.network_chart_consumed_keys.remove(&key);
            }
            return true;
        }
        if key != VirtualKeyCode::Escape
            || !self.network_chart_owns_stronger_escape()
            || !(self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT))
                .is_empty()
        {
            return false;
        }
        let action = self
            .network_chart_dialog
            .as_mut()
            .map(|dialog| dialog.handle_key(KeyCode::Escape, state == ElementState::Pressed))
            .unwrap_or(clonk_frontend::network_chart::NetworkChartDialogAction::Ignored);
        match action {
            clonk_frontend::network_chart::NetworkChartDialogAction::Ignored => false,
            clonk_frontend::network_chart::NetworkChartDialogAction::Handled
            | clonk_frontend::network_chart::NetworkChartDialogAction::Captured => {
                if state == ElementState::Pressed {
                    self.network_chart_consumed_keys.insert(key);
                }
                true
            }
            clonk_frontend::network_chart::NetworkChartDialogAction::Close => {
                self.cancel_network_chart_pointer_capture();
                self.network_chart_dialog = None;
                self.hide_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
                self.network_chart_consumed_keys.insert(key);
                true
            }
        }
    }

    pub(crate) fn handle_network_chart_pointer_button(&mut self, state: ElementState) -> bool {
        if !matches!(self.mode, AppMode::Running) || self.network_chart_dialog.is_none() {
            self.cancel_network_chart_pointer_capture();
            return false;
        }
        let release_captured =
            state == ElementState::Released && self.network_chart_pointer_capture;
        let Some(point) = self.running_pointer_position else {
            if release_captured {
                self.cancel_network_chart_pointer_capture();
            }
            return release_captured;
        };
        let assets = Arc::clone(&self.assets);
        let Some(resources) = assets.network_chart_resources() else {
            return release_captured;
        };
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        let action = self
            .network_chart_dialog
            .as_mut()
            .map(|dialog| match state {
                ElementState::Pressed => dialog.pointer_down(point, preferred, resources),
                ElementState::Released => dialog.pointer_up(point, preferred, resources),
            })
            .unwrap_or(clonk_frontend::network_chart::NetworkChartDialogAction::Ignored);
        let handled = release_captured
            || action != clonk_frontend::network_chart::NetworkChartDialogAction::Ignored;
        if action == clonk_frontend::network_chart::NetworkChartDialogAction::Close {
            self.network_chart_dialog = None;
            self.hide_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
        }
        match state {
            ElementState::Pressed => {
                self.network_chart_pointer_capture =
                    action == clonk_frontend::network_chart::NetworkChartDialogAction::Captured
            }
            ElementState::Released => self.network_chart_pointer_capture = false,
        }
        if handled {
            self.cancel_ingame_mouse_gestures();
        }
        handled
    }

    pub(crate) fn cancel_network_chart_pointer_capture(&mut self) {
        self.network_chart_pointer_capture = false;
        if let Some(dialog) = self.network_chart_dialog.as_mut() {
            dialog.cancel_pointer_capture();
        }
    }

    pub(crate) fn handle_network_chart_pointer_move(&mut self, point: GuiPoint) -> bool {
        let (Some(dialog), Some(resources)) = (
            self.network_chart_dialog.as_mut(),
            self.assets.network_chart_resources(),
        ) else {
            return false;
        };
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        dialog.pointer_move(point, preferred, resources)
    }

    pub(crate) fn runtime_keyboard_binding_matches(
        &self,
        name: &str,
        key: VirtualKeyCode,
        default_matches: bool,
    ) -> bool {
        self.runtime_key_config()
            .ok()
            .and_then(|config| config.keyboard_override_matches(name, key, self.keyboard_modifiers))
            .unwrap_or(default_matches)
    }

    pub(crate) fn runtime_control_candidates_for_keyboard(
        &self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Vec<(usize, Option<ControlEvent>)> {
        let config = self.runtime_key_config().ok();
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let mut candidates = Vec::new();
        for control_set in 0..KeyboardBindings::SET_COUNT {
            for (control_index, id) in ControlBindingId::ALL.into_iter().enumerate() {
                let name = format!("Kbd{}Key{}", control_set + 1, control_index + 1);
                let matches = config
                    .and_then(|config| config.override_for(&name))
                    .map(|codes| codes.iter().any(|code| code.matches(key, modifiers)))
                    .unwrap_or_else(|| {
                        modifiers.is_empty()
                            && self.bindings.key_for_set(control_set, id) == Some(key)
                    });
                if matches {
                    if let Some(candidate) =
                        KeyboardBindings::control_candidate_for_set(control_set, id, state)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates
    }

    pub(crate) fn runtime_control_candidates_for_gamepad_button(
        &self,
        slot: u8,
        button: u8,
        state: ElementState,
    ) -> Vec<(usize, Option<ControlEvent>)> {
        let mut candidates = Vec::new();
        let Some(config) = self.runtime_key_config().ok() else {
            return candidates;
        };
        for control_set in 0..KeyboardBindings::SET_COUNT {
            for (control_index, id) in ControlBindingId::ALL.into_iter().enumerate() {
                let name = format!("Kbd{}Key{}", control_set + 1, control_index + 1);
                let matches = config.override_for(&name).is_some_and(|codes| {
                    codes
                        .iter()
                        .any(|code| code.matches_gamepad_button(slot, button))
                });
                if matches {
                    if let Some(candidate) =
                        KeyboardBindings::control_candidate_for_set(control_set, id, state)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates
    }

    fn runtime_control_candidates_for_gamepad_direction(
        &self,
        slot: u8,
        direction: ControlButton,
        state: ElementState,
    ) -> Vec<(usize, Option<ControlEvent>)> {
        let mut candidates = Vec::new();
        let Some(config) = self.runtime_key_config().ok() else {
            return candidates;
        };
        for control_set in 0..KeyboardBindings::SET_COUNT {
            for (control_index, id) in ControlBindingId::ALL.into_iter().enumerate() {
                let name = format!("Kbd{}Key{}", control_set + 1, control_index + 1);
                let matches = config.override_for(&name).is_some_and(|codes| {
                    codes
                        .iter()
                        .any(|code| code.matches_gamepad_direction(slot, direction))
                });
                if matches {
                    if let Some(candidate) =
                        KeyboardBindings::control_candidate_for_set(control_set, id, state)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates
    }

    fn runtime_gamepad_button_override_matches(&self, name: &str, slot: u8, button: u8) -> bool {
        self.runtime_key_config()
            .ok()
            .and_then(|config| config.override_for(name))
            .is_some_and(|codes| {
                codes
                    .iter()
                    .any(|code| code.matches_gamepad_button(slot, button))
            })
    }

    fn runtime_gamepad_direction_override_matches(
        &self,
        name: &str,
        slot: u8,
        direction: ControlButton,
    ) -> bool {
        self.runtime_key_config()
            .ok()
            .and_then(|config| config.override_for(name))
            .is_some_and(|codes| {
                codes
                    .iter()
                    .any(|code| code.matches_gamepad_direction(slot, direction))
            })
    }

    /// Keep resource/group ownership failures ahead of keyboard mutation.
    /// Individual unknown or malformed entries are warning-only like
    /// `CompileFromBuf_LogWarn` and therefore never reach this boundary.
    fn guard_runtime_key_dispatch(&self, key: VirtualKeyCode) -> Result<(), EngineError> {
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if c4_modifiers == ModifiersState::CONTROL
            && matches!(
                key,
                VirtualKeyCode::F5 | VirtualKeyCode::F6 | VirtualKeyCode::F7 | VirtualKeyCode::F8
            )
        {
            // Native retains these registered defaults when no custom list
            // can be loaded. More importantly, a resource failure must not
            // turn the four diagnostic keys into process-fatal input.
            return Ok(());
        }
        self.runtime_key_config().map(|_| ()).map_err(|error| {
            let boundary = match key {
                VirtualKeyCode::F1 => ClassicParityBoundary::RuntimeHelpResources {
                    detail: error.to_string(),
                },
                VirtualKeyCode::F3 => ClassicParityBoundary::RuntimeFlashResources {
                    detail: error.to_string(),
                },
                _ => ClassicParityBoundary::RuntimeKeyConfig {
                    detail: error.to_string(),
                },
            };
            classic_parity_engine_error(report_classic_parity_boundary(boundary))
        })
    }

    fn local_player_key_binding_owner_in_scope(&self, key: VirtualKeyCode) -> Option<i32> {
        if self.game_over_dialog.is_some()
            || (self.runtime_gui_has_keyboard_focus() && !self.network_chart_elevated)
            || self.runtime_top_default_dialog_is_exclusive()
        {
            return None;
        }
        self.runtime_control_candidates_for_keyboard(key, ElementState::Pressed)
            .into_iter()
            .find_map(|(control_set, _)| {
                i32::try_from(control_set)
                    .ok()
                    .and_then(|control_set| self.local_controls.owner_for_set(control_set))
                    .filter(|owner| {
                        self.snapshot.hud.local_players.contains(owner)
                            && self.engine.player(*owner).is_some()
                    })
            })
    }

    pub(crate) fn local_player_key_binding_in_scope(&self, key: VirtualKeyCode) -> bool {
        self.local_player_key_binding_owner_in_scope(key).is_some()
    }

    pub(crate) fn scoreboard_pointer_target_cached(
        &self,
        point: GuiPoint,
    ) -> Option<ScoreboardPointerTarget> {
        let layout = self.scoreboard_runtime.presentation.as_ref()?.layout();
        let contains = |rect: clonk_frontend::classic_gui::IntRect| {
            point.x >= rect.x as f32
                && point.x < (rect.x + rect.w) as f32
                && point.y >= rect.y as f32
                && point.y < (rect.y + rect.h) as f32
        };
        if layout.close_button.is_some_and(contains) {
            Some(ScoreboardPointerTarget::Close)
        } else if layout.caption.is_some_and(contains) {
            Some(ScoreboardPointerTarget::Title)
        } else if contains(layout.bounds) {
            Some(ScoreboardPointerTarget::Dialog)
        } else {
            None
        }
    }

    pub(crate) fn scoreboard_pointer_target(
        &mut self,
        point: GuiPoint,
    ) -> Result<Option<ScoreboardPointerTarget>, EngineError> {
        // Synchronous script callbacks mutate the engine between ticks. Pull
        // their model/lifecycle effects before routing input, but preserve an
        // existing dialog's geometry until Draw performs C4ScoreboardDlg's
        // lazy Update. This also prevents pairing a new engine revision with
        // the previous snapshot's matrix.
        self.sync_scoreboard_presentation();
        if self.scoreboard_dialog.is_none() {
            return Ok(None);
        }
        if self.scoreboard_runtime.presentation.is_none() {
            let trigger = ClassicScoreboardTrigger::UserToggle;
            if let Err(error) = self.materialize_scoreboard_presentation() {
                tracing::error!(%error, "classic scoreboard pointer layout failed");
                return Err(classic_parity_engine_error(report_classic_parity_boundary(
                    self.scoreboard_boundary(trigger),
                )));
            }
        }
        Ok(self.scoreboard_pointer_target_cached(point))
    }

    pub(crate) fn running_shared_gui_has_keyboard_focus(&self) -> bool {
        if self.mode != AppMode::Running {
            return true;
        }
        if self.network_chart_renders_elevated() {
            return false;
        }
        match self.running_dialog_stack.last().copied() {
            Some(RunningDialogStackEntry::Chat) => true,
            Some(RunningDialogStackEntry::Message(stack_id)) => self
                .running_message_index(stack_id)
                .and_then(|index| self.message_dialogs.get(index))
                .is_some_and(|dialog| {
                    matches!(
                        dialog.continuation,
                        MessageDialogContinuation::AbortGame { .. }
                            | MessageDialogContinuation::LeagueVote { .. }
                            | MessageDialogContinuation::LeagueSurrender
                    )
                }),
            Some(
                RunningDialogStackEntry::Scoreboard | RunningDialogStackEntry::RuntimeClientList,
            )
            | None => false,
        }
    }

    pub(crate) fn message_dialog_owns_gamepad_input(&self) -> bool {
        if self.message_dialogs.is_empty() {
            return false;
        }
        if self.mode != AppMode::Running {
            return true;
        }
        matches!(
            self.running_active_dialog,
            Some(RunningDialogStackEntry::Message(_))
        ) && self.running_shared_gui_has_keyboard_focus()
    }

    pub(crate) fn runtime_gui_has_keyboard_focus(&self) -> bool {
        self.mode == AppMode::Running
            && (self.running_shared_gui_has_keyboard_focus() || self.game_over_dialog_is_active())
    }

    pub(crate) fn release_all_running_pointer_elements(&mut self) {
        self.scoreboard_pointer_left();
        self.release_message_dialog_pointer_elements();
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.pointer_left();
        }
        let game_over_sounds = self
            .game_over_dialog
            .as_mut()
            .map(|dialog| {
                dialog.pointer_left();
                dialog.take_sound_events()
            })
            .unwrap_or_default();
        self.play_game_over_sound_events(game_over_sounds);
        self.release_game_option_input_pointer_elements();
        self.close_context_menu_silently();
    }

    fn occlude_running_dialog_pointer_hovers(&mut self) {
        self.scoreboard_pointer_occluded();
        for index in 0..self.message_dialogs.len() {
            self.message_dialog_pointer_left_at(index);
        }
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.pointer_occluded();
        }
        let sounds = self
            .running_chat_controller_mut()
            .map(|controller| {
                controller.pointer_left();
                controller.take_sound_events()
            })
            .unwrap_or_default();
        self.play_input_dialog_sound_events(sounds);
    }

    pub(crate) fn top_scoreboard_message_pointer_target_cached(
        &self,
        point: GuiPoint,
    ) -> Option<RunningDialogStackEntry> {
        self.running_dialog_stack
            .iter()
            .rev()
            .find_map(|entry| match *entry {
                RunningDialogStackEntry::Scoreboard
                    if self.scoreboard_pointer_target_cached(point).is_some() =>
                {
                    Some(*entry)
                }
                RunningDialogStackEntry::Message(stack_id) => {
                    let index = self.running_message_index(stack_id)?;
                    self.message_dialog_layout_at(index)
                        .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout))
                        .then_some(*entry)
                }
                RunningDialogStackEntry::RuntimeClientList
                    if self.runtime_client_list_contains_point(point) =>
                {
                    Some(*entry)
                }
                RunningDialogStackEntry::Chat
                    if self.game_option_input_layout().is_some_and(|layout| {
                        Self::point_in_input_dialog_bounds(point, &layout)
                    }) =>
                {
                    Some(*entry)
                }
                RunningDialogStackEntry::Scoreboard
                | RunningDialogStackEntry::RuntimeClientList
                | RunningDialogStackEntry::Chat => None,
            })
    }

    fn top_scoreboard_message_pointer_target(
        &mut self,
        point: GuiPoint,
        include_capture: bool,
    ) -> Result<Option<RunningDialogStackEntry>, EngineError> {
        // CMouse owns one screen-global pDragElement. While it exists, the
        // active shared dialog remains a hit outside its bounds regardless of
        // which dialog owns the captured element.
        let global_drag_open = include_capture && self.running_shared_pointer_capture_open();
        let stack = self.running_dialog_stack.clone();
        for entry in stack.into_iter().rev() {
            match entry {
                RunningDialogStackEntry::Scoreboard => {
                    let captured = global_drag_open && self.running_active_dialog == Some(entry);
                    if captured || self.scoreboard_pointer_target(point)?.is_some() {
                        return Ok(Some(entry));
                    }
                }
                RunningDialogStackEntry::Message(stack_id) => {
                    let Some(index) = self.running_message_index(stack_id) else {
                        continue;
                    };
                    let captured = global_drag_open && self.running_active_dialog == Some(entry);
                    let hit = self
                        .message_dialog_layout_at(index)
                        .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout));
                    if captured || hit {
                        return Ok(Some(entry));
                    }
                }
                RunningDialogStackEntry::RuntimeClientList => {
                    let captured = global_drag_open && self.running_active_dialog == Some(entry);
                    if captured || self.runtime_client_list_contains_point(point) {
                        return Ok(Some(entry));
                    }
                }
                RunningDialogStackEntry::Chat => {
                    let captured = global_drag_open && self.running_active_dialog == Some(entry);
                    let hit = self
                        .game_option_input_layout()
                        .is_some_and(|layout| Self::point_in_input_dialog_bounds(point, &layout));
                    if captured || hit {
                        return Ok(Some(entry));
                    }
                }
            }
        }
        Ok(None)
    }

    fn running_shared_pointer_capture_open(&self) -> bool {
        self.scoreboard_close_pointer_capture
            || self.scoreboard_runtime.title_drag.is_some()
            || self.message_dialog_pointer_capture_index.is_some()
            || self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.has_pointer_capture())
            || self.game_option_input_pointer_capture == Some(ContextMenuPointerButton::Left)
    }

    fn top_running_shared_pointer_target(
        &mut self,
        point: GuiPoint,
        include_capture: bool,
    ) -> Result<Option<RunningDialogStackEntry>, EngineError> {
        let target = self.top_scoreboard_message_pointer_target(point, include_capture)?;
        if include_capture && self.running_shared_pointer_capture_open() {
            return Ok(target);
        }
        let Some(entry) = target else {
            return Ok(None);
        };
        let shared_default = match entry {
            RunningDialogStackEntry::Scoreboard => Some(RuntimeDefaultDialog::Scoreboard),
            RunningDialogStackEntry::RuntimeClientList => Some(RuntimeDefaultDialog::ClientList),
            RunningDialogStackEntry::Message(_) | RunningDialogStackEntry::Chat => None,
        };
        let Some(shared_default) = shared_default else {
            return Ok(Some(entry));
        };
        if self.running_shared_entry_is_in_tail(entry) {
            return Ok(Some(entry));
        }
        if self
            .top_runtime_default_dialog_at(point)?
            .is_some_and(|dialog| dialog != shared_default)
        {
            return Ok(None);
        }
        Ok(Some(entry))
    }

    fn scoreboard_pointer_occluded(&mut self) {
        if self.scoreboard_close_pointer_capture && self.scoreboard_runtime.close_hovered {
            self.play_ui_sound("ArrowHit");
        }
        self.scoreboard_runtime.close_hovered = false;
    }

    fn handle_scoreboard_message_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        let target = self.top_running_shared_pointer_target(point, true)?;
        match target {
            Some(RunningDialogStackEntry::Scoreboard) => {
                if self.primary_pointer_left_down {
                    self.activate_running_dialog(RunningDialogStackEntry::Scoreboard);
                }
                for index in 0..self.message_dialogs.len() {
                    self.message_dialog_pointer_left_at(index);
                }
                self.handle_scoreboard_pointer_move(point)
            }
            Some(RunningDialogStackEntry::Message(stack_id)) => {
                let Some(index) = self.running_message_index(stack_id) else {
                    return Ok(false);
                };
                self.scoreboard_pointer_occluded();
                if self.primary_pointer_left_down {
                    self.message_dialog_active_index = Some(index);
                    self.activate_running_dialog(RunningDialogStackEntry::Message(stack_id));
                }
                for other in 0..self.message_dialogs.len() {
                    if other != index {
                        self.message_dialog_pointer_left_at(other);
                    }
                }
                Ok(self.handle_message_dialog_pointer_move_at(index, point))
            }
            Some(RunningDialogStackEntry::RuntimeClientList) => {
                self.scoreboard_pointer_occluded();
                if self.primary_pointer_left_down {
                    self.activate_running_dialog(RunningDialogStackEntry::RuntimeClientList);
                }
                for index in 0..self.message_dialogs.len() {
                    self.message_dialog_pointer_left_at(index);
                }
                Ok(self.handle_runtime_client_list_pointer_move(point))
            }
            Some(RunningDialogStackEntry::Chat) => {
                unreachable!("chat routing is handled by its z=+2 controller")
            }
            None => {
                self.scoreboard_pointer_occluded();
                for index in 0..self.message_dialogs.len() {
                    self.message_dialog_pointer_left_at(index);
                }
                Ok(false)
            }
        }
    }

    fn handle_scoreboard_message_pointer_button(
        &mut self,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let Some(point) = self.running_pointer_position else {
            return Ok(false);
        };
        let target = self.top_running_shared_pointer_target(point, false)?;
        if self.mode == AppMode::Running && state == ElementState::Released {
            self.release_occluded_running_pointer_captures(target);
        }
        match target {
            Some(RunningDialogStackEntry::Scoreboard) => {
                if state == ElementState::Pressed {
                    if let Some(captured) = self.captured_message_dialog_index() {
                        self.cancel_message_dialog_pointer_capture_at(captured);
                    }
                }
                self.handle_scoreboard_pointer_button(state)
            }
            Some(RunningDialogStackEntry::Message(stack_id)) => {
                let Some(index) = self.running_message_index(stack_id) else {
                    return Ok(false);
                };
                if state == ElementState::Pressed {
                    self.scoreboard_pointer_left();
                    self.message_dialog_active_index = Some(index);
                    self.activate_running_dialog(RunningDialogStackEntry::Message(stack_id));
                } else if self.scoreboard_close_pointer_capture
                    || self.scoreboard_runtime.title_drag.is_some()
                {
                    self.scoreboard_pointer_left();
                }
                self.handle_message_dialog_pointer_button_at(index, state)
            }
            Some(RunningDialogStackEntry::RuntimeClientList) => {
                if state == ElementState::Pressed {
                    self.scoreboard_pointer_left();
                    if let Some(captured) = self.captured_message_dialog_index() {
                        self.cancel_message_dialog_pointer_capture_at(captured);
                    }
                    self.activate_running_dialog(RunningDialogStackEntry::RuntimeClientList);
                }
                self.handle_runtime_client_list_pointer_button(state)
            }
            Some(RunningDialogStackEntry::Chat) => {
                unreachable!("chat routing is handled by its z=+2 controller")
            }
            None => {
                let scoreboard_consumed = self.handle_scoreboard_pointer_button(state)?;
                if scoreboard_consumed {
                    return Ok(true);
                }
                self.handle_message_dialog_pointer_button(state)
            }
        }
    }

    fn release_occluded_running_pointer_captures(
        &mut self,
        target: Option<RunningDialogStackEntry>,
    ) {
        // CMouse clears pDragElement before reverse dialog hit-testing.
        // Cancel captures owned by any now-occluded dialog, then deliver
        // LeftUp only to the actual release-time stack hit.
        if target != Some(RunningDialogStackEntry::Scoreboard) {
            if self.scoreboard_close_pointer_capture || self.scoreboard_runtime.title_drag.is_some()
            {
                self.scoreboard_pointer_left();
            } else {
                self.scoreboard_pointer_occluded();
            }
        }
        if let Some(index) = self.captured_message_dialog_index() {
            let captured = self
                .message_dialogs
                .get(index)
                .map(|dialog| RunningDialogStackEntry::Message(dialog.running_stack_id));
            if captured != target {
                self.cancel_message_dialog_pointer_capture_at(index);
            }
        }
        if target != Some(RunningDialogStackEntry::RuntimeClientList)
            && self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.has_pointer_capture())
        {
            if let Some(dialog) = self.runtime_client_list.as_mut() {
                dialog.pointer_left();
            }
        }
    }

    fn sync_scoreboard_before_running_pointer_input(&mut self) {
        if self.mode == AppMode::Running {
            self.reconcile_initial_scoreboard();
            self.sync_scoreboard_presentation();
        }
    }

    fn custom_scoreboard_key_has_higher_priority_route(&self, key: VirtualKeyCode) -> bool {
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if self.game_over_dialog_is_active() || self.definition_selector.is_some() {
            return true;
        }
        if self.context_menu.is_some() {
            return true;
        }
        if self.running_chat_active() && !modifiers.contains(ModifiersState::ALT) {
            return true;
        }
        if self.game_option_input_dialog.is_some()
            && modifiers.is_empty()
            && map_key_code(key).is_some()
        {
            return true;
        }
        if self.active_message_dialog_index().is_some()
            && self.top_message_dialog_is_exclusive()
            && (modifiers.is_empty()
                || (key == VirtualKeyCode::Tab && modifiers == ModifiersState::SHIFT))
        {
            return true;
        }
        false
    }

    /// Process-global F3/Ctrl+F3 remain registered while C4Game is not
    /// running (startup and the GUI-owned loading phase). They toggle the
    /// frontend FEMusic/FESamples flags and never install an in-game flash.
    /// C4StartupOptionsDlg owns bare F3 at higher PRIO_Dlg so its checkbox is
    /// synchronized while that dialog is active. Ctrl+F3 remains the global
    /// binding and deliberately leaves the FE-sound checkbox stale.
    fn handle_frontend_global_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if matches!(self.mode, AppMode::Running)
            || !matches!(key, VirtualKeyCode::F3 | VirtualKeyCode::F9)
        {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if key == VirtualKeyCode::F9 {
            // `Screenshot` is registered `KEYSCOPE_Fullscreen | KEYSCOPE_Gui`,
            // so bare F9 also captures the startup screens; `ScreenshotEx` is
            // Fullscreen-only and stays inert here (C4Game.cpp:3387-3388).
            let screenshot =
                self.runtime_keyboard_binding_matches("Screenshot", key, c4_modifiers.is_empty());
            if !screenshot {
                return Ok(false);
            }
            if state == ElementState::Pressed {
                let gamma = self
                    .graphics
                    .active_gamma_ramp(&self.snapshot.environment.gamma);
                self.pending_screenshots.push_back(ScreenshotRequest {
                    kind: ScreenshotKind::PresentedFrame,
                    gamma,
                });
            }
            return Ok(true);
        }
        if c4_modifiers.is_empty() {
            if state == ElementState::Pressed {
                let enabled = self.toggle_frontend_music_option()?;
                // OptionsMusicToggle is the higher-priority bare-F3 binding
                // only while C4StartupOptionsDlg is the active dialog. A
                // modal/context above it falls through to the process-global
                // toggle, which changes FEMusic but leaves the retained
                // checkbox stale.
                if self.startup_options_dialog_is_active() {
                    if let Some(dialog) = self.startup_options_dialog.as_mut() {
                        dialog.sync_frontend_music_from_f3(enabled);
                    }
                }
            }
            return Ok(true);
        }
        if c4_modifiers == ModifiersState::CONTROL {
            if state == ElementState::Pressed {
                self.toggle_frontend_sound_option()?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Handles the exact default ScoreboardToggle key before generic app input
    /// dirties or mutates any UI state. Unlike F1/F4/Pause this is not a
    /// highest-priority runtime global: the guard above preserves C4's key
    /// priority stack. Logo is absent from C4KeyCodeEx's modifier mask.
    pub(crate) fn handle_scoreboard_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        self.handle_scoreboard_key_inner(key, state, false)
    }

    fn handle_scoreboard_key_after_higher_priority(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        self.handle_scoreboard_key_inner(key, state, true)
    }

    fn handle_scoreboard_key_inner(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
        higher_priority_checked: bool,
    ) -> Result<bool, EngineError> {
        if self.context_menu.is_none()
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc)
            && self.message_dialogs.is_empty()
            && self.game_option_input_dialog.is_none()
        {
            return Ok(false);
        }
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let custom_binding = self
            .runtime_key_config()
            .ok()
            .is_some_and(|config| config.override_for("ScoreboardToggle").is_some());
        if !self.runtime_keyboard_binding_matches(
            "ScoreboardToggle",
            key,
            key == VirtualKeyCode::Tab,
        ) {
            return Ok(false);
        }
        if custom_binding
            && !higher_priority_checked
            && self.custom_scoreboard_key_has_higher_priority_route(key)
            && !self.local_player_key_binding_in_scope(key)
        {
            return Ok(false);
        }
        let raw_repeated = match state {
            ElementState::Pressed => std::mem::replace(&mut self.scoreboard_tab_raw_pressed, true),
            ElementState::Released => {
                // Raw physical key-up clears repeat tracking before C4's
                // scoped bindings run. An exact in-scope control route below
                // may still emit its release callback; a newly exclusive
                // dialog suppresses that callback without a stuck latch.
                self.scoreboard_tab_raw_pressed = false;
                self.pressed_engine_keys.remove(&key);
                false
            }
        };
        if !c4_modifiers.is_empty() && !custom_binding {
            if self.game_over_dialog.is_some()
                || self.context_menu.is_some()
                || (self.runtime_client_list.is_some() && c4_modifiers == ModifiersState::SHIFT)
            {
                return Ok(false);
            }
            return Ok(true);
        }
        self.reconcile_initial_scoreboard();
        self.sync_scoreboard_presentation();
        if (self.game_over_dialog.is_some() || self.runtime_client_list.is_some())
            && self.context_menu.is_some()
        {
            // Context rejects Tab and suppresses the game-over DlgKeyCB. The
            // remaining generic callback reaches DoDlgShow, which consumes the
            // key but may not create a scoreboard during game over. Control
            // scope is absent here even when the physical Tab was rebound.
            return Ok(true);
        }
        if self.local_player_key_binding_in_scope(key) {
            // Run this directly: the app's context-menu release barrier is an
            // input-safety latch, but C++ PRIO_PlrControl precedes PRIO_Context
            // and must still receive a rebound Tab on both edges.
            if state == ElementState::Pressed {
                self.pressed_engine_keys.insert(key);
            }
            self.dispatch_engine_key_binding(key, state, raw_repeated)?;
            return Ok(true);
        }
        if self.scoreboard_tab_has_higher_priority_route() {
            return Ok(false);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        if self.close_scoreboard_dialog() {
            // User toggle closes an existing dialog regardless of a now-
            // negative refcount (C4Scoreboard.cpp:243-255).
            return Ok(true);
        }
        if !self.snapshot.hud.scoreboard.can_be_shown() {
            return Ok(true);
        }
        self.open_scoreboard_dialog(self.scoreboard_request());
        Ok(true)
    }

    pub(crate) fn runtime_client_list_input_geometry(
        &self,
    ) -> Option<(clonk_frontend::classic_gui::IntRect, i32)> {
        let dialog = self.runtime_client_list.as_ref()?;
        let font = &self.assets.clonk_fonts.as_deref()?.text;
        let preferred = scoreboard_preferred_rect(
            self.graphics
                .preferred_dialog_rect(self.mouse_control.then_some(self.local_owner)),
        );
        dialog.prepare_info_lines(preferred, font);
        Some((preferred, font.line_height))
    }

    pub(crate) fn game_over_pointer_route_hit(&self, point: GuiPoint) -> bool {
        self.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver)
            || self.game_over_dialog_contains_point(point)
            || self
                .game_over_dialog
                .as_ref()
                .is_some_and(GameOverState::has_pointer_capture)
    }

    pub(crate) fn runtime_client_list_strong_gamepad_callback_is_active(&self) -> bool {
        self.mode == AppMode::Running
            && self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| !dialog.is_info_only())
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList)
            && self.running_active_dialog == Some(RunningDialogStackEntry::RuntimeClientList)
            && (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
            && self.context_menu.is_none()
    }

    pub(crate) fn game_over_dialog_is_mouse_active(&self) -> bool {
        self.mode == AppMode::Running
            && self.game_over_dialog.is_some()
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver)
            && self.message_dialogs.is_empty()
            && self.game_option_input_dialog.is_none()
            && self.definition_selector.is_none()
            && self
                .network_start_wait
                .as_ref()
                .is_none_or(|wait| !wait.visible)
    }

    fn handle_runtime_client_list_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.runtime_client_list_consumed_keys.contains(&key) {
            let mut action = None;
            if state == ElementState::Released {
                self.runtime_client_list_consumed_keys.remove(&key);
                if let Some(gui_key) =
                    map_key_code(key).filter(|key| matches!(key, KeyCode::Enter | KeyCode::Space))
                {
                    action = self
                        .runtime_client_list
                        .as_mut()
                        .and_then(|dialog| dialog.handle_key_release(gui_key).1);
                }
            }
            if let Some(action) = action {
                self.handle_runtime_client_list_action(action)?;
            }
            return Ok(true);
        }
        let info_only = self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only());
        if self.mode == AppMode::Running {
            let active = self.running_active_dialog
                == Some(RunningDialogStackEntry::RuntimeClientList)
                && (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.context_menu.is_none();
            if !active {
                return Ok(false);
            }
            if !info_only {
                let modifiers = self.keyboard_modifiers
                    & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
                if key != VirtualKeyCode::Escape || !modifiers.is_empty() {
                    return Ok(false);
                }
                if state == ElementState::Released {
                    return Ok(false);
                }
                self.runtime_client_list_consumed_keys.insert(key);
                let action = self
                    .runtime_client_list
                    .as_mut()
                    .and_then(|dialog| dialog.handle_escape(true));
                if let Some(action) = action {
                    self.handle_runtime_client_list_action(action)?;
                }
                return Ok(true);
            }
        }
        if info_only {
            if state != ElementState::Pressed {
                return Ok(true);
            }
            self.runtime_client_list_consumed_keys.insert(key);
            let action = map_key_code(key)
                .filter(|key| {
                    matches!(
                        key,
                        KeyCode::Escape
                            | KeyCode::Enter
                            | KeyCode::Up
                            | KeyCode::Down
                            | KeyCode::Home
                            | KeyCode::End
                            | KeyCode::PageUp
                            | KeyCode::PageDown
                    )
                })
                .zip(self.runtime_client_list_input_geometry())
                .and_then(|(gui_key, (preferred, line_height))| {
                    self.runtime_client_list.as_mut().and_then(|dialog| {
                        dialog.handle_key(gui_key, false, preferred, line_height).1
                    })
                });
            if let Some(action) = action {
                self.handle_runtime_client_list_action(action)?;
            }
            return Ok(true);
        }
        if self.runtime_client_list.is_none() || state == ElementState::Released {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let valid_modifiers = if key == VirtualKeyCode::Tab {
            modifiers.is_empty() || modifiers == ModifiersState::SHIFT
        } else {
            modifiers.is_empty()
        };
        let Some(gui_key) = valid_modifiers.then(|| map_key_code(key)).flatten() else {
            return Ok(false);
        };
        if !matches!(
            gui_key,
            KeyCode::Escape
                | KeyCode::Tab
                | KeyCode::Up
                | KeyCode::Down
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown
                | KeyCode::Enter
                | KeyCode::Space
        ) {
            return Ok(false);
        }
        let Some((preferred, line_height)) = self.runtime_client_list_input_geometry() else {
            return Ok(false);
        };
        let (captured, action) = self
            .runtime_client_list
            .as_mut()
            .map(|dialog| {
                dialog.handle_key(
                    gui_key,
                    modifiers == ModifiersState::SHIFT,
                    preferred,
                    line_height,
                )
            })
            .unwrap_or_default();
        if captured {
            self.runtime_client_list_consumed_keys.insert(key);
        }
        if let Some(action) = action {
            self.handle_runtime_client_list_action(action)?;
        }
        Ok(captured)
    }

    pub(crate) fn handle_runtime_client_list_pointer_move(&mut self, point: GuiPoint) -> bool {
        let Some((preferred, line_height)) = self.runtime_client_list_input_geometry() else {
            return false;
        };
        self.runtime_client_list.as_mut().is_some_and(|dialog| {
            dialog.handle_pointer_move(point, preferred, line_height) || dialog.is_info_only()
        })
    }

    pub(crate) fn handle_runtime_client_list_pointer_button(
        &mut self,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let Some((preferred, line_height)) = self.runtime_client_list_input_geometry() else {
            return Ok(false);
        };
        let info_only = self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only());
        let Some(point) = self.running_pointer_position else {
            return Ok(info_only);
        };
        let (consumed, action) = self
            .runtime_client_list
            .as_mut()
            .map(|dialog| match state {
                ElementState::Pressed => (
                    dialog.handle_pointer_down(point, preferred, line_height),
                    None,
                ),
                ElementState::Released => {
                    let consumed = dialog.handle_pointer_move(point, preferred, line_height);
                    let action = dialog.handle_pointer_up(point, preferred, line_height);
                    (consumed, action)
                }
            })
            .unwrap_or_default();
        if consumed || info_only {
            if state == ElementState::Pressed {
                self.activate_running_dialog(RunningDialogStackEntry::RuntimeClientList);
            }
            self.suspend_ingame_pointer_for_gui();
            self.cancel_ingame_mouse_gestures();
        }
        if let Some(action) = action {
            self.play_ui_sound("Click");
            self.handle_runtime_client_list_action(action)?;
        }
        Ok(consumed || info_only)
    }

    fn handle_runtime_client_list_touch(
        &mut self,
        position: GuiPoint,
        phase: TouchPhase,
    ) -> Result<bool, EngineError> {
        if self.runtime_client_list.is_none() {
            return Ok(false);
        }
        self.running_pointer_position = Some(position);
        let move_captured = self.handle_runtime_client_list_pointer_move(position);
        let button_captured = match phase {
            TouchPhase::Started => {
                self.handle_runtime_client_list_pointer_button(ElementState::Pressed)?
            }
            TouchPhase::Ended => {
                self.handle_runtime_client_list_pointer_button(ElementState::Released)?
            }
            TouchPhase::Cancelled => {
                if let Some(dialog) = self.runtime_client_list.as_mut() {
                    dialog.pointer_left();
                }
                false
            }
            TouchPhase::Moved => false,
        };
        let captured = move_captured || button_captured;
        if captured {
            self.suspend_ingame_pointer_for_gui();
            self.cancel_ingame_mouse_gestures();
        }
        if captured && matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
            self.pointer_left_unchecked();
        }
        Ok(captured)
    }

    /// Handles modeled runtime-global keys through the per-game named
    /// registry, including exact Alt/Ctrl/Shift custom chords.
    /// `C4KeyCodeEx` masks Alt/Ctrl/Shift but has no platform Logo bit, so
    /// Logo alone retains the bare-key route. F1/F3 first give an exact
    /// custom local-player binding its higher `PRIO_PlrControl` refusal.
    ///
    fn handle_runtime_global_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<RuntimeGlobalKeyOutcome, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(RuntimeGlobalKeyOutcome::Unhandled);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let help_binding = self.runtime_keyboard_binding_matches(
            "ToggleShowHelp",
            key,
            key == VirtualKeyCode::F1 && c4_modifiers.is_empty(),
        );
        let music_binding = self.runtime_keyboard_binding_matches(
            "MusicToggle",
            key,
            key == VirtualKeyCode::F3 && c4_modifiers.is_empty(),
        );
        let speed_up_binding = self.runtime_keyboard_binding_matches(
            "GameSpeedUp",
            key,
            key == VirtualKeyCode::NumpadAdd && c4_modifiers == ModifiersState::SHIFT,
        );
        let speed_down_binding = self.runtime_keyboard_binding_matches(
            "GameSlowDown",
            key,
            key == VirtualKeyCode::NumpadSubtract && c4_modifiers == ModifiersState::SHIFT,
        );
        // X11/SDL update C4KeyboardInput::PressedKeys from the raw physical
        // edge before scope/priority dispatch. Keep the latch even when the
        // first down belongs to a global or modified route, so a later
        // in-scope AutoStop player binding sees the held-key repeat.
        let raw_repeated = (help_binding
            || music_binding
            || (!c4_modifiers.is_empty()
                && matches!(key, VirtualKeyCode::F1 | VirtualKeyCode::F3)))
            && match state {
                ElementState::Pressed => !self.pressed_engine_keys.insert(key),
                ElementState::Released => {
                    self.pressed_engine_keys.remove(&key);
                    false
                }
            };
        let screenshot = self.runtime_keyboard_binding_matches(
            "Screenshot",
            key,
            key == VirtualKeyCode::F9 && c4_modifiers.is_empty(),
        );
        let screenshot_ex = self.runtime_keyboard_binding_matches(
            "ScreenshotEx",
            key,
            key == VirtualKeyCode::F9 && c4_modifiers == ModifiersState::CONTROL,
        );
        if screenshot || screenshot_ex {
            if state == ElementState::Pressed {
                let kind = if screenshot_ex {
                    ScreenshotKind::FullLandscape
                } else {
                    ScreenshotKind::PresentedFrame
                };
                let gamma = self
                    .graphics
                    .active_gamma_ramp(&self.snapshot.environment.gamma);
                self.pending_screenshots
                    .push_back(ScreenshotRequest { kind, gamma });
            }
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        let scroll_up = self.runtime_keyboard_binding_matches(
            "MsgBoardScrollUp",
            key,
            key == VirtualKeyCode::ArrowUp && c4_modifiers == ModifiersState::SHIFT,
        );
        let scroll_down = self.runtime_keyboard_binding_matches(
            "MsgBoardScrollDown",
            key,
            key == VirtualKeyCode::ArrowDown && c4_modifiers == ModifiersState::SHIFT,
        );
        if !self.runtime_top_default_dialog_is_exclusive() && (scroll_up || scroll_down) {
            if state == ElementState::Pressed {
                self.scroll_message_board(scroll_up);
            }
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        let sound_binding = self.runtime_keyboard_binding_matches(
            "SoundToggle",
            key,
            key == VirtualKeyCode::F3 && c4_modifiers == ModifiersState::CONTROL,
        );
        if sound_binding {
            if state == ElementState::Pressed {
                self.toggle_sound_option()?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
        }
        if help_binding {
            if let Some(owner) = self.local_player_key_binding_owner_in_scope(key) {
                let control_style = self
                    .engine
                    .player(owner)
                    .is_some_and(|player| player.control_style());
                if state == ElementState::Released && !control_style {
                    self.pressed_engine_keys.remove(&key);
                    return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
                }
                self.dispatch_engine_key_binding(key, state, raw_repeated)?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            if state == ElementState::Released {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            if self.runtime_help_visible {
                self.runtime_help_visible = false;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            self.runtime_help_resources().map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::RuntimeHelpResources {
                        detail: error.to_string(),
                    },
                ))
            })?;
            self.runtime_help_visible = true;
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        if music_binding {
            if let Some(owner) = self.local_player_key_binding_owner_in_scope(key) {
                let control_style = self
                    .engine
                    .player(owner)
                    .is_some_and(|player| player.control_style());
                if state == ElementState::Released && !control_style {
                    self.pressed_engine_keys.remove(&key);
                    return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
                }
                self.dispatch_engine_key_binding(key, state, raw_repeated)?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            if state == ElementState::Released {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            self.toggle_runtime_music_playback()?;
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        let client_list_binding = self.runtime_keyboard_binding_matches(
            "NetClientListDlgToggle",
            key,
            key == VirtualKeyCode::F4 && c4_modifiers.is_empty(),
        );
        let pause_binding = self.runtime_keyboard_binding_matches(
            if self.console_mode {
                "ConsolePauseToggle"
            } else {
                "FullscreenPauseToggle"
            },
            key,
            key == VirtualKeyCode::Pause && c4_modifiers.is_empty(),
        );
        if client_list_binding {
            if state == ElementState::Released {
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            self.toggle_runtime_client_list()?;
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        let debug_keys = [
            (
                "DbgModeToggle",
                RuntimeDebugKey::Mode,
                key == VirtualKeyCode::F5 && c4_modifiers == ModifiersState::CONTROL,
            ),
            (
                "DbgShowVtxToggle",
                RuntimeDebugKey::Vertices,
                key == VirtualKeyCode::F6 && c4_modifiers == ModifiersState::CONTROL,
            ),
            (
                "DbgShowActionToggle",
                RuntimeDebugKey::ActionCycle,
                key == VirtualKeyCode::F7 && c4_modifiers == ModifiersState::CONTROL,
            ),
            (
                "DbgShowSolidMaskToggle",
                RuntimeDebugKey::SolidMask,
                key == VirtualKeyCode::F8 && c4_modifiers == ModifiersState::CONTROL,
            ),
        ]
        .into_iter()
        .filter_map(|(name, action, default_matches)| {
            self.runtime_keyboard_binding_matches(name, key, default_matches)
                .then_some(action)
        })
        .collect::<Vec<_>>();
        let mut denied_debug = false;
        if !debug_keys.is_empty() {
            // Player controls have PRIO_PlrControl and therefore own an exact
            // remapped collision before the PRIO_Base debug callbacks.
            if self.local_player_key_binding_owner_in_scope(key).is_some() {
                self.handle_engine_key(key, state)?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            // Context callbacks have PRIO_Context above these PRIO_Base
            // registrations. Let an owned navigation key or mnemonic reach
            // the menu before considering a remapped debug action.
            let active_context_key = c4_modifiers.is_empty()
                && self.context_menu.as_ref().is_some_and(|menu| {
                    context_menu_key_code(key).is_some_and(|key| menu.owns_key(key))
                        || context_menu_hotkey(key).is_some_and(|hotkey| menu.owns_hotkey(hotkey))
                });
            if active_context_key {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            let stronger_dialog_key = (self.running_chat_keyboard_active()
                && self.context_menu.is_none()
                && self.running_chat_key_has_higher_priority_route(key))
                || self.message_dialog_key_has_higher_priority_route(key)
                || self.game_over_key_has_higher_priority_route(key);
            if stronger_dialog_key {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            // ToggleChat was registered before every debug callback but is
            // routed later so active GUI owners can refuse it first.
            let earlier_toggle_chat = self.runtime_keyboard_binding_matches(
                "ToggleChat",
                key,
                key == VirtualKeyCode::KeyC && c4_modifiers == ModifiersState::ALT,
            );
            if earlier_toggle_chat {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            if state == ElementState::Pressed {
                for action in debug_keys {
                    if self.handle_runtime_debug_key(action)? {
                        return Ok(RuntimeGlobalKeyOutcome::Handled);
                    }
                    denied_debug = true;
                }
                // Native callbacks return false for a denied toggle. Keep
                // walking the registration list, including later debug keys.
            }
            if state == ElementState::Released {
                // The callbacks have no Up handler. Suppress a modified
                // physical release from leaking into modifier-blind fallback
                // controls.
                return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
            }
        }
        if speed_up_binding || speed_down_binding {
            // Native player controls have PRIO_PlrControl and therefore own
            // an exact custom collision before these PRIO_Base callbacks.
            if self.local_player_key_binding_owner_in_scope(key).is_some() {
                self.handle_engine_key(key, state)?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            // ToggleChat was registered before the speed callbacks but is
            // routed later so higher-priority GUI owners can refuse it.
            let earlier_toggle_chat = self.runtime_keyboard_binding_matches(
                "ToggleChat",
                key,
                key == VirtualKeyCode::KeyC && c4_modifiers == ModifiersState::ALT,
            );
            if earlier_toggle_chat {
                return Ok(RuntimeGlobalKeyOutcome::Unhandled);
            }
            if state == ElementState::Pressed {
                self.step_runtime_speed(speed_up_binding)?;
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            // The callbacks have no Up handler. Suppress a modified physical
            // release from leaking into modifier-blind fallback controls.
            return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
        }
        if pause_binding {
            if state == ElementState::Released || self.game_over_dialog.is_some() {
                return Ok(RuntimeGlobalKeyOutcome::Handled);
            }
            self.toggle_runtime_pause();
            return Ok(RuntimeGlobalKeyOutcome::Handled);
        }
        if !c4_modifiers.is_empty() {
            // Config.Controls stores the physical player key without a
            // modifier mask. C4KeyCodeEx therefore lets other modified
            // function keys continue to lower-priority UI handlers, but they
            // must not subsequently be interpreted by Rust's modifier-blind
            // player bindings.
            if matches!(key, VirtualKeyCode::F1 | VirtualKeyCode::F3) {
                return Ok(RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch);
            }
            return Ok(if denied_debug {
                RuntimeGlobalKeyOutcome::UnhandledAfterDeniedDebug
            } else {
                RuntimeGlobalKeyOutcome::Unhandled
            });
        }
        Ok(if denied_debug {
            RuntimeGlobalKeyOutcome::UnhandledAfterDeniedDebug
        } else {
            RuntimeGlobalKeyOutcome::Unhandled
        })
    }

    /// Runs the default-unbound PRIO_Base ChartToggle after the modeled
    /// higher- and earlier-priority native routes have had first refusal.
    pub(crate) fn handle_runtime_chart_toggle_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        if !matches!(self.mode, AppMode::Running) {
            return false;
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let unmodified_arrow = c4_modifiers.is_empty()
            && matches!(
                key,
                VirtualKeyCode::ArrowLeft
                    | VirtualKeyCode::ArrowRight
                    | VirtualKeyCode::ArrowUp
                    | VirtualKeyCode::ArrowDown
            );
        let edit_cursor_key = !c4_modifiers.contains(ModifiersState::ALT)
            && matches!(key, VirtualKeyCode::ArrowLeft | VirtualKeyCode::ArrowRight);
        let alt_hotkey_modifiers = c4_modifiers == ModifiersState::ALT
            || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
        let dialog_callbacks_active = self.context_menu.is_none();
        let active_context_key = c4_modifiers.is_empty()
            && self.context_menu.as_ref().is_some_and(|menu| {
                context_menu_key_code(key).is_some_and(|key| menu.owns_key(key))
                    || context_menu_hotkey(key).is_some_and(|hotkey| menu.owns_hotkey(hotkey))
            });
        let active_game_over_hotkey = dialog_callbacks_active
            && self
                .runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver)
                .then(|| startup_dialog_hotkey(key))
                .flatten()
                .and_then(|hotkey| {
                    self.game_over_dialog
                        .as_ref()
                        .and_then(|dialog| dialog.hotkey_action(hotkey))
                })
                .is_some()
            && alt_hotkey_modifiers;
        let active_game_over_list_key = dialog_callbacks_active
            && unmodified_arrow
            && matches!(key, VirtualKeyCode::ArrowUp | VirtualKeyCode::ArrowDown)
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::GameOver)
            && self.game_over_dialog.as_ref().is_some_and(|dialog| {
                matches!(dialog.focused(), Some(GameOverFocus::PlayerList(_)))
            });
        let active_vote_hotkey = dialog_callbacks_active
            && alt_hotkey_modifiers
            && self.top_message_dialog_is_exclusive()
            && message_dialog_hotkey(key).is_some_and(|hotkey| {
                self.active_message_dialog_index()
                    .and_then(|index| self.message_dialogs.get(index))
                    .is_some_and(|dialog| dialog.state.has_hotkey(hotkey))
            });
        let external_irc_control_key = dialog_callbacks_active
            && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc)
            && self.external_irc_dialog.as_ref().is_some_and(|dialog| {
                let edit_shortcut = c4_modifiers == ModifiersState::CONTROL
                    && matches!(
                        key,
                        VirtualKeyCode::KeyA
                            | VirtualKeyCode::KeyC
                            | VirtualKeyCode::KeyV
                            | VirtualKeyCode::KeyX
                    );
                let chat_page =
                    dialog.chat_page() == clonk_frontend::startup_netdlg::NetDlgChatPage::Chats;
                (dialog.chat_edit_is_focused() && (edit_cursor_key || edit_shortcut))
                    || (chat_page
                        && ((c4_modifiers.is_empty()
                            && matches!(key, VirtualKeyCode::ArrowUp | VirtualKeyCode::ArrowDown))
                            || (c4_modifiers == ModifiersState::CONTROL
                                && key == VirtualKeyCode::F4)))
                    || (alt_hotkey_modifiers
                        && context_menu_hotkey(key)
                            .is_some_and(|hotkey| dialog.chat_dialog_has_hotkey(hotkey)))
            });
        let chat_open_binding =
            !self.running_chat_active() && self.runtime_running_chat_open_mode(key).is_some();
        let toggle_chat_binding = self.runtime_keyboard_binding_matches(
            "ToggleChat",
            key,
            key == VirtualKeyCode::KeyC && c4_modifiers == ModifiersState::ALT,
        );
        let film_next_player_binding = self.engine.film_replay()
            && self.viewport_cycle_scope_available()
            && self.runtime_keyboard_binding_matches(
                "FilmNextPlayer",
                key,
                key == VirtualKeyCode::ArrowRight && c4_modifiers.is_empty(),
            );
        let free_view_scroll_binding = !self.engine.film_replay()
            && self.primary_physical_viewport_is_no_owner()
            && self.viewport_cycle_scope_available()
            && [
                ("FreeViewScrollLeft", VirtualKeyCode::ArrowLeft),
                ("FreeViewScrollRight", VirtualKeyCode::ArrowRight),
                ("FreeViewScrollUp", VirtualKeyCode::ArrowUp),
                ("FreeViewScrollDown", VirtualKeyCode::ArrowDown),
            ]
            .into_iter()
            .any(|(name, default_key)| {
                self.runtime_keyboard_binding_matches(
                    name,
                    key,
                    key == default_key && c4_modifiers.is_empty(),
                )
            });
        let ownerless_fullscreen = self.primary_physical_viewport_is_no_owner();
        let fullscreen_menu_binding = self.game_over_dialog.is_none()
            && self.running_chat_controller().is_none()
            && if self.ingame_menu_belongs_to(OWNER_NONE)
                || (ownerless_fullscreen && self.ingame_menu.is_some())
            {
                [
                    ("FullscreenMenuLeft", VirtualKeyCode::ArrowLeft),
                    ("FullscreenMenuRight", VirtualKeyCode::ArrowRight),
                    ("FullscreenMenuUp", VirtualKeyCode::ArrowUp),
                    ("FullscreenMenuDown", VirtualKeyCode::ArrowDown),
                    ("FullscreenMenuOK", VirtualKeyCode::Space),
                    ("FullscreenMenuOK", VirtualKeyCode::Enter),
                    ("FullscreenMenuCancel", VirtualKeyCode::Escape),
                ]
                .into_iter()
                .any(|(name, default_key)| {
                    self.runtime_keyboard_binding_matches(
                        name,
                        key,
                        key == default_key && c4_modifiers.is_empty(),
                    )
                })
            } else {
                ownerless_fullscreen
                    && self.runtime_keyboard_binding_matches(
                        "FullscreenMenuOpen",
                        key,
                        key == VirtualKeyCode::Space && c4_modifiers.is_empty(),
                    )
            };
        let game_abort_binding = self.runtime_keyboard_binding_matches(
            "GameAbort",
            key,
            key == VirtualKeyCode::Escape && c4_modifiers.is_empty(),
        );
        // These built-ins are registered before ChartToggle at PRIO_Base (or
        // are represented by a stronger callback) and therefore win an exact
        // duplicate custom chord.
        if chat_open_binding
            || toggle_chat_binding
            || film_next_player_binding
            || free_view_scroll_binding
            || fullscreen_menu_binding
            || game_abort_binding
            || (dialog_callbacks_active
                && self.running_chat_active()
                && (c4_modifiers == ModifiersState::CONTROL
                    || edit_cursor_key
                    || (c4_modifiers.is_empty()
                        && matches!(
                            key,
                            VirtualKeyCode::F2
                                | VirtualKeyCode::ArrowUp
                                | VirtualKeyCode::ArrowDown
                        ))))
            || active_context_key
            || active_game_over_hotkey
            || active_game_over_list_key
            || active_vote_hotkey
            || external_irc_control_key
        {
            return false;
        }
        let matches = self.runtime_key_config().is_ok_and(|config| {
            config
                .chart_toggle
                .iter()
                .any(|binding| binding.matches(key, self.keyboard_modifiers))
        });
        if !matches || self.local_player_key_binding_in_scope(key) {
            return false;
        }
        if state == ElementState::Pressed {
            self.toggle_network_chart();
        }
        true
    }

    /// Runs the default-unbound PRIO_Base NetStatsToggle, the last entry of
    /// the native "no default keys assigned" block (src/C4Game.cpp:3462).
    /// Every other PRIO_Base action registered before it — including
    /// ChartToggle — and the stronger PRIO_PlrControl player bindings own an
    /// exact duplicate chord first. Unlike its `C4GraphicsSystem::ToggleShow*`
    /// neighbours, `ToggleShowNetStatus` has no `Game.DebugMode` guard and
    /// flashes no message (src/C4GraphicsSystem.cpp:811-815).
    pub(crate) fn handle_runtime_net_stats_toggle_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        if !matches!(self.mode, AppMode::Running)
            || !self.runtime_keyboard_binding_matches("NetStatsToggle", key, false)
            || self.runtime_keyboard_binding_matches("ChartToggle", key, false)
            || self.local_player_key_binding_in_scope(key)
        {
            return false;
        }
        // The callback has no Up handler, so a release stays unprocessed.
        if state != ElementState::Pressed {
            return false;
        }
        let mut flags = self.graphics.debug_draw_flags();
        flags.show_net_status = !flags.show_net_status;
        self.graphics.set_debug_draw_flags(flags);
        true
    }

    pub(crate) fn handle_runtime_fullscreen_menu_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running)
            || self.game_over_dialog.is_some()
            || self.running_chat_controller().is_some()
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if self.ingame_menu_belongs_to(OWNER_NONE) {
            let command = [
                (
                    "FullscreenMenuLeft",
                    ControlCommand::MenuLeft,
                    key == VirtualKeyCode::ArrowLeft && modifiers.is_empty(),
                ),
                (
                    "FullscreenMenuRight",
                    ControlCommand::MenuRight,
                    key == VirtualKeyCode::ArrowRight && modifiers.is_empty(),
                ),
                (
                    "FullscreenMenuUp",
                    ControlCommand::MenuUp,
                    key == VirtualKeyCode::ArrowUp && modifiers.is_empty(),
                ),
                (
                    "FullscreenMenuDown",
                    ControlCommand::MenuDown,
                    key == VirtualKeyCode::ArrowDown && modifiers.is_empty(),
                ),
                (
                    "FullscreenMenuOK",
                    ControlCommand::MenuEnter,
                    matches!(key, VirtualKeyCode::Space | VirtualKeyCode::Enter)
                        && modifiers.is_empty(),
                ),
                (
                    "FullscreenMenuCancel",
                    ControlCommand::MenuClose,
                    key == VirtualKeyCode::Escape && modifiers.is_empty(),
                ),
            ]
            .into_iter()
            .find_map(|(name, command, default_matches)| {
                self.runtime_keyboard_binding_matches(name, key, default_matches)
                    .then_some(command)
            });
            if let Some(command) = command {
                if state == ElementState::Pressed {
                    self.handle_menu_command_failsafe(OWNER_NONE, command, CommandKind::Press)?;
                }
                return Ok(true);
            }
            return Ok(false);
        }
        if !self.primary_physical_viewport_is_no_owner()
            || !self.runtime_keyboard_binding_matches(
                "FullscreenMenuOpen",
                key,
                key == VirtualKeyCode::Space && modifiers.is_empty(),
            )
        {
            return Ok(false);
        }
        if state == ElementState::Pressed {
            self.ingame_menu.replace(
                OWNER_NONE,
                IngameMenuState::main_menu(
                    &self.main_menu_conditions_for(OWNER_NONE),
                    &self.ingame_menu_labels(),
                ),
            );
        }
        Ok(true)
    }

    fn runtime_custom_gamepad_button_action(
        &self,
        slot: u8,
        button: u8,
    ) -> Option<RuntimeCustomGamepadAction> {
        self.runtime_custom_gamepad_action(|name| {
            self.runtime_gamepad_button_override_matches(name, slot, button)
        })
    }

    fn runtime_custom_gamepad_direction_action(
        &self,
        slot: u8,
        direction: ControlButton,
    ) -> Option<RuntimeCustomGamepadAction> {
        self.runtime_custom_gamepad_action(|name| {
            self.runtime_gamepad_direction_override_matches(name, slot, direction)
        })
    }

    fn runtime_custom_gamepad_action(
        &self,
        matches: impl Fn(&str) -> bool,
    ) -> Option<RuntimeCustomGamepadAction> {
        if !matches!(self.mode, AppMode::Running)
            || self.running_chat_active()
            || self.game_over_dialog.is_some()
            || self.external_irc_dialog_visible
        {
            return None;
        }
        if matches("GameSpeedUp") {
            return Some(RuntimeCustomGamepadAction::SpeedUp);
        }
        if matches("GameSlowDown") {
            return Some(RuntimeCustomGamepadAction::SpeedDown);
        }
        if self.ingame_menu_belongs_to(OWNER_NONE) {
            for (name, command) in [
                ("FullscreenMenuLeft", ControlCommand::MenuLeft),
                ("FullscreenMenuRight", ControlCommand::MenuRight),
                ("FullscreenMenuUp", ControlCommand::MenuUp),
                ("FullscreenMenuDown", ControlCommand::MenuDown),
                ("FullscreenMenuOK", ControlCommand::MenuEnter),
                ("FullscreenMenuCancel", ControlCommand::MenuClose),
            ] {
                if matches(name) {
                    return Some(RuntimeCustomGamepadAction::Menu(command));
                }
            }
        } else if self.primary_physical_viewport_is_no_owner() && matches("FullscreenMenuOpen") {
            return Some(RuntimeCustomGamepadAction::MenuOpen);
        }
        for (name, mode) in [
            ("ChatOpen", RunningChatMode::All),
            ("ChatOpen2Allies", RunningChatMode::Allies),
            ("ChatOpen2Say", RunningChatMode::Say),
        ] {
            if matches(name) {
                return Some(RuntimeCustomGamepadAction::Chat(mode));
            }
        }
        if matches("ScoreboardToggle") {
            return Some(RuntimeCustomGamepadAction::Scoreboard);
        }
        if matches("GameAbort") {
            return Some(RuntimeCustomGamepadAction::Abort);
        }
        matches("ChartToggle").then_some(RuntimeCustomGamepadAction::Chart)
    }

    fn execute_runtime_custom_gamepad_action(
        &mut self,
        action: RuntimeCustomGamepadAction,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if state == ElementState::Released {
            return Ok(());
        }
        match action {
            RuntimeCustomGamepadAction::Chat(mode) => {
                if !self.running_chat_active() {
                    self.start_running_chat(mode);
                }
            }
            RuntimeCustomGamepadAction::Scoreboard => {
                self.reconcile_initial_scoreboard();
                self.sync_scoreboard_presentation();
                if !self.close_scoreboard_dialog()
                    && self.snapshot.hud.scoreboard.can_be_shown()
                    && !self.scoreboard_opening_blocked_by_game_over()
                {
                    self.open_scoreboard_dialog(self.scoreboard_request());
                }
            }
            RuntimeCustomGamepadAction::Abort => {
                if self.game_over_dialog.is_none() {
                    let dialog_owner = if self.primary_physical_viewport_is_no_owner() {
                        OWNER_NONE
                    } else {
                        self.local_owner
                    };
                    self.show_abort_dialog(dialog_owner);
                }
            }
            RuntimeCustomGamepadAction::Chart => self.toggle_network_chart(),
            RuntimeCustomGamepadAction::SpeedUp => self.step_runtime_speed(true)?,
            RuntimeCustomGamepadAction::SpeedDown => self.step_runtime_speed(false)?,
            RuntimeCustomGamepadAction::Menu(command) => {
                self.handle_menu_command_failsafe(OWNER_NONE, command, CommandKind::Press)?;
            }
            RuntimeCustomGamepadAction::MenuOpen => {
                self.ingame_menu.replace(
                    OWNER_NONE,
                    IngameMenuState::main_menu(
                        &self.main_menu_conditions_for(OWNER_NONE),
                        &self.ingame_menu_labels(),
                    ),
                );
            }
        }
        Ok(())
    }

    pub(crate) fn handle_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.startup_tooltip.note_non_pointer_input();
        self.note_classic_lobby_non_pointer_input();
        if let Some(dialog) = self.runtime_client_list.as_mut() {
            dialog.note_non_pointer_input();
        }
        self.context_menu_pointer_dismissed_lobby_team_player = None;
        self.context_menu_pointer_dismissed_lobby_option = None;
        if state == ElementState::Released && self.chat_paste_consumed_keys.remove(&key) {
            return Ok(());
        }
        if self.handle_options_control_capture_key(key, state)? {
            return Ok(());
        }
        if let Err(error) = self.guard_runtime_key_dispatch(key) {
            if key == VirtualKeyCode::Pause {
                // An unknown global KeyConfig may have rebound the physical
                // key. Refuse that default action without letting Pause cross
                // the event loop's fatal EngineError boundary.
                tracing::error!(%error, "suppressing Pause under unavailable runtime KeyConfig");
                return Ok(());
            }
            return Err(error);
        }
        if self.running_chat_keyboard_active() && self.context_menu.is_none() {
            let modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            let replacement_mode = self
                .runtime_running_chat_open_mode(key)
                .filter(|mode| !matches!(mode, RunningChatMode::All))
                .filter(|_| !self.running_chat_key_has_higher_priority_route(key));
            if let Some(replacement_mode) = replacement_mode {
                if state == ElementState::Pressed
                    && self.running_chat_text().is_some_and(str::is_empty)
                {
                    self.close_running_chat()?;
                    self.start_running_chat(replacement_mode);
                }
                return Ok(());
            }
            let edit_priority_key = (modifiers == ModifiersState::CONTROL
                && matches!(
                    key,
                    VirtualKeyCode::KeyA
                        | VirtualKeyCode::KeyC
                        | VirtualKeyCode::KeyV
                        | VirtualKeyCode::KeyX
                ))
                || (modifiers.control_key()
                    && !modifiers.alt_key()
                    && matches!(
                        key,
                        VirtualKeyCode::Backspace
                            | VirtualKeyCode::Delete
                            | VirtualKeyCode::End
                            | VirtualKeyCode::Home
                            | VirtualKeyCode::ArrowLeft
                            | VirtualKeyCode::ArrowRight
                    ));
            if modifiers == ModifiersState::CONTROL
                && !(key == VirtualKeyCode::F9 && modifiers == ModifiersState::CONTROL)
                && !edit_priority_key
            {
                self.handle_engine_key(key, state)?;
                return Ok(());
            }
            let empty_backspace = key == VirtualKeyCode::Backspace
                && self.running_chat_text().is_none_or(str::is_empty);
            if modifiers.is_empty()
                && (matches!(
                    key,
                    VirtualKeyCode::Tab
                        | VirtualKeyCode::ArrowUp
                        | VirtualKeyCode::ArrowDown
                        | VirtualKeyCode::F2
                ) || empty_backspace)
            {
                self.handle_running_chat_key(key, state)?;
                return Ok(());
            }
        }
        if self.handle_frontend_global_key(key, state)? {
            return Ok(());
        }
        if self.network_chart_dialog.is_some()
            && key == VirtualKeyCode::Escape
            && (self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT))
                .is_empty()
            && self.local_player_key_binding_in_scope(key)
        {
            // PRIO_PlrControl precedes the chart's PRIO_Dlg stronger Escape.
            self.handle_engine_key(key, state)?;
            return Ok(());
        }
        // C4ChartDialog's fullscreen PRIO_Dlg Escape callback precedes all
        // process-global PRIO_Base actions, including a configured
        // ChartToggle using the same physical key.
        if self.handle_network_chart_key(key, state) {
            return Ok(());
        }
        let (runtime_engine_dispatch_suppressed, denied_debug_callback) =
            match self.handle_runtime_global_key(key, state)? {
                RuntimeGlobalKeyOutcome::Handled => return Ok(()),
                RuntimeGlobalKeyOutcome::DownstreamWithoutEngineDispatch => (true, false),
                RuntimeGlobalKeyOutcome::Unhandled => (false, false),
                RuntimeGlobalKeyOutcome::UnhandledAfterDeniedDebug => (false, true),
            };
        if self.handle_scoreboard_key(key, state)? {
            return Ok(());
        }
        if self.handle_runtime_chart_toggle_key(key, state) {
            return Ok(());
        }
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if state == ElementState::Released
            && self.game_option_input_dialog.is_none()
            && self.game_option_input_consumed_keys.remove(&key)
        {
            // A context/button action may close the top input dialog on
            // key-down. Its matching release still belongs to that dialog,
            // not to a newly exposed message dialog underneath it.
            return Ok(());
        }
        let definition_release_latched =
            state == ElementState::Released && self.definition_selector_consumed_keys.remove(&key);
        let input_dialog_release_latched =
            state == ElementState::Released && self.game_option_input_consumed_keys.remove(&key);
        if self.running_chat_keyboard_active() {
            let context_menu_was_open = self.context_menu.is_some();
            let modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            let context_menu_handled =
                modifiers.is_empty() && self.handle_context_menu_key(key, state)?;
            if context_menu_handled || input_dialog_release_latched {
                return Ok(());
            }
            if context_menu_was_open {
                if self.handle_runtime_irc_toggle_key(key, state)? {
                    return Ok(());
                }
                // A popup makes its parent Edit and chat callbacks inactive,
                // but the lower-priority global chat-open bindings still run.
                // Reopening is only permitted while the existing input is
                // empty; custom lists replace the three default chords here
                // just as they do without the popup.
                let replacement_mode = self.runtime_running_chat_open_mode(key);
                let no_replacement_mode = replacement_mode.is_none();
                if let Some(replacement_mode) = replacement_mode.filter(|_| {
                    state == ElementState::Pressed
                        && self.running_chat_text().is_some_and(str::is_empty)
                }) {
                    self.close_running_chat()?;
                    self.start_running_chat(replacement_mode);
                }
                if no_replacement_mode
                    && self.handle_scoreboard_key_after_higher_priority(key, state)?
                {
                    return Ok(());
                }
                return Ok(());
            }
            if self.handle_game_option_input_dialog_key(key, state)? {
                return Ok(());
            }
        }
        let league_signup_release_latched =
            state == ElementState::Released && self.league_signup_consumed_keys.remove(&key);
        if league_signup_release_latched && !self.message_dialogs.is_empty() {
            return Ok(());
        }
        let message_dialog_was_open = !self.message_dialogs.is_empty();
        if self.handle_message_dialog_key(key, state)? {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() && self.context_menu.is_some() {
            let modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            if modifiers.is_empty() {
                let _ = self.handle_context_menu_key(key, state)?;
            }
            return Ok(());
        }
        if self.handle_league_signup_key(key, state)? {
            return Ok(());
        }
        if league_signup_release_latched {
            return Ok(());
        }
        if message_dialog_was_open && self.handle_running_chat_open_key(key, state) {
            return Ok(());
        }
        let lobby_client_info_owns_key = self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only())
            || self.runtime_client_list_consumed_keys.contains(&key);
        if lobby_client_info_owns_key && self.handle_runtime_client_list_key(key, state)? {
            return Ok(());
        }
        if self.external_irc_dialog_visible {
            let c4_modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            if c4_modifiers.is_empty() && self.handle_context_menu_key(key, state)? {
                return Ok(());
            }
            if self.context_menu.is_some() {
                if self.handle_runtime_irc_toggle_key(key, state)? {
                    return Ok(());
                }
                return Ok(());
            }
        }
        if (self.external_irc_dialog_visible || self.running_chat_active())
            && self.handle_runtime_irc_toggle_key(key, state)?
        {
            return Ok(());
        }
        if !message_dialog_was_open && self.handle_external_irc_dialog_key(key, state)? {
            return Ok(());
        }
        if !message_dialog_was_open
            && self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkGame
            && key == VirtualKeyCode::F4
            && self.keyboard_modifiers == ModifiersState::CONTROL
            && self.startup_network_dialog.as_ref().is_some_and(|dialog| {
                dialog.mode() == clonk_frontend::startup_netdlg::NetDlgMode::Chat
                    && dialog.chat_page() == clonk_frontend::startup_netdlg::NetDlgChatPage::Chats
            })
        {
            let actions = if state == ElementState::Pressed {
                self.startup_network_dialog
                    .as_mut()
                    .map(clonk_frontend::startup_netdlg::NetDlgController::close_active_chat_sheet)
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.process_network_dialog_actions(actions)?;
            return Ok(());
        }
        if self.handle_network_start_wait_key(key, state)? {
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let modifiers = self.keyboard_modifiers;
            let ctrl = modifiers.control_key();
            let shift = modifiers.shift_key();
            let edit_modifiers = !modifiers.intersects(ModifiersState::ALT | ModifiersState::SUPER);
            let control_only = modifiers == ModifiersState::CONTROL;
            let unmodified = modifiers.is_empty();
            let hotkey_modifiers = modifiers == ModifiersState::ALT
                || modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
            let actions = if state == ElementState::Pressed && hotkey_modifiers {
                context_menu_hotkey(key)
                    .and_then(|character| {
                        self.startup_options_advanced_dialog
                            .as_mut()
                            .map(|pending| pending.controller.handle_hotkey(character))
                    })
                    .unwrap_or_default()
            } else if key == VirtualKeyCode::Tab {
                if state == ElementState::Pressed {
                    if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                        if edit_modifiers && ctrl {
                            pending.controller.select_relative_section(shift);
                        } else if edit_modifiers && !ctrl {
                            pending.controller.handle_focus_step(shift);
                        }
                    }
                }
                Vec::new()
            } else if unmodified && matches!(key, VirtualKeyCode::PageUp | VirtualKeyCode::PageDown)
            {
                if state == ElementState::Pressed {
                    let delta = if key == VirtualKeyCode::PageUp {
                        10
                    } else {
                        -10
                    };
                    if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                        pending.controller.handle_integer_page_step(delta);
                    }
                }
                Vec::new()
            } else if edit_modifiers
                && state == ElementState::Pressed
                && key == VirtualKeyCode::Backspace
            {
                if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    pending
                        .controller
                        .handle_backspace_with_modifiers(ctrl, shift);
                }
                Vec::new()
            } else if edit_modifiers
                && state == ElementState::Pressed
                && key == VirtualKeyCode::Delete
            {
                if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    pending.controller.handle_delete(ctrl, shift);
                }
                Vec::new()
            } else if edit_modifiers
                && state == ElementState::Pressed
                && matches!(
                    key,
                    VirtualKeyCode::ArrowLeft
                        | VirtualKeyCode::ArrowRight
                        | VirtualKeyCode::Home
                        | VirtualKeyCode::End
                )
            {
                use clonk_frontend::rename_edit::RenameEditCursorOperation;
                let operation = match key {
                    VirtualKeyCode::ArrowLeft => RenameEditCursorOperation::Left,
                    VirtualKeyCode::ArrowRight => RenameEditCursorOperation::Right,
                    VirtualKeyCode::Home => RenameEditCursorOperation::Home,
                    VirtualKeyCode::End => RenameEditCursorOperation::End,
                    _ => unreachable!(),
                };
                if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    pending.controller.move_edit_cursor(operation, ctrl, shift);
                }
                Vec::new()
            } else if state == ElementState::Pressed && control_only && key == VirtualKeyCode::KeyA
            {
                if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    pending.controller.select_all_edit_text();
                }
                Vec::new()
            } else if state == ElementState::Pressed
                && control_only
                && matches!(key, VirtualKeyCode::KeyC | VirtualKeyCode::KeyX)
            {
                let selected = self
                    .startup_options_advanced_dialog
                    .as_ref()
                    .and_then(|pending| pending.controller.selected_edit_text())
                    .map(str::to_string);
                if let Some(selected) = selected {
                    match arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(selected))
                    {
                        Ok(()) if key == VirtualKeyCode::KeyX => {
                            if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                                pending.controller.delete_edit_selection();
                            }
                        }
                        Ok(()) => {}
                        Err(error) => {
                            tracing::warn!(%error, "failed to copy advanced setting text")
                        }
                    }
                }
                Vec::new()
            } else if state == ElementState::Pressed && control_only && key == VirtualKeyCode::KeyV
            {
                match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.get_text()) {
                    Ok(text) => {
                        if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                            pending.controller.handle_text_input(&text);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "failed to paste advanced setting text")
                    }
                }
                Vec::new()
            } else if let Some(gui_key) = unmodified.then(|| map_key_code(key)).flatten() {
                self.startup_options_advanced_dialog
                    .as_mut()
                    .map(|pending| match state {
                        ElementState::Pressed => pending.controller.handle_key_down(gui_key),
                        ElementState::Released => pending.controller.handle_key_up(gui_key),
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.process_options_advanced_actions(actions)?;
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let c4_modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            if state == ElementState::Pressed
                && c4_modifiers.is_empty()
                && key == VirtualKeyCode::F5
            {
                let location = self
                    .startup_player_properties_dialog
                    .as_ref()
                    .and_then(|pending| pending.controller.portrait_selector())
                    .filter(|selector| !selector.is_location_popup_open())
                    .and_then(|selector| {
                        selector.current_location().map(|location| {
                            (selector.current_location_index(), location.path.clone())
                        })
                    });
                if let Some((index, path)) = location {
                    self.reload_startup_player_portrait_location(index, &path);
                }
                return Ok(());
            }
            #[cfg(target_os = "windows")]
            let gui_key = map_key_code(key);
            #[cfg(not(target_os = "windows"))]
            let gui_key = (key != VirtualKeyCode::NumpadEnter)
                .then(|| map_key_code(key))
                .flatten();
            let portrait_ok_hotkey = state == ElementState::Pressed
                && key == VirtualKeyCode::KeyO
                && (c4_modifiers == ModifiersState::ALT
                    || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT))
                && self
                    .startup_player_properties_dialog
                    .as_ref()
                    .and_then(|pending| pending.controller.portrait_selector())
                    .is_some_and(|selector| !selector.is_location_popup_open());
            let exact_tab = gui_key == Some(KeyCode::Tab)
                && (c4_modifiers.is_empty() || c4_modifiers == ModifiersState::SHIFT);
            let alt_control_binding = c4_modifiers == ModifiersState::ALT
                && gui_key.is_some_and(|gui_key| {
                    self.startup_player_properties_dialog
                        .as_ref()
                        .and_then(|pending| pending.controller.portrait_selector())
                        .is_some_and(|selector| {
                            !selector.is_location_popup_open()
                                && match (selector.focus(), gui_key) {
                                    (
                                        clonk_frontend::startup_portraitsel::PortraitSelControl::Location,
                                        KeyCode::Down | KeyCode::Space,
                                    ) => true,
                                    (
                                        clonk_frontend::startup_portraitsel::PortraitSelControl::Grid,
                                        KeyCode::Enter,
                                    ) => selector.selected_index().is_some(),
                                    _ => false,
                                }
                        })
                });
            let actions = if state == ElementState::Pressed
                && c4_modifiers.is_empty()
                && matches!(key, VirtualKeyCode::Backspace | VirtualKeyCode::Delete)
            {
                if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                    pending.controller.delete_name_char();
                }
                Vec::new()
            } else if portrait_ok_hotkey {
                self.startup_player_properties_dialog
                    .as_mut()
                    .map(|pending| pending.controller.handle_key_down(KeyCode::Enter))
                    .unwrap_or_default()
            } else if exact_tab {
                self.startup_player_properties_dialog
                    .as_mut()
                    .map(|pending| match state {
                        ElementState::Pressed => {
                            pending.controller.handle_key_down_with_tab_direction(
                                KeyCode::Tab,
                                c4_modifiers == ModifiersState::SHIFT,
                            )
                        }
                        ElementState::Released => pending.controller.handle_key_up(KeyCode::Tab),
                    })
                    .unwrap_or_default()
            } else if c4_modifiers.is_empty() || alt_control_binding {
                self.startup_player_properties_dialog
                    .as_mut()
                    .zip(gui_key)
                    .map(|(pending, gui_key)| match state {
                        ElementState::Pressed => pending.controller.handle_key_down(gui_key),
                        ElementState::Released => pending.controller.handle_key_up(gui_key),
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.handle_definition_selector_key(key, state)? {
            return Ok(());
        }
        if definition_release_latched {
            return Ok(());
        }
        let context_menu_was_open = self.context_menu.is_some();
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if c4_modifiers.is_empty() && self.handle_context_menu_key(key, state)? {
            return Ok(());
        }
        if context_menu_was_open {
            if self.handle_running_chat_open_key(key, state) {
                return Ok(());
            }
            if self.handle_scoreboard_key_after_higher_priority(key, state)? {
                return Ok(());
            }
            return Ok(());
        }
        if self.handle_game_option_input_dialog_key(key, state)? {
            return Ok(());
        }
        if input_dialog_release_latched {
            return Ok(());
        }
        // Modal/context owners above the startup dialog remain active. Only
        // the fading base dialogs mirror Dialog::IsActive(false).
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.handle_startup_hotkey(key, state)? {
            return Ok(());
        }
        if self.handle_options_tab_key(key, state)? {
            return Ok(());
        }
        if self.handle_startup_tab_key(key, state)? {
            return Ok(());
        }
        if self.options_modified_gui_key_is_inert(key) {
            return Ok(());
        }
        let runtime_client_list_active = self.runtime_client_list_is_active()
            || self.runtime_client_list_consumed_keys.contains(&key);
        if runtime_client_list_active && self.handle_runtime_client_list_key(key, state)? {
            return Ok(());
        }
        if self.game_over_dialog_is_active() {
            // C4GUI compares the exact Alt/Ctrl/Shift mask for these global
            // bindings. The platform Logo bit is not part of C4KeyCodeEx.
            let c4_modifiers = self.keyboard_modifiers
                & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
            if c4_modifiers == ModifiersState::ALT
                || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT)
            {
                if state == ElementState::Pressed {
                    let action = startup_dialog_hotkey(key).and_then(|hotkey| {
                        self.game_over_dialog
                            .as_ref()
                            .and_then(|dialog| dialog.hotkey_action(hotkey))
                    });
                    if let Some(action) = action {
                        self.handle_game_over_action(action)?;
                        return Ok(());
                    }
                }
                // A visible button mnemonic has PRIO_Ctrl. Only an unmatched
                // exact Alt+Return continues to the lower Say-chat binding.
                if self.handle_running_chat_open_key(key, state) {
                    return Ok(());
                }
                return Ok(());
            }
            match (key, c4_modifiers, state) {
                (VirtualKeyCode::Tab, modifiers, ElementState::Pressed)
                    if modifiers.is_empty() || modifiers == ModifiersState::SHIFT =>
                {
                    if let Some(dialog) = self.game_over_dialog.as_mut() {
                        dialog.advance_focus(modifiers == ModifiersState::SHIFT);
                    }
                }
                (VirtualKeyCode::Tab, modifiers, ElementState::Released)
                    if modifiers.is_empty() || modifiers == ModifiersState::SHIFT => {}
                (VirtualKeyCode::F2, modifiers, event_state) if modifiers.is_empty() => {
                    self.handle_running_chat_open_key(key, event_state);
                }
                (VirtualKeyCode::Enter, modifiers, event_state)
                    if modifiers.is_empty()
                        && self.game_over_dialog.as_ref().is_some_and(|dialog| {
                            !matches!(
                                dialog.focused(),
                                Some(GameOverFocus::Close | GameOverFocus::Button(_))
                            )
                        }) =>
                {
                    self.handle_game_over_enter_chat(event_state);
                }
                (VirtualKeyCode::Enter, ModifiersState::SHIFT, event_state) => {
                    self.handle_running_chat_open_key(key, event_state);
                }
                (VirtualKeyCode::Enter, modifiers, ElementState::Pressed)
                    if modifiers.is_empty() =>
                {
                    let captured = self.game_over_dialog.as_mut().is_some_and(|dialog| {
                        dialog.handle_activation_down(GameOverActivationKey::Confirm)
                    });
                    let sounds = self
                        .game_over_dialog
                        .as_mut()
                        .map(GameOverState::take_sound_events)
                        .unwrap_or_default();
                    self.play_game_over_sound_events(sounds);
                    if !captured {
                        self.handle_game_over_enter_chat(state);
                    }
                }
                (VirtualKeyCode::Enter, modifiers, ElementState::Released)
                    if modifiers.is_empty() =>
                {
                    let action = self.game_over_dialog.as_mut().and_then(|dialog| {
                        dialog.handle_activation_up(GameOverActivationKey::Confirm)
                    });
                    let sounds = self
                        .game_over_dialog
                        .as_mut()
                        .map(GameOverState::take_sound_events)
                        .unwrap_or_default();
                    self.play_game_over_sound_events(sounds);
                    if let Some(action) = action {
                        self.handle_game_over_action(action)?;
                    }
                }
                (VirtualKeyCode::Space, modifiers, ElementState::Pressed)
                    if modifiers.is_empty() =>
                {
                    if let Some(dialog) = self.game_over_dialog.as_mut() {
                        dialog.handle_activation_down(GameOverActivationKey::Space);
                    }
                    let sounds = self
                        .game_over_dialog
                        .as_mut()
                        .map(GameOverState::take_sound_events)
                        .unwrap_or_default();
                    self.play_game_over_sound_events(sounds);
                }
                (VirtualKeyCode::Space, modifiers, ElementState::Released)
                    if modifiers.is_empty() =>
                {
                    let action = self.game_over_dialog.as_mut().and_then(|dialog| {
                        dialog.handle_activation_up(GameOverActivationKey::Space)
                    });
                    let sounds = self
                        .game_over_dialog
                        .as_mut()
                        .map(GameOverState::take_sound_events)
                        .unwrap_or_default();
                    self.play_game_over_sound_events(sounds);
                    if let Some(action) = action {
                        self.handle_game_over_action(action)?;
                    }
                }
                (VirtualKeyCode::Escape, modifiers, ElementState::Pressed)
                    if modifiers.is_empty()
                        && self
                            .game_over_dialog
                            .as_ref()
                            .is_some_and(GameOverState::allows_escape_close) =>
                {
                    self.handle_game_over_action(GameOverAction::End)?;
                }
                _ => {}
            }
            // Unrecognized dialog keys continue to the lower-priority
            // Generic chat registrations. This also covers custom physical
            // chords that are absent from the hard-coded dialog key arms.
            self.handle_running_chat_open_key(key, state);
            return Ok(());
        }
        if self.handle_runtime_irc_toggle_key(key, state)? {
            return Ok(());
        }
        if self.handle_running_chat_key(key, state)? {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_key(key, state);
        }
        if self.handle_joined_lobby_hotkey(key, state)? {
            return Ok(());
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.network_lobby.is_some()
            && key == VirtualKeyCode::ContextMenu
            && self.keyboard_modifiers.is_empty()
        {
            if state == ElementState::Pressed {
                self.handle_network_lobby_context_key()?;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.network_lobby.is_some()
            && self.handle_joined_lobby_roster_key(key, state)?
        {
            return Ok(());
        }
        if self.handle_network_lobby_chat_key(key, state)? {
            return Ok(());
        }
        if self.handle_joined_lobby_controller_key(key, state)? {
            return Ok(());
        }

        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    if state == ElementState::Pressed
                        && matches!(
                            key,
                            VirtualKeyCode::Enter
                                | VirtualKeyCode::NumpadEnter
                                | VirtualKeyCode::Space
                                | VirtualKeyCode::Escape
                        )
                    {
                        self.dismiss_game_over_dialog();
                    }
                    return Ok(());
                }
                if self.handle_network_edit_key(key, state)? {
                    return Ok(());
                }
                if self.startup_view == StartupView::PlayerSelection
                    && self.handle_startup_crew_rename_key(key, state)?
                {
                    return Ok(());
                }
                if self.startup_view == StartupView::ScenarioBrowser {
                    if self.handle_scenario_rename_key(key, state)? {
                        return Ok(());
                    }
                    if self.handle_scenario_selector_override_key(key, state)? {
                        return Ok(());
                    }
                    let discovery_loading = self.scenario_selector_discovery.is_some();
                    let discovery_modifiers = self.keyboard_modifiers
                        & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
                    if discovery_loading && !self.menu_state.search_focused() {
                        if state == ElementState::Pressed && discovery_modifiers.is_empty() {
                            match key {
                                VirtualKeyCode::Escape => self.close_scenario_browser(),
                                VirtualKeyCode::ArrowLeft => self.scensel_do_back()?,
                                _ => {}
                            }
                        }
                        return Ok(());
                    }
                    if !discovery_loading && self.handle_scenario_game_option_key(key, state)? {
                        return Ok(());
                    }
                    if self.scenario_game_options.focused_button().is_none()
                        && self.menu_state.dialog_focus() == ScenselDialogFocus::Back
                        && matches!(
                            key,
                            VirtualKeyCode::Enter
                                | VirtualKeyCode::NumpadEnter
                                | VirtualKeyCode::Space
                        )
                    {
                        if state == ElementState::Released {
                            self.scensel_do_back()?;
                        }
                        return Ok(());
                    }
                    if state == ElementState::Pressed
                        && key == VirtualKeyCode::ContextMenu
                        && self.open_scenario_search_context_menu(true)?
                    {
                        return Ok(());
                    }
                    if self.context_menu.is_none()
                        && key == VirtualKeyCode::KeyD
                        && self.keyboard_modifiers.alt_key()
                    {
                        if state == ElementState::Pressed
                            && self.menu_state.toggle_definition_checkbox()
                        {
                            self.play_ui_sound("ArrowHit");
                        }
                        // CheckBox::OnHotkey consumes its mnemonic even while
                        // disabled; ToggleCheck itself rejects the mutation.
                        return Ok(());
                    }
                    if self.context_menu.is_none() && self.menu_state.definition_checkbox_focused {
                        match (state, key) {
                            (ElementState::Pressed, VirtualKeyCode::Space) => {
                                if self.menu_state.toggle_definition_checkbox() {
                                    self.play_ui_sound("ArrowHit");
                                }
                                return Ok(());
                            }
                            (ElementState::Pressed, VirtualKeyCode::Tab) => {
                                self.menu_state.set_definition_checkbox_focused(false);
                                return Ok(());
                            }
                            (
                                _,
                                VirtualKeyCode::Space
                                | VirtualKeyCode::ArrowUp
                                | VirtualKeyCode::ArrowDown
                                | VirtualKeyCode::Home
                                | VirtualKeyCode::End
                                | VirtualKeyCode::PageUp
                                | VirtualKeyCode::PageDown
                                | VirtualKeyCode::Enter
                                | VirtualKeyCode::NumpadEnter,
                            ) => return Ok(()),
                            _ => {}
                        }
                    } else if self.context_menu.is_none()
                        && state == ElementState::Pressed
                        && key == VirtualKeyCode::Tab
                        && !self.menu_state.search_focused()
                        && self.menu_state.definition_checkbox_enabled
                    {
                        self.menu_state.set_definition_checkbox_focused(true);
                        return Ok(());
                    }
                    let search_shortcut = self.keyboard_modifiers.control_key()
                        || (cfg!(target_os = "macos")
                            && self.keyboard_modifiers.intersects(ModifiersState::SUPER));
                    if state == ElementState::Pressed
                        && key == VirtualKeyCode::KeyF
                        && search_shortcut
                        && self.context_menu.is_none()
                        && self.menu_state.current_map().is_none()
                    {
                        self.menu_state.set_search_focused(true);
                        self.menu_state.search_edit.select_all();
                        return Ok(());
                    }
                    if self.menu_state.search_focused() && self.context_menu.is_none() {
                        let ctrl = self.keyboard_modifiers.control_key();
                        let edit_shortcut = search_shortcut;
                        let shift = self.keyboard_modifiers.shift_key();
                        let consumed = match (state, key) {
                            (ElementState::Pressed, VirtualKeyCode::Backspace) => {
                                if self.menu_state.search_edit.backspace(ctrl, shift) {
                                    self.submit_scenario_search()?;
                                }
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::Delete)
                                if !self.keyboard_modifiers.alt_key() =>
                            {
                                if self.menu_state.search_edit.delete(ctrl, shift) {
                                    self.submit_scenario_search()?;
                                }
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::ArrowLeft) => {
                                self.menu_state.search_edit.move_cursor(
                                    SearchCursorOperation::Left,
                                    ctrl,
                                    shift,
                                );
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::ArrowRight) => {
                                self.menu_state.search_edit.move_cursor(
                                    SearchCursorOperation::Right,
                                    ctrl,
                                    shift,
                                );
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::Home) => {
                                self.menu_state.search_edit.move_cursor(
                                    SearchCursorOperation::Home,
                                    ctrl,
                                    shift,
                                );
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::End) => {
                                self.menu_state.search_edit.move_cursor(
                                    SearchCursorOperation::End,
                                    ctrl,
                                    shift,
                                );
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::KeyA) if edit_shortcut => {
                                self.menu_state.search_edit.select_all();
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::KeyC) if edit_shortcut => {
                                let _ = self.copy_search_edit_selection(false);
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::KeyX) if edit_shortcut => {
                                if self.copy_search_edit_selection(true) {
                                    self.submit_scenario_search()?;
                                }
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::KeyV) if edit_shortcut => {
                                self.paste_search_edit_clipboard()?;
                                true
                            }
                            (
                                ElementState::Pressed,
                                VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter,
                            ) => {
                                self.submit_scenario_search()?;
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::Escape) => {
                                if self.menu_state.search_text().is_empty() {
                                    self.menu_state.set_search_focused(false);
                                } else {
                                    self.menu_state.set_search_text("");
                                    self.submit_scenario_search()?;
                                }
                                true
                            }
                            (ElementState::Pressed, VirtualKeyCode::Tab) => {
                                self.menu_state.set_search_focused(false);
                                true
                            }
                            (
                                _,
                                VirtualKeyCode::Backspace
                                | VirtualKeyCode::Home
                                | VirtualKeyCode::End
                                | VirtualKeyCode::PageUp
                                | VirtualKeyCode::PageDown
                                | VirtualKeyCode::Enter
                                | VirtualKeyCode::NumpadEnter
                                | VirtualKeyCode::Escape
                                | VirtualKeyCode::Tab
                                | VirtualKeyCode::Space,
                            ) => true,
                            (_, VirtualKeyCode::Delete) if !self.keyboard_modifiers.alt_key() => {
                                true
                            }
                            (_, VirtualKeyCode::ArrowLeft | VirtualKeyCode::ArrowRight) => true,
                            (
                                _,
                                VirtualKeyCode::KeyA
                                | VirtualKeyCode::KeyC
                                | VirtualKeyCode::KeyX
                                | VirtualKeyCode::KeyV,
                            ) if edit_shortcut => true,
                            _ => false,
                        };
                        if consumed {
                            return Ok(());
                        }
                    }
                    if discovery_loading {
                        if state == ElementState::Pressed
                            && discovery_modifiers.is_empty()
                            && key == VirtualKeyCode::ArrowLeft
                        {
                            self.scensel_do_back()?;
                        }
                        return Ok(());
                    }
                    if self.handle_scensel_list_navigation_key(key, state)? {
                        return Ok(());
                    }
                }
                if state == ElementState::Pressed {
                    let no_shortcut_modifiers = (self.keyboard_modifiers
                        & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT))
                        .is_empty();
                    if self.startup_view == StartupView::NetworkGame
                        && key == VirtualKeyCode::F5
                        && no_shortcut_modifiers
                    {
                        self.process_network_dialog_actions(vec![
                            clonk_frontend::startup_netdlg::NetDlgAction::Refresh,
                        ])?;
                        return Ok(());
                    }
                    if self.startup_view == StartupView::PlayerSelection && no_shortcut_modifiers {
                        if key == VirtualKeyCode::ContextMenu
                            && self.open_startup_player_context_menu(true)?
                        {
                            return Ok(());
                        }
                        let actions = self
                            .startup_player_dialog
                            .as_ref()
                            .map(|dialog| match key {
                                VirtualKeyCode::Insert if !dialog.is_crew_mode() => {
                                    vec![clonk_frontend::startup_plrsel::PlrSelAction::NewPlayer]
                                }
                                VirtualKeyCode::Delete => dialog
                                    .selected_index()
                                    .map(|index| {
                                        if dialog.is_crew_mode() {
                                            clonk_frontend::startup_plrsel::PlrSelAction::DeleteCrew(
                                                index,
                                            )
                                        } else {
                                            clonk_frontend::startup_plrsel::PlrSelAction::DeletePlayer(
                                                index,
                                            )
                                        }
                                    })
                                    .into_iter()
                                    .collect(),
                                VirtualKeyCode::F2 => dialog.handle_edit_shortcut(),
                                _ => Vec::new(),
                            })
                            .unwrap_or_default();
                        if !actions.is_empty() {
                            self.process_player_dialog_actions(actions)?;
                            return Ok(());
                        }
                    }
                }
                if self.startup_view == StartupView::MainMenu
                    && state == ElementState::Pressed
                    && key == VirtualKeyCode::Escape
                {
                    self.request_exit();
                    return Ok(());
                }
                // `C4StartupMainDlg` registers bare F6 at control-override
                // priority within its own dialog scope
                // (C4StartupMainDlg.cpp:95-100). `SwitchToEditor` returning
                // false leaves the key unconsumed.
                if self.startup_view == StartupView::MainMenu
                    && state == ElementState::Pressed
                    && key == VirtualKeyCode::F6
                    && (self.keyboard_modifiers
                        & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT))
                        .is_empty()
                    && self.switch_to_editor()
                {
                    return Ok(());
                }
                if let Some(gui_key) = map_key_code(key) {
                    if self.handle_startup_dialog_key(gui_key, state)? {
                        return Ok(());
                    }
                    match self.startup_view {
                        StartupView::ScenarioBrowser => match state {
                            ElementState::Pressed => match gui_key {
                                // Dialog escape returns to the main screen
                                // (C4StartupScenSelDlg::OnClosed, cpp:1445-1463).
                                KeyCode::Escape => self.close_scenario_browser(),
                                // K_LEFT = KeyBack = DoBack(true): folder up,
                                // or close at root (cpp:1388,413,1705-1725).
                                KeyCode::Left => self.scensel_do_back()?,
                                // K_RIGHT = KeyForward = DoOK (cpp:1392,415).
                                KeyCode::Right => {
                                    if self.menu_state.current_map().is_some() {
                                        self.start_selected_map_scenario_from_ui()?;
                                    } else {
                                        self.handle_menu_input(|menu| {
                                            menu.menu().handle_key_down(KeyCode::Enter)
                                        })?;
                                        self.handle_menu_input(|menu| {
                                            menu.menu().handle_key_up(KeyCode::Enter)
                                        })?;
                                    }
                                }
                                KeyCode::Enter if self.menu_state.current_map().is_some() => {
                                    self.start_selected_map_scenario_from_ui()?;
                                }
                                _ if self.menu_state.current_map().is_some() => {}
                                _ => self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_down(gui_key)
                                })?,
                            },
                            ElementState::Released => {
                                if self.menu_state.current_map().is_none()
                                    && !matches!(
                                        gui_key,
                                        KeyCode::Escape | KeyCode::Left | KeyCode::Right
                                    )
                                {
                                    self.handle_menu_input(|menu| {
                                        menu.menu().handle_key_up(gui_key)
                                    })?
                                }
                            }
                        },
                        StartupView::NetworkGame | StartupView::PlayerSelection => {}
                        StartupView::MainMenu => {
                            let actions = match state {
                                ElementState::Pressed => {
                                    self.main_menu_state.handle_key_down(gui_key)
                                }
                                ElementState::Released => {
                                    self.main_menu_state.handle_key_up(gui_key)
                                }
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        StartupView::NetworkLobby => {
                            if let Some(action) = self
                                .network_lobby
                                .as_mut()
                                .and_then(|lobby| lobby.handle_key(gui_key, state))
                            {
                                self.process_lobby_action(action)?;
                                return Ok(());
                            }
                            match state {
                                ElementState::Pressed => self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_down(gui_key)
                                })?,
                                ElementState::Released => self
                                    .handle_menu_input(|menu| menu.menu().handle_key_up(gui_key))?,
                            }
                        }
                        StartupView::Options | StartupView::About => {}
                    }
                }
                Ok(())
            }
            AppMode::Running => {
                let c4_modifiers = self.keyboard_modifiers
                    & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
                if self.handle_runtime_fullscreen_menu_key(key, state)? {
                    return Ok(());
                }
                let game_abort = self.runtime_keyboard_binding_matches(
                    "GameAbort",
                    key,
                    key == VirtualKeyCode::Escape && c4_modifiers.is_empty(),
                );
                if game_abort {
                    if self.local_player_key_binding_in_scope(key) {
                        self.handle_engine_key(key, state)?;
                        return Ok(());
                    }
                    if state == ElementState::Released {
                        return Ok(());
                    }
                    if self.object_menu.is_some() {
                        self.close_object_menu();
                    } else if self.ingame_menu_belongs_to(self.local_owner) {
                        // Route through TryClose so submenus run their close
                        // command back to the main menu (C4Menu.cpp:317-334).
                        self.handle_menu_command_failsafe(
                            self.local_owner,
                            ControlCommand::MenuClose,
                            CommandKind::Press,
                        )?;
                    } else if self.ingame_menu_belongs_to(OWNER_NONE) {
                        // C4FullScreen owns the observer menu under NO_OWNER;
                        // closing it never queues a player control.
                        self.handle_menu_command_failsafe(
                            OWNER_NONE,
                            ControlCommand::MenuClose,
                            CommandKind::Press,
                        )?;
                    } else {
                        let dialog_owner = if self.primary_physical_viewport_is_no_owner() {
                            OWNER_NONE
                        } else {
                            self.local_owner
                        };
                        self.show_abort_dialog(dialog_owner);
                    }
                    return Ok(());
                }
                let viewport_scope_excludes_player_control =
                    self.viewport_scope_excludes_player_control();
                let unsupported_running_shortcut =
                    if state == ElementState::Pressed && c4_modifiers.is_empty() {
                        match key {
                            VirtualKeyCode::F5 => Some("F5"),
                            VirtualKeyCode::F6 => Some("F6"),
                            VirtualKeyCode::F7 => Some("F7"),
                            _ => None,
                        }
                    } else {
                        None
                    };
                if self.handle_viewport_player_cycle_key(key, state) {
                    return Ok(());
                }
                if self.handle_runtime_net_stats_toggle_key(key, state) {
                    return Ok(());
                }
                if let Some(key) = unsupported_running_shortcut.filter(|_| !denied_debug_callback) {
                    return Err(classic_parity_engine_error(report_classic_parity_boundary(
                        ClassicParityBoundary::RunningShortcut { key },
                    )));
                }
                if !runtime_engine_dispatch_suppressed && !viewport_scope_excludes_player_control {
                    self.handle_engine_key(key, state)?;
                }
                Ok(())
            }
            AppMode::Loading => Ok(()),
        }
    }

    fn handle_engine_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let already_pressed = match state {
            ElementState::Pressed => !self.pressed_engine_keys.insert(key),
            ElementState::Released => {
                self.pressed_engine_keys.remove(&key);
                false
            }
        };
        let repeated = engine_key_repeated(already_pressed, BACKEND_SYNTHESIZES_KEY_REPEAT);
        self.dispatch_engine_key_binding(key, state, repeated)
    }

    pub(crate) fn dispatch_engine_key_binding(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
        repeated: bool,
    ) -> Result<bool, EngineError> {
        let candidates = self.runtime_control_candidates_for_keyboard(key, state);
        let routing =
            self.local_controls
                .route_keyboard_candidates(candidates, state, repeated, |owner| {
                    self.engine
                        .player(owner)
                        .map(|player| player.control_style())
                });
        if let KeyboardRoutingOutcome::Consumed {
            owner: Some(owner),
            event: Some(event),
        } = routing
        {
            self.dispatch_control_event_for_local_player(owner, event)?;
        }
        Ok(!matches!(routing, KeyboardRoutingOutcome::Unhandled))
    }

    pub(crate) fn current_network_input_frame(&self) -> i32 {
        i32::try_from(self.engine.frame()).unwrap_or(i32::MAX)
    }

    fn handle_viewport_player_cycle_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        self.handle_viewport_player_cycle_key_at(key, state, Instant::now())
    }

    pub(crate) fn handle_viewport_player_cycle_key_at(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
        now: Instant,
    ) -> bool {
        if !self.viewport_cycle_scope_available() {
            return false;
        }
        if self.engine.film_replay() {
            return self.handle_film_view_key_for_mode(key, state, true);
        }
        if state != ElementState::Pressed || !self.primary_physical_viewport_is_no_owner() {
            return false;
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let requested = [
            (
                "FreeViewScrollLeft",
                Vector2::new(-5, 0),
                key == VirtualKeyCode::ArrowLeft && c4_modifiers.is_empty(),
            ),
            (
                "FreeViewScrollRight",
                Vector2::new(5, 0),
                key == VirtualKeyCode::ArrowRight && c4_modifiers.is_empty(),
            ),
            (
                "FreeViewScrollUp",
                Vector2::new(0, -5),
                key == VirtualKeyCode::ArrowUp && c4_modifiers.is_empty(),
            ),
            (
                "FreeViewScrollDown",
                Vector2::new(0, 5),
                key == VirtualKeyCode::ArrowDown && c4_modifiers.is_empty(),
            ),
        ]
        .into_iter()
        .find_map(|(name, requested, default_matches)| {
            self.runtime_keyboard_binding_matches(name, key, default_matches)
                .then_some(requested)
        });
        if let Some(requested) = requested {
            let applied = self.free_view_scroll_momentum.apply(requested, now);
            // Native mutates Viewports.front() after checking that a
            // NO_OWNER-classified viewport exists. The active camera may
            // not be projected yet, but the built-in callback still owns
            // this key ahead of a custom NetObsNextPlayer binding.
            if !self.graphics.scroll_observer_viewport(0, applied) {
                self.graphics.queue_primary_observer_scroll(applied);
            }
            return true;
        }
        let binding_matches = self.runtime_key_config().is_ok_and(|config| {
            config
                .net_observer_next_player
                .iter()
                .any(|binding| binding.matches(key, self.keyboard_modifiers))
        });
        if !binding_matches {
            return false;
        }
        self.cycle_primary_viewport_player(false);
        // ViewportNextPlayer reports the valid physical viewport dispatch as
        // handled even when an empty/one-player list leaves its owner intact.
        true
    }

    /// Exact `ToggleDebugMode` / `C4GraphicsSystem::ToggleShow*` callbacks.
    /// The existing frontend flags drive the native-shaped overlay renderers;
    /// every flash is prepared before the corresponding state mutation.
    fn handle_runtime_debug_key(&mut self, key: RuntimeDebugKey) -> Result<bool, EngineError> {
        if key != RuntimeDebugKey::Mode && !self.engine.debug_mode() {
            let flash =
                self.prepare_runtime_resource_flash(|resources| resources.no_debug_mode.clone());
            self.runtime_flash_message = flash;
            return Ok(false);
        }

        match key {
            RuntimeDebugKey::Mode => {
                let enabled = self.engine.debug_mode();
                if !self.engine.allow_debug() && !enabled {
                    let flash = self.prepare_runtime_resource_flash(|resources| {
                        resources.debug_mode_not_allowed.clone()
                    });
                    self.runtime_flash_message = flash;
                    return Ok(false);
                }
                let enabled = !enabled;
                let flash = self.prepare_runtime_resource_flash(|resources| {
                    resources.debug_mode_on_off(enabled)
                });
                self.engine.set_debug_mode(enabled);
                if !enabled {
                    self.graphics
                        .set_debug_draw_flags(clonk_frontend::DebugDrawFlags::default());
                }
                self.runtime_flash_message = flash;
            }
            RuntimeDebugKey::Vertices => {
                let mut flags = self.graphics.debug_draw_flags();
                flags.show_vertices = !flags.show_vertices;
                flags.show_entrance = !flags.show_entrance;
                let enabled = flags.show_vertices || flags.show_entrance;
                let flash = self.prepare_runtime_resource_flash(|resources| {
                    resources.on_off("Entrance+Vertices", enabled)
                });
                self.graphics.set_debug_draw_flags(flags);
                self.runtime_flash_message = flash;
            }
            RuntimeDebugKey::ActionCycle => {
                let mut flags = self.graphics.debug_draw_flags();
                let flash = if !(flags.show_action || flags.show_command || flags.show_pathfinder) {
                    flags.show_action = true;
                    self.prepare_runtime_resource_flash(|_| "Actions".to_string())
                } else if flags.show_action {
                    flags.show_action = false;
                    flags.show_command = true;
                    self.prepare_runtime_resource_flash(|_| "Commands".to_string())
                } else if flags.show_command {
                    flags.show_command = false;
                    flags.show_pathfinder = true;
                    self.prepare_runtime_resource_flash(|_| "Pathfinder".to_string())
                } else {
                    flags.show_pathfinder = false;
                    self.prepare_runtime_resource_flash(|resources| {
                        resources.on_off("Actions/Commands/Pathfinder", false)
                    })
                };
                self.graphics.set_debug_draw_flags(flags);
                self.runtime_flash_message = flash;
            }
            RuntimeDebugKey::SolidMask => {
                let mut flags = self.graphics.debug_draw_flags();
                flags.show_solid_mask = !flags.show_solid_mask;
                let enabled = flags.show_solid_mask;
                let flash = self.prepare_runtime_resource_flash(|resources| {
                    resources.on_off("SolidMasks", enabled)
                });
                self.graphics.set_debug_draw_flags(flags);
                self.runtime_flash_message = flash;
            }
        }
        Ok(true)
    }

    pub(crate) fn store_message_input_history(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(position) = self
            .message_input_history
            .iter()
            .position(|previous| previous == text)
        {
            self.message_input_history.remove(position);
        }
        self.message_input_history.push_front(text.to_string());
        self.message_input_history.truncate(20);
        let history = self
            .message_input_history
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.set_chat_history(history.clone());
        }
        if let Some(dialog) = self.external_irc_dialog.as_mut() {
            dialog.set_chat_history(history);
        }
    }

    pub(crate) fn process_message_input_text(&mut self, text: &str, store_history: bool) {
        if store_history {
            self.store_message_input_history(text);
        }
        if self.process_control_message_local_command(text) {
            return;
        }
        if is_team_message_syntax(text) && self.engine.team_distribution() == 4 {
            self.append_control_message_log(
                "Can't send team message: Teams not known.".to_string(),
                CONTROL_LOG_COLOR,
                None,
            );
            return;
        }
        let parsed_control = if self.mode == AppMode::Running {
            let player = self
                .snapshot
                .hud
                .local_players
                .first()
                .copied()
                .unwrap_or(-1);
            parse_running_message_control(
                text,
                player,
                self.engine.cinematic_film(),
                &self.snapshot,
            )
        } else {
            parse_lobby_message_control(text)
        };
        let control = match parsed_control {
            Ok(control) => control,
            Err(_error) if clonk_script::c4_string_bytes(text).first() == Some(&b'/') => {
                match self.process_running_chat_command(text) {
                    Ok(true) => {}
                    Ok(false) => self.append_unknown_running_command(text),
                    Err(error) => {
                        tracing::error!(%error, "failed to process classic game chat command");
                        self.status_text = format!("Unable to process chat command: {error}");
                    }
                }
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "classic game chat message is invalid");
                return;
            }
        };
        let Some(mut control) = control else {
            return;
        };
        if let Some(network) = self.network.as_ref() {
            if let Err(error) = network.submit_message(control) {
                tracing::error!(%error, "failed to submit classic game message");
            }
        } else {
            control.by_client = 0;
            self.record_control_packet(&clonk_engine::ControlPacket::Message(control.clone()));
            self.execute_message_control(control);
        }
    }

    fn message_dialog_key_has_higher_priority_route(&self, key: VirtualKeyCode) -> bool {
        let Some(active_index) = self.active_message_dialog_index() else {
            return false;
        };
        if !self.running_shared_gui_has_keyboard_focus() {
            return false;
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if modifiers.is_empty() {
            return matches!(
                key,
                VirtualKeyCode::Tab
                    | VirtualKeyCode::Escape
                    | VirtualKeyCode::Enter
                    | VirtualKeyCode::Space
            );
        }
        if modifiers == ModifiersState::SHIFT {
            return key == VirtualKeyCode::Tab;
        }
        if modifiers == ModifiersState::ALT
            || modifiers == (ModifiersState::ALT | ModifiersState::SHIFT)
        {
            return message_dialog_hotkey(key).is_some_and(|hotkey| {
                self.message_dialogs
                    .get(active_index)
                    .is_some_and(|dialog| dialog.state.has_hotkey(hotkey))
            });
        }
        false
    }

    fn game_over_key_has_higher_priority_route(&self, key: VirtualKeyCode) -> bool {
        if !self.game_over_dialog_is_active() {
            return false;
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let Some(dialog) = self.game_over_dialog.as_ref() else {
            return false;
        };
        if modifiers == ModifiersState::ALT
            || modifiers == (ModifiersState::ALT | ModifiersState::SHIFT)
        {
            return startup_dialog_hotkey(key)
                .and_then(|hotkey| dialog.hotkey_action(hotkey))
                .is_some();
        }
        match (key, modifiers) {
            (VirtualKeyCode::Tab, modifiers)
                if modifiers.is_empty() || modifiers == ModifiersState::SHIFT =>
            {
                true
            }
            (VirtualKeyCode::Enter, modifiers) if modifiers.is_empty() => true,
            (VirtualKeyCode::Space, modifiers) if modifiers.is_empty() => matches!(
                dialog.focused(),
                Some(GameOverFocus::Close | GameOverFocus::Button(_))
            ),
            (VirtualKeyCode::Escape, modifiers) if modifiers.is_empty() => {
                dialog.allows_escape_close()
            }
            (VirtualKeyCode::ArrowUp | VirtualKeyCode::ArrowDown, modifiers)
                if modifiers.is_empty() =>
            {
                matches!(dialog.focused(), Some(GameOverFocus::PlayerList(_)))
            }
            _ => false,
        }
    }

    pub(crate) fn process_gamepad_events(&mut self) -> Result<(), EngineError> {
        if !self.gamepad_input_enabled {
            return Ok(());
        }
        self.guard_classic_global_gui_bootstrap()?;
        #[cfg(test)]
        {
            self.gamepad_poll_count += 1;
        }
        let events = self.gamepads.poll();
        if let Some(calibrations) = self.gamepads.take_axis_calibration_update() {
            self.gamepad_bindings
                .replace_axis_calibrations(calibrations);
        }
        let gamepad_gui_control = self.gamepad_gui_control;
        self.process_sourced_gamepad_event_batch(events, gamepad_gui_control)
    }

    #[cfg(test)]
    pub(crate) fn process_gamepad_event_batch(
        &mut self,
        events: impl IntoIterator<Item = GamepadEvent>,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        let mut cluster = 0_u64;
        let mut started = false;
        let mut previous_was_axis = false;
        let mut sourced = Vec::new();
        for event in events {
            let starts_physical_cluster = matches!(
                event,
                GamepadEvent::GuiButton { .. } | GamepadEvent::Axis { .. }
            );
            let starts_cluster = !started
                || starts_physical_cluster
                || matches!(event, GamepadEvent::Direction { .. }) && !previous_was_axis;
            if starts_cluster {
                if started {
                    cluster = cluster.wrapping_add(1);
                }
                started = true;
            }
            previous_was_axis = matches!(event, GamepadEvent::Axis { .. });
            sourced.push(SourcedGamepadEvent {
                gamepad: 0,
                cluster,
                event,
            });
        }
        self.process_sourced_gamepad_event_batch(sourced, true)
    }

    pub(crate) fn process_sourced_gamepad_event_batch(
        &mut self,
        events: impl IntoIterator<Item = SourcedGamepadEvent>,
        gamepad_gui_control: bool,
    ) -> Result<(), EngineError> {
        use clonk_frontend::runtime_client_list::RuntimeClientListAction;

        if !self.gamepad_input_enabled {
            return Ok(());
        }
        self.guard_classic_global_gui_bootstrap()?;
        let events = events.into_iter().collect::<Vec<_>>();
        if !events.is_empty() {
            self.startup_tooltip.note_non_pointer_input();
            self.note_classic_lobby_non_pointer_input();
            if let Some(dialog) = self.runtime_client_list.as_mut() {
                dialog.note_non_pointer_input();
            }
            if self.startup_network_transition_blocks_input() {
                return Ok(());
            }
        }
        #[derive(Clone, Copy)]
        enum ClusterOwner {
            Suppressed,
            GamepadCapture,
            OptionsDevice,
            Message,
            LeagueSignup,
            RuntimeClientInfo,
            Definition,
            ContextPending,
            Context,
            Input,
            Properties,
            Advanced,
            Chat,
            NetworkChart,
            GameOver,
            Base,
        }

        let mut events = events.into_iter().peekable();
        while let Some(first) = events.next() {
            let gamepad = first.gamepad;
            let cluster = first.cluster;
            let source_slot = first.event.slot();
            let mut cluster_events = vec![first.event];
            while events
                .peek()
                .is_some_and(|event| event.gamepad == gamepad && event.cluster == cluster)
            {
                cluster_events.push(events.next().expect("peeked cluster event").event);
            }

            let game_over_active = self.game_over_dialog_is_active();
            let screen_gamepad_open = gamepad_gui_control && gamepad == 0;
            let options_input_scope =
                self.mode == AppMode::Menu && self.startup_view == StartupView::Options;
            let options_gamepad_open =
                options_input_scope && self.gamepads.options_open_slot() == Some(source_slot);
            let eligible_gamepad_gui = screen_gamepad_open;
            let network_chart_gamepad_open =
                eligible_gamepad_gui && self.network_chart_is_active_dialog();
            let chart_player_control_owns_cluster = network_chart_gamepad_open
                && cluster_events.iter().any(|event| match event {
                    GamepadEvent::Button { slot, button, .. } => {
                        self.gamepad_player_button_in_scope(*slot, *button)
                    }
                    GamepadEvent::Axis { slot, axis, .. } => {
                        self.gamepad_player_axis_in_scope(*slot, *axis)
                    }
                    _ => false,
                });
            let gamepad_capture_open = self.message_dialogs.last().is_some_and(|pending| {
                matches!(
                    pending.continuation,
                    MessageDialogContinuation::OptionsControlCapture(target)
                        if target.device
                            == clonk_frontend::startup_options_controls::ControlDevice::Gamepad
                )
            });
            let mut owner = if network_chart_gamepad_open {
                ClusterOwner::NetworkChart
            } else if self.running_chat_keyboard_active() {
                if self.context_menu.is_some() {
                    ClusterOwner::ContextPending
                } else {
                    // The z=+2 chat dialog and its raw KEY_Any callback sit
                    // above default-z runtime dialogs for every gamepad.
                    ClusterOwner::Chat
                }
            } else if game_over_active && !eligible_gamepad_gui {
                // The exclusive evaluation screen owns the whole GUI stack,
                // but C++ did not register a receiver for this source.
                ClusterOwner::Suppressed
            } else if gamepad_capture_open {
                ClusterOwner::GamepadCapture
            } else if options_input_scope && !screen_gamepad_open {
                if options_gamepad_open {
                    // The selected device is live, but ControlConfigArea's
                    // opener does not grant GUI bindings. Those remain gated
                    // to configured gamepad 0 by C4GUI controls.
                    ClusterOwner::OptionsDevice
                } else {
                    // Keep every alias from an unopened source out of the
                    // Options consumer. Capture above deliberately sees all
                    // sources so it can consume and reject a wrong pad.
                    ClusterOwner::Suppressed
                }
            } else if self.context_menu.is_some() {
                ClusterOwner::ContextPending
            } else if self.message_dialog_owns_gamepad_input() {
                ClusterOwner::Message
            } else if self.league_signup_dialog.is_some() {
                ClusterOwner::LeagueSignup
            } else if self.runtime_client_list_keyboard_active()
                && self
                    .runtime_client_list
                    .as_ref()
                    .is_some_and(|dialog| dialog.is_info_only())
            {
                ClusterOwner::RuntimeClientInfo
            } else if self.external_irc_dialog_visible {
                // C4ChatDlg is the active shared-screen dialog. It has no
                // legacy gamepad navigation callbacks, but the raw cluster
                // must not leak to the lobby or running game behind it.
                ClusterOwner::Suppressed
            } else if self.definition_selector.is_some() {
                ClusterOwner::Definition
            } else if self.game_option_input_dialog.is_some() {
                ClusterOwner::Input
            } else if self.startup_player_properties_dialog.is_some() {
                if eligible_gamepad_gui {
                    ClusterOwner::Properties
                } else {
                    ClusterOwner::Suppressed
                }
            } else if self.startup_options_advanced_dialog.is_some() {
                if eligible_gamepad_gui {
                    ClusterOwner::Advanced
                } else {
                    ClusterOwner::Suppressed
                }
            } else if game_over_active {
                ClusterOwner::GameOver
            } else if self.startup_dialog_fade_active()
                && self.startup_player_properties_dialog.is_none()
            {
                ClusterOwner::Suppressed
            } else {
                ClusterOwner::Base
            };
            let mut suppress_base_select_alias = false;
            let mut suppress_base_cancel_alias = false;
            let mut previous_was_axis = false;

            for event in cluster_events {
                let axis_alias =
                    previous_was_axis && matches!(event, GamepadEvent::Direction { .. });
                previous_was_axis = matches!(event, GamepadEvent::Axis { .. });
                let mut pending = Some(event);
                while let Some(event) = pending.take() {
                    match owner {
                        ClusterOwner::Suppressed => {}
                        ClusterOwner::GamepadCapture => {
                            // KeySelDialog's high-priority raw listener owns the
                            // entire physical input cluster. GUI aliases emitted
                            // before the raw button must neither cancel nor
                            // activate the modal, and a wrong-pad raw button is
                            // consumed while leaving capture open.
                            self.handle_gamepad_raw_event(event)?;
                        }
                        ClusterOwner::OptionsDevice => {
                            // Preserve the selected device's raw-key path
                            // without turning selection into permission to
                            // operate C4GUI. In startup mode this currently
                            // has no gameplay receiver, just like C++.
                            self.handle_gamepad_raw_event(event)?;
                        }
                        ClusterOwner::Message => {
                            if !self.handle_gamepad_raw_event(event)? {
                                self.handle_message_dialog_gamepad_event(event)?;
                            }
                        }
                        ClusterOwner::LeagueSignup => {
                            if !self.handle_gamepad_raw_event(event)? {
                                self.handle_league_signup_gamepad_event(event)?;
                            }
                        }
                        ClusterOwner::RuntimeClientInfo => match event {
                            GamepadEvent::GuiButton {
                                class: GuiButtonClass::High,
                                state: ElementState::Pressed,
                                ..
                            }
                            | GamepadEvent::Action {
                                action: GamepadActionType::Cancel,
                                state: ElementState::Pressed,
                                ..
                            } => {
                                if self
                                    .runtime_client_list
                                    .as_ref()
                                    .is_some_and(|dialog| dialog.is_info_only())
                                {
                                    self.handle_runtime_client_list_action(
                                        RuntimeClientListAction::CloseInfo,
                                    )?;
                                }
                            }
                            GamepadEvent::Clear { .. } => {
                                if let Some(dialog) = self.runtime_client_list.as_mut() {
                                    dialog.pointer_left();
                                }
                            }
                            GamepadEvent::Axis { .. }
                            | GamepadEvent::Direction { .. }
                            | GamepadEvent::Button { .. }
                            | GamepadEvent::GuiButton { .. }
                            | GamepadEvent::Action { .. } => {}
                        },
                        ClusterOwner::Definition => {
                            self.handle_definition_selector_gamepad_event(event)?;
                        }
                        ClusterOwner::ContextPending => {
                            if matches!(event, GamepadEvent::Axis { .. }) {
                                // A raw axis precedes its semantic GUI alias
                                // in the same physical-input cluster. Keep the
                                // context pending until that direction arrives.
                                continue;
                            }
                            if self.handle_context_menu_gamepad_event(event)? {
                                owner = ClusterOwner::Context;
                            } else {
                                owner = if self.context_menu.is_some() {
                                    // A root context may decline Left without
                                    // closing. C4GUI still keeps its parent
                                    // dialog inactive until the context is
                                    // gone, so this raw cluster cannot fall
                                    // through to evaluation focus traversal.
                                    ClusterOwner::Suppressed
                                } else if self.running_chat_keyboard_active() {
                                    // An open context makes its parent dialog
                                    // keyboard-inactive, including the chat's
                                    // raw gamepad forwarding callback.
                                    ClusterOwner::Suppressed
                                } else if self.message_dialog_owns_gamepad_input() {
                                    ClusterOwner::Message
                                } else if self.runtime_client_list_keyboard_active()
                                    && self
                                        .runtime_client_list
                                        .as_ref()
                                        .is_some_and(|dialog| dialog.is_info_only())
                                {
                                    ClusterOwner::RuntimeClientInfo
                                } else if self.game_option_input_dialog.is_some() {
                                    ClusterOwner::Input
                                } else if self.league_signup_dialog.is_some() {
                                    ClusterOwner::LeagueSignup
                                } else if self.startup_player_properties_dialog.is_some() {
                                    if eligible_gamepad_gui {
                                        ClusterOwner::Properties
                                    } else {
                                        ClusterOwner::Suppressed
                                    }
                                } else if self.game_over_dialog_is_active() {
                                    ClusterOwner::GameOver
                                } else if (self.startup_dialog_fade_active()
                                    && self.startup_player_properties_dialog.is_none())
                                    || self.external_irc_dialog_visible
                                {
                                    ClusterOwner::Suppressed
                                } else if eligible_gamepad_gui
                                    && self.network_chart_is_active_dialog()
                                {
                                    ClusterOwner::NetworkChart
                                } else {
                                    ClusterOwner::Base
                                };
                                pending = Some(event);
                            }
                        }
                        ClusterOwner::Context => {}
                        ClusterOwner::Input => {
                            self.handle_game_option_input_dialog_gamepad_event(event)?;
                        }
                        ClusterOwner::Properties => match event {
                            GamepadEvent::GuiButton {
                                class: GuiButtonClass::Low,
                                ..
                            } => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?;
                                // C4GUI's AnyLowButton binding owns every
                                // semantic alias from this physical input.
                                owner = ClusterOwner::Suppressed;
                            }
                            GamepadEvent::GuiButton {
                                class: GuiButtonClass::High,
                                ..
                            } => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?;
                                // C4GUI's AnyHighButton binding likewise owns
                                // Select's MenuToggle alias and every remap.
                                owner = ClusterOwner::Suppressed;
                            }
                            GamepadEvent::Action {
                                action: GamepadActionType::Select,
                                ..
                            } if suppress_base_select_alias => {}
                            GamepadEvent::Action {
                                action: GamepadActionType::Cancel,
                                ..
                            } if suppress_base_cancel_alias => {}
                            event => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?
                            }
                        },
                        ClusterOwner::Advanced => match event {
                            GamepadEvent::GuiButton {
                                class: GuiButtonClass::Low,
                                ..
                            } => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?;
                                suppress_base_select_alias = true;
                            }
                            GamepadEvent::GuiButton {
                                class: GuiButtonClass::High,
                                ..
                            } => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?;
                                suppress_base_cancel_alias = true;
                            }
                            GamepadEvent::Action {
                                action: GamepadActionType::Select,
                                ..
                            } if suppress_base_select_alias => {}
                            GamepadEvent::Action {
                                action: GamepadActionType::Cancel,
                                ..
                            } if suppress_base_cancel_alias => {}
                            event => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?
                            }
                        },
                        ClusterOwner::Chat => match event {
                            event @ (GamepadEvent::Axis { .. }
                            | GamepadEvent::Direction { .. }
                            | GamepadEvent::Button { .. }
                            | GamepadEvent::Clear { .. }) => {
                                self.handle_gamepad_event_with_axis_alias(event, axis_alias)?;
                            }
                            GamepadEvent::GuiButton { .. } | GamepadEvent::Action { .. } => {}
                        },
                        ClusterOwner::NetworkChart => {
                            if chart_player_control_owns_cluster {
                                self.handle_gamepad_raw_event(event)?;
                            } else {
                                match event {
                                    GamepadEvent::GuiButton {
                                        class: GuiButtonClass::High,
                                        state: ElementState::Pressed,
                                        ..
                                    } => {
                                        self.toggle_network_chart();
                                        owner = ClusterOwner::Suppressed;
                                    }
                                    GamepadEvent::GuiButton {
                                        class: GuiButtonClass::High,
                                        ..
                                    }
                                    | GamepadEvent::Action {
                                        action: GamepadActionType::Cancel,
                                        ..
                                    } => {}
                                    event => self
                                        .handle_gamepad_event_with_axis_alias(event, axis_alias)?,
                                }
                            }
                        }
                        ClusterOwner::GameOver => {
                            self.handle_game_over_gamepad_event(event)?;
                        }
                        ClusterOwner::Base => {
                            if suppress_base_select_alias
                                && matches!(
                                    event,
                                    GamepadEvent::Action {
                                        action: GamepadActionType::Select,
                                        ..
                                    }
                                )
                            {
                                continue;
                            }
                            if suppress_base_cancel_alias
                                && matches!(
                                    event,
                                    GamepadEvent::Action {
                                        action: GamepadActionType::Cancel,
                                        ..
                                    }
                                )
                            {
                                continue;
                            }
                            let scenario_rename_active = self.mode == AppMode::Menu
                                && self.startup_view == StartupView::ScenarioBrowser
                                && self.menu_state.rename_edit.is_some();
                            let crew_rename_active = self.mode == AppMode::Menu
                                && self.startup_view == StartupView::PlayerSelection
                                && self.startup_crew_rename.is_some();
                            let rename_active = scenario_rename_active || crew_rename_active;
                            let rename_owns_raw_gui_button = eligible_gamepad_gui && rename_active;
                            let scenario_rename_owns_raw_gui_button =
                                eligible_gamepad_gui && scenario_rename_active;
                            let options_owns_raw_gui_button = eligible_gamepad_gui
                                && self.mode == AppMode::Menu
                                && self.startup_view == StartupView::Options
                                && self.startup_options_advanced_dialog.is_none();
                            match event {
                                GamepadEvent::GuiButton {
                                    class: GuiButtonClass::High,
                                    state: ElementState::Pressed,
                                    ..
                                } if self
                                    .runtime_client_list_strong_gamepad_callback_is_active() =>
                                {
                                    let action = self
                                        .runtime_client_list
                                        .as_mut()
                                        .and_then(|dialog| dialog.handle_escape(true));
                                    if let Some(action) = action {
                                        self.handle_runtime_client_list_action(action)?;
                                    }
                                    // C4Network2ClientListDlg's stronger
                                    // AnyHighButton callback owns the complete
                                    // raw cluster, including Cancel/MenuToggle
                                    // aliases emitted after the physical key.
                                    owner = ClusterOwner::Suppressed;
                                }
                                GamepadEvent::Direction {
                                    button: ControlButton::Left | ControlButton::Right,
                                    ..
                                } if !eligible_gamepad_gui
                                    && self.mode == AppMode::Menu
                                    && matches!(
                                        self.startup_view,
                                        StartupView::NetworkGame
                                            | StartupView::PlayerSelection
                                            | StartupView::Options
                                            | StartupView::About
                                    ) =>
                                {
                                    // Dialog registers its gamepad focus keys
                                    // only for configured primary-GUI input.
                                }
                                GamepadEvent::Direction { .. }
                                    if rename_active && !eligible_gamepad_gui =>
                                {
                                    // Dialog registered no gamepad focus keys
                                    // for a disabled or non-primary source.
                                }
                                GamepadEvent::GuiButton {
                                    class: GuiButtonClass::High,
                                    state,
                                    ..
                                } if rename_owns_raw_gui_button => {
                                    if state == ElementState::Pressed {
                                        if scenario_rename_active {
                                            self.abort_scenario_rename();
                                        } else {
                                            self.abort_startup_crew_rename();
                                        }
                                    }
                                    // RenameEdit's AnyHighButton binding owns
                                    // the complete physical cluster, including
                                    // a possible Cancel/MenuToggle alias.
                                    owner = ClusterOwner::Suppressed;
                                }
                                GamepadEvent::GuiButton {
                                    class: GuiButtonClass::Low,
                                    state,
                                    ..
                                } if scenario_rename_owns_raw_gui_button => {
                                    if state == ElementState::Pressed {
                                        self.activate_scensel_after_gamepad_low_rename_abort()?;
                                    }
                                    // Dialog's AnyLowButton binding owns the
                                    // cluster and calls DoOK, whose first step
                                    // is AbortRenaming. Do not also route an
                                    // abstract Select/Cancel alias.
                                    owner = ClusterOwner::Suppressed;
                                }
                                GamepadEvent::GuiButton {
                                    class: GuiButtonClass::Low,
                                    state,
                                    ..
                                } if options_owns_raw_gui_button => {
                                    let actions = self
                                        .startup_options_dialog
                                        .as_mut()
                                        .map(|dialog| match state {
                                            ElementState::Pressed => {
                                                dialog.handle_gamepad_low_down()
                                            }
                                            ElementState::Released => {
                                                dialog.handle_gamepad_low_up()
                                            }
                                        })
                                        .unwrap_or_default();
                                    self.process_options_dialog_actions(actions)?;
                                    // Every AnyLowButton may also produce the
                                    // abstract Select event in this cluster.
                                    suppress_base_select_alias = true;
                                }
                                GamepadEvent::GuiButton {
                                    class: GuiButtonClass::High,
                                    state,
                                    ..
                                } if options_owns_raw_gui_button => {
                                    let actions = if state == ElementState::Pressed {
                                        self.startup_options_dialog
                                            .as_mut()
                                            .map(|dialog| dialog.handle_gamepad_high_down())
                                            .unwrap_or_default()
                                    } else {
                                        Vec::new()
                                    };
                                    self.process_options_dialog_actions(actions)?;
                                    suppress_base_cancel_alias = true;
                                }
                                event => {
                                    self.handle_gamepad_event_with_axis_alias(event, axis_alias)?
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_message_dialog_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<(), EngineError> {
        let Some(active_index) = self.active_message_dialog_index() else {
            return Ok(());
        };
        let result = match event {
            GamepadEvent::Direction {
                button: button @ (ControlButton::Left | ControlButton::Right),
                state: ElementState::Pressed,
                ..
            } => self
                .message_dialogs
                .get_mut(active_index)
                .and_then(|dialog| {
                    dialog
                        .state
                        .handle_key_down(KeyCode::Tab, button == ControlButton::Left)
                }),
            GamepadEvent::GuiButton {
                class: GuiButtonClass::Low,
                state,
                ..
            } => self
                .message_dialogs
                .get_mut(active_index)
                .and_then(|dialog| match state {
                    ElementState::Pressed => dialog.state.handle_gamepad_low_down(),
                    ElementState::Released => dialog.state.handle_gamepad_low_up(),
                }),
            GamepadEvent::GuiButton {
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
                ..
            } => self
                .message_dialogs
                .get_mut(active_index)
                .and_then(|dialog| dialog.state.handle_key_down(KeyCode::Escape, false)),
            GamepadEvent::Clear { .. } => {
                if let Some(dialog) = self.message_dialogs.get_mut(active_index) {
                    dialog.state.cancel_interaction();
                }
                if self.message_dialog_pointer_capture_index == Some(active_index) {
                    self.message_dialog_pointer_capture_index = None;
                }
                None
            }
            GamepadEvent::Axis { .. }
            | GamepadEvent::Direction { .. }
            | GamepadEvent::Button { .. }
            | GamepadEvent::Action { .. }
            | GamepadEvent::GuiButton { .. } => None,
        };
        let sounds = self
            .message_dialogs
            .get_mut(active_index)
            .map(|dialog| dialog.state.take_sound_events())
            .unwrap_or_default();
        self.play_message_dialog_sound_events(sounds);
        self.persist_message_dialog_checkbox_changes(active_index);
        if let Some(result) = result {
            self.finish_message_dialog_at(active_index, result)?;
        }
        Ok(())
    }

    fn handle_league_signup_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<(), EngineError> {
        let actions = self
            .league_signup_dialog
            .as_mut()
            .map(|dialog| match event {
                GamepadEvent::Direction {
                    button,
                    state: ElementState::Pressed,
                    ..
                } if matches!(button, ControlButton::Left | ControlButton::Right) => dialog
                    .controller
                    .handle_key_down(KeyCode::Tab, button == ControlButton::Left),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                    ..
                } => {
                    use clonk_frontend::league_signup::LeagueSignupControl;
                    let key = match dialog.controller.focused_control() {
                        Some(
                            LeagueSignupControl::Account
                            | LeagueSignupControl::Password
                            | LeagueSignupControl::PasswordConfirmation,
                        )
                        | None => KeyCode::Enter,
                        Some(
                            LeagueSignupControl::PasswordCheckbox
                            | LeagueSignupControl::Close
                            | LeagueSignupControl::Ok
                            | LeagueSignupControl::Cancel,
                        ) => KeyCode::Space,
                    };
                    dialog.controller.handle_key_down(key, false)
                }
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Released,
                    ..
                } => dialog.controller.handle_key_up(KeyCode::Space),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                    ..
                } => dialog.controller.handle_key_down(KeyCode::Escape, false),
                GamepadEvent::Clear { .. } => {
                    dialog.controller.cancel_interaction();
                    Vec::new()
                }
                GamepadEvent::Axis { .. }
                | GamepadEvent::Direction { .. }
                | GamepadEvent::Button { .. }
                | GamepadEvent::Action { .. }
                | GamepadEvent::GuiButton { .. } => Vec::new(),
            })
            .unwrap_or_default();
        self.process_league_signup_actions(actions)
    }

    pub(crate) fn handle_game_over_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<(), EngineError> {
        match event {
            GamepadEvent::GuiButton {
                class: GuiButtonClass::Low,
                state: ElementState::Pressed,
                ..
            } => {
                let captured = self.game_over_dialog.as_mut().is_some_and(|dialog| {
                    dialog.handle_activation_down(GameOverActivationKey::Confirm)
                });
                let sounds = self
                    .game_over_dialog
                    .as_mut()
                    .map(GameOverState::take_sound_events)
                    .unwrap_or_default();
                self.play_game_over_sound_events(sounds);
                if !captured {
                    self.start_running_chat(RunningChatMode::All);
                }
                Ok(())
            }
            GamepadEvent::GuiButton {
                class: GuiButtonClass::Low,
                state: ElementState::Released,
                ..
            } => {
                let action = self
                    .game_over_dialog
                    .as_mut()
                    .and_then(|dialog| dialog.handle_activation_up(GameOverActivationKey::Confirm));
                let sounds = self
                    .game_over_dialog
                    .as_mut()
                    .map(GameOverState::take_sound_events)
                    .unwrap_or_default();
                self.play_game_over_sound_events(sounds);
                if let Some(action) = action {
                    self.handle_game_over_action(action)?;
                }
                Ok(())
            }
            GamepadEvent::GuiButton {
                class: GuiButtonClass::High,
                state: ElementState::Pressed,
                ..
            } => {
                if self
                    .game_over_dialog
                    .as_ref()
                    .is_some_and(GameOverState::allows_escape_close)
                {
                    self.handle_game_over_action(GameOverAction::End)?;
                }
                Ok(())
            }
            GamepadEvent::Direction {
                button: ControlButton::Left,
                state: ElementState::Pressed,
                ..
            } => {
                if let Some(dialog) = self.game_over_dialog.as_mut() {
                    dialog.advance_focus(true);
                }
                Ok(())
            }
            GamepadEvent::Direction {
                button: ControlButton::Right,
                state: ElementState::Pressed,
                ..
            } => {
                if let Some(dialog) = self.game_over_dialog.as_mut() {
                    dialog.advance_focus(false);
                }
                Ok(())
            }
            GamepadEvent::Clear { .. } => {
                if let Some(dialog) = self.game_over_dialog.as_mut() {
                    dialog.cancel_interaction();
                }
                Ok(())
            }
            GamepadEvent::Axis { .. }
            | GamepadEvent::Direction { .. }
            | GamepadEvent::Button { .. }
            | GamepadEvent::GuiButton { .. }
            | GamepadEvent::Action { .. } => Ok(()),
        }
    }

    fn handle_gamepad_raw_event(&mut self, event: GamepadEvent) -> Result<bool, EngineError> {
        match event {
            GamepadEvent::Axis { slot, axis, state } => {
                self.handle_gamepad_axis(slot, axis, state)?;
                Ok(true)
            }
            GamepadEvent::Button {
                slot,
                button,
                state,
            } => {
                self.handle_gamepad_button(slot, button, state)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub(crate) fn handle_gamepad_event(&mut self, event: GamepadEvent) -> Result<(), EngineError> {
        self.handle_gamepad_event_with_axis_alias(event, false)
    }

    fn handle_gamepad_event_with_axis_alias(
        &mut self,
        event: GamepadEvent,
        axis_alias: bool,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.note_classic_lobby_non_pointer_input();
        self.context_menu_pointer_dismissed_lobby_team_player = None;
        self.context_menu_pointer_dismissed_lobby_option = None;
        if self.runtime_client_list_keyboard_active()
            && self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.is_info_only())
        {
            let close = matches!(
                &event,
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                    ..
                } | GamepadEvent::Action {
                    action: GamepadActionType::Cancel,
                    state: ElementState::Pressed,
                    ..
                }
            );
            if close {
                self.handle_runtime_client_list_action(
                    clonk_frontend::runtime_client_list::RuntimeClientListAction::CloseInfo,
                )?;
            } else if matches!(&event, GamepadEvent::Clear { .. }) {
                if let Some(dialog) = self.runtime_client_list.as_mut() {
                    dialog.pointer_left();
                }
            }
            return Ok(());
        }
        match event {
            GamepadEvent::Axis { slot, axis, state } => {
                self.handle_gamepad_axis(slot, axis, state)?;
            }
            GamepadEvent::Direction {
                slot,
                button,
                state,
            } => {
                self.handle_gamepad_direction_inner(slot, button, state, !axis_alias)?;
            }
            GamepadEvent::Button {
                slot,
                button,
                state,
            } => {
                self.handle_gamepad_button(slot, button, state)?;
            }
            GamepadEvent::Clear { .. } => {
                if self
                    .network_start_wait
                    .as_ref()
                    .is_some_and(|wait| wait.visible)
                {
                    if let Some(wait) = self.network_start_wait.as_mut() {
                        wait.controller.cancel_interaction();
                    }
                } else if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    pending.controller.cancel_interaction();
                } else if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
                    pending.controller.pointer_left();
                } else if self.mode == AppMode::Menu
                    && self.startup_view == StartupView::NetworkLobby
                    && (self.classic_host_lobby.is_some() || self.network_lobby.is_some())
                {
                    self.cancel_classic_lobby_interaction();
                } else if self.mode == AppMode::Menu
                    && self.startup_view == StartupView::ScenarioBrowser
                {
                    self.scenario_game_options.cancel_interaction();
                } else if self.mode == AppMode::Menu && self.startup_view == StartupView::Options {
                    let actions = self
                        .startup_options_dialog
                        .as_mut()
                        .map(|dialog| dialog.handle_pointer_left())
                        .unwrap_or_default();
                    self.process_options_dialog_actions(actions)?;
                } else if matches!(self.mode, AppMode::Running)
                    && (!self.game_over_dialog_is_active()
                        || self.running_chat_controller().is_some())
                {
                    self.dispatch_control_event(ControlEvent::ClearPressed)?;
                }
            }
            GamepadEvent::GuiButton { class, state, .. } => {
                if self
                    .network_start_wait
                    .as_ref()
                    .is_some_and(|wait| wait.visible)
                {
                    let actions = self
                        .network_start_wait
                        .as_mut()
                        .map(|wait| match (class, state) {
                            (GuiButtonClass::Low, ElementState::Pressed) => {
                                wait.controller.handle_gamepad_low_down()
                            }
                            (GuiButtonClass::Low, ElementState::Released) => {
                                wait.controller.handle_gamepad_low_up()
                            }
                            (GuiButtonClass::High, ElementState::Pressed) => {
                                wait.controller.handle_gamepad_high_down()
                            }
                            (GuiButtonClass::High, ElementState::Released) => Vec::new(),
                        })
                        .unwrap_or_default();
                    self.process_network_start_wait_actions(actions)?;
                } else if self.startup_options_advanced_dialog.is_some() {
                    let key = match class {
                        GuiButtonClass::Low => KeyCode::Space,
                        GuiButtonClass::High => KeyCode::Escape,
                    };
                    let actions = self
                        .startup_options_advanced_dialog
                        .as_mut()
                        .map(|pending| match state {
                            ElementState::Pressed => pending.controller.handle_key_down(key),
                            ElementState::Released => pending.controller.handle_key_up(key),
                        })
                        .unwrap_or_default();
                    self.process_options_advanced_actions(actions)?;
                } else if self.startup_player_properties_dialog.is_some() {
                    let actions = self
                        .startup_player_properties_dialog
                        .as_mut()
                        .map(|pending| match (class, state) {
                            (GuiButtonClass::Low, ElementState::Pressed) => {
                                pending.controller.handle_gamepad_low_down()
                            }
                            (GuiButtonClass::Low, ElementState::Released) => {
                                pending.controller.handle_gamepad_low_up()
                            }
                            (GuiButtonClass::High, ElementState::Pressed) => {
                                pending.controller.handle_gamepad_high_down()
                            }
                            (GuiButtonClass::High, ElementState::Released) => Vec::new(),
                        })
                        .unwrap_or_default();
                    self.process_startup_player_properties_actions(actions);
                }
            }
            GamepadEvent::Action {
                slot,
                action,
                state,
            } => {
                self.handle_gamepad_action(slot, action, state)?;
            }
        }
        Ok(())
    }

    fn handle_gamepad_axis(
        &mut self,
        slot: GamepadSlot,
        axis: LegacyGamepadAxis,
        state: ElementState,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_controls::ControlDevice;

        let capture = self
            .message_dialogs
            .last()
            .and_then(|pending| match pending.continuation {
                MessageDialogContinuation::OptionsControlCapture(target)
                    if target.device == ControlDevice::Gamepad =>
                {
                    Some(target)
                }
                _ => None,
            });
        if let Some(target) = capture {
            if state == ElementState::Pressed && target.set == usize::from(slot.index()) {
                if let (Some(id), Some(raw_key)) = (
                    ControlBindingId::ALL.get(target.control).copied(),
                    input::legacy_gamepad_axis_key(slot.index(), axis.index(), axis.high()),
                ) {
                    if self.gamepad_bindings.rebind_raw(target.set, id, raw_key) {
                        let label = self.gamepad_bindings.key_label_for_set(target.set, id);
                        if let Some(dialog) = self.startup_options_dialog.as_mut() {
                            dialog.controls_mut().set_label(target, label);
                        }
                        self.finish_message_dialog(
                            clonk_frontend::message_dialog::MessageDialogResult::Ok,
                        )?;
                    }
                }
            }
            return Ok(());
        }
        if self.message_dialog_owns_gamepad_input()
            || self
                .network_start_wait
                .as_ref()
                .is_some_and(|wait| wait.visible)
            || self.startup_options_advanced_dialog.is_some()
            || self.startup_player_properties_dialog.is_some()
            || self.definition_selector.is_some()
            || self.game_over_dialog_is_active() && self.running_chat_controller().is_none()
            || !matches!(self.mode, AppMode::Running)
        {
            return Ok(());
        }

        let direction = axis.direction();
        let mut candidates = Vec::new();
        if let Some(raw_key) =
            input::legacy_gamepad_axis_key(slot.index(), axis.index(), axis.high())
        {
            candidates.extend(
                self.gamepad_bindings
                    .control_candidates_for_raw_key(raw_key, state),
            );
        }
        candidates.extend(self.runtime_control_candidates_for_gamepad_direction(
            slot.index(),
            direction,
            state,
        ));
        if let Some(raw_key) =
            input::legacy_gamepad_axis_alias_key(slot.index(), axis.index(), axis.high())
        {
            candidates.extend(
                self.gamepad_bindings
                    .control_candidates_for_raw_key(raw_key, state),
            );
        }
        let routing =
            self.local_controls
                .route_keyboard_candidates(candidates, state, false, |owner| {
                    self.engine
                        .player(owner)
                        .map(|player| player.control_style())
                });
        if let KeyboardRoutingOutcome::Consumed {
            owner: Some(owner),
            event: Some(event),
        } = routing
        {
            self.dispatch_control_event_for_local_player(owner, event)?;
        }
        if !matches!(routing, KeyboardRoutingOutcome::Unhandled) {
            return Ok(());
        }
        if let Some(action) = self.runtime_custom_gamepad_direction_action(slot.index(), direction)
        {
            self.execute_runtime_custom_gamepad_action(action, state)?;
        }
        Ok(())
    }

    pub(crate) fn handle_gamepad_button(
        &mut self,
        slot: GamepadSlot,
        button: LegacyGamepadButton,
        state: ElementState,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_controls::ControlDevice;
        let capture = self
            .message_dialogs
            .last()
            .and_then(|pending| match pending.continuation {
                MessageDialogContinuation::OptionsControlCapture(target)
                    if target.device == ControlDevice::Gamepad =>
                {
                    Some(target)
                }
                _ => None,
            });
        if let Some(target) = capture {
            if state == ElementState::Pressed && target.set == usize::from(slot.index()) {
                if let Some(id) = ControlBindingId::ALL.get(target.control).copied() {
                    if self.gamepad_bindings.rebind_button(
                        target.set,
                        id,
                        slot.index(),
                        button.index(),
                    ) {
                        let label = self.gamepad_bindings.key_label_for_set(target.set, id);
                        if let Some(dialog) = self.startup_options_dialog.as_mut() {
                            dialog.controls_mut().set_label(target, label);
                        }
                        self.finish_message_dialog(
                            clonk_frontend::message_dialog::MessageDialogResult::Ok,
                        )?;
                    }
                }
            }
            return Ok(());
        }
        if self.message_dialog_owns_gamepad_input() {
            return Ok(());
        }
        if self
            .network_start_wait
            .as_ref()
            .is_some_and(|wait| wait.visible)
        {
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            return Ok(());
        }
        if self.definition_selector.is_some() {
            return Ok(());
        }
        if self.game_over_dialog_is_active() && self.running_chat_controller().is_none() {
            return Ok(());
        }
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        let mut candidates =
            self.runtime_control_candidates_for_gamepad_button(slot.index(), button.index(), state);
        candidates.extend(self.gamepad_bindings.control_candidates_for_button(
            slot.index(),
            button.index(),
            state,
        ));
        let routing =
            self.local_controls
                .route_keyboard_candidates(candidates, state, false, |owner| {
                    self.engine
                        .player(owner)
                        .map(|player| player.control_style())
                });
        if let KeyboardRoutingOutcome::Consumed {
            owner: Some(owner),
            event: Some(event),
        } = routing
        {
            self.dispatch_control_event_for_local_player(owner, event)?;
        }
        if !matches!(routing, KeyboardRoutingOutcome::Unhandled) {
            return Ok(());
        }
        if let Some(action) =
            self.runtime_custom_gamepad_button_action(slot.index(), button.index())
        {
            self.execute_runtime_custom_gamepad_action(action, state)?;
        }
        Ok(())
    }

    pub(crate) fn handle_gamepad_direction(
        &mut self,
        slot: GamepadSlot,
        button: ControlButton,
        state: ElementState,
    ) -> Result<(), EngineError> {
        self.handle_gamepad_direction_inner(slot, button, state, true)
    }

    fn handle_gamepad_direction_inner(
        &mut self,
        slot: GamepadSlot,
        button: ControlButton,
        state: ElementState,
        route_runtime_gameplay: bool,
    ) -> Result<(), EngineError> {
        if self.message_dialog_owns_gamepad_input() {
            return self.handle_message_dialog_gamepad_event(GamepadEvent::Direction {
                slot,
                button,
                state,
            });
        }
        if self
            .network_start_wait
            .as_ref()
            .is_some_and(|wait| wait.visible)
        {
            if state == ElementState::Pressed {
                let backwards = matches!(button, ControlButton::Left | ControlButton::Up);
                if let Some(wait) = self.network_start_wait.as_mut() {
                    wait.controller.handle_gamepad_horizontal(backwards);
                }
            }
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            if state == ElementState::Pressed {
                if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
                    match button {
                        ControlButton::Left => pending.controller.handle_focus_step(true),
                        ControlButton::Right => pending.controller.handle_focus_step(false),
                        ControlButton::Up
                            if pending.controller.focus()
                                == clonk_frontend::startup_options_advanced::AdvancedConfigFocus::SectionTabs =>
                        {
                            pending.controller.handle_key_down(KeyCode::Up);
                        }
                        ControlButton::Down
                            if pending.controller.focus()
                                == clonk_frontend::startup_options_advanced::AdvancedConfigFocus::SectionTabs =>
                        {
                            pending.controller.handle_key_down(KeyCode::Down);
                        }
                        ControlButton::Up | ControlButton::Down => {}
                    }
                }
            }
            self.process_options_advanced_actions(Vec::new())?;
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let key = match button {
                ControlButton::Left => KeyCode::Left,
                ControlButton::Right => KeyCode::Right,
                ControlButton::Up => KeyCode::Up,
                ControlButton::Down => KeyCode::Down,
            };
            let actions = self
                .startup_player_properties_dialog
                .as_mut()
                .map(|pending| match state {
                    ElementState::Pressed => pending.controller.handle_gamepad_direction(key),
                    ElementState::Released => Vec::new(),
                })
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.definition_selector.is_some() {
            return self.handle_definition_selector_gamepad_event(GamepadEvent::Direction {
                slot,
                button,
                state,
            });
        }
        if self.game_over_dialog_is_active() && self.running_chat_controller().is_none() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_gamepad_direction(button, state);
        }
        if self.joined_network_lobby_active() {
            let option_focused = self.network_lobby.as_mut().is_some_and(|lobby| {
                lobby.sync_classic_controller();
                matches!(lobby.controller.focus(), LobbyControl::GameOption(_))
            });
            if option_focused {
                if state == ElementState::Pressed {
                    let (horizontal, vertical) = match button {
                        ControlButton::Left => (-1, 0),
                        ControlButton::Right => (1, 0),
                        ControlButton::Up => (0, -1),
                        ControlButton::Down => (0, 1),
                    };
                    let assets = Arc::clone(&self.assets);
                    let actions = self
                        .network_lobby
                        .as_mut()
                        .expect("joined lobby was checked above")
                        .with_classic_controller_input(
                            self.graphics.surface(),
                            assets.as_ref(),
                            &self.scenario_game_options,
                            |controller, layout, roster| {
                                controller.gamepad_direction(horizontal, vertical, layout, roster)
                            },
                        )
                        .map_err(Self::joined_lobby_input_error)?;
                    self.process_joined_lobby_controller_actions(actions)?;
                }
                return Ok(());
            }
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::PlayerSelection
            && self.startup_crew_rename.is_some()
        {
            if state == ElementState::Pressed
                && matches!(button, ControlButton::Left | ControlButton::Right)
            {
                self.commit_startup_crew_rename(true)?;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::ScenarioBrowser {
            if self.menu_state.rename_edit.is_some() {
                if state == ElementState::Pressed
                    && matches!(button, ControlButton::Left | ControlButton::Right)
                {
                    self.commit_scenario_rename(true)?;
                }
                // RenameEdit has no Up/Down binding. Left/Right attempt a
                // focus transfer, but FinishRename restores the saved focus
                // and cancels that original transfer in Dialog::SetFocus.
                return Ok(());
            }
            if self.scenario_selector_discovery.is_some() {
                return Ok(());
            }
            let direction = match button {
                ControlButton::Left => GameOptionGamepadDirection::Left,
                ControlButton::Right => GameOptionGamepadDirection::Right,
                ControlButton::Up => GameOptionGamepadDirection::Up,
                ControlButton::Down => GameOptionGamepadDirection::Down,
            };
            if state == ElementState::Pressed
                && self.scenario_game_options.focused_button().is_some()
            {
                self.menu_state
                    .set_dialog_focus(ScenselDialogFocus::Options);
                let outcome = self
                    .scenario_game_options
                    .handle_gamepad_direction(direction);
                let captured = outcome.captured;
                self.finish_game_option_input(outcome.actions)?;
                if captured {
                    return Ok(());
                }
            }
            if matches!(button, ControlButton::Left | ControlButton::Right) {
                if state == ElementState::Pressed {
                    self.advance_scensel_dialog_focus(button == ControlButton::Left);
                }
                return Ok(());
            }
        }
        if self.mode == AppMode::Menu
            && matches!(button, ControlButton::Left | ControlButton::Right)
            && matches!(
                self.startup_view,
                StartupView::NetworkGame
                    | StartupView::PlayerSelection
                    | StartupView::Options
                    | StartupView::About
            )
        {
            if state == ElementState::Pressed {
                let backwards = button == ControlButton::Left;
                match self.startup_view {
                    StartupView::NetworkGame => {
                        let actions = self
                            .startup_network_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_gamepad_horizontal(backwards))
                            .unwrap_or_default();
                        self.process_network_dialog_actions(actions)?;
                    }
                    StartupView::PlayerSelection => {
                        let actions = self
                            .startup_player_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_gamepad_horizontal(backwards))
                            .unwrap_or_default();
                        self.process_player_dialog_actions(actions)?;
                    }
                    StartupView::Options => {
                        let actions = self
                            .startup_options_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_gamepad_horizontal(backwards))
                            .unwrap_or_default();
                        self.process_options_dialog_actions(actions)?;
                    }
                    StartupView::About => {
                        let actions = self
                            .startup_about_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_gamepad_horizontal(backwards))
                            .unwrap_or_default();
                        self.process_about_dialog_actions(actions)?;
                    }
                    _ => unreachable!(),
                }
            }
            return Ok(());
        }
        match self.mode {
            AppMode::Menu => {
                if let Some(key) = menu_key_from_control_button(button) {
                    if self.handle_startup_dialog_key(key, state)? {
                        return Ok(());
                    }
                    match self.startup_view {
                        StartupView::ScenarioBrowser => match (state, key) {
                            _ if self.menu_state.current_map().is_some()
                                && state == ElementState::Pressed
                                && key == KeyCode::Left =>
                            {
                                self.scensel_do_back()?
                            }
                            _ if self.menu_state.current_map().is_some()
                                && state == ElementState::Pressed
                                && key == KeyCode::Right =>
                            {
                                self.start_selected_map_scenario_from_ui()?
                            }
                            _ if self.menu_state.current_map().is_some() => {}
                            (ElementState::Pressed, KeyCode::Up) => {
                                self.handle_menu_input(|menu| menu.move_list_selection_clamped(-1))?
                            }
                            (ElementState::Pressed, KeyCode::Down) => {
                                self.handle_menu_input(|menu| menu.move_list_selection_clamped(1))?
                            }
                            (ElementState::Pressed, KeyCode::Left) => self.scensel_do_back()?,
                            (ElementState::Pressed, KeyCode::Right) => {
                                self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_down(KeyCode::Enter)
                                })?;
                                self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_up(KeyCode::Enter)
                                })?;
                            }
                            (ElementState::Pressed, _) => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_down(key))?
                            }
                            (
                                ElementState::Released,
                                KeyCode::Up | KeyCode::Down | KeyCode::Left | KeyCode::Right,
                            ) => {}
                            (ElementState::Released, _) => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_up(key))?
                            }
                        },
                        StartupView::NetworkGame | StartupView::PlayerSelection => {}
                        StartupView::MainMenu => {
                            let actions = match state {
                                ElementState::Pressed => self.main_menu_state.handle_key_down(key),
                                ElementState::Released => self.main_menu_state.handle_key_up(key),
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        StartupView::NetworkLobby => match state {
                            ElementState::Pressed => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_down(key))?
                            }
                            ElementState::Released => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_up(key))?
                            }
                        },
                        StartupView::Options | StartupView::About => {}
                    }
                }
            }
            AppMode::Running => {
                if !route_runtime_gameplay {
                    return Ok(());
                }
                let candidates = self.runtime_control_candidates_for_gamepad_direction(
                    slot.index(),
                    button,
                    state,
                );
                let routing = self.local_controls.route_keyboard_candidates(
                    candidates,
                    state,
                    false,
                    |owner| {
                        self.engine
                            .player(owner)
                            .map(|player| player.control_style())
                    },
                );
                if let KeyboardRoutingOutcome::Consumed {
                    owner: Some(owner),
                    event: Some(event),
                } = routing
                {
                    self.dispatch_control_event_for_local_player(owner, event)?;
                }
                if !matches!(routing, KeyboardRoutingOutcome::Unhandled) {
                    return Ok(());
                }
                if let Some(action) =
                    self.runtime_custom_gamepad_direction_action(slot.index(), button)
                {
                    self.execute_runtime_custom_gamepad_action(action, state)?;
                    return Ok(());
                }
            }
            AppMode::Loading => {}
        }
        Ok(())
    }

    pub(crate) fn handle_gamepad_action(
        &mut self,
        slot: GamepadSlot,
        action: GamepadActionType,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if self.message_dialog_owns_gamepad_input() {
            return self.handle_message_dialog_gamepad_event(GamepadEvent::Action {
                slot,
                action,
                state,
            });
        }
        if self.startup_options_advanced_dialog.is_some() {
            let key = match action {
                GamepadActionType::Select => Some(KeyCode::Space),
                GamepadActionType::Cancel => Some(KeyCode::Escape),
                GamepadActionType::MenuToggle => None,
            };
            let actions = key
                .and_then(|key| {
                    self.startup_options_advanced_dialog
                        .as_mut()
                        .map(|pending| match state {
                            ElementState::Pressed => pending.controller.handle_key_down(key),
                            ElementState::Released => pending.controller.handle_key_up(key),
                        })
                })
                .unwrap_or_default();
            self.process_options_advanced_actions(actions)?;
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let actions = self
                .startup_player_properties_dialog
                .as_mut()
                .map(|pending| match (action, state) {
                    (GamepadActionType::Select, ElementState::Pressed) => {
                        pending.controller.handle_gamepad_low_down()
                    }
                    (GamepadActionType::Select, ElementState::Released) => {
                        pending.controller.handle_gamepad_low_up()
                    }
                    (GamepadActionType::Cancel, ElementState::Pressed) => {
                        pending.controller.handle_gamepad_high_down()
                    }
                    (GamepadActionType::Cancel, ElementState::Released)
                    | (GamepadActionType::MenuToggle, _) => Vec::new(),
                })
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.definition_selector.is_some() {
            return Ok(());
        }
        if self.game_over_dialog_is_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_gamepad_action(action, state);
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::ScenarioBrowser
            && self.menu_state.rename_edit.is_some()
        {
            // RenameEdit and Dialog bind the physical AnyHigh/AnyLow inputs,
            // respectively. Those eligibility-gated raw events are handled
            // above and own their complete alias cluster.
            return Ok(());
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::ScenarioBrowser
            && self.scenario_selector_discovery.is_some()
            && action == GamepadActionType::Select
        {
            return Ok(());
        }
        match action {
            GamepadActionType::Select => match self.mode {
                AppMode::Menu => {
                    if self.startup_view == StartupView::ScenarioBrowser
                        && self.scenario_game_options.focused_button().is_some()
                    {
                        self.menu_state
                            .set_dialog_focus(ScenselDialogFocus::Options);
                        let outcome = match state {
                            ElementState::Pressed => {
                                self.scenario_game_options.handle_gamepad_low_down()
                            }
                            ElementState::Released => {
                                self.scenario_game_options.handle_gamepad_low_up()
                            }
                        };
                        let captured = outcome.captured;
                        self.finish_game_option_input(outcome.actions)?;
                        if captured {
                            return Ok(());
                        }
                    }
                    if self.startup_view == StartupView::ScenarioBrowser
                        && self.menu_state.definition_checkbox_focused
                    {
                        if state == ElementState::Pressed
                            && self.menu_state.toggle_definition_checkbox()
                        {
                            self.play_ui_sound("ArrowHit");
                        }
                        return Ok(());
                    }
                    if self.startup_view == StartupView::ScenarioBrowser {
                        match self.menu_state.dialog_focus() {
                            ScenselDialogFocus::Back => {
                                if state == ElementState::Pressed {
                                    self.scensel_do_back()?;
                                }
                                return Ok(());
                            }
                            ScenselDialogFocus::Search => return Ok(()),
                            ScenselDialogFocus::Definitions => return Ok(()),
                            ScenselDialogFocus::List
                            | ScenselDialogFocus::Options
                            | ScenselDialogFocus::Open => {}
                        }
                    }
                    if self.handle_startup_dialog_key(KeyCode::Enter, state)? {
                        return Ok(());
                    }
                    match self.startup_view {
                        StartupView::ScenarioBrowser => match state {
                            ElementState::Pressed if self.menu_state.current_map().is_some() => {
                                self.start_selected_map_scenario_from_ui()?
                            }
                            ElementState::Released if self.menu_state.current_map().is_some() => {}
                            ElementState::Pressed => self.handle_menu_input(|menu| {
                                menu.menu().handle_key_down(KeyCode::Enter)
                            })?,
                            ElementState::Released => self.handle_menu_input(|menu| {
                                menu.menu().handle_key_up(KeyCode::Enter)
                            })?,
                        },
                        StartupView::NetworkGame | StartupView::PlayerSelection => {}
                        StartupView::MainMenu => {
                            let actions = match state {
                                ElementState::Pressed => {
                                    self.main_menu_state.handle_key_down(KeyCode::Enter)
                                }
                                ElementState::Released => {
                                    self.main_menu_state.handle_key_up(KeyCode::Enter)
                                }
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        StartupView::NetworkLobby => {
                            let option_focused = self.joined_network_lobby_active()
                                && self.network_lobby.as_mut().is_some_and(|lobby| {
                                    lobby.sync_classic_controller();
                                    matches!(lobby.controller.focus(), LobbyControl::GameOption(_))
                                });
                            if option_focused {
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
                                                    controller.gamepad_low_down(
                                                        Instant::now(),
                                                        layout,
                                                        roster,
                                                    )
                                                },
                                            )
                                            .map_err(Self::joined_lobby_input_error)?,
                                        ElementState::Released => lobby.controller.gamepad_low_up(),
                                    }
                                };
                                self.process_joined_lobby_controller_actions(actions)?;
                                return Ok(());
                            }
                            match state {
                                ElementState::Pressed => self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_down(KeyCode::Enter)
                                })?,
                                ElementState::Released => self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_up(KeyCode::Enter)
                                })?,
                            }
                        }
                        StartupView::Options | StartupView::About => {}
                    }
                }
                AppMode::Running | AppMode::Loading => {}
            },
            GamepadActionType::Cancel => match self.mode {
                AppMode::Menu => {
                    self.handle_menu_cancel_action(state)?;
                }
                AppMode::Running | AppMode::Loading => {}
            },
            GamepadActionType::MenuToggle => match self.mode {
                AppMode::Menu => {
                    self.handle_menu_cancel_action(state)?;
                }
                AppMode::Running => {
                    if state == ElementState::Pressed {
                        if self.ingame_menu_belongs_to(self.local_owner) {
                            self.close_ingame_menu_by_user()?;
                        } else {
                            self.open_ingame_menu()?;
                        }
                    }
                }
                AppMode::Loading => {}
            },
        }
        Ok(())
    }

    fn handle_league_signup_pointer_move(&mut self, point: GuiPoint) -> Result<bool, EngineError> {
        if self.league_signup_dialog.is_none() {
            return Ok(false);
        }
        self.league_signup_pointer_position = Some(point);
        let layout = self.league_signup_layout();
        let fonts = self.assets.clonk_fonts.clone();
        let actions = layout
            .as_ref()
            .and_then(|layout| {
                fonts.as_deref().and_then(|fonts| {
                    self.league_signup_dialog.as_mut().map(|dialog| {
                        dialog
                            .controller
                            .handle_pointer_move(point, layout, &fonts.text)
                    })
                })
            })
            .unwrap_or_default();
        self.process_league_signup_actions(actions)?;
        Ok(true)
    }

    fn league_signup_pointer_left(&mut self, clear_position: bool) {
        let sounds = self
            .league_signup_dialog
            .as_mut()
            .map(|dialog| {
                dialog.controller.pointer_left();
                dialog.controller.take_sound_events()
            })
            .unwrap_or_default();
        for sound in sounds {
            self.play_ui_sound(match sound {
                clonk_frontend::league_signup::LeagueSignupSound::ArrowHit => "ArrowHit",
                clonk_frontend::league_signup::LeagueSignupSound::Click => "Click",
            });
        }
        self.league_signup_pointer_capture = false;
        if clear_position {
            self.league_signup_pointer_position = None;
        }
    }

    fn handle_league_signup_pointer_button(
        &mut self,
        state: ElementState,
        left_double_click: bool,
    ) -> Result<bool, EngineError> {
        if self.league_signup_dialog.is_none() {
            return Ok(false);
        }
        let Some(point) = self
            .league_signup_pointer_position
            .or(self.running_pointer_position)
        else {
            return Ok(true);
        };
        self.league_signup_pointer_position = Some(point);
        let Some(layout) = self.league_signup_layout() else {
            return Ok(true);
        };
        let fonts = self.assets.clonk_fonts.clone();
        let actions = self
            .league_signup_dialog
            .as_mut()
            .map(|dialog| match state {
                ElementState::Pressed => fonts.as_deref().map_or_else(Vec::new, |fonts| {
                    if left_double_click {
                        dialog
                            .controller
                            .handle_pointer_double_click(point, &layout, &fonts.text)
                    } else {
                        dialog
                            .controller
                            .handle_pointer_down(point, &layout, &fonts.text)
                    }
                }),
                ElementState::Released => fonts.as_deref().map_or_else(Vec::new, |fonts| {
                    dialog
                        .controller
                        .handle_pointer_up(point, &layout, &fonts.text)
                }),
            })
            .unwrap_or_default();
        self.process_league_signup_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn handle_cursor_moved(
        &mut self,
        position: PhysicalPosition<f64>,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        // C4GraphicsSystem::MouseMove ceil-quantizes the scale-adjusted
        // coordinates once before either C4GUI::CMouse or viewport routing.
        let raw_point = gui_point_from_position(position);
        let point = GuiPoint::new(raw_point.x.ceil(), raw_point.y.ceil());
        self.window_mouse_position = Some(point);
        self.pointer_inside_window = true;
        if self.mode == AppMode::Running {
            // C4GraphicsSystem first offers every new move to C4GUI, then
            // returns ownership to C4MouseControl unless a GUI route wins.
            self.running_gui_mouse_owned = false;
            self.running_world_mouse_owned = true;
        }
        self.startup_tooltip.note_pointer_move(point);
        if self.startup_network_transition_blocks_input() {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        self.running_pointer_position = Some(point);
        if let Some(index) = self.captured_message_dialog_index().filter(|index| {
            self.message_dialogs
                .get(*index)
                .is_some_and(|dialog| dialog.state.has_positional_pointer_drag())
        }) {
            // `CMouse` runs pDragElement::DoDragging before ordinary screen
            // and context-menu hit-testing, even on a shared dialog below a
            // higher interactive layer.
            self.handle_message_dialog_pointer_move_at(index, point);
        }
        if self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.has_positional_pointer_drag())
        {
            // `CMouse` updates its retained pDragElement before top-down
            // routing, so a newly higher layer cannot freeze a dialog drag.
            self.handle_runtime_client_list_pointer_move(point);
        }
        if self.network_chart_pointer_capture {
            // CMouse advances the title drag before ordinary hit-testing and
            // retains only the caption/close control through release.
            let _ = self.handle_network_chart_pointer_move(point);
            self.suspend_ingame_pointer_for_gui();
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        if self.mode == AppMode::Running {
            self.ingame_gui_pointer = Some(point);
            // CMouse invokes DoDragging before top-down dialog routing, even
            // when another shared dialog now owns the pointer location.
            self.update_scoreboard_title_drag(point);
            if self.update_menu_title_drag(point) {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
            if self.update_construction_menu_drag(point)? {
                return Ok(());
            }
            if matches!(
                self.construction_menu_drag,
                Some(ConstructionMenuDrag::Candidate { .. })
            ) {
                // CMouse retains the originating menu element as its drag
                // owner until the five-pixel threshold is crossed.
                self.running_gui_mouse_owned = true;
                self.running_world_mouse_owned = false;
            }
            if self.ingame_moving_drag_active() {
                self.update_ingame_pointer(point)?;
                return Ok(());
            }
        }
        if self.context_menu.is_some()
            && self
                .game_option_input_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.controller.has_positional_pointer_drag())
        {
            self.game_option_input_pointer_position = Some(point);
            let layout = self.game_option_input_layout();
            let fonts = self.assets.clonk_fonts.clone();
            let actions = layout
                .as_ref()
                .zip(fonts.as_deref())
                .and_then(|(layout, fonts)| {
                    self.game_option_input_dialog.as_mut().map(|dialog| {
                        dialog
                            .controller
                            .handle_pointer_move(point, layout, &fonts.text)
                    })
                })
                .unwrap_or_default();
            self.finish_game_option_input_dialog_actions(actions)?;
        }
        let context_routed_before_running_dialogs = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && !self.running_dialog_stack.is_empty();
        if context_routed_before_running_dialogs && self.handle_context_menu_pointer_move(point)? {
            self.occlude_running_dialog_pointer_hovers();
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.running_chat_controller().is_some() {
            // CMouse retains the original button as pDragElement, but classic
            // buttons have no DoDragging implementation. Screen hit-testing
            // still runs top-down; while capture exists, the active dialog is
            // also a match outside its bounds and therefore blocks lower hits.
            let lower_capture = self.captured_message_dialog_index();
            if !context_routed_before_running_dialogs
                && self.handle_context_menu_pointer_move(point)?
            {
                if let Some(controller) = self.running_chat_controller_mut() {
                    controller.pointer_left();
                }
                for index in 0..self.message_dialogs.len() {
                    self.message_dialog_pointer_left_at(index);
                }
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
            if self.network_chart_is_elevated_pointer_layer()
                && self.network_chart_contains_point(point)
            {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
            let shared_target = self.top_running_shared_pointer_target(point, true)?;
            if shared_target.is_some() && shared_target != Some(RunningDialogStackEntry::Chat) {
                if let Some(controller) = self.running_chat_controller_mut() {
                    controller.pointer_left();
                }
                self.finish_game_option_input_dialog_actions(Vec::new())?;
                self.handle_scoreboard_message_pointer_move(point)?;
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
            let layout = self.game_option_input_layout();
            let chat_hit = layout
                .as_ref()
                .is_some_and(|layout| Self::point_in_input_dialog_bounds(point, layout))
                || self.game_option_input_pointer_capture == Some(ContextMenuPointerButton::Left)
                || (self.running_chat_active() && lower_capture.is_some());
            let mut shared_pointer_consumed = chat_hit;
            self.game_option_input_pointer_position = Some(point);
            if chat_hit {
                if self.primary_pointer_left_down {
                    self.set_running_chat_active(true);
                }
                let fonts = self.assets.clonk_fonts.clone();
                let actions = layout
                    .as_ref()
                    .zip(fonts.as_deref())
                    .and_then(|(layout, fonts)| {
                        self.game_option_input_dialog.as_mut().map(|dialog| {
                            dialog
                                .controller
                                .handle_pointer_move(point, layout, &fonts.text)
                        })
                    })
                    .unwrap_or_default();
                self.finish_game_option_input_dialog_actions(actions)?;
                for index in 0..self.message_dialogs.len() {
                    self.message_dialog_pointer_left_at(index);
                }
            } else {
                if let Some(controller) = self.running_chat_controller_mut() {
                    controller.pointer_left();
                }
                self.finish_game_option_input_dialog_actions(Vec::new())?;
                let active_message = self.active_message_dialog_index();
                let message_target = (0..self.message_dialogs.len()).rev().find(|index| {
                    self.message_dialog_layout_at(*index)
                        .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout))
                        || (lower_capture.is_some() && active_message == Some(*index))
                });
                shared_pointer_consumed = message_target.is_some();
                if self.primary_pointer_left_down {
                    let target_is_hit = message_target.is_some_and(|index| {
                        self.message_dialog_layout_at(index).is_some_and(|layout| {
                            Self::point_in_message_dialog_bounds(point, &layout)
                        })
                    });
                    if target_is_hit {
                        self.set_running_chat_active(false);
                        self.message_dialog_active_index = message_target;
                    }
                }
                for index in 0..self.message_dialogs.len() {
                    if Some(index) != message_target {
                        self.message_dialog_pointer_left_at(index);
                    }
                }
                if let Some(target) = message_target {
                    self.handle_message_dialog_pointer_move_at(target, point);
                }
            }
            let lower_default_hit = if !chat_hit
                && lower_capture.is_none()
                && self.top_message_dialog_hit_index(point).is_none()
            {
                self.handle_runtime_default_dialog_pointer_move(point)?
            } else {
                false
            };
            if shared_pointer_consumed
                || lower_default_hit
                || (!self.network_chart_elevated && self.running_shared_gui_has_keyboard_focus())
            {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
        }
        if self.network_chart_is_elevated_pointer_layer()
            && self.network_chart_contains_point(point)
        {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.running_chat_controller().is_none() && !self.message_dialogs.is_empty() {
            // Startup keeps C4GUI::Screen exclusive, so its active message
            // dialog owns motion without joining the running shared stack.
            let consumed = if self.mode == AppMode::Running {
                self.handle_scoreboard_message_pointer_move(point)?
            } else {
                self.handle_message_dialog_pointer_move(point)
            };
            if consumed {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
        }
        if self.league_signup_dialog.is_some() && self.context_menu.is_some() {
            self.league_signup_pointer_position = Some(point);
            if !context_routed_before_running_dialogs
                && self.handle_context_menu_pointer_move(point)?
            {
                self.league_signup_pointer_left(false);
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
        }
        if self.handle_league_signup_pointer_move(point)? {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.external_irc_dialog_visible {
            if !context_routed_before_running_dialogs
                && self.handle_context_menu_pointer_move(point)?
            {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
            if self.context_menu.is_some() {
                self.suspend_ingame_pointer_for_gui();
                return Ok(());
            }
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            if self.message_dialogs.is_empty() {
                let actions = self
                    .assets
                    .clonk_fonts
                    .clone()
                    .and_then(|fonts| {
                        self.external_irc_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_pointer_move(point, &fonts.text))
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(actions)?;
            }
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if let Some(layout) = self.network_start_wait_layout() {
            if let Some(wait) = self.network_start_wait.as_mut() {
                wait.pointer = Some(point);
                wait.controller.handle_pointer_move(point, &layout);
            }
            self.play_network_start_wait_sounds();
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let font = self.assets.clonk_fonts.as_deref().map(|fonts| &fonts.text);
            let actions = self
                .startup_options_advanced_dialog
                .as_mut()
                .map(|pending| match font {
                    Some(font) => pending
                        .controller
                        .handle_pointer_move_with_font(point, font),
                    None => pending.controller.handle_pointer_move(point),
                })
                .unwrap_or_default();
            self.process_options_advanced_actions(actions)?;
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let actions = self
                .startup_player_properties_dialog
                .as_mut()
                .map(|pending| pending.controller.handle_pointer_move(point))
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.definition_selector.is_some() {
            let actions = self
                .definition_selector_layout()
                .and_then(|layout| {
                    self.definition_selector
                        .as_mut()
                        .map(|controller| controller.handle_pointer_move(point, &layout))
                })
                .unwrap_or_default();
            self.finish_definition_selector_input(actions)?;
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if !context_routed_before_running_dialogs && self.handle_context_menu_pointer_move(point)? {
            if let Some(dialog) = self.game_option_input_dialog.as_mut() {
                dialog.controller.pointer_left();
                let sounds = dialog.controller.take_sound_events();
                self.play_input_dialog_sound_events(sounds);
            }
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.game_option_input_dialog.is_some()
            && self.game_option_input_owns_running_pointer_event()
        {
            self.game_option_input_pointer_position = Some(point);
            let layout = self.game_option_input_layout();
            let fonts = self.assets.clonk_fonts.clone();
            let actions = layout
                .as_ref()
                .zip(fonts.as_deref())
                .and_then(|(layout, fonts)| {
                    self.game_option_input_dialog.as_mut().map(|dialog| {
                        dialog
                            .controller
                            .handle_pointer_move(point, layout, &fonts.text)
                    })
                })
                .unwrap_or_default();
            self.finish_game_option_input_dialog_actions(actions)?;
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if !matches!(self.mode, AppMode::Running)
            && self.handle_runtime_client_list_pointer_move(point)
        {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.handle_runtime_default_dialog_pointer_move(point)? {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.runtime_pointer_fallback_is_exclusive() {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            self.suspend_ingame_pointer_for_gui();
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            self.suspend_ingame_pointer_for_gui();
            return self.handle_classic_lobby_pointer_move(point);
        }
        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    self.pointer_left_unchecked();
                    return Ok(());
                }
                match self.startup_view {
                    StartupView::ScenarioBrowser => {
                        self.menu_state.set_pointer_position(Some(point));
                        let actions = self.scenario_game_options.handle_pointer_move(point);
                        self.finish_game_option_input(actions)?;
                        if self.scenario_selector_discovery.is_some() {
                            let _ = self.handle_scensel_search_pointer_move(point);
                            return Ok(());
                        }
                        if self.menu_state.current_map().is_some() {
                            return Ok(());
                        }
                        if self.handle_scensel_rename_pointer_move(point)
                            || self.handle_scensel_search_pointer_move(point)
                            || self.handle_scensel_scrollbar_move(point)
                        {
                            Ok(())
                        } else {
                            self.handle_menu_input(|state| state.menu().handle_pointer_move(point))
                        }
                    }
                    StartupView::NetworkGame => {
                        let fonts = self.assets.clonk_fonts.clone();
                        let actions = fonts
                            .as_deref()
                            .and_then(|fonts| {
                                self.startup_network_dialog
                                    .as_mut()
                                    .map(|dialog| dialog.handle_pointer_move(point, &fonts.text))
                            })
                            .unwrap_or_default();
                        self.process_network_dialog_actions(actions)
                    }
                    StartupView::PlayerSelection => {
                        if self
                            .startup_crew_rename
                            .as_ref()
                            .is_some_and(|rename| rename.edit.is_dragging())
                        {
                            if let Some(dialog) = self.startup_player_dialog.as_mut() {
                                dialog.set_pointer_position(Some(point));
                            }
                        }
                        if self.handle_startup_crew_rename_pointer_move(point) {
                            return Ok(());
                        }
                        let actions = self
                            .startup_player_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_pointer_move(point))
                            .unwrap_or_default();
                        self.process_player_dialog_actions(actions)
                    }
                    StartupView::MainMenu => {
                        self.main_menu_state.set_pointer_position(Some(point));
                        let actions = self.main_menu_state.handle_pointer_move(point);
                        self.process_main_menu_actions(actions)
                    }
                    StartupView::NetworkLobby => {
                        if self.network_lobby.is_some() {
                            let (width, height) = {
                                let surface = self.graphics.surface();
                                (surface.width() as f32, surface.height() as f32)
                            };
                            let region = self
                                .network_lobby
                                .as_mut()
                                .map(|lobby| {
                                    lobby.update_layout(width, height);
                                    lobby.pointer_region(point)
                                })
                                .unwrap_or(LobbyPointerRegion::Menu);
                            match region {
                                LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                    state.set_pointer_position(Some(point));
                                    state.menu().handle_pointer_move(point)
                                }),
                                LobbyPointerRegion::Panel => {
                                    self.handle_network_lobby_pointer_move(point)
                                }
                            }
                        } else {
                            Ok(())
                        }
                    }
                    StartupView::Options => {
                        let actions = self
                            .startup_options_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_pointer_move(point))
                            .unwrap_or_default();
                        self.process_options_dialog_actions(actions)?;
                        Ok(())
                    }
                    StartupView::About => {
                        let left_down = self.primary_pointer_left_down;
                        let actions = self
                            .startup_about_dialog
                            .as_mut()
                            .map(|dialog| {
                                dialog.handle_pointer_move_with_left_down(point, left_down)
                            })
                            .unwrap_or_default();
                        self.process_about_dialog_actions(actions)
                    }
                }
            }
            AppMode::Running => {
                if self.handle_scoreboard_pointer_move(point)? {
                    self.suspend_ingame_pointer_for_gui();
                    return Ok(());
                }
                if self.handle_ingame_menu_pointer_move(point) {
                    self.suspend_ingame_pointer_for_gui();
                    return Ok(());
                }
                let script_menu_owner = self.local_controls.mouse_owner();
                let script_menu_target = match script_menu_owner {
                    Some(owner) => {
                        match self.script_menu_pointer_target_for_owner(owner, point) {
                            Ok(target) => target,
                            Err(error) => {
                                // The pre-hit-test ordering keeps an active
                                // drag from crossing its threshold behind the
                                // menu. With no button gesture, retain native's
                                // projected viewport pointer even when menu
                                // resources fail before ownership resolves.
                                if self.mouse_state.is_none()
                                    && self.ingame_right_mouse_state.is_none()
                                {
                                    let _ = self.update_ingame_pointer(point);
                                }
                                return Err(error);
                            }
                        }
                    }
                    None => None,
                };
                if let Some(target) = script_menu_target {
                    if let EngineScriptMenuPointerTarget::Item(index) = target {
                        if self.select_script_menu_pointer_item(
                            script_menu_owner.expect("script-menu target has an owner"),
                            index,
                        )? {
                            self.refresh_after_script_menu_pointer();
                        }
                    }
                    // GUI hit-testing precedes MouseMoveToViewport. Do not mark
                    // this gesture moved merely because the menu owns the event.
                    self.running_gui_mouse_owned = true;
                    self.running_world_mouse_owned = false;
                    self.ingame_pointer = None;
                    self.ingame_edge_scroll = None;
                    self.ingame_mouse_caption = IngameMouseCaptionState::default();
                    return Ok(());
                }
                self.update_ingame_pointer(point)?;
                Ok(())
            }
            AppMode::Loading => Ok(()),
        }
    }

    pub(crate) fn suspend_ingame_pointer_for_gui(&mut self) {
        self.running_gui_mouse_owned = true;
        self.running_world_mouse_owned = false;
        if self.mode != AppMode::Running {
            return;
        }
        if let Some(state) = self.mouse_state.as_mut() {
            state.motion.moved = true;
        }
        if let Some(state) = self.ingame_right_mouse_state.as_mut() {
            state.motion.moved = true;
        }
        self.ingame_pointer = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
    }

    pub(crate) fn ingame_edge_cursor_active(&self) -> bool {
        let Some(scroll) = self.ingame_edge_scroll else {
            return false;
        };
        let Some(gui_point) = self.ingame_gui_pointer else {
            return false;
        };
        if self.mode != AppMode::Running
            || !self.window_active
            || !self.message_dialogs.is_empty()
            || self.startup_player_properties_dialog.is_some()
            || self.definition_selector.is_some()
            || self.context_menu.is_some()
            || self.game_option_input_dialog.is_some()
            || self.game_over_dialog.is_some()
            || self.scoreboard_close_pointer_capture
        {
            return false;
        }
        if self.scoreboard_pointer_target_cached(gui_point).is_some()
            || self.ingame_menu_pointer_target(gui_point).is_some()
            || self
                .ingame_viewport_region(scroll.owner, scroll.screen)
                .is_some()
        {
            return false;
        }
        matches!(
            self.script_menu_pointer_target_for_owner(scroll.owner, gui_point),
            Ok(None)
        )
    }

    pub(crate) fn ingame_help_cursor_active(&self) -> bool {
        let Some(pointer) = self.ingame_pointer else {
            return false;
        };
        self.ingame_mouse_help
            && matches!(self.mode, AppMode::Running)
            && self.window_active
            && self.ingame_mouse_controls_owner(pointer.owner)
            && self.message_dialogs.is_empty()
            && self.startup_player_properties_dialog.is_none()
            && self.definition_selector.is_none()
            && self.context_menu.is_none()
            && self.game_option_input_dialog.is_none()
            && self.game_over_dialog.is_none()
            && !self.scoreboard_close_pointer_capture
            && !self.ingame_edge_cursor_active()
    }

    pub(crate) fn ingame_custom_cursor_active(&self) -> bool {
        self.ingame_construction_drag_active()
            || self.ingame_edge_cursor_active()
            || self.ingame_help_cursor_active()
    }

    pub(crate) fn platform_cursor_visible(&self) -> bool {
        self.console_mode
            || classic_platform_cursor_visible(self.window_active, self.pointer_inside_window)
    }

    pub(crate) fn classic_gui_cursor_request(&self) -> Option<(GuiPoint, bool)> {
        let gui_owned = match self.mode {
            AppMode::Menu => true,
            AppMode::Loading => {
                self.network_start_wait
                    .as_ref()
                    .is_some_and(|wait| wait.visible)
                    || self.league_signup_dialog.is_some()
                    || !self.message_dialogs.is_empty()
            }
            AppMode::Running => self.running_gui_mouse_owned,
        };
        let position = (gui_owned && self.window_active && self.pointer_inside_window)
            .then_some(self.window_mouse_position)
            .flatten()?;
        Some((
            position,
            self.mode == AppMode::Running && self.ingame_mouse_help,
        ))
    }

    pub(crate) fn draw_classic_gui_cursor(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some((position, help)) = self.classic_gui_cursor_request() else {
            return false;
        };
        self.graphics.draw_gui_mouse_cursor(position, help, gamma)
    }

    pub(crate) fn draw_classic_gui_cursor_to_surface(
        &self,
        surface: &mut Surface,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> bool {
        let Some((position, help)) = self.classic_gui_cursor_request() else {
            return false;
        };
        self.graphics
            .draw_gui_mouse_cursor_to_surface(surface, position, help, gamma)
    }

    pub(crate) fn active_ingame_mouse_viewport(&self) -> Option<ActiveViewportProjection> {
        let projections = self.graphics.active_viewport_projections();
        match self.local_controls.mouse_owner() {
            Some(owner) => projections
                .into_iter()
                .find(|viewport| viewport.owner == owner),
            None => projections
                .into_iter()
                .find(|viewport| viewport.is_no_owner_viewport),
        }
    }

    pub(crate) fn ingame_mouse_controls_owner(&self, owner: i32) -> bool {
        let Some(viewport) = self.active_ingame_mouse_viewport() else {
            return false;
        };
        if viewport.owner != owner {
            return false;
        }
        match self.local_controls.mouse_owner() {
            Some(mouse_owner) => self.mouse_control && mouse_owner == owner,
            None => owner == OWNER_NONE && viewport.is_no_owner_viewport,
        }
    }

    pub(crate) fn reset_ingame_mouse_control(&mut self) {
        // C4MouseControl::Init calls Default, which restores fMouseOwned but
        // does not touch C4GUI::CMouse ownership. Both may therefore remain
        // true until the next platform move resolves one side.
        self.running_world_mouse_owned = true;
        self.ingame_mouse_init_centered = false;
        self.ingame_pointer = None;
        self.ingame_viewport_mouse = None;
        self.ingame_edge_scroll = None;
        self.ingame_mouse_help = false;
        self.ingame_mouse_help_caption = None;
        self.ingame_mouse_caption = IngameMouseCaptionState::default();
        self.cancel_ingame_mouse_gestures();
    }

    /// Consume C4MouseControl's first synthetic or button Move after `Init`.
    /// Native runs this from Execute on Tick5 even if the OS has not emitted
    /// motion, so later edge input must not be mistaken for the first move.
    pub(crate) fn initialize_ingame_mouse_center(&mut self) -> Result<bool, EngineError> {
        if self.ingame_mouse_init_centered
            || self.mode != AppMode::Running
            || !self.window_active
            || !self.message_dialogs.is_empty()
            || self.startup_player_properties_dialog.is_some()
            || self.definition_selector.is_some()
            || self.context_menu.is_some()
            || self.game_option_input_dialog.is_some()
            || self.game_over_dialog.is_some()
            || self.network_chart_pointer_capture
        {
            return Ok(false);
        }
        let Some(viewport) = self.active_ingame_mouse_viewport() else {
            return Ok(false);
        };
        let center = GuiPoint::new(
            viewport
                .rect
                .x
                .saturating_add(i32::try_from(viewport.rect.width / 2).unwrap_or(i32::MAX))
                as f32,
            viewport
                .rect
                .y
                .saturating_add(i32::try_from(viewport.rect.height / 2).unwrap_or(i32::MAX))
                as f32,
        );
        self.update_ingame_pointer(center)?;
        Ok(self.ingame_mouse_init_centered)
    }

    /// Wheel skips C4MouseControl's position block, but the Move prologue
    /// still consumes InitCentered before dispatching the wheel command.
    fn initialize_ingame_mouse_for_wheel(&mut self) {
        if !self.ingame_mouse_init_centered && self.active_ingame_mouse_viewport().is_some() {
            self.ingame_mouse_init_centered = true;
        }
    }

    pub(crate) fn clear_ingame_world_mouse_gestures(&mut self) {
        self.mouse_state = None;
        self.ingame_right_mouse_state = None;
        self.ingame_dragged_objects.clear();
        self.ingame_last_left_down = None;
        self.ingame_ignore_left_up = false;
    }

    pub(crate) fn update_ingame_pointer(&mut self, point: GuiPoint) -> Result<(), EngineError> {
        self.running_world_mouse_owned = true;
        self.advance_ingame_mouse_caption_lifetime();
        let moving_drag_before_move = self.ingame_moving_drag_active();
        let selection_drag_before_move = self.ingame_selection_drag_active();
        // GraphicsSystem::MouseMove applies ceil after dividing by the
        // presentation scale, before clamping into the assigned viewport.
        // The event path has already divided by scale and normally quantized
        // the running GUI point. Keep this seam idempotent for direct callers
        // so a fractional outside position still reaches the exact inclusive
        // border pixel (C4GraphicsSystem.cpp:445-484).
        let point = GuiPoint::new(point.x.ceil(), point.y.ceil());
        let mouse_owner = self.local_controls.mouse_owner();
        let viewport = self.active_ingame_mouse_viewport();
        let pointer = viewport.and_then(|viewport| {
            let point =
                if self.ingame_mouse_init_centered {
                    point
                } else {
                    // C4MouseControl::Move replaces the first coordinates after
                    // Init with ViewWdt/2, ViewHgt/2 before target/edge handling
                    // (C4MouseControl.cpp:216-239). SDL does not warp the OS
                    // cursor; only the retained gameplay point is centered.
                    GuiPoint::new(
                        viewport.rect.x.saturating_add(
                            i32::try_from(viewport.rect.width / 2).unwrap_or(i32::MAX),
                        ) as f32,
                        viewport.rect.y.saturating_add(
                            i32::try_from(viewport.rect.height / 2).unwrap_or(i32::MAX),
                        ) as f32,
                    )
                };
            self.graphics
                .viewport_output_point_for_index(viewport.index, point)
        });
        if let (Some(pointer), Some(viewport)) = (pointer, viewport) {
            self.ingame_mouse_init_centered = true;
            let observer = mouse_owner.is_none() && viewport.is_no_owner_viewport;
            self.ingame_viewport_mouse = Some(RetainedViewportMouse {
                viewport_index: viewport.index,
                owner: viewport.owner,
                observer,
                position: Vector2::new(
                    pointer.screen.x as i32 - viewport.rect.x,
                    pointer.screen.y as i32 - viewport.rect.y,
                ),
            });
            let over_region = self
                .ingame_viewport_region(pointer.owner, pointer.screen)
                .is_some();
            let fog_blocked = self.ingame_pointer_fog_blocked(pointer);
            let cancel_left_selection = over_region
                && self
                    .mouse_state
                    .is_some_and(|state| state.motion.moved && state.motion.selection_frame);
            let cancel_right_selection = over_region
                && self
                    .ingame_right_mouse_state
                    .is_some_and(|state| state.motion.moved && state.motion.selection_frame);
            let refresh_left_region_drag = self
                .mouse_state
                .is_some_and(|state| state.motion.region_drag_started);
            let refresh_right_region_drag = self
                .ingame_right_mouse_state
                .is_some_and(|state| state.motion.region_drag_started);
            let mut left_region_drag = None;
            let mut left_world_drag = None;
            if let Some(state) = self.mouse_state.as_mut() {
                if state.update_with_fog(pointer, fog_blocked) {
                    left_region_drag = state.motion.down_region.and_then(|region| match region {
                        IngameViewportRegion::Inventory(target) => Some(target),
                        IngameViewportRegion::Command(_)
                        | IngameViewportRegion::ViewportButton(_) => None,
                    });
                    if !state.down_region && !state.motion.selection_frame {
                        left_world_drag = state.down_target.map(|target| {
                            (
                                state.motion.start.owner,
                                target,
                                ingame_pointer_world_pixel(state.motion.start),
                            )
                        });
                    }
                }
            }
            let mut right_region_drag = None;
            let mut right_world_drag = None;
            if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                if state.update_with_fog(pointer, fog_blocked) {
                    right_region_drag = state.motion.down_region.and_then(|region| match region {
                        IngameViewportRegion::Inventory(target) => Some(target),
                        IngameViewportRegion::Command(_)
                        | IngameViewportRegion::ViewportButton(_) => None,
                    });
                    if !state.down_region && !state.motion.selection_frame {
                        right_world_drag = state.down_target.map(|target| {
                            (
                                state.motion.start.owner,
                                target,
                                ingame_pointer_world_pixel(state.motion.start),
                            )
                        });
                    }
                }
            }
            if let Some(target) = left_region_drag {
                let source = self.engine.mouse_region_drag_source(target);
                if source.is_some() {
                    self.ingame_dragged_objects =
                        self.engine.mouse_region_drag_objects(target, false);
                }
                if let Some(state) = self.mouse_state.as_mut() {
                    state.motion.region_drag_started = source.is_some();
                }
            }
            if let Some(target) = right_region_drag {
                let source = self.engine.mouse_region_drag_source(target);
                if source.is_some() {
                    self.ingame_dragged_objects =
                        self.engine.mouse_region_drag_objects(target, true);
                }
                if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                    state.motion.region_drag_started = source.is_some();
                }
            }
            if let Some((owner, target, position)) = left_world_drag {
                let started = self
                    .engine
                    .mouse_world_drag_source(owner, target, position)
                    .is_some();
                if let Some(state) = self.mouse_state.as_mut() {
                    state.motion.world_drag_started = started;
                }
            }
            if let Some((owner, target, position)) = right_world_drag {
                let started = self
                    .engine
                    .mouse_world_drag_source(owner, target, position)
                    .is_some();
                if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                    state.motion.world_drag_started = started;
                }
            }
            if refresh_left_region_drag || refresh_right_region_drag {
                let region_drag_cursor = self.current_ingame_region_drag_cursor(pointer);
                if let Some(state) = self.mouse_state.as_mut() {
                    if refresh_left_region_drag {
                        state.motion.region_drag_cursor = region_drag_cursor;
                    }
                }
                if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                    if refresh_right_region_drag {
                        state.motion.region_drag_cursor = region_drag_cursor;
                    }
                }
            }
            if cancel_left_selection || cancel_right_selection {
                self.cancel_ingame_selection_for_region(
                    cancel_left_selection,
                    cancel_right_selection,
                );
            }
            self.ingame_pointer = Some(pointer);
            self.update_ingame_drag_selection_kinds();
            self.refresh_ingame_mouse_help_region_caption(pointer);
            // A fallible script-menu region lookup must never leave a prior
            // border direction armed after this new pointer position.
            self.ingame_edge_scroll = None;
            let target_region = self
                .ingame_viewport_region(pointer.owner, pointer.screen)
                .is_some()
                || (!self.ingame_construction_drag_active()
                    && self
                        .script_menu_pointer_target_for_owner(pointer.owner, point)?
                        .is_some());
            self.ingame_edge_scroll = if target_region {
                None
            } else {
                viewport_edge_scroll(viewport.rect, pointer.screen).map(|edge| {
                    ActiveViewportEdgeScroll {
                        viewport_index: viewport.index,
                        owner: viewport.owner,
                        observer,
                        screen: pointer.screen,
                        edge,
                    }
                })
            };
            if self.apply_ingame_edge_scroll()? {
                self.snapshot = self.engine.snapshot();
            }
            self.advance_ingame_mouse_caption(
                pointer,
                moving_drag_before_move,
                selection_drag_before_move,
            );
        } else {
            if let Some(state) = self.mouse_state.as_mut() {
                state.motion.moved = true;
            }
            if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                state.motion.moved = true;
            }
            self.ingame_pointer = None;
            self.ingame_viewport_mouse = None;
            self.ingame_edge_scroll = None;
            self.ingame_mouse_caption = IngameMouseCaptionState::default();
        }
        Ok(())
    }

    /// C4GraphicsSystem::Execute releases CMouse only after the last shown
    /// C4GUI dialog closes, then sends the retained point straight to the
    /// viewport mouse without repeating GUI hit-testing.
    pub(crate) fn reconcile_running_mouse_after_last_gui_close(
        &mut self,
        external_menu_shown: bool,
    ) -> Result<(), EngineError> {
        if self.mode != AppMode::Running
            || !self.running_gui_mouse_owned
            || self.running_classic_gui_is_active(external_menu_shown)
        {
            return Ok(());
        }
        self.running_gui_mouse_owned = false;
        if self.running_world_mouse_owned {
            // SetMouseInGUI(false, false) only replays the retained GUI point
            // when C4MouseControl did not already own the mouse. A preceding
            // MouseControl::Init leaves the world cursor at its own position.
            return Ok(());
        }
        self.running_world_mouse_owned = true;
        let Some(point) = self
            .window_mouse_position
            .filter(|_| self.window_active && self.pointer_inside_window)
        else {
            return Ok(());
        };
        self.running_pointer_position = Some(point);
        self.ingame_gui_pointer = Some(point);
        self.update_ingame_pointer(point)
    }

    fn current_ingame_region_drag_cursor(
        &mut self,
        pointer: ViewportPointer,
    ) -> Option<IngameRegionDragCursor> {
        if self.ingame_pointer_fog_blocked(pointer) {
            // DragMoving forces Cursor_Nothing throughout fog, including
            // drags that originated in a viewport inventory region.
            return None;
        }
        let carryable = self.ingame_dragged_objects.iter().find_map(|object| {
            self.engine
                .object_snapshot(*object)
                .filter(|object| object.status != clonk_engine::ObjectStatus::Deleted)
                .map(|object| object.ocf & clonk_engine::ocf::CARRYABLE != 0)
        })?;
        let put_target = self
            .keyboard_modifiers
            .control_key()
            .then(|| {
                self.graphics.object_at_point_with_ocf(
                    &self.snapshot,
                    self.local_owner,
                    pointer.screen,
                    clonk_engine::ocf::CONTAINER,
                )
            })
            .flatten();
        if carryable {
            if let Some(target) = put_target {
                return Some(IngameRegionDragCursor::Put(target));
            }
            return match self
                .engine
                .mouse_drag_carryable_command(self.local_owner, ingame_pointer_world_pixel(pointer))
            {
                Some(CommandId::Drop) => Some(IngameRegionDragCursor::Drop),
                Some(CommandId::Throw) => Some(IngameRegionDragCursor::Throw),
                _ => None,
            };
        }
        Some(match put_target {
            Some(target) => IngameRegionDragCursor::VehiclePut(target),
            None => IngameRegionDragCursor::Vehicle,
        })
    }

    pub(crate) fn refresh_ingame_region_drag_cursor_for_execute(&mut self) {
        if !self.engine.frame().is_multiple_of(5) {
            return;
        }
        let refresh_left = self
            .mouse_state
            .is_some_and(|state| state.motion.region_drag_started);
        let refresh_right = self
            .ingame_right_mouse_state
            .is_some_and(|state| state.motion.region_drag_started);
        if !refresh_left && !refresh_right {
            return;
        }
        let Some(pointer) = self.ingame_pointer else {
            return;
        };
        let cursor = self.current_ingame_region_drag_cursor(pointer);
        if refresh_left {
            if let Some(state) = self.mouse_state.as_mut() {
                state.motion.region_drag_cursor = cursor;
            }
        }
        if refresh_right {
            if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                state.motion.region_drag_cursor = cursor;
            }
        }
    }

    pub(crate) fn ingame_menu_pointer_target(
        &self,
        point: GuiPoint,
    ) -> Option<(i32, IngameMenuPointerTarget)> {
        if self.engine.film_replay() {
            return None;
        }
        let player = match self.local_controls.mouse_owner() {
            Some(player) if self.mouse_control => player,
            None if self
                .active_ingame_mouse_viewport()
                .is_some_and(|viewport| viewport.is_no_owner_viewport) =>
            {
                OWNER_NONE
            }
            _ => return None,
        };
        if !self.ingame_menu_owner_has_visible_surface(player) {
            return None;
        }
        let area = if player == OWNER_NONE {
            let surface = self.graphics.surface();
            Rect::new(0, 0, surface.width(), surface.height())
        } else {
            self.graphics.viewport_rect(player)?
        };
        let menu = self.ingame_menu.get(player)?;
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let gfx = IngameMenuGraphics {
            show_commands: self.display_flags.show_commands,
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        menu.pointer_target(area, &font, &gfx, point)
            .map(|target| (player, target))
    }

    /// Route wheel input through the external dialog first. The dialog owns
    /// every point in its bounds, while only its ScrollWindow client mutates
    /// the pixel offset.
    fn handle_ingame_menu_wheel(&mut self, point: GuiPoint, amount: i32) -> bool {
        let Some((player, _)) = self.ingame_menu_pointer_target(point) else {
            return false;
        };
        let Some(area) = self.ingame_menu_area(player) else {
            return true;
        };
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let gfx = IngameMenuGraphics {
            show_commands: self.display_flags.show_commands,
            show_close_button: true,
            ..IngameMenuGraphics::default()
        };
        let _ = self.ingame_menu.get(player).is_some_and(|menu| {
            menu.client_contains(area, &font, &gfx, point)
                && menu.scroll_by(amount, area, &font, &gfx)
        });
        true
    }

    fn handle_ingame_menu_pointer_move(&mut self, point: GuiPoint) -> bool {
        let Some((player, target)) = self.ingame_menu_pointer_target(point) else {
            return false;
        };
        let mut preview_target = None;
        if let IngameMenuPointerTarget::Item(index) = target {
            if let Some(menu) = self.ingame_menu.get_mut(player) {
                // C4MenuItem::MouseEnter directly selects the hovered item
                // (C4Menu.cpp:239-244; C4MainMenu.cpp:299-303).
                if menu.selection() != index {
                    menu.set_selection(index);
                    preview_target = menu.selected_observer_target();
                }
            }
        }
        if let Some(target) = preview_target {
            let _ = self.apply_observer_target(target);
        }
        true
    }

    fn handle_ingame_menu_pointer_button(
        &mut self,
        button_state: ElementState,
        enter_all: bool,
    ) -> Result<bool, EngineError> {
        if !enter_all && button_state == ElementState::Pressed {
            self.ingame_menu_close_pointer_capture = None;
        }
        let close_capture = (!enter_all && button_state == ElementState::Released)
            .then(|| self.ingame_menu_close_pointer_capture.take())
            .flatten();
        let Some(point) = self.ingame_gui_pointer else {
            return Ok(false);
        };
        let Some((player, target)) = self.ingame_menu_pointer_target(point) else {
            return Ok(false);
        };
        self.cancel_ingame_mouse_gestures();
        if !enter_all
            && button_state == ElementState::Pressed
            && target == IngameMenuPointerTarget::Close
        {
            self.ingame_menu_close_pointer_capture = Some(player);
        }
        if !enter_all
            && button_state == ElementState::Pressed
            && target == IngameMenuPointerTarget::Title
        {
            self.arm_ingame_menu_title_drag(player, point);
        }
        if button_state == ElementState::Released {
            let outcome = match target {
                IngameMenuPointerTarget::Item(_) if close_capture.is_some() => None,
                IngameMenuPointerTarget::Item(index) => {
                    self.ingame_menu.get_mut(player).and_then(|menu| {
                        menu.set_selection(index);
                        menu.handle_command(
                            if enter_all {
                                ControlCommand::MenuEnterAll
                            } else {
                                ControlCommand::MenuEnter
                            },
                            CommandKind::Press,
                        )
                    })
                }
                // C4GUI::IconButton invokes Dialog::OnUserClose on left
                // button-up; right-button input is consumed without closing
                // (C4GuiDialogs.cpp:386-425; C4Gui.cpp:2029-2037).
                IngameMenuPointerTarget::Close if !enter_all && close_capture == Some(player) => {
                    self.ingame_menu.get_mut(player).and_then(|menu| {
                        menu.handle_command(ControlCommand::MenuClose, CommandKind::Press)
                    })
                }
                IngameMenuPointerTarget::Close
                | IngameMenuPointerTarget::Title
                | IngameMenuPointerTarget::Background => None,
            };
            if let Some(outcome) = outcome {
                // C4MenuItem enters on button-up; C4MainMenu executes its
                // own Player-owned command directly (C4Menu.cpp:213-233;
                // C4MainMenu.cpp:305-310).
                self.execute_ingame_menu_outcome_for_player(player, outcome)?;
            }
        }
        Ok(true)
    }

    fn ingame_mouse_selectable_object(&self, owner: i32, object: ObjectId) -> bool {
        self.snapshot.object(object).is_some_and(|snapshot| {
            snapshot.category & clonk_engine::CATEGORY_MOUSE_SELECT != 0
                || snapshot.ocf & clonk_engine::ocf::ALIVE != 0
                    && self
                        .snapshot
                        .players
                        .iter()
                        .find(|player| player.id == owner)
                        .is_some_and(|player| player.crew.contains(&object))
        })
    }

    pub(crate) fn ingame_pointer_fog_blocked(&self, pointer: ViewportPointer) -> bool {
        let point = ingame_pointer_world_pixel(pointer);
        if self.snapshot.landscape.as_ref().is_some_and(|landscape| {
            point.x < 0
                || point.y < 0
                || i64::from(point.x) >= i64::from(landscape.width())
                || point.y >= landscape.estimated_height()
        }) {
            return true;
        }
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == pointer.owner)
            .is_some_and(|player| {
                player.fog_of_war && !fow_point_is_visible(&self.snapshot, pointer.owner, point)
            })
    }

    pub(crate) fn ingame_primary_mouse_target(
        &self,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        let primary_ocf = clonk_engine::ocf::GRAB
            | clonk_engine::ocf::CHOP
            | clonk_engine::ocf::CONTAINER
            | clonk_engine::ocf::CONSTRUCT
            | clonk_engine::ocf::LIVING
            | clonk_engine::ocf::CARRYABLE
            | clonk_engine::ocf::EXCLUSIVE;
        let target =
            self.graphics
                .object_at_point_with_ocf(&self.snapshot, owner, point, primary_ocf)?;
        let blocked = self
            .graphics
            .viewport_point_at(point)
            .filter(|pointer| pointer.owner == owner)
            .is_some_and(|pointer| !self.ingame_fog_allows_target(pointer, target));
        (!blocked).then_some(target)
    }

    /// Help widens C4MouseControl's ordinary interaction mask to OCF_All,
    /// while retaining the same viewport visibility and fog gates.
    pub(crate) fn ingame_help_mouse_target(&self, owner: i32, point: GuiPoint) -> Option<ObjectId> {
        let target = self
            .graphics
            .object_at_point(&self.snapshot, owner, point)?;
        let blocked = self
            .graphics
            .viewport_point_at(point)
            .filter(|pointer| pointer.owner == owner)
            .is_some_and(|pointer| !self.ingame_fog_allows_target(pointer, target));
        (!blocked).then_some(target)
    }

    fn set_ingame_mouse_help_caption(&mut self, target: ObjectId, keep: bool) {
        self.ingame_mouse_caption.caption = None;
        let Some(text) = self.engine.object_help_caption(target) else {
            return;
        };
        let keep_moves = if keep {
            clonk_script::c4_string_bytes(&text).len() / 2
        } else {
            0
        };
        self.ingame_mouse_help_caption = Some(IngameMouseHelpCaption { text, keep_moves });
    }

    /// Top-of-`C4MouseControl::Move` caption lifetime update. A countdown
    /// reaching zero remains visible through that move and clears on the
    /// following one.
    fn advance_ingame_mouse_help_caption(&mut self) {
        match self.ingame_mouse_help_caption.as_mut() {
            Some(caption) if caption.keep_moves != 0 => caption.keep_moves -= 1,
            Some(_) => self.ingame_mouse_help_caption = None,
            None => {}
        }
    }

    pub(crate) fn advance_ingame_mouse_caption_lifetime(&mut self) {
        self.advance_ingame_mouse_help_caption();
        self.ingame_mouse_caption.begin_move();
    }

    pub(crate) fn refresh_ingame_mouse_help_region_caption(&mut self, pointer: ViewportPointer) {
        if !self.ingame_mouse_help {
            return;
        }
        if let Some(IngameViewportRegion::Inventory(target)) =
            self.ingame_viewport_region(pointer.owner, pointer.screen)
        {
            self.set_ingame_mouse_help_caption(target, false);
        }
    }

    pub(crate) fn ingame_mouse_select_target(
        &self,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        self.ingame_primary_mouse_target(owner, point)
            .filter(|object| self.ingame_mouse_selectable_object(owner, *object))
    }

    pub(crate) fn localized_ingame_mouse_caption(
        &self,
        key: &str,
        fallback: &str,
        arguments: &[&str],
        double_click: bool,
    ) -> String {
        let template = self
            .startup_tooltip_resources
            .get(key)
            .cloned()
            .unwrap_or_else(|| fallback.to_string());
        let mut caption = format_resource_string(template, arguments);
        if double_click {
            caption.push('|');
            caption.push_str(
                self.startup_tooltip_resources
                    .get("IDS_CON_DOUBLECLICK")
                    .map(String::as_str)
                    .unwrap_or("(Double click)"),
            );
        }
        caption
    }

    fn ingame_mouse_cursor_kind(cursor: MouseWorldCursor) -> IngameMouseCursorKind {
        match cursor {
            MouseWorldCursor::Crosshair => IngameMouseCursorKind::Crosshair,
            MouseWorldCursor::Dig { material: false } => IngameMouseCursorKind::Dig,
            MouseWorldCursor::Dig { material: true } => IngameMouseCursorKind::DigMaterial,
            MouseWorldCursor::Enter(_) => IngameMouseCursorKind::Enter,
            MouseWorldCursor::Grab(_) => IngameMouseCursorKind::Grab,
            MouseWorldCursor::Ungrab(_) => IngameMouseCursorKind::Ungrab,
            MouseWorldCursor::Carryable(_) => IngameMouseCursorKind::Carryable,
            MouseWorldCursor::DigObject(_) => IngameMouseCursorKind::DigObject,
            MouseWorldCursor::Chop(_) => IngameMouseCursorKind::Chop,
            MouseWorldCursor::Build(_) => IngameMouseCursorKind::Build,
            MouseWorldCursor::Select(_) => IngameMouseCursorKind::Select,
            MouseWorldCursor::Attack(_) => IngameMouseCursorKind::Attack,
            MouseWorldCursor::JumpLeft => IngameMouseCursorKind::JumpLeft,
            MouseWorldCursor::JumpRight => IngameMouseCursorKind::JumpRight,
        }
    }

    pub(crate) fn ingame_world_cursor_caption(
        &self,
        cursor: MouseWorldCursor,
        point: Vector2,
    ) -> Option<String> {
        let target_caption = |key: &str, fallback: &str, target: ObjectId, double_click: bool| {
            let name = self.ingame_object_caption_name(target)?;
            Some(self.localized_ingame_mouse_caption(key, fallback, &[name.as_str()], double_click))
        };
        match cursor {
            MouseWorldCursor::Select(target) => {
                target_caption("IDS_CON_SELECT", "Select %s.", target, false)
            }
            MouseWorldCursor::JumpLeft | MouseWorldCursor::JumpRight => {
                Some(self.localized_ingame_mouse_caption("IDS_CON_JUMP", "Jump.", &[], false))
            }
            MouseWorldCursor::Grab(target) => {
                target_caption("IDS_CON_GRAB", "Grab %s.", target, true)
            }
            MouseWorldCursor::Ungrab(target) => {
                target_caption("IDS_CON_UNGRAB", "Let go of %s.", target, true)
            }
            MouseWorldCursor::Build(target) => {
                target_caption("IDS_CON_BUILD", "Build %s.", target, true)
            }
            MouseWorldCursor::Chop(target) => {
                target_caption("IDS_CON_CHOP", "Chop %s.", target, true)
            }
            MouseWorldCursor::Carryable(target) => {
                target_caption("IDS_CON_COLLECT", "Collect %s.", target, true)
            }
            MouseWorldCursor::DigObject(target) => {
                target_caption("IDS_CON_DIGOUT", "Dig out %s.", target, true)
            }
            MouseWorldCursor::Enter(target) => {
                target_caption("IDS_CON_ENTER", "Enter %s.", target, true)
            }
            MouseWorldCursor::Attack(target) => {
                target_caption("IDS_CON_ATTACK", "Attack %s.", target, true)
            }
            MouseWorldCursor::Dig { material: true } => {
                let material = self
                    .snapshot
                    .landscape
                    .as_ref()
                    .and_then(|landscape| landscape.material_at(point.x, point.y))?;
                let material = self.engine.materials().get_by_id(material)?;
                let name = material
                    .dig_to_object_name()
                    .and_then(|definition| self.engine.definition_name(definition))
                    .map(c4_presentation_text)
                    .unwrap_or_default();
                Some(self.localized_ingame_mouse_caption(
                    "IDS_CON_DIGOUT",
                    "Dig out %s.",
                    &[name.as_str()],
                    true,
                ))
            }
            MouseWorldCursor::Crosshair | MouseWorldCursor::Dig { material: false } => None,
        }
    }

    fn set_ingame_mouse_caption(&mut self, text: String, caption_bottom_y: Option<i32>) {
        self.ingame_mouse_help_caption = None;
        let Some(retained) = self.ingame_viewport_mouse else {
            return;
        };
        let viewport_y = self
            .graphics
            .active_viewport_projections()
            .into_iter()
            .find(|viewport| viewport.index == retained.viewport_index)
            .map(|viewport| viewport.rect.y);
        let Some(viewport_y) = viewport_y else {
            return;
        };
        self.ingame_mouse_caption.caption = Some(IngameMouseCaption {
            text,
            viewport_index: retained.viewport_index,
            position: retained.position,
            caption_bottom_y: caption_bottom_y.map(|bottom| bottom - viewport_y),
        });
    }

    fn restore_ingame_mouse_region_caption(&mut self) -> bool {
        let Some(pointer) = self.ingame_pointer else {
            return false;
        };
        let Some(region) = self.ingame_viewport_region(pointer.owner, pointer.screen) else {
            return false;
        };
        self.ingame_mouse_caption.cursor = IngameMouseCursorKind::Region;
        match region {
            IngameViewportRegion::ViewportButton(button) => {
                let viewport = self
                    .graphics
                    .active_viewport_projections()
                    .into_iter()
                    .rev()
                    .find(|viewport| {
                        viewport.owner == pointer.owner
                            && viewport.contains_output_point((pointer.screen.x, pointer.screen.y))
                    });
                if let Some(viewport) = viewport {
                    let (key, fallback) = match button {
                        clonk_frontend::hud::ViewportButton::Help => ("IDS_CON_HELP", "Help"),
                        clonk_frontend::hud::ViewportButton::PlayerMenu => {
                            ("IDS_CON_PLAYERMENU", "Player menu")
                        }
                        clonk_frontend::hud::ViewportButton::Chat => ("IDS_DLG_CHAT", "Chat"),
                    };
                    let caption = self.localized_ingame_mouse_caption(key, fallback, &[], false);
                    let rect = clonk_frontend::hud::viewport_button_rect(viewport.rect, button);
                    self.set_ingame_mouse_caption(caption, Some(rect.y));
                }
            }
            IngameViewportRegion::Command(_) => {
                if let Some((_, caption, rect)) =
                    self.ingame_command_region_hit(pointer.owner, pointer.screen)
                {
                    self.set_ingame_mouse_caption(caption, Some(rect.y));
                }
            }
            IngameViewportRegion::Inventory(target) if self.ingame_mouse_help => {
                self.set_ingame_mouse_help_caption(target, false);
            }
            IngameViewportRegion::Inventory(target) => {
                if let (Some(name), Some((_, rect))) = (
                    self.ingame_object_caption_name(target),
                    self.ingame_inventory_region_hit(pointer.owner, pointer.screen),
                ) {
                    self.set_ingame_mouse_caption(name, Some(rect.y));
                }
            }
        }
        true
    }

    pub(crate) fn advance_ingame_mouse_caption(
        &mut self,
        pointer: ViewportPointer,
        moving_drag_before_move: bool,
        selection_drag_before_move: bool,
    ) {
        let over_region = self.restore_ingame_mouse_region_caption();

        if moving_drag_before_move && self.ingame_moving_drag_active() {
            if let Some((kind, caption)) = self.ingame_moving_drag_caption(pointer) {
                self.ingame_mouse_caption.cursor = kind;
                if let Some(caption) = caption {
                    self.set_ingame_mouse_caption(caption, None);
                }
            }
            return;
        }

        if self.ingame_construction_drag_active() {
            self.ingame_mouse_caption.cursor = IngameMouseCursorKind::Construct;
            return;
        }

        if over_region {
            return;
        }

        if let Some(scroll) = self.ingame_edge_scroll {
            self.ingame_mouse_caption.cursor = IngameMouseCursorKind::Scrolling(scroll.edge.cursor);
            return;
        }

        if selection_drag_before_move && self.ingame_selection_drag_active() {
            return;
        }

        if self.ingame_mouse_help {
            let show_caption = self.advance_ingame_time_on_target(IngameMouseCursorKind::Help);
            if show_caption && self.ingame_mouse_help_caption.is_none() {
                let caption =
                    self.localized_ingame_mouse_caption("IDS_CON_HELP", "Help", &[], false);
                self.set_ingame_mouse_caption(caption, None);
            }
            return;
        }

        if !self.mouse_control {
            self.ingame_mouse_caption.cursor = if pointer.owner == OWNER_NONE {
                IngameMouseCursorKind::Region
            } else {
                IngameMouseCursorKind::Nothing
            };
            return;
        }
        let point = ingame_pointer_world_pixel(pointer);
        let target = self.ingame_primary_mouse_target(pointer.owner, pointer.screen);
        let cursor = self.engine.mouse_world_cursor(
            pointer.owner,
            target,
            point,
            self.keyboard_modifiers.control_key(),
        );
        if self.ingame_pointer_fog_blocked(pointer)
            && target.is_none()
            && !matches!(
                cursor,
                MouseWorldCursor::JumpLeft | MouseWorldCursor::JumpRight
            )
        {
            self.ingame_mouse_caption.cursor = IngameMouseCursorKind::Nothing;
            return;
        }
        let kind = Self::ingame_mouse_cursor_kind(cursor);
        let show_caption = self.advance_ingame_time_on_target(kind);
        if show_caption {
            if let Some(caption) = self.ingame_world_cursor_caption(cursor, point) {
                self.set_ingame_mouse_caption(caption, None);
            }
        }
    }

    pub(crate) fn handle_ingame_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        self.advance_ingame_mouse_caption_lifetime();
        if self.ingame_construction_drag_active() {
            self.restore_ingame_mouse_region_caption();
            if button_state == ElementState::Released {
                self.finish_construction_menu_drag()?;
            }
            return Ok(());
        }
        let candidate_release = button_state == ElementState::Released
            && matches!(
                self.construction_menu_drag.as_ref(),
                Some(ConstructionMenuDrag::Candidate { .. })
            );
        // C4GraphicsSystem bypasses GUI mouse handling only for an active
        // moving/construct drag; ordinary releases over a menu stay GUI-owned.
        let moving_drag = self.ingame_moving_drag_active();
        if moving_drag && button_state == ElementState::Released {
            self.restore_ingame_mouse_region_caption();
            return self.on_ingame_mouse_up();
        }
        if button_state == ElementState::Pressed {
            self.script_menu_close_pointer_capture = None;
        }
        let script_close_capture = (button_state == ElementState::Released)
            .then(|| self.script_menu_close_pointer_capture.take())
            .flatten();
        let script_menu_owner = self.local_controls.mouse_owner();
        let script_menu_target = if moving_drag || self.ingame_mouse_help {
            None
        } else {
            match (script_menu_owner, self.ingame_gui_pointer) {
                (Some(owner), Some(gui_point)) => {
                    self.script_menu_pointer_target_for_owner(owner, gui_point)?
                }
                _ => None,
            }
        };
        if let Some(target) = script_menu_target {
            self.cancel_ingame_mouse_gestures();
            match button_state {
                ElementState::Pressed => match target {
                    EngineScriptMenuPointerTarget::Item(index) => {
                        let owner = script_menu_owner.expect("script-menu target has an owner");
                        self.select_script_menu_pointer_item(owner, index)?;
                        if let (Some(owner), Some(gui_point)) =
                            (script_menu_owner, self.ingame_gui_pointer)
                        {
                            // C4MenuItem::IsDragElement depends only on the
                            // raw item ID's Constructable definition, not on
                            // the row's ordinary menu selectability.
                            self.arm_construction_menu_drag(owner, index, gui_point);
                        }
                    }
                    EngineScriptMenuPointerTarget::Title => {
                        if let (Some(owner), Some(gui_point)) =
                            (script_menu_owner, self.ingame_gui_pointer)
                        {
                            self.arm_script_menu_title_drag(owner, gui_point)?;
                        }
                    }
                    EngineScriptMenuPointerTarget::Close => {
                        if let Some(owner) = script_menu_owner {
                            if let Some((target, _)) = self.engine.cursor_object_menu(owner) {
                                self.script_menu_close_pointer_capture = Some((owner, target));
                            }
                        }
                    }
                    EngineScriptMenuPointerTarget::Background => {}
                },
                ElementState::Released => {
                    if let Some((owner, captured_target)) = script_close_capture {
                        if target == EngineScriptMenuPointerTarget::Close
                            && script_menu_owner == Some(owner)
                            && self
                                .engine
                                .cursor_object_menu(owner)
                                .is_some_and(|(current, _)| current == captured_target)
                        {
                            self.dispatch_control_event_for_local_player(
                                owner,
                                ControlEvent::RawPlayerControl {
                                    command: clonk_engine::COM_MENU_CLOSE,
                                    data: 0,
                                },
                            )?;
                        }
                    } else if let EngineScriptMenuPointerTarget::Item(index) = target {
                        let owner = script_menu_owner.expect("script-menu target has an owner");
                        if self.select_script_menu_pointer_item(owner, index)? {
                            let data = i32::try_from(index).unwrap_or(i32::MAX);
                            self.dispatch_control_event_for_local_player(
                                owner,
                                ControlEvent::RawPlayerControl {
                                    command: clonk_engine::COM_MENU_ENTER,
                                    data,
                                },
                            )?;
                        }
                    }
                }
            }
            if button_state == ElementState::Released {
                self.refresh_after_script_menu_pointer();
            }
            return Ok(());
        }
        if script_close_capture.is_some() {
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        if candidate_release {
            // A sub-threshold release outside the source menu is still owned
            // by C4GUI's retained drag element and must not become a world
            // click merely because there is no release-time menu hit.
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        self.restore_ingame_mouse_region_caption();
        match button_state {
            ElementState::Pressed => {
                let now = Instant::now();
                let is_double = self.ingame_last_left_down.take().is_some_and(|last| {
                    now.saturating_duration_since(last) < CPP_DOUBLE_CLICK_INTERVAL
                });
                if is_double {
                    // The platform emits LeftDouble instead of a second
                    // LeftDown. C4MouseControl clears the down state and
                    // consumes the subsequent LeftUp (cpp:982-988).
                    self.mouse_state = None;
                    self.ingame_ignore_left_up = true;
                    self.on_ingame_mouse_double()
                } else {
                    self.ingame_last_left_down = Some(now);
                    self.ingame_ignore_left_up = false;
                    self.on_ingame_mouse_down()
                }
            }
            ElementState::Released => {
                if std::mem::take(&mut self.ingame_ignore_left_up) {
                    if let Some(pointer) = self.ingame_pointer {
                        self.refresh_ingame_mouse_help_region_caption(pointer);
                    }
                    self.mouse_state = None;
                    Ok(())
                } else {
                    self.on_ingame_mouse_up()
                }
            }
        }
    }

    pub(crate) fn handle_scoreboard_pointer_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<bool, EngineError> {
        if button_state == ElementState::Pressed {
            self.scoreboard_close_pointer_capture = false;
        }
        let Some(point) = self.running_pointer_position else {
            return Ok(false);
        };
        let target = self.scoreboard_pointer_target(point)?;
        if button_state == ElementState::Released {
            let close_captured = std::mem::take(&mut self.scoreboard_close_pointer_capture);
            let title_dragged = self.scoreboard_runtime.title_drag.take().is_some();
            if close_captured {
                if target == Some(ScoreboardPointerTarget::Close) {
                    self.play_ui_sound("Click");
                    self.close_scoreboard_dialog();
                } else if self.scoreboard_runtime.close_hovered {
                    // A release can cross the button edge without an
                    // intermediate cursor event. Native MouseLeave calls
                    // SetUp(false) and emits the second ArrowHit here.
                    self.play_ui_sound("ArrowHit");
                    self.scoreboard_runtime.close_hovered = false;
                }
            }
            // CMouse clears pDragElement before release hit-testing. A release
            // back over this dialog remains consumed; one outside falls
            // through to the next shared dialog or the game world.
            let consumed = target.is_some();
            if consumed || close_captured || title_dragged {
                self.cancel_ingame_mouse_gestures();
            }
            return Ok(consumed);
        }

        if target.is_some() {
            self.activate_running_dialog(RunningDialogStackEntry::Scoreboard);
        }
        match target {
            Some(ScoreboardPointerTarget::Close) => {
                self.scoreboard_close_pointer_capture = true;
                self.scoreboard_runtime.close_hovered = true;
                self.play_ui_sound("ArrowHit");
            }
            Some(ScoreboardPointerTarget::Title) => {
                if let Some(layout) = self
                    .scoreboard_runtime
                    .presentation
                    .as_ref()
                    .map(|presentation| presentation.layout())
                {
                    self.scoreboard_runtime.title_drag = Some(ScoreboardTitleDrag {
                        pointer: point,
                        origin: (layout.bounds.x, layout.bounds.y),
                    });
                }
            }
            Some(ScoreboardPointerTarget::Dialog) | None => {}
        }
        if target.is_some() {
            self.cancel_ingame_mouse_gestures();
        }
        Ok(target.is_some())
    }

    fn handle_scoreboard_pointer_move(&mut self, point: GuiPoint) -> Result<bool, EngineError> {
        if self.scoreboard_dialog.is_none() {
            return Ok(false);
        }
        self.scoreboard_runtime.pointer = Some(point);
        let dragging = self.update_scoreboard_title_drag(point);
        let target = self.scoreboard_pointer_target(point)?;
        let close_hovered = target == Some(ScoreboardPointerTarget::Close);
        if self.scoreboard_close_pointer_capture
            && self.scoreboard_runtime.close_hovered != close_hovered
        {
            // MouseLeave calls SetUp(false), and re-entry calls SetDown().
            self.play_ui_sound("ArrowHit");
        }
        self.scoreboard_runtime.close_hovered = close_hovered;
        Ok(dragging || self.scoreboard_close_pointer_capture || target.is_some())
    }

    pub(crate) fn scoreboard_pointer_left(&mut self) {
        if self.scoreboard_close_pointer_capture && self.scoreboard_runtime.close_hovered {
            self.play_ui_sound("ArrowHit");
        }
        self.scoreboard_close_pointer_capture = false;
        self.scoreboard_runtime.pointer = None;
        self.scoreboard_runtime.title_drag = None;
        self.scoreboard_runtime.close_hovered = false;
    }

    fn scoreboard_contains_running_pointer(&mut self) -> Result<bool, EngineError> {
        let Some(point) = self.running_pointer_position else {
            return Ok(false);
        };
        Ok(self.scoreboard_pointer_target(point)?.is_some())
    }

    fn scoreboard_is_top_scoreboard_message_at_running_pointer(
        &mut self,
    ) -> Result<bool, EngineError> {
        let Some(point) = self.running_pointer_position else {
            return Ok(false);
        };
        Ok(matches!(
            self.top_running_shared_pointer_target(point, false)?,
            Some(RunningDialogStackEntry::Scoreboard)
        ))
    }

    fn handle_scoreboard_touch(
        &mut self,
        position: GuiPoint,
        phase: TouchPhase,
    ) -> Result<bool, EngineError> {
        self.running_pointer_position = Some(position);
        match phase {
            TouchPhase::Started => {
                self.handle_scoreboard_pointer_move(position)?;
                self.handle_scoreboard_pointer_button(ElementState::Pressed)
            }
            TouchPhase::Moved => self.handle_scoreboard_pointer_move(position),
            TouchPhase::Ended => {
                self.handle_scoreboard_pointer_move(position)?;
                self.handle_scoreboard_pointer_button(ElementState::Released)
            }
            TouchPhase::Cancelled => {
                let captured = self.scoreboard_close_pointer_capture
                    || self.scoreboard_runtime.title_drag.is_some();
                self.scoreboard_pointer_left();
                Ok(captured)
            }
        }
    }

    fn handle_runtime_default_dialog_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }
        for dialog_kind in self
            .runtime_default_dialog_order_snapshot()
            .into_iter()
            .rev()
        {
            let captured = match dialog_kind {
                RuntimeDefaultDialog::ExternalIrc => {
                    self.handle_runtime_external_irc_pointer_move(point)?
                }
                RuntimeDefaultDialog::GameOver => {
                    if self.game_over_pointer_route_hit(point) {
                        let surface = self.graphics.surface();
                        let (width, height) = (surface.width(), surface.height());
                        if let Some(dialog) = self.game_over_dialog.as_mut() {
                            dialog.handle_pointer_move(point.x, point.y, width, height);
                        }
                        let sounds = self
                            .game_over_dialog
                            .as_mut()
                            .map(GameOverState::take_sound_events)
                            .unwrap_or_default();
                        self.play_game_over_sound_events(sounds);
                        true
                    } else {
                        false
                    }
                }
                RuntimeDefaultDialog::ClientList => {
                    self.handle_runtime_client_list_pointer_move(point)
                }
                RuntimeDefaultDialog::NetworkChart => self.network_chart_contains_point(point),
                RuntimeDefaultDialog::Scoreboard => self.handle_scoreboard_pointer_move(point)?,
            };
            if captured {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) fn handle_right_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        self.startup_tooltip.note_pointer_button();
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if self.mode == AppMode::Running && self.ingame_captured_drag_active() {
            return self.handle_ingame_right_mouse_button(button_state);
        }
        if button_state == ElementState::Pressed {
            // A context action closes on down. If its matching up was lost,
            // the next physical press starts a new gesture and invalidates
            // the old release latch before any underlying control sees it.
            self.context_menu_pointer_capture = None;
        }
        if self
            .consume_closed_context_pointer_release(button_state, ContextMenuPointerButton::Right)
        {
            return Ok(());
        }
        if self.network_chart_is_elevated_pointer_layer()
            && self.context_menu.is_none()
            && self
                .running_pointer_position
                .is_some_and(|point| self.network_chart_contains_point(point))
        {
            return Ok(());
        }
        let message_dialog_fallback_blocks_world = self.runtime_pointer_fallback_is_exclusive();
        let context_routed_before_running_dialogs = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && !self.running_dialog_stack.is_empty();
        if context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Right)?
        {
            self.occlude_running_dialog_pointer_hovers();
            return Ok(());
        }
        if self.mode == AppMode::Running {
            let shared_target = self
                .running_pointer_position
                .map(|point| self.top_running_shared_pointer_target(point, false))
                .transpose()?
                .flatten();
            if matches!(
                shared_target,
                Some(
                    RunningDialogStackEntry::Scoreboard
                        | RunningDialogStackEntry::Message(_)
                        | RunningDialogStackEntry::RuntimeClientList
                )
            ) || shared_target.is_none()
                && !self.network_chart_elevated
                && self.running_shared_gui_has_keyboard_focus()
            {
                self.suspend_ingame_pointer_for_gui();
                self.cancel_ingame_mouse_gestures();
                return Ok(());
            }
        } else if !self.message_dialogs.is_empty() && self.running_chat_controller().is_none() {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            if self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Right)?
            {
                return Ok(());
            }
            if self.context_menu.is_some() {
                return Ok(());
            }
            if button_state == ElementState::Pressed {
                let layout = self.league_signup_layout();
                let point = self.league_signup_pointer_position;
                let actions = point
                    .zip(layout.as_ref())
                    .and_then(|(point, layout)| {
                        self.league_signup_dialog.as_mut().map(|dialog| {
                            dialog.controller.request_context_menu_at(
                                point,
                                clipboard_text_available(),
                                layout,
                            )
                        })
                    })
                    .unwrap_or_default();
                self.process_league_signup_actions(actions)?;
            }
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            if button_state == ElementState::Pressed {
                let point = self
                    .startup_player_properties_dialog
                    .as_ref()
                    .and_then(|pending| pending.controller.pointer_position());
                let actions = point
                    .and_then(|point| {
                        self.startup_player_properties_dialog
                            .as_mut()
                            .map(|pending| pending.controller.handle_pointer_right_down(point))
                    })
                    .unwrap_or_default();
                self.process_startup_player_properties_actions(actions);
            }
            return Ok(());
        }
        if self.definition_selector.is_some() {
            return Ok(());
        }
        if !context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Right)?
        {
            return Ok(());
        }
        if !matches!(self.mode, AppMode::Running)
            && self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.is_info_only())
        {
            return Ok(());
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            if button_state == ElementState::Pressed {
                let outcome = self
                    .external_irc_dialog
                    .as_mut()
                    .and_then(|dialog| {
                        dialog.pointer_position().map(|point| {
                            dialog.request_context_menu_at(point, clipboard_text_available())
                        })
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(outcome.actions)?;
            }
            return Ok(());
        }
        if self.game_option_input_dialog.is_some()
            && self.game_option_input_owns_running_pointer_event()
        {
            if button_state == ElementState::Pressed {
                let point = self.game_option_input_pointer_position;
                let layout = self.game_option_input_layout();
                let outcome = point.zip(layout.as_ref()).and_then(|(point, layout)| {
                    self.game_option_input_dialog.as_mut().map(|dialog| {
                        dialog.controller.request_context_menu_at(
                            point,
                            layout,
                            clipboard_text_available(),
                            &InputDialogContextLabels::default(),
                        )
                    })
                });
                if let Some(outcome) = outcome {
                    self.finish_game_option_input_dialog_actions(outcome.actions)?;
                }
            }
            return Ok(());
        }
        if matches!(self.mode, AppMode::Running) {
            if let Some(point) = self.running_pointer_position {
                for dialog_kind in self
                    .runtime_default_dialog_order_snapshot()
                    .into_iter()
                    .rev()
                {
                    if !self.runtime_default_dialog_contains_point(dialog_kind, point)? {
                        continue;
                    }
                    match dialog_kind {
                        RuntimeDefaultDialog::ExternalIrc
                            if button_state == ElementState::Pressed =>
                        {
                            let outcome = self
                                .external_irc_dialog
                                .as_mut()
                                .map(|dialog| {
                                    dialog
                                        .request_context_menu_at(point, clipboard_text_available())
                                })
                                .unwrap_or_default();
                            self.process_network_dialog_actions(outcome.actions)?;
                        }
                        RuntimeDefaultDialog::ClientList => {
                            self.handle_runtime_client_list_pointer_move(point);
                        }
                        RuntimeDefaultDialog::Scoreboard
                        | RuntimeDefaultDialog::NetworkChart
                        | RuntimeDefaultDialog::GameOver
                        | RuntimeDefaultDialog::ExternalIrc => {}
                    }
                    self.suspend_ingame_pointer_for_gui();
                    self.cancel_ingame_mouse_gestures();
                    return Ok(());
                }
            }
        }
        if message_dialog_fallback_blocks_world {
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_secondary_button(button_state);
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.network_lobby.is_some()
        {
            return self.handle_network_lobby_secondary_button(button_state);
        }
        match self.mode {
            AppMode::Menu => {
                if button_state == ElementState::Pressed {
                    match self.startup_view {
                        StartupView::MainMenu => {
                            self.open_startup_participants_context_menu()?;
                        }
                        StartupView::ScenarioBrowser => {
                            self.open_scenario_search_context_menu(false)?;
                        }
                        StartupView::PlayerSelection => {
                            if self.startup_crew_rename.is_some() {
                                let point = self
                                    .startup_player_dialog
                                    .as_ref()
                                    .and_then(|dialog| dialog.pointer_position());
                                if let Some(point) = point.filter(|point| {
                                    self.startup_crew_rename_char_pos(*point, true).is_some()
                                }) {
                                    self.open_startup_crew_rename_context_menu(point)?;
                                    return Ok(());
                                }
                            }
                            self.open_startup_player_context_menu(false)?;
                        }
                        StartupView::NetworkGame => {
                            let outcome = self
                                .startup_network_dialog
                                .as_mut()
                                .and_then(|dialog| {
                                    dialog.pointer_position().map(|point| {
                                        dialog.request_context_menu_at(
                                            point,
                                            clipboard_text_available(),
                                        )
                                    })
                                })
                                .unwrap_or_default();
                            self.process_network_dialog_actions(outcome.actions)?;
                        }
                        _ => {}
                    }
                }
                Ok(())
            }
            AppMode::Running => {
                if self.ingame_mouse_help || self.ingame_captured_drag_active() {
                    self.handle_ingame_right_mouse_button(button_state)
                } else {
                    let scoreboard_hit = self.scoreboard_contains_running_pointer()?;
                    if scoreboard_hit
                        || self.handle_ingame_menu_pointer_button(button_state, true)?
                    {
                        Ok(())
                    } else {
                        self.initialize_ingame_mouse_center()?;
                        self.handle_ingame_right_mouse_button(button_state)
                    }
                }
            }
            AppMode::Loading => Ok(()),
        }
    }

    pub(crate) fn handle_other_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        self.startup_tooltip.note_pointer_button();
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if self.mode == AppMode::Running && self.ingame_captured_drag_active() {
            return Ok(());
        }
        if button_state == ElementState::Pressed {
            self.context_menu_pointer_capture = None;
            self.game_option_pointer_capture = false;
            let chat_hit = self.running_chat_controller().is_some()
                && self
                    .game_option_input_pointer_position
                    .zip(self.game_option_input_layout().as_ref())
                    .is_some_and(|(point, layout)| {
                        Self::point_in_input_dialog_bounds(point, layout)
                    });
            self.game_option_input_pointer_capture = (self.game_option_input_dialog.is_some()
                && self.context_menu.is_none()
                && if self.running_chat_controller().is_some() {
                    chat_hit
                } else {
                    self.message_dialogs.is_empty()
                })
            .then_some(ContextMenuPointerButton::Other);
        }
        let input_dialog_release_latched = button_state == ElementState::Released
            && self.game_option_input_pointer_capture == Some(ContextMenuPointerButton::Other);
        if input_dialog_release_latched {
            self.game_option_input_pointer_capture = None;
        }
        if self
            .consume_closed_context_pointer_release(button_state, ContextMenuPointerButton::Other)
        {
            return Ok(());
        }
        if self.network_chart_is_elevated_pointer_layer()
            && self.context_menu.is_none()
            && self
                .running_pointer_position
                .is_some_and(|point| self.network_chart_contains_point(point))
        {
            return Ok(());
        }
        let message_dialog_fallback_blocks_world = self.runtime_pointer_fallback_is_exclusive();
        let context_routed_before_running_dialogs = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && !self.running_dialog_stack.is_empty();
        if context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Other)?
        {
            self.occlude_running_dialog_pointer_hovers();
            return Ok(());
        }
        if self.mode == AppMode::Running {
            let shared_target = self
                .running_pointer_position
                .map(|point| self.top_running_shared_pointer_target(point, false))
                .transpose()?
                .flatten();
            if matches!(
                shared_target,
                Some(
                    RunningDialogStackEntry::Scoreboard
                        | RunningDialogStackEntry::Message(_)
                        | RunningDialogStackEntry::RuntimeClientList
                )
            ) || shared_target.is_none()
                && !self.network_chart_elevated
                && self.running_shared_gui_has_keyboard_focus()
            {
                self.suspend_ingame_pointer_for_gui();
                self.cancel_ingame_mouse_gestures();
                return Ok(());
            }
        }
        if self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only())
        {
            return Ok(());
        }
        if self.mode != AppMode::Running
            && !self.message_dialogs.is_empty()
            && self.running_chat_controller().is_none()
        {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            if self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Other)?
            {
                return Ok(());
            }
            if self.context_menu.is_some() {
                return Ok(());
            }
            if button_state == ElementState::Pressed {
                let point = self.league_signup_pointer_position;
                let layout = self.league_signup_layout();
                let fonts = self.assets.clonk_fonts.clone();
                let primary = primary_clipboard_text();
                let actions = point
                    .zip(layout.as_ref())
                    .zip(fonts.as_deref())
                    .and_then(|((point, layout), fonts)| {
                        self.league_signup_dialog.as_mut().map(|dialog| {
                            dialog.controller.handle_pointer_middle_down(
                                point,
                                primary.as_deref(),
                                layout,
                                &fonts.text,
                            )
                        })
                    })
                    .unwrap_or_default();
                self.process_league_signup_actions(actions)?;
            }
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            return Ok(());
        }
        if self.definition_selector.is_some() {
            return Ok(());
        }
        if !context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Other)?
        {
            return Ok(());
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            if button_state == ElementState::Pressed {
                let primary = primary_clipboard_text();
                let fonts = self.assets.clonk_fonts.clone();
                let outcome = fonts
                    .as_deref()
                    .and_then(|fonts| {
                        self.external_irc_dialog.as_mut().and_then(|dialog| {
                            dialog.pointer_position().map(|point| {
                                dialog.handle_pointer_middle_down(
                                    point,
                                    primary.as_deref(),
                                    &fonts.text,
                                )
                            })
                        })
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(outcome.actions)?;
            }
            return Ok(());
        }
        if self.game_option_input_dialog.is_some()
            && self.game_option_input_owns_running_pointer_event()
        {
            if button_state == ElementState::Pressed {
                let point = self.game_option_input_pointer_position;
                let layout = self.game_option_input_layout();
                let fonts = self.assets.clonk_fonts.clone();
                let primary = arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.get_text())
                    .ok();
                let outcome = point.zip(layout.as_ref()).zip(fonts.as_deref()).and_then(
                    |((point, layout), fonts)| {
                        self.game_option_input_dialog.as_mut().map(|dialog| {
                            dialog.controller.handle_pointer_middle_down(
                                point,
                                primary.as_deref(),
                                layout,
                                &fonts.text,
                            )
                        })
                    },
                );
                if let Some(outcome) = outcome {
                    self.finish_game_option_input_dialog_actions(outcome.actions)?;
                }
            }
            return Ok(());
        }
        if input_dialog_release_latched {
            return Ok(());
        }
        if matches!(self.mode, AppMode::Running) {
            if let Some(point) = self.running_pointer_position {
                for dialog_kind in self
                    .runtime_default_dialog_order_snapshot()
                    .into_iter()
                    .rev()
                {
                    if !self.runtime_default_dialog_contains_point(dialog_kind, point)? {
                        continue;
                    }
                    if dialog_kind == RuntimeDefaultDialog::ExternalIrc
                        && button_state == ElementState::Pressed
                    {
                        let primary = primary_clipboard_text();
                        let fonts = self.assets.clonk_fonts.clone();
                        let outcome = fonts
                            .as_deref()
                            .and_then(|fonts| {
                                self.external_irc_dialog.as_mut().map(|dialog| {
                                    dialog.handle_pointer_middle_down(
                                        point,
                                        primary.as_deref(),
                                        &fonts.text,
                                    )
                                })
                            })
                            .unwrap_or_default();
                        self.process_network_dialog_actions(outcome.actions)?;
                    }
                    return Ok(());
                }
            }
        }
        if message_dialog_fallback_blocks_world {
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::NetworkGame {
            if button_state == ElementState::Pressed {
                let primary = primary_clipboard_text();
                let fonts = self.assets.clonk_fonts.clone();
                let outcome = fonts
                    .as_deref()
                    .and_then(|fonts| {
                        self.startup_network_dialog.as_mut().and_then(|dialog| {
                            dialog.pointer_position().map(|point| {
                                dialog.handle_pointer_middle_down(
                                    point,
                                    primary.as_deref(),
                                    &fonts.text,
                                )
                            })
                        })
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(outcome.actions)?;
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu && self.startup_view == StartupView::ScenarioBrowser {
            if button_state == ElementState::Pressed {
                if let Some(point) = self.menu_state.pointer_position() {
                    if self.scensel_search_char_pos(point, true).is_some() {
                        let primary = primary_clipboard_text();
                        let before = self.menu_state.search_text().to_string();
                        self.handle_scensel_search_middle_down(point, primary.as_deref());
                        if self.menu_state.search_text() != before {
                            self.submit_scenario_search()?;
                        }
                    }
                }
            }
            return Ok(());
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::PlayerSelection
            && self.startup_crew_rename.is_some()
        {
            if button_state == ElementState::Pressed {
                let point = self
                    .startup_player_dialog
                    .as_ref()
                    .and_then(|dialog| dialog.pointer_position());
                if let Some(point) = point {
                    let primary = primary_clipboard_text();
                    self.handle_startup_crew_rename_middle_down(point, primary.as_deref());
                }
            }
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_middle_button(button_state);
        }
        if self.mode == AppMode::Menu
            && self.startup_view == StartupView::NetworkLobby
            && self.network_lobby.is_some()
        {
            return self.handle_network_lobby_middle_button(button_state);
        }
        if self.mode == AppMode::Running {
            if self.scoreboard_contains_running_pointer()? {
                self.suspend_ingame_pointer_for_gui();
                self.cancel_ingame_mouse_gestures();
                return Ok(());
            }
            if !self.initialize_ingame_mouse_center()? {
                self.advance_ingame_mouse_caption_lifetime();
                self.restore_ingame_mouse_region_caption();
            }
        }
        Ok(())
    }

    fn handle_ingame_right_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        self.advance_ingame_mouse_caption_lifetime();
        if self.ingame_construction_drag_active() {
            self.restore_ingame_mouse_region_caption();
            if button_state == ElementState::Released {
                self.finish_construction_menu_drag()?;
            }
            return Ok(());
        }
        if self.ingame_mouse_help && button_state == ElementState::Released {
            self.ingame_right_mouse_state = None;
            self.ingame_mouse_help = false;
            if let Some(caption) = self.ingame_mouse_help_caption.as_mut() {
                caption.keep_moves = 0;
            }
            self.ingame_dragged_objects.clear();
            return Ok(());
        }
        let moving_drag = self.ingame_moving_drag_active();
        let captured_release = button_state == ElementState::Released
            && self.ingame_right_mouse_state.is_some_and(|state| {
                state.motion.region_drag_started || state.motion.world_drag_started
            });
        let script_menu_owner = self.local_controls.mouse_owner();
        let script_menu_target = if moving_drag {
            None
        } else {
            match (script_menu_owner, self.ingame_gui_pointer) {
                (Some(owner), Some(gui_point)) => {
                    self.script_menu_pointer_target_for_owner(owner, gui_point)?
                }
                _ => None,
            }
        };
        if !captured_release {
            if let Some(target) = script_menu_target {
                self.cancel_ingame_mouse_gestures();
                if button_state == ElementState::Released {
                    if let EngineScriptMenuPointerTarget::Item(index) = target {
                        let owner = script_menu_owner.expect("script-menu target has an owner");
                        if self.select_script_menu_pointer_item(owner, index)? {
                            let data = i32::try_from(index).unwrap_or(i32::MAX);
                            self.dispatch_control_event_for_local_player(
                                owner,
                                ControlEvent::RawPlayerControl {
                                    command: clonk_engine::COM_MENU_ENTER_ALL,
                                    data,
                                },
                            )?;
                        }
                    }
                    self.refresh_after_script_menu_pointer();
                }
                return Ok(());
            }
        }
        if !self.mouse_control {
            self.ingame_right_mouse_state = None;
            self.ingame_dragged_objects.clear();
            return Ok(());
        }

        self.restore_ingame_mouse_region_caption();

        if button_state == ElementState::Pressed {
            let Some(pointer) = self.ingame_pointer else {
                self.ingame_right_mouse_state = None;
                return Ok(());
            };
            if pointer.owner != self.local_owner {
                self.ingame_right_mouse_state = None;
                return Ok(());
            }
            let region = self.ingame_viewport_region(self.local_owner, pointer.screen);
            let region_target = region.and_then(|region| match region {
                IngameViewportRegion::Inventory(target) => Some(target),
                IngameViewportRegion::Command(_) | IngameViewportRegion::ViewportButton(_) => None,
            });
            let down_target = region_target.or_else(|| {
                if region.is_some() {
                    return None;
                }
                if self.ingame_mouse_help {
                    self.ingame_help_mouse_target(self.local_owner, pointer.screen)
                } else {
                    self.ingame_primary_mouse_target(self.local_owner, pointer.screen)
                }
            });
            let mut state = IngameButtonMouseState::new(pointer, down_target, region.is_some());
            state.motion.down_region = region;
            state.down_cursor_help = self.ingame_mouse_help;
            if state.down_cursor_help {
                state.motion.selection_frame = false;
            }
            let fog_blocked = region.is_none() && self.ingame_pointer_fog_blocked(pointer);
            if fog_blocked {
                state.motion.selection_frame = false;
                state.down_cursor_nothing = down_target.is_none();
            }
            self.ingame_right_mouse_state = Some(state);
            if self.ingame_mouse_help {
                self.refresh_ingame_mouse_help_region_caption(pointer);
            }
            return Ok(());
        }

        let drag = self.ingame_right_mouse_state.take();
        if let Some(drag) = drag {
            if drag.motion.start.owner != self.local_owner {
                self.ingame_dragged_objects.clear();
                return Ok(());
            }
            if drag.motion.moved && !drag.motion.selection_cancelled_by_region {
                if drag.motion.region_drag_started {
                    let mut selected = std::mem::take(&mut self.ingame_dragged_objects);
                    selected.retain(|object| {
                        self.engine.object_snapshot(*object).is_some_and(|object| {
                            object.status != clonk_engine::ObjectStatus::Deleted
                        })
                    });
                    if self
                        .ingame_viewport_region(drag.motion.last.owner, drag.motion.last.screen)
                        .is_some()
                    {
                        return self.finish_ingame_noop_drag(drag.motion, selected.len());
                    }
                    if selected.is_empty() {
                        return Ok(());
                    }
                    let cursor = drag.motion.region_drag_cursor;
                    return self.finish_ingame_region_drag(drag.motion, selected, cursor);
                }
                // The over-world left-button drag path owns world-origin and
                // landscape-frame moving drags.
                // HUD-origin eligibility was latched at threshold above and
                // must never be reclassified from the live release cursor.
                if !drag.down_region && self.finish_ingame_moved_drag(drag, true)? {
                    return Ok(());
                }
            }
        }

        let Some(pointer) = self.ingame_pointer else {
            return Ok(());
        };
        if pointer.owner != self.local_owner {
            return Ok(());
        }
        // RightUpDragNone sends the copied DownRegion.RightCom whenever the
        // release cursor is a region. Inventory regions leave RightCom at
        // COM_None, so they consume the click without opening world context
        // or cycling crew (C4MouseControl.cpp:1230-1237).
        if self
            .ingame_viewport_region(self.local_owner, pointer.screen)
            .is_some()
        {
            return self.dispatch_control_event_for_local_player(
                self.local_owner,
                ControlEvent::RawPlayerControl {
                    command: 0,
                    data: 0,
                },
            );
        }
        let primary_target = self.ingame_primary_mouse_target(self.local_owner, pointer.screen);
        let context_target = primary_target.or_else(|| {
            self.graphics
                .object_at_point(&self.snapshot, self.local_owner, pointer.screen)
                .filter(|target| self.ingame_fog_allows_target(pointer, *target))
        });
        // RightUpDragNone makes one exact-object exclusion pass for the
        // windmill wing. Do not loop: another WWNG behind it is the target.
        let context_target = match context_target {
            Some(target)
                if self
                    .snapshot
                    .object(target)
                    .is_some_and(|object| object.definition_id == "WWNG") =>
            {
                // C++ does not re-run its fog gate after the excluded pick.
                self.graphics.object_at_point_excluding(
                    &self.snapshot,
                    self.local_owner,
                    pointer.screen,
                    target,
                )
            }
            target => target,
        };
        // A Select cursor queues its selection before the secondary context
        // lookup, even when that lookup falls through to select-next.
        if let Some(select_target) = primary_target
            .filter(|target| self.ingame_mouse_selectable_object(self.local_owner, *target))
        {
            self.submit_or_execute_player_select(PlayerSelectControlData {
                player: self.local_owner,
                objects: vec![select_target.as_u64() as i32],
                by_client: -1,
            })?;
        }
        if let Some(target) = context_target {
            self.show_startup_hint = false;
            let add_mode = 2 | if self.keyboard_modifiers.shift_key() {
                4
            } else {
                0
            };
            let (x, y) = self
                .graphics
                .active_viewport_projections()
                .into_iter()
                .find(|viewport| viewport.owner == self.local_owner)
                .map(|viewport| ingame_pointer_viewport_pixel(pointer, viewport))
                .unwrap_or((pointer.world.x as i32, pointer.world.y as i32));
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: self.local_owner,
                command: CommandId::Context as i32,
                x,
                y,
                target: 0,
                target2: target.as_u64() as i32,
                data: 0,
                add_mode,
                by_client: -1,
            })?;
        } else {
            // C4MouseControl::RightUpDragNone cycles crew on a free click by
            // queuing a one-object CID_PlrSelect packet.
            if let Some(next) = self
                .engine
                .player_mouse_select_next_object(self.local_owner)
            {
                self.submit_or_execute_player_select(PlayerSelectControlData {
                    player: self.local_owner,
                    objects: vec![next.as_u64() as i32],
                    by_client: -1,
                })?;
            }
            self.snapshot = self.engine.snapshot();
            self.refresh_focus();
        }
        Ok(())
    }

    pub(crate) fn script_menu_pointer_target(
        &self,
        point: GuiPoint,
    ) -> Result<Option<EngineScriptMenuPointerTarget>, EngineError> {
        self.script_menu_pointer_target_for_owner(self.local_owner, point)
    }

    pub(crate) fn script_menu_pointer_target_for_owner(
        &self,
        owner: i32,
        point: GuiPoint,
    ) -> Result<Option<EngineScriptMenuPointerTarget>, EngineError> {
        if self.engine.film_replay() || !self.mouse_control {
            return Ok(None);
        }
        if !self.menu_owner_has_unsuppressed_viewport(owner) {
            return Ok(None);
        }
        let Some((target, menu)) = self.engine.cursor_object_menu(owner) else {
            return Ok(None);
        };
        self.assets
            .require_classic_ingame_menu_resources()
            .map_err(|error| classic_parity_engine_error(report_classic_parity_boundary(error)))?;
        let fallback = self.assets.font_arc();
        let font = clonk_frontend::hud::HudFont::from_set(
            self.assets.clonk_fonts.as_deref(),
            fallback.as_ref(),
        );
        let area = self.graphics.viewport_rect(owner).unwrap_or_else(|| {
            let surface = self.graphics.surface();
            Rect::new(0, 0, surface.width(), surface.height())
        });
        let resources = self.script_text_spec_resources();
        let font_images =
            resolve_script_menu_font_images(&self.engine, menu, resources).map_err(|error| {
                classic_parity_engine_error(report_classic_parity_boundary(
                    ClassicParityBoundary::ScriptMenuPointerResources {
                        detail: error.to_string(),
                    },
                ))
            })?;
        let presentation = self
            .script_menu_presentations
            .get(&owner)
            .filter(|state| same_script_menu_presentation(state, target, menu));
        let location = presentation
            .and_then(|state| state.location)
            .or_else(|| self.script_menu_free_location(owner, menu));
        let scroll_y = presentation.map_or(0, |state| state.scroll_y);
        let explicit_lines = presentation.and_then(|state| state.explicit_lines);
        let use_free_anchor = presentation.map_or(location.is_some(), |state| {
            state.location_needs_initialization
        });
        Ok(if use_free_anchor {
            location.and_then(|free_location| {
                engine_script_menu_pointer_target_with_free_anchor(
                    area,
                    &font,
                    menu,
                    self.display_flags.show_commands,
                    true,
                    point,
                    &font_images,
                    free_location,
                    scroll_y,
                    explicit_lines,
                )
            })
        } else {
            engine_script_menu_pointer_target_with_presentation(
                area,
                &font,
                menu,
                self.display_flags.show_commands,
                true,
                point,
                &font_images,
                location,
                scroll_y,
                explicit_lines,
            )
        })
    }

    fn handle_script_menu_wheel(
        &mut self,
        point: GuiPoint,
        amount: i32,
    ) -> Result<bool, EngineError> {
        let Some(owner) = self.local_controls.mouse_owner() else {
            return Ok(false);
        };
        if !self.ensure_script_menu_presentation_for_owner(owner) {
            return Ok(false);
        }
        if self
            .script_menu_pointer_target_for_owner(owner, point)?
            .is_none()
        {
            return Ok(false);
        }
        let adjust_selection = self
            .script_menu_presentations
            .get(&owner)
            .is_none_or(|state| state.selection_needs_adjustment);
        let Some((target, layout)) = self.script_menu_layout_for_owner(owner, adjust_selection)?
        else {
            // Dialog-style menus have no normal C4Menu ScrollWindow client,
            // but their external dialog still consumes the wheel event.
            return Ok(true);
        };
        let menu_selection = self
            .engine
            .cursor_object_menu(owner)
            .filter(|(current, _)| *current == target)
            .map_or(-1, |(_, menu)| menu.selection);
        let client_contains = point.x >= layout.client.x as f32
            && point.y >= layout.client.y as f32
            && point.x < (layout.client.x + layout.client.width as i32) as f32
            && point.y < (layout.client.y + layout.client.height as i32) as f32;
        if let Some(state) = self
            .script_menu_presentations
            .get_mut(&owner)
            .filter(|state| state.key.target == target)
        {
            if state.location_needs_initialization {
                state.location = Some((layout.bounds.x, layout.bounds.y));
                state.location_needs_initialization = false;
            }
            state.scroll_y = if client_contains {
                layout
                    .scroll_y
                    .saturating_add(amount)
                    .clamp(0, layout.max_scroll_y)
            } else {
                layout.scroll_y
            };
            state.scroll_selection = menu_selection;
            state.selection_needs_adjustment = false;
        }
        Ok(true)
    }

    fn select_script_menu_pointer_item(
        &mut self,
        owner: i32,
        index: usize,
    ) -> Result<bool, EngineError> {
        let Some((selection, selectable)) =
            self.engine.cursor_object_menu(owner).and_then(|(_, menu)| {
                menu.items
                    .get(index)
                    .map(|item| (menu.selection, item.selectable))
            })
        else {
            return Ok(false);
        };
        if !selectable {
            return Ok(false);
        }
        if selection != index as i32 {
            let data =
                i32::try_from(index).unwrap_or(i32::MAX) | clonk_engine::C4MN_ADJUST_POSITION;
            self.dispatch_control_event_for_local_player(
                owner,
                ControlEvent::RawPlayerControl {
                    command: clonk_engine::COM_MENU_SELECT,
                    data,
                },
            )?;
        }
        Ok(true)
    }

    fn refresh_after_script_menu_pointer(&mut self) {
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
    }

    pub(crate) fn cancel_ingame_mouse_gestures(&mut self) {
        self.clear_ingame_world_mouse_gestures();
        self.construction_menu_drag = None;
    }

    fn on_ingame_mouse_down(&mut self) -> Result<(), EngineError> {
        let Some(pointer) = self.ingame_pointer else {
            self.mouse_state = None;
            return Ok(());
        };
        let region = self.ingame_viewport_region(pointer.owner, pointer.screen);
        if !self.ingame_mouse_controls_owner(pointer.owner)
            || (!self.mouse_control
                && !self.ingame_mouse_help
                && !matches!(region, Some(IngameViewportRegion::ViewportButton(_))))
        {
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        if self.ingame_mouse_help {
            self.refresh_ingame_mouse_help_region_caption(pointer);
        }
        let region_target = region.and_then(|region| match region {
            IngameViewportRegion::Inventory(target) => Some(target),
            IngameViewportRegion::Command(_) | IngameViewportRegion::ViewportButton(_) => None,
        });
        let down_target = region_target.or_else(|| {
            region
                .is_none()
                .then(|| {
                    if self.ingame_mouse_help {
                        self.ingame_help_mouse_target(pointer.owner, pointer.screen)
                    } else {
                        self.ingame_primary_mouse_target(pointer.owner, pointer.screen)
                    }
                })
                .flatten()
        });
        let mut state = IngameButtonMouseState::new(pointer, down_target, region.is_some());
        state.motion.down_region = region;
        state.down_cursor_help = self.ingame_mouse_help;
        if state.down_cursor_help {
            state.motion.selection_frame = false;
        }
        let fog_blocked = region.is_none() && self.ingame_pointer_fog_blocked(pointer);
        if fog_blocked {
            state.motion.selection_frame = false;
            state.down_cursor_nothing = down_target.is_none();
        }
        self.mouse_state = Some(state);

        if !self.ingame_mouse_help {
            if let Some(region) = region {
                let (command, _) = region.control();
                let control_style = self
                    .engine
                    .player(pointer.owner)
                    .is_some_and(|player| player.control_style());
                if control_style
                    && command & (clonk_engine::COM_SINGLE | clonk_engine::COM_DOUBLE) == 0
                {
                    self.dispatch_ingame_region_control(pointer.owner, region, false)?;
                }
            }
        }
        Ok(())
    }

    fn on_ingame_mouse_up(&mut self) -> Result<(), EngineError> {
        if let Some(pointer) = self.ingame_pointer {
            self.refresh_ingame_mouse_help_region_caption(pointer);
        }
        let Some(drag) = self.mouse_state.take() else {
            return Ok(());
        };
        let motion = drag.motion;
        if !self.ingame_mouse_controls_owner(motion.start.owner) {
            self.ingame_last_left_down = None;
            self.ingame_ignore_left_up = false;
            self.ingame_dragged_objects.clear();
            return Ok(());
        }
        if motion.moved {
            // A completed drag is not a click candidate for the platform's
            // next LeftDouble synthesis; an immediate object-frame-to-member
            // drag must begin a fresh gesture.
            self.ingame_last_left_down = None;
        }
        if drag.down_cursor_help {
            self.ingame_dragged_objects.clear();
            let release_is_region = self
                .ingame_viewport_region(motion.last.owner, motion.last.screen)
                .is_some();
            if release_is_region {
                self.refresh_ingame_mouse_help_region_caption(motion.last);
            } else if motion.down_region.is_none() {
                if let Some(target) = drag.down_target {
                    self.set_ingame_mouse_help_caption(target, true);
                }
            }
            return Ok(());
        }
        let current_is_region = self
            .ingame_viewport_region(motion.last.owner, motion.last.screen)
            .is_some();
        if let Some(down_region) = motion.down_region {
            if motion.moved
                && motion.region_drag_started
                && matches!(down_region, IngameViewportRegion::Inventory(_))
            {
                let mut selected = std::mem::take(&mut self.ingame_dragged_objects);
                selected.retain(|object| {
                    self.engine
                        .object_snapshot(*object)
                        .is_some_and(|object| object.status != clonk_engine::ObjectStatus::Deleted)
                });
                if current_is_region {
                    return self.finish_ingame_noop_drag(motion, selected.len());
                }
                if selected.is_empty() {
                    return Ok(());
                }
                let cursor = motion.region_drag_cursor;
                return self.finish_ingame_region_drag(motion, selected, cursor);
            }

            let (command, _) = down_region.control();
            let control_style = self
                .engine
                .player(motion.start.owner)
                .is_some_and(|player| player.control_style());
            if control_style && command & (clonk_engine::COM_SINGLE | clonk_engine::COM_DOUBLE) == 0
            {
                return self.dispatch_ingame_region_control(motion.start.owner, down_region, true);
            }
            if current_is_region {
                self.ingame_dragged_objects.clear();
                return self.dispatch_ingame_region_control(motion.start.owner, down_region, false);
            }
            // Classic control evaluates the current cursor on button-up. A
            // stored region payload released outside can therefore fall
            // through to the world, unlike AutoStop's early release branch.
            let result = self.handle_ingame_mouse_click(motion.last);
            self.ingame_dragged_objects.clear();
            return result;
        }
        if motion.selection_cancelled_by_region {
            self.ingame_dragged_objects.clear();
            return if current_is_region {
                self.dispatch_control_event_for_local_player(
                    motion.start.owner,
                    ControlEvent::RawPlayerControl {
                        command: 0,
                        data: 0,
                    },
                )
            } else {
                self.handle_ingame_mouse_click(motion.last)
            };
        }
        if current_is_region {
            self.ingame_dragged_objects.clear();
            return self.dispatch_control_event_for_local_player(
                motion.start.owner,
                ControlEvent::RawPlayerControl {
                    command: 0,
                    data: 0,
                },
            );
        }
        if motion.moved {
            if !self.finish_ingame_moved_drag(drag, false)? {
                // A non-draggable world DownCursor (for example Entrance) or
                // an empty landscape frame remains a consumed drag.
                self.ingame_dragged_objects.clear();
            }
            return Ok(());
        }

        // LeftUpDragNone clears C4MouseControl's local Selection after
        // dispatching the click command. Clear first so an error cannot
        // strand the local Selection lifecycle.
        self.ingame_dragged_objects.clear();
        self.handle_ingame_mouse_click(motion.last)?;
        Ok(())
    }

    pub(crate) fn handle_ingame_mouse_click(
        &mut self,
        pointer: ViewportPointer,
    ) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running)
            || !self.mouse_control
            || self.local_controls.mouse_owner() != Some(pointer.owner)
        {
            return Ok(());
        }
        if self.ingame_mouse_help {
            return Ok(());
        }
        let point = ingame_pointer_world_pixel(pointer);
        let fog_blocked = self.ingame_pointer_fog_blocked(pointer);
        // Move snapshots dwKeyFlags before dispatching LeftUp, and every
        // SendCommand in that event observes the same ShiftDown value.
        let add_mode = 1 | if self.keyboard_modifiers.shift_key() {
            4
        } else {
            0
        };
        // UpdateCursorTarget evaluates the nearby Jump cursor after Select,
        // so an eligible jump zone owns the click even over another crew
        // member (C4MouseControl.cpp:522-534,1129-1132).
        if !fog_blocked && self.engine.mouse_jump_zone(pointer.owner, point) {
            self.show_startup_hint = false;
            self.submit_or_execute_player_command(PlayerCommandControlData {
                player: pointer.owner,
                command: CommandId::Jump as i32,
                x: point.x,
                y: point.y,
                target: 0,
                target2: 0,
                data: 0,
                add_mode,
                by_client: -1,
            })?;
            return Ok(());
        }
        // C4MC_Cursor_Select queues CID_PlrSelect on LeftUp for both crew and
        // C4D_MouseSelect objects (C4MouseControl.cpp:1106-1129).
        if let Some(target) = self.ingame_mouse_select_target(pointer.owner, pointer.screen) {
            self.submit_or_execute_player_select(PlayerSelectControlData {
                player: pointer.owner,
                objects: vec![target.as_u64() as i32],
                by_client: -1,
            })?;
            self.snapshot = self.engine.snapshot();
            self.refresh_object_menu();
            self.refresh_focus();
            return Ok(());
        }
        if fog_blocked {
            return Ok(());
        }
        self.show_startup_hint = false;
        self.submit_or_execute_player_command(PlayerCommandControlData {
            player: pointer.owner,
            command: CommandId::MoveTo as i32,
            x: point.x,
            y: point.y,
            target: 0,
            target2: 0,
            data: 0,
            add_mode,
            by_client: -1,
        })?;
        Ok(())
    }

    pub(crate) fn on_ingame_mouse_double(&mut self) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        if let Some(pointer) = self.ingame_pointer {
            self.refresh_ingame_mouse_help_region_caption(pointer);
        }
        if self.ingame_mouse_help || !self.mouse_control {
            return Ok(());
        }
        let Some(pointer) = self.ingame_pointer else {
            return Ok(());
        };
        if self.local_controls.mouse_owner() != Some(pointer.owner)
            || self
                .ingame_viewport_region(pointer.owner, pointer.screen)
                .is_some()
        {
            return Ok(());
        }
        let point = ingame_pointer_world_pixel(pointer);
        let target = self.ingame_primary_mouse_target(pointer.owner, pointer.screen);
        if self.ingame_pointer_fog_blocked(pointer) && target.is_none() {
            return Ok(());
        }
        let Some(command) = self.engine.mouse_left_double_command(
            pointer.owner,
            target,
            point,
            self.keyboard_modifiers.control_key(),
            self.keyboard_modifiers.shift_key(),
        ) else {
            return Ok(());
        };

        self.show_startup_hint = false;
        self.submit_or_execute_player_command(command)?;
        Ok(())
    }

    pub(crate) fn handle_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        let left_double_click = button_state == ElementState::Pressed
            && classic_press_is_double_click(&mut self.last_application_left_press, Instant::now());
        self.handle_mouse_button_classified(button_state, left_double_click)
    }

    pub(crate) fn handle_mouse_button_classified(
        &mut self,
        button_state: ElementState,
        left_double_click: bool,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        self.primary_pointer_left_down = button_state == ElementState::Pressed;
        self.context_menu_pointer_dismissed_lobby_team_player = None;
        self.context_menu_pointer_dismissed_lobby_option = None;
        self.startup_tooltip.note_pointer_button();
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if button_state == ElementState::Released {
            self.stop_runtime_client_list_title_drag_at_current_position();
        }
        if self.mode == AppMode::Running
            && button_state == ElementState::Released
            && self.finish_menu_title_drag(self.ingame_gui_pointer)
        {
            return Ok(());
        }
        if button_state == ElementState::Pressed {
            self.menu_title_drag = None;
        }
        if button_state == ElementState::Released {
            self.stop_message_dialog_pointer_drag_at_current_position();
        }
        if self.mode == AppMode::Running
            && (self.ingame_moving_drag_active() || self.construction_menu_drag_captured())
        {
            return self.handle_ingame_mouse_button(button_state);
        }
        if button_state == ElementState::Pressed {
            if let Some(captured) = self.captured_message_dialog_index() {
                self.cancel_message_dialog_pointer_capture_at(captured);
            }
            if self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.has_pointer_capture())
            {
                if let Some(dialog) = self.runtime_client_list.as_mut() {
                    dialog.pointer_left();
                }
            }
            self.context_menu_pointer_capture = None;
            // A fresh gesture supersedes any stale modal capture. Only the
            // topmost selector itself may acquire this latch.
            self.definition_selector_pointer_capture =
                self.definition_selector.is_some() && self.message_dialogs.is_empty();
            self.league_signup_pointer_capture = self.league_signup_dialog.is_some()
                && self.context_menu.is_none()
                && self.message_dialogs.is_empty();
            self.game_option_input_pointer_capture = (self.game_option_input_dialog.is_some()
                && self.running_chat_controller().is_none()
                && self.context_menu.is_none()
                && self.message_dialogs.is_empty())
            .then_some(ContextMenuPointerButton::Left);
        }
        let definition_selector_release_latched = button_state == ElementState::Released
            && std::mem::take(&mut self.definition_selector_pointer_capture);
        let league_signup_release_latched = button_state == ElementState::Released
            && std::mem::take(&mut self.league_signup_pointer_capture);
        let input_dialog_release_latched = button_state == ElementState::Released
            && self.game_option_input_pointer_capture == Some(ContextMenuPointerButton::Left);
        let network_chart_release_latched =
            button_state == ElementState::Released && self.network_chart_pointer_capture;
        if input_dialog_release_latched {
            self.game_option_input_pointer_capture = None;
            self.stop_game_option_input_pointer_drag_at_current_position();
            if self.running_chat_controller().is_some() {
                // CMouse stops and clears pDragElement before ordinary
                // top-down LeftUp hit-testing. Compact chat captures only its
                // Edit; releasing elsewhere may therefore reach B/A below.
                self.release_game_option_input_pointer_elements();
            }
        }
        if network_chart_release_latched {
            let _ = self.handle_network_chart_pointer_button(ElementState::Released);
            self.release_occluded_running_pointer_captures(None);
            return Ok(());
        }
        if self.consume_closed_context_pointer_release(button_state, ContextMenuPointerButton::Left)
        {
            if input_dialog_release_latched {
                self.release_game_option_input_pointer_elements();
            }
            return Ok(());
        }
        let context_routed_before_running_dialogs = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && !self.running_dialog_stack.is_empty();
        if context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Left)?
        {
            if button_state == ElementState::Released {
                self.release_occluded_running_pointer_captures(None);
            }
            self.occlude_running_dialog_pointer_hovers();
            return Ok(());
        }
        if self.network_chart_is_elevated_pointer_layer()
            && self.context_menu.is_none()
            && self.handle_network_chart_pointer_button(button_state)
        {
            if button_state == ElementState::Pressed && self.network_chart_dialog.is_some() {
                self.activate_runtime_default_dialog(RuntimeDefaultDialog::NetworkChart);
            }
            if button_state == ElementState::Released {
                self.release_occluded_running_pointer_captures(None);
            }
            return Ok(());
        }
        if self.running_chat_controller().is_some() {
            let lower_capture = self.captured_message_dialog_index();
            if !context_routed_before_running_dialogs
                && self.handle_context_menu_pointer_button(
                    button_state,
                    ContextMenuPointerButton::Left,
                )?
            {
                if button_state == ElementState::Released {
                    if let Some(captured) = lower_capture {
                        self.cancel_message_dialog_pointer_capture_at(captured);
                    }
                }
                return Ok(());
            }
            let shared_target = self
                .running_pointer_position
                .map(|point| self.top_running_shared_pointer_target(point, false))
                .transpose()?
                .flatten();
            if shared_target.is_some() && shared_target != Some(RunningDialogStackEntry::Chat) {
                if button_state == ElementState::Pressed {
                    self.set_running_chat_active(false);
                }
                if let Some(controller) = self.running_chat_controller_mut() {
                    controller.pointer_left();
                }
                self.finish_game_option_input_dialog_actions(Vec::new())?;
                self.handle_scoreboard_message_pointer_button(button_state)?;
                return Ok(());
            }
            let point = self.running_pointer_position;
            let chat_hit = point
                .zip(self.game_option_input_layout().as_ref())
                .is_some_and(|(point, layout)| Self::point_in_input_dialog_bounds(point, layout));
            if chat_hit {
                if button_state == ElementState::Released {
                    if let Some(captured) = lower_capture {
                        self.cancel_message_dialog_pointer_capture_at(captured);
                    }
                }
                if button_state == ElementState::Pressed {
                    self.set_running_chat_active(true);
                }
                self.game_option_input_pointer_position = point;
                self.handle_game_option_input_primary_pointer(button_state)?;
                if button_state == ElementState::Pressed {
                    self.game_option_input_pointer_capture = self
                        .running_chat_controller()
                        .is_some_and(InputDialogController::has_pointer_capture)
                        .then_some(ContextMenuPointerButton::Left);
                }
                return Ok(());
            }
            let message_hit = point.and_then(|point| self.top_message_dialog_hit_index(point));
            if self.handle_message_dialog_pointer_button(button_state)? {
                if button_state == ElementState::Pressed {
                    self.set_running_chat_active(false);
                    self.message_dialog_active_index = message_hit;
                    if let Some(controller) = self.running_chat_controller_mut() {
                        controller.pointer_left();
                    }
                    self.finish_game_option_input_dialog_actions(Vec::new())?;
                }
                return Ok(());
            }
            // Shared Screen hit-testing continues below the compact z=+2
            // chat. It remains exclusive only as the no-dialog-hit fallback.
            let lower_default_hit =
                self.handle_runtime_default_dialog_primary_button(button_state)?;
            if lower_default_hit
                || (!self.network_chart_elevated && self.running_shared_gui_has_keyboard_focus())
            {
                if lower_default_hit && button_state == ElementState::Released {
                    self.release_occluded_running_pointer_captures(None);
                }
                return Ok(());
            }
        }
        if self.running_chat_controller().is_none()
            && !self.message_dialogs.is_empty()
            && self.handle_scoreboard_message_pointer_button(button_state)?
        {
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            if self.context_menu.is_some() {
                if self.handle_context_menu_pointer_button(
                    button_state,
                    ContextMenuPointerButton::Left,
                )? {
                    self.league_signup_pointer_left(false);
                    return Ok(());
                }
                if button_state == ElementState::Pressed {
                    self.league_signup_pointer_capture = true;
                }
            }
            if self.handle_league_signup_pointer_button(button_state, left_double_click)? {
                return Ok(());
            }
        }
        if league_signup_release_latched {
            return Ok(());
        }
        if self.external_irc_dialog_visible {
            if self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Left)?
            {
                return Ok(());
            }
            if self.context_menu.is_some() {
                return Ok(());
            }
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            if !self.message_dialogs.is_empty() {
                return Ok(());
            }
            let Some(point) = self.running_pointer_position else {
                return Ok(());
            };
            let Some(fonts) = self.assets.clonk_fonts.clone() else {
                return Ok(());
            };
            let mut actions = self
                .external_irc_dialog
                .as_mut()
                .map(|dialog| match button_state {
                    ElementState::Pressed => dialog.handle_pointer_down(point, &fonts.text),
                    ElementState::Released => dialog.handle_pointer_up(point, &fonts.text),
                })
                .unwrap_or_default();
            if button_state == ElementState::Released {
                let now = Instant::now();
                let double = self.irc_dialog_last_click.is_some_and(|(last, at)| {
                    now.saturating_duration_since(at) < CPP_DOUBLE_CLICK_INTERVAL
                        && (last.x - point.x).abs() <= 4.0
                        && (last.y - point.y).abs() <= 4.0
                });
                self.irc_dialog_last_click = (!double).then_some((point, now));
                if double {
                    actions.extend(
                        self.external_irc_dialog
                            .as_mut()
                            .map(|dialog| dialog.handle_pointer_double_click(point, &fonts.text))
                            .unwrap_or_default(),
                    );
                }
            }
            self.process_network_dialog_actions(actions)?;
            return Ok(());
        }
        if let Some(layout) = self.network_start_wait_layout() {
            let actions = self
                .network_start_wait
                .as_mut()
                .and_then(|wait| {
                    wait.pointer.map(|point| match button_state {
                        ElementState::Pressed => {
                            wait.controller.handle_pointer_down(point, &layout)
                        }
                        ElementState::Released => wait.controller.handle_pointer_up(point, &layout),
                    })
                })
                .unwrap_or_default();
            self.process_network_start_wait_actions(actions)?;
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let font = self.assets.clonk_fonts.as_deref().map(|fonts| &fonts.text);
            let point = self
                .startup_options_advanced_dialog
                .as_ref()
                .and_then(|pending| pending.controller.pointer_position());
            let actions = point
                .and_then(|point| {
                    self.startup_options_advanced_dialog
                        .as_mut()
                        .map(|pending| match (button_state, font) {
                            (ElementState::Pressed, Some(font)) => pending
                                .controller
                                .handle_pointer_down_with_font(point, font),
                            (ElementState::Released, Some(font)) => {
                                pending.controller.handle_pointer_up_with_font(point, font)
                            }
                            (ElementState::Pressed, None) => {
                                pending.controller.handle_pointer_down(point)
                            }
                            (ElementState::Released, None) => {
                                pending.controller.handle_pointer_up(point)
                            }
                        })
                })
                .unwrap_or_default();
            self.process_options_advanced_actions(actions)?;
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let point = self
                .startup_player_properties_dialog
                .as_ref()
                .and_then(|pending| pending.controller.pointer_position());
            let actions = point
                .and_then(|point| {
                    self.startup_player_properties_dialog
                        .as_mut()
                        .map(|pending| match button_state {
                            ElementState::Pressed if left_double_click => {
                                pending.controller.handle_pointer_double_click(point)
                            }
                            ElementState::Pressed => pending.controller.handle_pointer_down(point),
                            ElementState::Released => pending.controller.handle_pointer_up(point),
                        })
                })
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.definition_selector.is_some() {
            let layout = self.definition_selector_layout();
            let point = self
                .definition_selector
                .as_ref()
                .and_then(|controller| controller.pointer_position());
            let clicked_label_row = layout.as_ref().zip(point).and_then(|(layout, point)| {
                self.definition_selector.as_ref().and_then(|controller| {
                    definition_selector_label_row_at(controller, layout, point)
                })
            });
            let mut actions = layout
                .as_ref()
                .zip(point)
                .and_then(|(layout, point)| {
                    self.definition_selector
                        .as_mut()
                        .map(|controller| match button_state {
                            ElementState::Pressed => controller.handle_pointer_down(point, layout),
                            ElementState::Released => controller.handle_pointer_up(point, layout),
                        })
                })
                .unwrap_or_default();
            if button_state == ElementState::Released {
                if let (Some(index), Some(layout), Some(point)) =
                    (clicked_label_row, layout.as_ref(), point)
                {
                    let now = Instant::now();
                    let is_double =
                        self.definition_selector_last_click
                            .is_some_and(|(last_index, at)| {
                                last_index == index
                                    && now.duration_since(at) < Duration::from_millis(500)
                            });
                    self.definition_selector_last_click =
                        if is_double { None } else { Some((index, now)) };
                    if is_double {
                        actions.extend(
                            self.definition_selector
                                .as_mut()
                                .map(|controller| {
                                    controller.handle_pointer_double_click(point, layout)
                                })
                                .unwrap_or_default(),
                        );
                    }
                } else {
                    self.definition_selector_last_click = None;
                }
            }
            self.finish_definition_selector_input(actions)?;
            return Ok(());
        }
        if definition_selector_release_latched {
            return Ok(());
        }
        if !context_routed_before_running_dialogs
            && self
                .handle_context_menu_pointer_button(button_state, ContextMenuPointerButton::Left)?
        {
            if input_dialog_release_latched {
                self.release_game_option_input_pointer_elements();
            }
            return Ok(());
        }
        if self.game_option_input_dialog.is_some()
            && self.game_option_input_owns_running_pointer_event()
        {
            self.handle_game_option_input_primary_pointer(button_state)?;
            return Ok(());
        }
        if input_dialog_release_latched {
            return Ok(());
        }
        if !matches!(self.mode, AppMode::Running)
            && self.handle_runtime_client_list_pointer_button(button_state)?
        {
            return Ok(());
        }
        if self.handle_runtime_default_dialog_primary_button(button_state)? {
            if button_state == ElementState::Released {
                self.release_occluded_running_pointer_captures(None);
            }
            return Ok(());
        }
        if self.mode == AppMode::Running && button_state == ElementState::Released {
            let target = self
                .running_pointer_position
                .map(|point| self.top_running_shared_pointer_target(point, false))
                .transpose()?
                .flatten();
            self.release_occluded_running_pointer_captures(target);
        }
        if self.runtime_pointer_fallback_is_exclusive() {
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_pointer_button(button_state, left_double_click);
        }
        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    if button_state == ElementState::Released {
                        self.dismiss_game_over_dialog();
                    }
                    return Ok(());
                }
                match self.startup_view {
                    StartupView::NetworkGame => {
                        let Some(fonts) = self.assets.clonk_fonts.clone() else {
                            return Ok(());
                        };
                        let point = self
                            .startup_network_dialog
                            .as_ref()
                            .and_then(|dialog| dialog.pointer_position());
                        let clicked_row = point.and_then(|point| {
                            self.startup_network_dialog
                                .as_ref()
                                .and_then(|dialog| dialog.game_index_at(point))
                        });
                        let clicked_edit = point.is_some_and(|point| {
                            self.startup_network_dialog
                                .as_ref()
                                .is_some_and(|dialog| dialog.join_address_contains(point))
                        });
                        let row_double = if button_state == ElementState::Released {
                            let now = Instant::now();
                            let row_double = clicked_row.is_some_and(|index| {
                                self.netdlg_last_click.is_some_and(|(last_index, at)| {
                                    last_index == index
                                        && now.duration_since(at) < Duration::from_millis(500)
                                })
                            });
                            self.netdlg_last_click = (!row_double)
                                .then(|| clicked_row.map(|index| (index, now)))
                                .flatten();
                            row_double
                        } else {
                            false
                        };
                        let edit_double = if button_state == ElementState::Pressed {
                            let now = Instant::now();
                            let edit_double = clicked_edit
                                && self.netdlg_join_edit_last_click.is_some_and(|at| {
                                    now.saturating_duration_since(at) < CPP_DOUBLE_CLICK_INTERVAL
                                });
                            self.netdlg_join_edit_last_click =
                                (clicked_edit && !edit_double).then_some(now);
                            edit_double
                        } else {
                            false
                        };
                        let mut actions = if edit_double {
                            self.startup_network_dialog
                                .as_mut()
                                .and_then(|dialog| {
                                    point.map(|point| {
                                        dialog.handle_pointer_double_click(point, &fonts.text)
                                    })
                                })
                                .unwrap_or_default()
                        } else {
                            self.startup_network_dialog
                                .as_mut()
                                .and_then(|dialog| {
                                    point.map(|point| match button_state {
                                        ElementState::Pressed => {
                                            dialog.handle_pointer_down(point, &fonts.text)
                                        }
                                        ElementState::Released => {
                                            dialog.handle_pointer_up(point, &fonts.text)
                                        }
                                    })
                                })
                                .unwrap_or_default()
                        };
                        if row_double {
                            actions.extend(
                                self.startup_network_dialog
                                    .as_mut()
                                    .and_then(|dialog| {
                                        point.map(|point| {
                                            dialog.handle_pointer_double_click(point, &fonts.text)
                                        })
                                    })
                                    .unwrap_or_default(),
                            );
                            self.netdlg_last_click = None;
                        }
                        let chat_double = button_state == ElementState::Released
                            && self.startup_network_dialog.as_ref().is_some_and(|dialog| {
                                dialog.mode() == clonk_frontend::startup_netdlg::NetDlgMode::Chat
                            })
                            && point.is_some_and(|point| {
                                let now = Instant::now();
                                let double =
                                    self.irc_dialog_last_click.is_some_and(|(last, at)| {
                                        now.saturating_duration_since(at)
                                            < CPP_DOUBLE_CLICK_INTERVAL
                                            && (last.x - point.x).abs() <= 4.0
                                            && (last.y - point.y).abs() <= 4.0
                                    });
                                self.irc_dialog_last_click = (!double).then_some((point, now));
                                double
                            });
                        if chat_double {
                            actions.extend(
                                self.startup_network_dialog
                                    .as_mut()
                                    .and_then(|dialog| {
                                        point.map(|point| {
                                            dialog.handle_pointer_double_click(point, &fonts.text)
                                        })
                                    })
                                    .unwrap_or_default(),
                            );
                            self.irc_dialog_last_click = None;
                        }
                        self.process_network_dialog_actions(actions)
                    }
                    StartupView::PlayerSelection => {
                        let point = self
                            .startup_player_dialog
                            .as_ref()
                            .and_then(|dialog| dialog.pointer_position());
                        let mut restore_rename_focus = None;
                        if self.startup_crew_rename.is_some() {
                            match (button_state, point) {
                                (ElementState::Pressed, Some(point)) => {
                                    if self.handle_startup_crew_rename_pointer_down(point) {
                                        return Ok(());
                                    }
                                    if let Some(rename) = self.startup_crew_rename.as_mut() {
                                        rename.last_click = None;
                                        rename.ignore_pointer_up = false;
                                    }
                                    restore_rename_focus = self
                                        .startup_player_dialog
                                        .as_ref()
                                        .map(|dialog| dialog.focused_control());
                                }
                                (ElementState::Released, Some(point))
                                    if self.handle_startup_crew_rename_pointer_up(point) =>
                                {
                                    return Ok(());
                                }
                                _ => {}
                            }
                        }
                        let scrollbar_captured = self
                            .startup_player_dialog
                            .as_ref()
                            .is_some_and(|dialog| dialog.scrollbar_pointer_captured());
                        let clicked_row = if scrollbar_captured {
                            None
                        } else {
                            point.and_then(|point| {
                                let dialog = self.startup_player_dialog.as_ref()?;
                                let layout = dialog.layout();
                                let in_name_column = point.x
                                    >= (layout.list_client.x + layout.item_height) as f32
                                    && point.x < (layout.list_client.x + layout.item_width) as f32;
                                in_name_column
                                    .then(|| dialog.context_index_at(point))
                                    .flatten()
                            })
                        };
                        let is_double = if button_state == ElementState::Released {
                            let now = Instant::now();
                            let is_double = clicked_row.is_some_and(|index| {
                                self.plrsel_last_click.is_some_and(|(last_index, at)| {
                                    last_index == index
                                        && now.duration_since(at) < Duration::from_millis(500)
                                })
                            });
                            self.plrsel_last_click = clicked_row.map(|index| (index, now));
                            is_double
                        } else {
                            false
                        };
                        let mut actions = self
                            .startup_player_dialog
                            .as_mut()
                            .and_then(|dialog| {
                                point.map(|point| match button_state {
                                    ElementState::Pressed => dialog.handle_pointer_down(point),
                                    ElementState::Released => dialog.handle_pointer_up(point),
                                })
                            })
                            .unwrap_or_default();
                        if is_double {
                            actions.extend(
                                self.startup_player_dialog
                                    .as_mut()
                                    .and_then(|dialog| {
                                        point.map(|point| dialog.handle_pointer_double_click(point))
                                    })
                                    .unwrap_or_default(),
                            );
                            self.plrsel_last_click = None;
                        }
                        self.process_player_dialog_actions(actions)?;
                        self.restore_startup_crew_focus(restore_rename_focus);
                        Ok(())
                    }
                    StartupView::ScenarioBrowser => {
                        if button_state == ElementState::Pressed {
                            self.scensel_rename_pointer_focus = None;
                        }
                        if button_state == ElementState::Released
                            && self.game_option_pointer_capture
                            && self.menu_state.pointer_position().is_none()
                        {
                            self.game_option_pointer_capture = false;
                            self.scenario_game_options.cancel_interaction();
                            self.scensel_rename_pointer_focus = None;
                            return Ok(());
                        }
                        if let Some(point) = self.menu_state.pointer_position() {
                            if self.menu_state.rename_edit.is_some() {
                                match button_state {
                                    ElementState::Pressed => {
                                        if self.handle_scensel_rename_pointer_down(point) {
                                            return Ok(());
                                        }
                                        self.commit_scenario_rename(true)?;
                                        if self.menu_state.rename_edit.is_some() {
                                            return Ok(());
                                        }
                                        self.scensel_rename_pointer_focus =
                                            Some(self.scensel_focus_snapshot());
                                    }
                                    ElementState::Released
                                        if self.handle_scensel_rename_pointer_up(point) =>
                                    {
                                        return Ok(());
                                    }
                                    ElementState::Released => {}
                                }
                            }
                            self.scenario_game_options.handle_pointer_move(point);
                            match button_state {
                                ElementState::Pressed => {
                                    self.game_option_pointer_capture = self
                                        .scenario_game_options
                                        .hovered_button()
                                        .and_then(|button| self.scenario_game_options.view(button))
                                        .is_some_and(|view| view.enabled);
                                    if self.game_option_pointer_capture {
                                        self.scenario_game_options.set_focused_button(
                                            self.scenario_game_options.hovered_button(),
                                        );
                                        let actions =
                                            self.scenario_game_options.handle_pointer_down(point);
                                        self.menu_state
                                            .set_dialog_focus(ScenselDialogFocus::Options);
                                        self.finish_game_option_input(actions)?;
                                        self.restore_scensel_rename_pointer_focus();
                                        return Ok(());
                                    }
                                }
                                ElementState::Released if self.game_option_pointer_capture => {
                                    self.game_option_pointer_capture = false;
                                    let actions =
                                        self.scenario_game_options.handle_pointer_up(point);
                                    self.finish_game_option_input(actions)?;
                                    self.scensel_rename_pointer_focus = None;
                                    return Ok(());
                                }
                                ElementState::Released => {}
                            }
                            match button_state {
                                ElementState::Pressed => {
                                    if !self.handle_scensel_search_clear_pointer_down(point)?
                                        && !self.handle_scensel_search_pointer_down(point)
                                        && !self.handle_scensel_scrollbar_down(point)
                                    {
                                        self.handle_scensel_map_pointer_down(point);
                                    }
                                }
                                ElementState::Released => {
                                    if !self.handle_scensel_search_pointer_up(point)
                                        && !self.handle_scensel_scrollbar_up(point)
                                    {
                                        self.handle_scensel_parity_click(
                                            point,
                                            self.scensel_rename_pointer_focus.is_some(),
                                        )?;
                                    }
                                }
                            }
                            if button_state == ElementState::Pressed {
                                self.restore_scensel_rename_pointer_focus();
                            } else {
                                self.scensel_rename_pointer_focus = None;
                            }
                        } else if button_state == ElementState::Released {
                            self.scensel_rename_pointer_focus = None;
                        }
                        Ok(())
                    }
                    StartupView::MainMenu => {
                        if let Some(point) = self.main_menu_state.pointer_position() {
                            let actions = match button_state {
                                ElementState::Pressed => {
                                    self.main_menu_state.handle_pointer_down(point)
                                }
                                ElementState::Released => {
                                    self.main_menu_state.handle_pointer_up(point)
                                }
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        Ok(())
                    }
                    StartupView::NetworkLobby => {
                        if self.network_lobby.is_some() {
                            let (width, height) = {
                                let surface = self.graphics.surface();
                                (surface.width() as f32, surface.height() as f32)
                            };
                            let panel_pointer = self.network_lobby.as_mut().and_then(|lobby| {
                                lobby.update_layout(width, height);
                                lobby.pointer_position().filter(|point| {
                                    matches!(
                                        lobby.pointer_region(*point),
                                        LobbyPointerRegion::Panel
                                    )
                                })
                            });
                            match button_state {
                                ElementState::Pressed => {
                                    if panel_pointer.is_some() {
                                        return self.handle_network_lobby_pointer_button(
                                            ElementState::Pressed,
                                            left_double_click,
                                        );
                                    }
                                    if let Some(point) = self.menu_state.pointer_position() {
                                        self.handle_menu_input(|state| {
                                            state.menu().handle_pointer_down(point)
                                        })?;
                                    }
                                    Ok(())
                                }
                                ElementState::Released => {
                                    if panel_pointer.is_some() {
                                        return self.handle_network_lobby_pointer_button(
                                            ElementState::Released,
                                            false,
                                        );
                                    }
                                    if let Some(point) = self.menu_state.pointer_position() {
                                        self.handle_menu_input(|state| {
                                            state.menu().handle_pointer_up(point)
                                        })?;
                                    }
                                    Ok(())
                                }
                            }
                        } else {
                            Ok(())
                        }
                    }
                    StartupView::Options => {
                        let actions = self
                            .startup_options_dialog
                            .as_mut()
                            .and_then(|dialog| {
                                dialog.pointer_position().map(|point| match button_state {
                                    ElementState::Pressed => dialog.handle_pointer_down(point),
                                    ElementState::Released => dialog.handle_pointer_up(point),
                                })
                            })
                            .unwrap_or_default();
                        self.process_options_dialog_actions(actions)?;
                        Ok(())
                    }
                    StartupView::About => {
                        let actions = self
                            .startup_about_dialog
                            .as_mut()
                            .and_then(|dialog| {
                                dialog.pointer_position().map(|point| match button_state {
                                    ElementState::Pressed => dialog.handle_pointer_down(point),
                                    ElementState::Released => dialog.handle_pointer_up(point),
                                })
                            })
                            .unwrap_or_default();
                        self.process_about_dialog_actions(actions)
                    }
                }
            }
            AppMode::Running => {
                if self.ingame_mouse_help
                    || self.ingame_moving_drag_active()
                    || self.construction_menu_drag_captured()
                {
                    self.handle_ingame_mouse_button(button_state)
                } else if self.handle_scoreboard_pointer_button(button_state)?
                    || self.handle_ingame_menu_pointer_button(button_state, false)?
                {
                    Ok(())
                } else {
                    self.initialize_ingame_mouse_center()?;
                    self.handle_ingame_mouse_button(button_state)
                }
            }
            AppMode::Loading => Ok(()),
        }
    }

    pub(crate) fn handle_touch(
        &mut self,
        phase: TouchPhase,
        position: GuiPoint,
    ) -> Result<(), EngineError> {
        let left_double_click = phase == TouchPhase::Started
            && classic_press_is_double_click(&mut self.last_application_left_press, Instant::now());
        self.guard_classic_global_gui_bootstrap()?;
        self.sync_scoreboard_before_running_pointer_input();
        match phase {
            TouchPhase::Started => self.primary_pointer_left_down = true,
            TouchPhase::Ended | TouchPhase::Cancelled => {
                self.primary_pointer_left_down = false;
                if phase == TouchPhase::Cancelled {
                    self.menu_title_drag = None;
                }
            }
            TouchPhase::Moved => {}
        }
        if phase != TouchPhase::Cancelled {
            self.running_pointer_position = Some(position);
        }
        self.context_menu_pointer_dismissed_lobby_team_player = None;
        self.context_menu_pointer_dismissed_lobby_option = None;
        if self.startup_network_transition_blocks_input() {
            return Ok(());
        }
        if self.mode == AppMode::Running && phase == TouchPhase::Cancelled {
            self.release_all_running_pointer_elements();
        } else if self.mode == AppMode::Running
            && self.scoreboard_runtime.title_drag.is_some()
            && matches!(phase, TouchPhase::Moved | TouchPhase::Ended)
        {
            self.update_scoreboard_title_drag(position);
            if phase == TouchPhase::Ended {
                self.scoreboard_runtime.title_drag = None;
            }
        }
        if self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.has_positional_pointer_drag())
        {
            match phase {
                TouchPhase::Moved => {
                    self.handle_runtime_client_list_pointer_move(position);
                }
                TouchPhase::Ended => {
                    self.stop_runtime_client_list_title_drag_at_current_position();
                }
                TouchPhase::Cancelled | TouchPhase::Started => {
                    if let Some(dialog) = self.runtime_client_list.as_mut() {
                        dialog.pointer_left();
                    }
                }
            }
        }
        if self.network_chart_pointer_capture && phase != TouchPhase::Started {
            match phase {
                TouchPhase::Moved => {
                    let _ = self.handle_network_chart_pointer_move(position);
                }
                TouchPhase::Ended => {
                    let _ = self.handle_network_chart_pointer_button(ElementState::Released);
                }
                TouchPhase::Cancelled => {
                    self.cancel_network_chart_pointer_capture();
                }
                TouchPhase::Started => unreachable!(),
            }
            self.cancel_ingame_mouse_gestures();
            return Ok(());
        }
        if self.external_irc_dialog_visible && !matches!(self.mode, AppMode::Running) {
            match phase {
                TouchPhase::Started => {
                    self.handle_cursor_moved(PhysicalPosition::new(
                        f64::from(position.x),
                        f64::from(position.y),
                    ))?;
                    self.handle_mouse_button_classified(ElementState::Pressed, left_double_click)?;
                }
                TouchPhase::Moved => {
                    self.handle_cursor_moved(PhysicalPosition::new(
                        f64::from(position.x),
                        f64::from(position.y),
                    ))?;
                }
                TouchPhase::Ended => self.handle_mouse_button(ElementState::Released)?,
                TouchPhase::Cancelled => {
                    if let Some(dialog) = self.external_irc_dialog.as_mut() {
                        dialog.pointer_left();
                    }
                }
            }
            return Ok(());
        }
        if self.running_chat_controller().is_some() {
            match phase {
                TouchPhase::Started => {
                    self.handle_cursor_moved(PhysicalPosition::new(
                        f64::from(position.x),
                        f64::from(position.y),
                    ))?;
                    self.handle_mouse_button_classified(ElementState::Pressed, left_double_click)?;
                }
                TouchPhase::Moved => {
                    self.handle_cursor_moved(PhysicalPosition::new(
                        f64::from(position.x),
                        f64::from(position.y),
                    ))?;
                }
                TouchPhase::Ended => {
                    self.handle_mouse_button(ElementState::Released)?;
                }
                TouchPhase::Cancelled => {
                    self.context_menu_pointer_capture = None;
                    self.release_message_dialog_pointer_elements();
                    self.release_game_option_input_pointer_elements();
                }
            }
            return Ok(());
        }
        if phase == TouchPhase::Started {
            self.definition_selector_pointer_capture =
                self.definition_selector.is_some() && self.message_dialogs.is_empty();
            self.league_signup_pointer_capture = self.league_signup_dialog.is_some()
                && self.context_menu.is_none()
                && self.message_dialogs.is_empty();
            self.game_option_input_pointer_capture = (self.game_option_input_dialog.is_some()
                && self.context_menu.is_none()
                && (self.message_dialogs.is_empty() || self.running_chat_controller().is_some()))
            .then_some(ContextMenuPointerButton::Left);
            self.game_option_pointer_capture = false;
        }
        let definition_selector_release_latched =
            matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
                && std::mem::take(&mut self.definition_selector_pointer_capture);
        let league_signup_release_latched =
            matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
                && std::mem::take(&mut self.league_signup_pointer_capture);
        if phase == TouchPhase::Ended
            && self.consume_closed_context_pointer_release(
                ElementState::Released,
                ContextMenuPointerButton::Left,
            )
        {
            return Ok(());
        }
        if phase == TouchPhase::Cancelled {
            self.context_menu_pointer_capture = None;
        }
        let context_before_running_messages = self.mode == AppMode::Running
            && self.context_menu.is_some()
            && self.running_chat_controller().is_none()
            && !self.message_dialogs.is_empty();
        if context_before_running_messages {
            if phase == TouchPhase::Cancelled {
                self.close_context_menu_silently();
            } else {
                let move_captured = self.handle_context_menu_pointer_move(position)?;
                let button_captured = match phase {
                    TouchPhase::Started => self.handle_context_menu_pointer_button(
                        ElementState::Pressed,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Ended => self.handle_context_menu_pointer_button(
                        ElementState::Released,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Moved => false,
                    TouchPhase::Cancelled => unreachable!("handled above"),
                };
                if move_captured || button_captured {
                    if phase == TouchPhase::Ended {
                        self.release_occluded_running_pointer_captures(None);
                    }
                    self.occlude_running_dialog_pointer_hovers();
                    return Ok(());
                }
            }
        }
        let elevated_chart_hit = self.network_chart_is_elevated_pointer_layer()
            && self.context_menu.is_none()
            && self.network_chart_contains_point(position);
        let retained_shared_capture = self.running_shared_pointer_capture_open();
        let running_message_shared_target = if self.mode == AppMode::Running
            && self.running_chat_controller().is_none()
            && (!self.message_dialogs.is_empty() || retained_shared_capture)
            && (!elevated_chart_hit || retained_shared_capture)
        {
            self.top_running_shared_pointer_target(position, true)?
        } else {
            None
        };
        if self.mode == AppMode::Running && phase == TouchPhase::Ended {
            self.release_occluded_running_pointer_captures(running_message_shared_target);
        }
        if self.running_chat_controller().is_none()
            && matches!(
                running_message_shared_target,
                Some(RunningDialogStackEntry::Scoreboard)
            )
            && self.handle_scoreboard_touch(position, phase)?
        {
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.pointer_left_unchecked();
            }
            return Ok(());
        }
        if self.running_chat_controller().is_none()
            && matches!(
                running_message_shared_target,
                Some(RunningDialogStackEntry::RuntimeClientList)
            )
            && self.handle_runtime_client_list_touch(position, phase)?
        {
            return Ok(());
        }
        if !self.message_dialogs.is_empty()
            && self.running_chat_controller().is_none()
            && (matches!(
                running_message_shared_target,
                Some(RunningDialogStackEntry::Message(_))
            ) || running_message_shared_target.is_none()
                && !self.network_chart_elevated
                && self.running_shared_gui_has_keyboard_focus())
        {
            let retained_title_drag = self.captured_message_dialog_index().filter(|index| {
                self.message_dialogs
                    .get(*index)
                    .is_some_and(|dialog| dialog.state.has_positional_pointer_drag())
            });
            if matches!(phase, TouchPhase::Moved | TouchPhase::Ended) {
                if let Some(index) = retained_title_drag {
                    self.handle_message_dialog_pointer_move_at(index, position);
                    if phase == TouchPhase::Ended {
                        self.stop_message_dialog_pointer_drag_at(index, position);
                    }
                }
            }
            if !matches!(phase, TouchPhase::Cancelled) {
                self.handle_message_dialog_pointer_move(position);
            }
            match phase {
                TouchPhase::Started => {
                    self.handle_message_dialog_pointer_button(ElementState::Pressed)?;
                }
                TouchPhase::Ended => {
                    self.handle_message_dialog_pointer_button(ElementState::Released)?;
                }
                TouchPhase::Cancelled => {
                    self.release_message_dialog_pointer_elements();
                }
                TouchPhase::Moved => {}
            }
            return Ok(());
        }
        if self.league_signup_dialog.is_some() {
            self.league_signup_pointer_position =
                (!matches!(phase, TouchPhase::Cancelled)).then_some(position);
            if phase == TouchPhase::Cancelled {
                self.close_context_menu_silently();
                self.league_signup_pointer_left(true);
                return Ok(());
            }
            if self.context_menu.is_some() {
                let move_captured = self.handle_context_menu_pointer_move(position)?;
                let button_captured = match phase {
                    TouchPhase::Started => self.handle_context_menu_pointer_button(
                        ElementState::Pressed,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Moved => false,
                    TouchPhase::Ended => self.handle_context_menu_pointer_button(
                        ElementState::Released,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Cancelled => unreachable!("handled above"),
                };
                if move_captured || button_captured {
                    self.league_signup_pointer_left(false);
                    return Ok(());
                }
                if phase == TouchPhase::Started {
                    self.league_signup_pointer_capture = true;
                }
            }
            let layout = self.league_signup_layout();
            let fonts = self.assets.clonk_fonts.clone();
            let actions = layout
                .as_ref()
                .and_then(|layout| {
                    self.league_signup_dialog
                        .as_mut()
                        .map(|dialog| match phase {
                            TouchPhase::Started => {
                                fonts.as_deref().map_or_else(Vec::new, |fonts| {
                                    if left_double_click {
                                        dialog.controller.handle_pointer_double_click(
                                            position,
                                            layout,
                                            &fonts.text,
                                        )
                                    } else {
                                        dialog.controller.handle_pointer_down(
                                            position,
                                            layout,
                                            &fonts.text,
                                        )
                                    }
                                })
                            }
                            TouchPhase::Moved => fonts.as_deref().map_or_else(Vec::new, |fonts| {
                                dialog
                                    .controller
                                    .handle_pointer_move(position, layout, &fonts.text)
                            }),
                            TouchPhase::Ended => fonts.as_deref().map_or_else(Vec::new, |fonts| {
                                dialog
                                    .controller
                                    .handle_pointer_up(position, layout, &fonts.text)
                            }),
                            TouchPhase::Cancelled => unreachable!("handled above"),
                        })
                })
                .unwrap_or_default();
            self.process_league_signup_actions(actions)?;
            return Ok(());
        }
        if league_signup_release_latched {
            return Ok(());
        }
        if let Some(layout) = self.network_start_wait_layout() {
            let actions = self
                .network_start_wait
                .as_mut()
                .map(|wait| {
                    wait.pointer = (!matches!(phase, TouchPhase::Cancelled)).then_some(position);
                    match phase {
                        TouchPhase::Started => {
                            wait.controller.handle_pointer_down(position, &layout)
                        }
                        TouchPhase::Moved => {
                            wait.controller.handle_pointer_move(position, &layout);
                            Vec::new()
                        }
                        TouchPhase::Ended => wait.controller.handle_pointer_up(position, &layout),
                        TouchPhase::Cancelled => {
                            wait.controller.cancel_pointer_capture();
                            Vec::new()
                        }
                    }
                })
                .unwrap_or_default();
            self.process_network_start_wait_actions(actions)?;
            return Ok(());
        }
        if self.startup_options_advanced_dialog.is_some() {
            let font = self.assets.clonk_fonts.as_deref().map(|fonts| &fonts.text);
            let actions = self
                .startup_options_advanced_dialog
                .as_mut()
                .map(|pending| match phase {
                    TouchPhase::Started => match font {
                        Some(font) => pending
                            .controller
                            .handle_pointer_down_with_font(position, font),
                        None => pending.controller.handle_pointer_down(position),
                    },
                    TouchPhase::Moved => match font {
                        Some(font) => pending
                            .controller
                            .handle_pointer_move_with_font(position, font),
                        None => pending.controller.handle_pointer_move(position),
                    },
                    TouchPhase::Ended => match font {
                        Some(font) => pending
                            .controller
                            .handle_pointer_up_with_font(position, font),
                        None => pending.controller.handle_pointer_up(position),
                    },
                    TouchPhase::Cancelled => {
                        pending.controller.cancel_interaction();
                        Vec::new()
                    }
                })
                .unwrap_or_default();
            self.process_options_advanced_actions(actions)?;
            return Ok(());
        }
        if self.startup_player_properties_dialog.is_some() {
            let actions = self
                .startup_player_properties_dialog
                .as_mut()
                .map(|pending| match phase {
                    TouchPhase::Started if left_double_click => {
                        pending.controller.handle_pointer_double_click(position)
                    }
                    TouchPhase::Started => pending.controller.handle_pointer_down(position),
                    TouchPhase::Moved => pending.controller.handle_pointer_move(position),
                    TouchPhase::Ended => pending.controller.handle_pointer_up(position),
                    TouchPhase::Cancelled => {
                        pending.controller.pointer_left();
                        Vec::new()
                    }
                })
                .unwrap_or_default();
            self.process_startup_player_properties_actions(actions);
            return Ok(());
        }
        if self.definition_selector.is_some() {
            let layout = self.definition_selector_layout();
            let clicked_label_row = layout.as_ref().and_then(|layout| {
                self.definition_selector.as_ref().and_then(|controller| {
                    definition_selector_label_row_at(controller, layout, position)
                })
            });
            let mut actions = layout
                .as_ref()
                .and_then(|layout| {
                    self.definition_selector
                        .as_mut()
                        .map(|controller| match phase {
                            TouchPhase::Started => controller.handle_touch_start(position, layout),
                            TouchPhase::Moved => controller.handle_touch_move(position, layout),
                            TouchPhase::Ended => controller.handle_touch_end(position, layout),
                            TouchPhase::Cancelled => {
                                controller.handle_touch_cancel();
                                Vec::new()
                            }
                        })
                })
                .unwrap_or_default();
            if phase == TouchPhase::Ended {
                if let (Some(index), Some(layout)) = (clicked_label_row, layout.as_ref()) {
                    let now = Instant::now();
                    let is_double =
                        self.definition_selector_last_click
                            .is_some_and(|(last_index, at)| {
                                last_index == index
                                    && now.duration_since(at) < Duration::from_millis(500)
                            });
                    self.definition_selector_last_click =
                        if is_double { None } else { Some((index, now)) };
                    if is_double {
                        actions.extend(
                            self.definition_selector
                                .as_mut()
                                .map(|controller| {
                                    controller.handle_pointer_double_click(position, layout)
                                })
                                .unwrap_or_default(),
                        );
                    }
                } else {
                    self.definition_selector_last_click = None;
                }
            } else if phase == TouchPhase::Cancelled {
                self.definition_selector_last_click = None;
            }
            self.finish_definition_selector_input(actions)?;
            return Ok(());
        }
        if definition_selector_release_latched {
            return Ok(());
        }
        if self.context_menu.is_some() {
            if phase == TouchPhase::Cancelled {
                self.close_context_menu_silently();
            } else {
                let move_captured = self.handle_context_menu_pointer_move(position)?;
                let button_captured = match phase {
                    TouchPhase::Started => self.handle_context_menu_pointer_button(
                        ElementState::Pressed,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Ended => self.handle_context_menu_pointer_button(
                        ElementState::Released,
                        ContextMenuPointerButton::Left,
                    )?,
                    TouchPhase::Moved => false,
                    TouchPhase::Cancelled => unreachable!(),
                };
                if move_captured || button_captured {
                    return Ok(());
                }
            }
        }
        if self.game_option_input_dialog.is_some() {
            self.game_option_input_pointer_position = Some(position);
            let layout = self.game_option_input_layout();
            let fonts = self.assets.clonk_fonts.clone();
            let actions = layout
                .as_ref()
                .zip(fonts.as_deref())
                .and_then(|(layout, fonts)| {
                    self.game_option_input_dialog
                        .as_mut()
                        .map(|dialog| match phase {
                            TouchPhase::Started => {
                                dialog
                                    .controller
                                    .handle_touch_start(position, layout, &fonts.text)
                            }
                            TouchPhase::Moved => {
                                dialog
                                    .controller
                                    .handle_touch_move(position, layout, &fonts.text)
                            }
                            TouchPhase::Ended => {
                                dialog
                                    .controller
                                    .handle_touch_end(position, layout, &fonts.text)
                            }
                            TouchPhase::Cancelled => {
                                dialog.controller.handle_touch_cancel();
                                Vec::new()
                            }
                        })
                })
                .unwrap_or_default();
            self.finish_game_option_input_dialog_actions(actions)?;
            if phase == TouchPhase::Started {
                self.game_option_input_pointer_capture = self
                    .game_option_input_dialog
                    .as_ref()
                    .is_some_and(|dialog| dialog.controller.has_pointer_capture())
                    .then_some(ContextMenuPointerButton::Left);
            }
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.game_option_input_pointer_capture = None;
                self.game_option_input_pointer_position = None;
            }
            return Ok(());
        }
        let input_dialog_release_latched =
            matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
                && self.game_option_input_pointer_capture == Some(ContextMenuPointerButton::Left);
        if input_dialog_release_latched {
            self.game_option_input_pointer_capture = None;
            return Ok(());
        }
        if !matches!(self.mode, AppMode::Running)
            && self.handle_runtime_client_list_touch(position, phase)?
        {
            return Ok(());
        }
        if matches!(self.mode, AppMode::Running) {
            for dialog_kind in self
                .runtime_default_dialog_order_snapshot()
                .into_iter()
                .rev()
            {
                let handled = match dialog_kind {
                    RuntimeDefaultDialog::ExternalIrc => {
                        let hit = self.external_irc_dialog_contains_point(position)
                            || self.external_irc_pointer_capture;
                        if !hit {
                            false
                        } else {
                            match phase {
                                TouchPhase::Started => {
                                    self.handle_runtime_external_irc_pointer_move(position)?;
                                    self.handle_runtime_external_irc_pointer_button(
                                        ElementState::Pressed,
                                    )?
                                }
                                TouchPhase::Moved => {
                                    self.handle_runtime_external_irc_pointer_move(position)?
                                }
                                TouchPhase::Ended => self
                                    .handle_runtime_external_irc_pointer_button(
                                        ElementState::Released,
                                    )?,
                                TouchPhase::Cancelled => {
                                    self.external_irc_pointer_capture = false;
                                    if let Some(dialog) = self.external_irc_dialog.as_mut() {
                                        dialog.pointer_left();
                                    }
                                    true
                                }
                            }
                        }
                    }
                    RuntimeDefaultDialog::GameOver => {
                        if !self.game_over_pointer_route_hit(position) {
                            false
                        } else {
                            let (width, height) = {
                                let surface = self.graphics.surface();
                                (surface.width(), surface.height())
                            };
                            if !matches!(phase, TouchPhase::Cancelled) {
                                if let Some(dialog) = self.game_over_dialog.as_mut() {
                                    dialog
                                        .handle_pointer_move(position.x, position.y, width, height);
                                }
                            }
                            let action = match phase {
                                TouchPhase::Started => {
                                    if let Some(dialog) = self.game_over_dialog.as_mut() {
                                        dialog.handle_pointer_down(width, height);
                                    }
                                    None
                                }
                                TouchPhase::Ended => self
                                    .game_over_dialog
                                    .as_mut()
                                    .and_then(|dialog| dialog.handle_pointer_up(width, height)),
                                TouchPhase::Cancelled => {
                                    self.pointer_left_unchecked();
                                    None
                                }
                                TouchPhase::Moved => None,
                            };
                            let sounds = self
                                .game_over_dialog
                                .as_mut()
                                .map(GameOverState::take_sound_events)
                                .unwrap_or_default();
                            self.play_game_over_sound_events(sounds);
                            if let Some(action) = action {
                                self.handle_game_over_action(action)?;
                            }
                            true
                        }
                    }
                    RuntimeDefaultDialog::ClientList => {
                        self.handle_runtime_client_list_touch(position, phase)?
                    }
                    RuntimeDefaultDialog::NetworkChart => match phase {
                        TouchPhase::Started => {
                            self.handle_network_chart_pointer_button(ElementState::Pressed)
                        }
                        TouchPhase::Moved => self.network_chart_contains_point(position),
                        TouchPhase::Ended => {
                            self.handle_network_chart_pointer_button(ElementState::Released)
                        }
                        TouchPhase::Cancelled => {
                            let captured = self.network_chart_pointer_capture;
                            self.cancel_network_chart_pointer_capture();
                            captured
                        }
                    },
                    RuntimeDefaultDialog::Scoreboard => {
                        self.handle_scoreboard_touch(position, phase)?
                    }
                };
                if handled {
                    if dialog_kind == RuntimeDefaultDialog::Scoreboard
                        && matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled)
                    {
                        self.pointer_left_unchecked();
                    }
                    if phase == TouchPhase::Started
                        && self.runtime_default_dialog_visible(dialog_kind)
                    {
                        self.activate_runtime_default_dialog(dialog_kind);
                    }
                    return Ok(());
                }
            }
        }
        if self.runtime_pointer_fallback_is_exclusive() {
            return Ok(());
        }
        if self.mode == AppMode::Running {
            let _ = self.handle_scoreboard_touch(position, phase)?;
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.pointer_left_unchecked();
            }
            return Ok(());
        }
        if self.mode != AppMode::Menu {
            return Ok(());
        }
        if self.startup_dialog_fade_active() {
            return Ok(());
        }
        if self.classic_host_lobby_active() {
            return self.handle_classic_lobby_touch(phase, position, left_double_click);
        }
        if self.game_over_dialog.is_some() {
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.dismiss_game_over_dialog();
            }
            return Ok(());
        }
        match self.startup_view {
            StartupView::NetworkGame => {
                let fonts = self.assets.clonk_fonts.clone();
                let actions = fonts
                    .as_deref()
                    .and_then(|fonts| {
                        self.startup_network_dialog
                            .as_mut()
                            .map(|dialog| match phase {
                                TouchPhase::Started => {
                                    dialog.handle_pointer_down(position, &fonts.text)
                                }
                                TouchPhase::Moved => {
                                    dialog.handle_pointer_move(position, &fonts.text)
                                }
                                TouchPhase::Ended => {
                                    dialog.handle_pointer_up(position, &fonts.text)
                                }
                                TouchPhase::Cancelled => Vec::new(),
                            })
                    })
                    .unwrap_or_default();
                self.process_network_dialog_actions(actions)?;
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.pointer_left_unchecked();
                }
                Ok(())
            }
            StartupView::PlayerSelection => {
                let mut restore_rename_focus = None;
                if self.startup_crew_rename.is_some() {
                    match phase {
                        TouchPhase::Started => {
                            if self.handle_startup_crew_rename_pointer_down(position) {
                                return Ok(());
                            }
                            if let Some(rename) = self.startup_crew_rename.as_mut() {
                                rename.last_click = None;
                                rename.ignore_pointer_up = false;
                            }
                            restore_rename_focus = self
                                .startup_player_dialog
                                .as_ref()
                                .map(|dialog| dialog.focused_control());
                        }
                        TouchPhase::Moved
                            if self.handle_startup_crew_rename_pointer_move(position) =>
                        {
                            return Ok(());
                        }
                        TouchPhase::Ended
                            if self.handle_startup_crew_rename_pointer_up(position) =>
                        {
                            self.pointer_left_unchecked();
                            return Ok(());
                        }
                        TouchPhase::Cancelled => {
                            if let Some(rename) = self.startup_crew_rename.as_mut() {
                                rename.edit.cancel_pointer_selection();
                                rename.ignore_pointer_up = false;
                            }
                            self.pointer_left_unchecked();
                            return Ok(());
                        }
                        TouchPhase::Moved | TouchPhase::Ended => {}
                    }
                }
                let actions = self
                    .startup_player_dialog
                    .as_mut()
                    .map(|dialog| match phase {
                        TouchPhase::Started => dialog.handle_pointer_down(position),
                        TouchPhase::Moved => dialog.handle_pointer_move(position),
                        TouchPhase::Ended => dialog.handle_pointer_up(position),
                        TouchPhase::Cancelled => Vec::new(),
                    })
                    .unwrap_or_default();
                self.process_player_dialog_actions(actions)?;
                self.restore_startup_crew_focus(restore_rename_focus);
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.pointer_left_unchecked();
                }
                Ok(())
            }
            StartupView::ScenarioBrowser => {
                if phase == TouchPhase::Started {
                    self.scensel_rename_pointer_focus = None;
                }
                self.menu_state.set_pointer_position(Some(position));
                if self.menu_state.rename_edit.is_some() {
                    match phase {
                        TouchPhase::Started => {
                            if self.handle_scensel_rename_pointer_down(position) {
                                return Ok(());
                            }
                            self.commit_scenario_rename(true)?;
                            if self.menu_state.rename_edit.is_some() {
                                return Ok(());
                            }
                            self.scensel_rename_pointer_focus = Some(self.scensel_focus_snapshot());
                        }
                        TouchPhase::Moved if self.handle_scensel_rename_pointer_move(position) => {
                            return Ok(());
                        }
                        TouchPhase::Ended if self.handle_scensel_rename_pointer_up(position) => {
                            self.pointer_left_unchecked();
                            return Ok(());
                        }
                        TouchPhase::Cancelled => {
                            if let Some(rename) = self.menu_state.rename_edit.as_mut() {
                                rename.edit.cancel_pointer_selection();
                            }
                            self.pointer_left_unchecked();
                            return Ok(());
                        }
                        TouchPhase::Moved | TouchPhase::Ended => {}
                    }
                }
                self.scenario_game_options.handle_pointer_move(position);
                if phase == TouchPhase::Started {
                    self.game_option_pointer_capture = self
                        .scenario_game_options
                        .hovered_button()
                        .and_then(|button| self.scenario_game_options.view(button))
                        .is_some_and(|view| view.enabled);
                }
                if self.game_option_pointer_capture {
                    let actions = match phase {
                        TouchPhase::Started => {
                            self.scenario_game_options
                                .set_focused_button(self.scenario_game_options.hovered_button());
                            self.menu_state
                                .set_dialog_focus(ScenselDialogFocus::Options);
                            self.scenario_game_options.handle_touch_start(position)
                        }
                        TouchPhase::Moved => self.scenario_game_options.handle_touch_move(position),
                        TouchPhase::Ended => self.scenario_game_options.handle_touch_end(position),
                        TouchPhase::Cancelled => {
                            self.scenario_game_options.handle_touch_cancel();
                            Vec::new()
                        }
                    };
                    self.finish_game_option_input(actions)?;
                    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        self.scensel_rename_pointer_focus = None;
                    } else {
                        self.restore_scensel_rename_pointer_focus();
                    }
                    if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                        self.game_option_pointer_capture = false;
                        self.pointer_left_unchecked();
                    }
                    return Ok(());
                }
                match phase {
                    TouchPhase::Started => {
                        if !self.handle_scensel_search_clear_pointer_down(position)?
                            && !self.handle_scensel_search_pointer_down(position)
                            && !self.handle_scensel_scrollbar_down(position)
                        {
                            self.handle_scensel_map_pointer_down(position);
                        }
                        self.restore_scensel_rename_pointer_focus();
                        Ok(())
                    }
                    TouchPhase::Moved => {
                        let _ = self.handle_scensel_rename_pointer_move(position)
                            || self.handle_scensel_search_pointer_move(position)
                            || self.handle_scensel_scrollbar_move(position);
                        Ok(())
                    }
                    TouchPhase::Ended => {
                        if !self.handle_scensel_search_pointer_up(position)
                            && !self.handle_scensel_scrollbar_up(position)
                        {
                            self.handle_scensel_parity_click(
                                position,
                                self.scensel_rename_pointer_focus.is_some(),
                            )?;
                        }
                        self.scensel_rename_pointer_focus = None;
                        self.pointer_left_unchecked();
                        Ok(())
                    }
                    TouchPhase::Cancelled => {
                        self.menu_state.search_edit.dragging = false;
                        self.menu_state.scrollbar_interaction = None;
                        self.scensel_rename_pointer_focus = None;
                        self.pointer_left_unchecked();
                        Ok(())
                    }
                }
            }
            StartupView::MainMenu => {
                self.main_menu_state.set_pointer_position(Some(position));
                let actions = match phase {
                    TouchPhase::Started => self.main_menu_state.handle_pointer_down(position),
                    TouchPhase::Moved => self.main_menu_state.handle_pointer_move(position),
                    TouchPhase::Ended => {
                        let actions = self.main_menu_state.handle_pointer_up(position);
                        self.pointer_left_unchecked();
                        actions
                    }
                    TouchPhase::Cancelled => {
                        self.pointer_left_unchecked();
                        Vec::new()
                    }
                };
                self.process_main_menu_actions(actions)
            }
            StartupView::NetworkLobby => {
                if self.network_lobby.is_some() {
                    let (width, height) = {
                        let surface = self.graphics.surface();
                        (surface.width() as f32, surface.height() as f32)
                    };
                    let region = self
                        .network_lobby
                        .as_mut()
                        .map(|lobby| {
                            lobby.update_layout(width, height);
                            lobby.pointer_region(position)
                        })
                        .unwrap_or(LobbyPointerRegion::Menu);
                    match phase {
                        TouchPhase::Started => match region {
                            LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                state.set_pointer_position(Some(position));
                                state.menu().handle_pointer_down(position)
                            }),
                            LobbyPointerRegion::Panel => self.handle_network_lobby_touch(
                                TouchPhase::Started,
                                position,
                                left_double_click,
                            ),
                        },
                        TouchPhase::Moved => match region {
                            LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                state.set_pointer_position(Some(position));
                                state.menu().handle_pointer_move(position)
                            }),
                            LobbyPointerRegion::Panel => {
                                self.handle_network_lobby_touch(TouchPhase::Moved, position, false)
                            }
                        },
                        TouchPhase::Ended => match region {
                            LobbyPointerRegion::Menu => {
                                let result = self.handle_menu_input(|state| {
                                    state.set_pointer_position(Some(position));
                                    state.menu().handle_pointer_up(position)
                                });
                                self.pointer_left_unchecked();
                                result
                            }
                            LobbyPointerRegion::Panel => {
                                self.handle_network_lobby_touch(TouchPhase::Ended, position, false)
                            }
                        },
                        TouchPhase::Cancelled => {
                            self.handle_network_lobby_touch(TouchPhase::Cancelled, position, false)
                        }
                    }
                } else {
                    Ok(())
                }
            }
            StartupView::Options => {
                let actions = self
                    .startup_options_dialog
                    .as_mut()
                    .map(|dialog| match phase {
                        TouchPhase::Started => dialog.handle_pointer_down(position),
                        TouchPhase::Moved => dialog.handle_pointer_move(position),
                        TouchPhase::Ended => dialog.handle_pointer_up(position),
                        TouchPhase::Cancelled => Vec::new(),
                    })
                    .unwrap_or_default();
                self.process_options_dialog_actions(actions)?;
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.pointer_left_unchecked();
                }
                Ok(())
            }
            StartupView::About => {
                let actions = self
                    .startup_about_dialog
                    .as_mut()
                    .map(|dialog| match phase {
                        TouchPhase::Started => dialog.handle_pointer_down(position),
                        TouchPhase::Moved => dialog.handle_pointer_move(position),
                        TouchPhase::Ended => dialog.handle_pointer_up(position),
                        TouchPhase::Cancelled => Vec::new(),
                    })
                    .unwrap_or_default();
                self.process_about_dialog_actions(actions)?;
                if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                    self.pointer_left_unchecked();
                }
                Ok(())
            }
        }
    }

    pub(crate) fn pointer_left(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        self.window_mouse_position = None;
        self.pointer_inside_window = false;
        self.running_pointer_position = None;
        self.pointer_left_unchecked();
        Ok(())
    }

    pub(crate) fn pointer_left_unchecked(&mut self) {
        self.startup_tooltip.pointer_left();
        if self.mode == AppMode::Running {
            self.scoreboard_pointer_left();
        }
        // CursorLeft may be the only lifecycle event after an activation
        // closed the menu on down. An open popup, however, must retain its
        // capture so a later outside up cannot leak to the underlying screen.
        if self.context_menu.is_none() {
            self.context_menu_pointer_capture = None;
        }
        self.cancel_network_chart_pointer_capture();
        let messages_open = !self.message_dialogs.is_empty();
        for index in 0..self.message_dialogs.len() {
            self.message_dialog_pointer_left_at(index);
        }
        if self.mode != AppMode::Running
            && messages_open
            && self.running_chat_controller().is_none()
        {
            return;
        }
        if self.league_signup_dialog.is_some() {
            if let Some(menu) = self.context_menu.as_mut() {
                let _ = menu.handle_pointer_left();
            }
            self.league_signup_pointer_left(true);
            return;
        }
        if self.external_irc_dialog_visible {
            if let Some(menu) = self.context_menu.as_mut() {
                let _ = menu.handle_pointer_left();
            }
        }
        if self.external_irc_dialog_visible {
            if let Some(dialog) = self.external_irc_dialog.as_mut() {
                dialog.pointer_left();
            }
            self.irc_dialog_last_click = None;
            return;
        }
        if self
            .network_start_wait
            .as_ref()
            .is_some_and(|wait| wait.visible)
        {
            if let Some(wait) = self.network_start_wait.as_mut() {
                wait.pointer = None;
                wait.controller.pointer_left();
            }
            self.play_network_start_wait_sounds();
            return;
        }
        if let Some(pending) = self.startup_options_advanced_dialog.as_mut() {
            pending.controller.cancel_interaction();
            return;
        }
        if let Some(pending) = self.startup_player_properties_dialog.as_mut() {
            pending.controller.pointer_left();
            return;
        }
        if self.definition_selector.is_some() {
            if let Some(layout) = self.definition_selector_layout() {
                if let Some(controller) = self.definition_selector.as_mut() {
                    controller.pointer_left(&layout);
                }
            }
            let sounds = self
                .definition_selector
                .as_mut()
                .map(|controller| controller.take_sound_events())
                .unwrap_or_default();
            self.play_definition_selector_sound_events(sounds);
            return;
        }
        if let Some(menu) = self.context_menu.as_mut() {
            let _ = menu.handle_pointer_left();
        }
        if let Some(dialog) = self.game_option_input_dialog.as_mut() {
            dialog.controller.pointer_left();
            let sounds = dialog.controller.take_sound_events();
            self.play_input_dialog_sound_events(sounds);
            self.game_option_input_pointer_position = None;
            return;
        }
        if self
            .runtime_client_list
            .as_ref()
            .is_some_and(|dialog| dialog.is_info_only())
        {
            if let Some(dialog) = self.runtime_client_list.as_mut() {
                dialog.pointer_left();
            }
            return;
        }
        if let Some(dialog) = self.game_over_dialog.as_mut() {
            dialog.pointer_left();
            let sounds = dialog.take_sound_events();
            self.play_game_over_sound_events(sounds);
            return;
        }
        match self.mode {
            AppMode::Menu => match self.startup_view {
                StartupView::NetworkGame => {
                    if let Some(dialog) = self.startup_network_dialog.as_mut() {
                        dialog.pointer_left();
                    }
                    self.netdlg_last_click = None;
                    self.netdlg_join_edit_last_click = None;
                }
                StartupView::PlayerSelection => {
                    if let Some(dialog) = self.startup_player_dialog.as_mut() {
                        dialog.pointer_left();
                    }
                }
                StartupView::ScenarioBrowser => {
                    self.scenario_game_options.pointer_left();
                    let sounds = self.scenario_game_options.take_sound_events();
                    self.play_game_option_sound_events(sounds);
                    self.menu_state.set_pointer_position(None);
                    self.menu_state.scrollbar_interaction = None;
                    self.menu_state.search_edit.dragging = false;
                    self.scensel_search_last_click = None;
                }
                StartupView::MainMenu => {
                    self.main_menu_state.pointer_left();
                }
                StartupView::NetworkLobby => {
                    if !self.classic_lobby_pointer_left() {
                        self.menu_state.set_pointer_position(None);
                    }
                }
                StartupView::Options => {
                    let sounds = self
                        .startup_options_dialog
                        .as_mut()
                        .map(|dialog| dialog.handle_pointer_left())
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|action| match action {
                            clonk_frontend::startup_options_dlg::OptionsDlgAction::Sound(
                                clonk_frontend::startup_options_dlg::SoundSheetAction::GuiSound(
                                    sound,
                                ),
                            ) => Some(sound),
                            unexpected => {
                                tracing::error!(
                                    ?unexpected,
                                    "unexpected mutating action while cancelling Options pointer capture"
                                );
                                None
                            }
                        })
                        .collect::<Vec<_>>();
                    for sound in sounds {
                        self.play_options_sound(sound);
                    }
                }
                StartupView::About => {
                    if let Some(dialog) = self.startup_about_dialog.as_mut() {
                        dialog.pointer_left();
                    }
                }
            },
            AppMode::Running => {
                self.construction_menu_drag = None;
                if let Some(dialog) = self.runtime_client_list.as_mut() {
                    dialog.pointer_left();
                }
                if let Some(state) = self.mouse_state.as_mut() {
                    state.motion.moved = true;
                }
                if let Some(state) = self.ingame_right_mouse_state.as_mut() {
                    state.motion.moved = true;
                }
                self.ingame_gui_pointer = None;
                self.ingame_pointer = None;
                self.ingame_viewport_mouse = None;
                self.ingame_edge_scroll = None;
                self.ingame_mouse_caption = IngameMouseCaptionState::default();
                self.running_pointer_position = None;
            }
            AppMode::Loading => {}
        }
    }

    pub(crate) fn handle_menu_input<F>(&mut self, handler: F) -> Result<(), EngineError>
    where
        F: FnOnce(&mut MenuState) -> Vec<StartupMenuAction>,
    {
        if self.game_over_dialog.is_some() || self.definition_selector.is_some() {
            return Ok(());
        }
        if self.mode != AppMode::Menu
            || !matches!(
                self.startup_view,
                StartupView::ScenarioBrowser | StartupView::NetworkLobby
            )
        {
            return Ok(());
        }

        let actions = handler(&mut self.menu_state);
        self.handle_menu_actions(actions)
    }

    pub(crate) fn handle_context_menu_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        let Some(menu) = self.context_menu.as_mut() else {
            return Ok(false);
        };
        let outcome = menu.handle_pointer_move(point);
        let captured = outcome.captured && !outcome.pass_through;
        self.process_context_menu_outcome(outcome)?;
        Ok(captured)
    }

    fn consume_closed_context_pointer_release(
        &mut self,
        state: ElementState,
        button: ContextMenuPointerButton,
    ) -> bool {
        if state == ElementState::Released
            && self.context_menu.is_none()
            && self.context_menu_pointer_capture == Some(button)
        {
            self.context_menu_pointer_capture = None;
            return true;
        }
        false
    }

    pub(crate) fn handle_context_menu_pointer_button(
        &mut self,
        state: ElementState,
        button: ContextMenuPointerButton,
    ) -> Result<bool, EngineError> {
        let retained_release =
            state == ElementState::Released && self.context_menu_pointer_capture == Some(button);
        if retained_release {
            self.context_menu_pointer_capture = None;
        }
        let Some(menu) = self.context_menu.as_mut() else {
            return Ok(retained_release);
        };
        let point = menu.pointer_position();
        let outcome = match state {
            ElementState::Pressed => menu.handle_pointer_down(point, button),
            ElementState::Released => menu.handle_pointer_up(point, button),
        };
        let dismissed_combo = state == ElementState::Pressed
            && button == ContextMenuPointerButton::Left
            && outcome.pass_through
            && outcome
                .events
                .iter()
                .any(|event| matches!(event, ContextMenuEvent::Closed));
        let dismissed_lobby_team_player = dismissed_combo
            .then_some(self.context_menu_lobby_team_player)
            .flatten();
        let dismissed_lobby_option = dismissed_combo
            .then_some(self.context_menu_lobby_option)
            .flatten();
        let captured = outcome.captured && !outcome.pass_through;
        if state == ElementState::Pressed && captured {
            self.context_menu_pointer_capture = Some(button);
        }
        self.process_context_menu_outcome(outcome)?;
        if let Some(player_id) = dismissed_lobby_team_player {
            self.context_menu_pointer_dismissed_lobby_team_player = Some(player_id);
        }
        if let Some(option) = dismissed_lobby_option {
            self.context_menu_pointer_dismissed_lobby_option = Some(option);
        }
        Ok(captured || retained_release)
    }

    pub(crate) fn handle_context_menu_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.context_menu.is_none() {
            return Ok(false);
        }
        if state == ElementState::Released {
            return Ok(false);
        }
        let outcome = {
            let menu = self.context_menu.as_mut().expect("checked above");
            menu.note_non_pointer_input();
            if let Some(key) = context_menu_key_code(key) {
                menu.handle_key(key)
            } else if let Some(hotkey) = context_menu_hotkey(key) {
                menu.handle_hotkey(hotkey)
            } else {
                return Ok(false);
            }
        };
        let captured = outcome.captured && !outcome.pass_through;
        if captured {
            if self.running_chat_controller().is_some() {
                self.game_option_input_consumed_keys.insert(key);
            } else {
                self.message_dialog_consumed_keys.insert(key);
            }
        }
        self.process_context_menu_outcome(outcome)?;
        Ok(captured)
    }

    fn handle_context_menu_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<bool, EngineError> {
        let outcome = self.context_menu.as_mut().and_then(|menu| {
            menu.note_non_pointer_input();
            match event {
                GamepadEvent::Direction {
                    button,
                    state: ElementState::Pressed,
                    ..
                } => Some(menu.handle_gamepad_direction(match button {
                    ControlButton::Up => ContextMenuDirection::Up,
                    ControlButton::Down => ContextMenuDirection::Down,
                    ControlButton::Left => ContextMenuDirection::Left,
                    ControlButton::Right => ContextMenuDirection::Right,
                })),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                    ..
                } => Some(menu.handle_gamepad_low()),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                    ..
                } => Some(menu.handle_gamepad_high()),
                GamepadEvent::Clear { .. } => Some(menu.dismiss(false)),
                GamepadEvent::Axis { .. }
                | GamepadEvent::Direction { .. }
                | GamepadEvent::Button { .. }
                | GamepadEvent::Action { .. }
                | GamepadEvent::GuiButton { .. } => None,
            }
        });
        if let Some(outcome) = outcome {
            let captured = outcome.captured && !outcome.pass_through;
            self.process_context_menu_outcome(outcome)?;
            return Ok(captured);
        }
        Ok(false)
    }

    fn handle_network_start_wait_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self
            .network_start_wait
            .as_ref()
            .is_none_or(|wait| !wait.visible)
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let actions = if state == ElementState::Pressed
            && modifiers.alt_key()
            && !modifiers.control_key()
            && key == VirtualKeyCode::KeyR
        {
            self.network_start_wait
                .as_mut()
                .map(|wait| wait.controller.handle_hotkey('R'))
                .unwrap_or_default()
        } else if let Some(gui_key) = map_key_code(key) {
            self.network_start_wait
                .as_mut()
                .map(|wait| match state {
                    ElementState::Pressed => wait.controller.handle_key_down_with_tab_direction(
                        gui_key,
                        self.keyboard_modifiers.shift_key(),
                    ),
                    ElementState::Released => wait.controller.handle_key_up(gui_key),
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.process_network_start_wait_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn catalog_host_preload_key(&self) -> Option<CatalogHostLobbyPreloadKey> {
        let scenario = self.catalog_host_preload_scenario()?;
        Some(CatalogHostLobbyPreloadKey {
            identifier: scenario.identifier.clone(),
            scenario_path: scenario.path.clone()?,
            definition_load: self.scenario_seed_definition_load(),
            languages: startup_language_sequence(self.app_paths.as_ref()),
        })
    }

    pub(crate) fn new_network_dialog_controller(
        &self,
    ) -> clonk_frontend::startup_netdlg::NetDlgController {
        let (masterserver_signup, _) = load_network_startup_settings(self.app_paths.as_ref());
        let metrics = self
            .assets
            .clonk_fonts
            .as_deref()
            .map(clonk_frontend::startup_netdlg::NetDlgFontMetrics::from_fonts)
            .unwrap_or(clonk_frontend::startup_netdlg::NetDlgFontMetrics {
                caption_back_extent: 51,
                text_ip_extent: 18,
                text_line_height: 22,
                caption_line_height: 25,
                title_line_height: 34,
            });
        let mut dialog = clonk_frontend::startup_netdlg::NetDlgController::new(
            clonk_frontend::startup_netdlg::NetDlgConfig {
                masterserver_signup,
                record: self.startup_view_flags.record,
            },
            metrics,
        );
        dialog.set_chat_strings(self.localized_irc_chat_strings());
        dialog.set_chat_login(load_irc_settings(self.app_paths.as_ref()).login());
        dialog.set_chat_history(self.message_input_history.iter().cloned().collect());
        if let Some(fonts) = self.assets.clonk_fonts.as_deref() {
            dialog.set_text_font(&fonts.text);
        }
        dialog.resize(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
        );
        dialog
    }

    pub(crate) fn apply_input_dialog_context_command(
        &mut self,
        command: InputDialogContextCommand,
    ) -> Result<(), EngineError> {
        let Some(layout) = self.game_option_input_layout() else {
            tracing::error!(?command, "stale input-dialog context command");
            return Ok(());
        };
        let Some(fonts) = self.assets.clonk_fonts.clone() else {
            tracing::error!(
                ?command,
                "input-dialog context command requires classic fonts"
            );
            return Ok(());
        };
        let clipboard = if matches!(command, InputDialogContextCommand::Paste) {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .ok()
        } else {
            None
        };
        let actions = self
            .game_option_input_dialog
            .as_mut()
            .map(|dialog| {
                dialog.controller.apply_context_command(
                    command,
                    clipboard.as_deref(),
                    &layout,
                    &fonts.text,
                )
            })
            .unwrap_or_default();
        self.finish_game_option_input_dialog_actions(actions)
    }

    pub(crate) fn point_in_input_dialog_bounds(
        point: GuiPoint,
        layout: &clonk_frontend::input_dialog::InputDialogLayout,
    ) -> bool {
        let bounds = layout.bounds;
        point.x >= bounds.x as f32
            && point.x < (bounds.x + bounds.w) as f32
            && point.y >= bounds.y as f32
            && point.y < (bounds.y + bounds.h) as f32
    }

    pub(crate) fn handle_message_dialog_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.message_dialogs.is_empty() {
            if state == ElementState::Released && self.message_dialog_consumed_keys.remove(&key) {
                return Ok(true);
            }
            return Ok(false);
        }
        let Some(active_index) = self.active_message_dialog_index() else {
            return Ok(false);
        };
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let chat_already_open = self.running_chat_controller().is_some();
        // Screen::HasKeyboardFocus asks whether the actual list-top dialog is
        // exclusive; keyboard input is then sent to pActiveDlg. Activation of
        // z+1/z+2 dialogs does not reorder them, so active and list-top can
        // intentionally differ after a default-z dialog was moved last.
        let gui_scope = self.running_shared_gui_has_keyboard_focus();
        let chat_open_fallthrough = matches!(self.mode, AppMode::Running)
            && !self.running_chat_active()
            && (matches!(
                (key, c4_modifiers),
                (VirtualKeyCode::F2, modifiers)
                    if modifiers.is_empty()
            ) || matches!(
                (key, c4_modifiers),
                (VirtualKeyCode::Enter, ModifiersState::SHIFT)
            ) || !chat_already_open
                && !gui_scope
                && matches!(
                    (key, c4_modifiers),
                    (VirtualKeyCode::Enter, modifiers) if modifiers.is_empty()
                )
                || !chat_already_open
                    && !gui_scope
                    && matches!(
                        (key, c4_modifiers),
                        (VirtualKeyCode::Enter, ModifiersState::ALT)
                    ));
        if chat_open_fallthrough {
            return Ok(false);
        }
        if !gui_scope {
            return Ok(false);
        }
        let hotkey_modifiers = c4_modifiers == ModifiersState::ALT
            || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
        let normal_modifiers = c4_modifiers.is_empty()
            || (key == VirtualKeyCode::Tab && c4_modifiers == ModifiersState::SHIFT);
        if !hotkey_modifiers && !normal_modifiers {
            if state == ElementState::Released {
                self.message_dialog_consumed_keys.remove(&key);
            }
            return Ok(true);
        }
        if hotkey_modifiers {
            if state == ElementState::Released {
                return Ok(false);
            }
            let Some(character) = message_dialog_hotkey(key) else {
                return Ok(false);
            };
            let (owns_hotkey, result, sounds) = self
                .message_dialogs
                .get_mut(active_index)
                .map(|dialog| {
                    let owns_hotkey = dialog.state.has_hotkey(character);
                    let result = owns_hotkey
                        .then(|| dialog.state.handle_hotkey(character))
                        .flatten();
                    (owns_hotkey, result, dialog.state.take_sound_events())
                })
                .unwrap_or_default();
            if !owns_hotkey {
                return Ok(false);
            }
            self.play_message_dialog_sound_events(sounds);
            self.persist_message_dialog_checkbox_changes(active_index);
            if let Some(result) = result {
                self.finish_message_dialog_at(active_index, result)?;
            }
            return Ok(true);
        }
        match state {
            ElementState::Pressed => {
                self.message_dialog_consumed_keys.insert(key);
            }
            ElementState::Released => {
                self.message_dialog_consumed_keys.remove(&key);
            }
        }
        let backwards = c4_modifiers == ModifiersState::SHIFT;
        let (result, sounds) = self
            .message_dialogs
            .get_mut(active_index)
            .map(|dialog| {
                let result = map_key_code(key).and_then(|key| match state {
                    ElementState::Pressed => dialog.state.handle_key_down(key, backwards),
                    ElementState::Released => dialog.state.handle_key_up(key),
                });
                (result, dialog.state.take_sound_events())
            })
            .unwrap_or_default();
        self.play_message_dialog_sound_events(sounds);
        self.persist_message_dialog_checkbox_changes(active_index);
        if let Some(result) = result {
            self.finish_message_dialog_at(active_index, result)?;
        }
        Ok(true)
    }

    fn message_dialog_pointer_left_at(&mut self, index: usize) {
        let sounds = self
            .message_dialogs
            .get_mut(index)
            .map(|dialog| {
                dialog.state.pointer_left();
                dialog.state.take_sound_events()
            })
            .unwrap_or_default();
        self.play_message_dialog_sound_events(sounds);
    }

    pub(crate) fn release_message_dialog_pointer_elements(&mut self) {
        let mut sounds = Vec::new();
        for dialog in &mut self.message_dialogs {
            dialog.state.cancel_pointer_capture();
            sounds.extend(dialog.state.take_sound_events());
        }
        self.message_dialog_pointer_capture_index = None;
        self.play_message_dialog_sound_events(sounds);
    }

    fn cancel_message_dialog_pointer_capture_at(&mut self, index: usize) {
        let sounds = self
            .message_dialogs
            .get_mut(index)
            .map(|dialog| {
                dialog.state.cancel_pointer_capture();
                dialog.state.take_sound_events()
            })
            .unwrap_or_default();
        if self.message_dialog_pointer_capture_index == Some(index) {
            self.message_dialog_pointer_capture_index = None;
        }
        self.play_message_dialog_sound_events(sounds);
    }

    fn stop_message_dialog_pointer_drag_at_current_position(&mut self) {
        let Some(index) = self.captured_message_dialog_index().filter(|index| {
            self.message_dialogs
                .get(*index)
                .is_some_and(|dialog| dialog.state.has_positional_pointer_drag())
        }) else {
            return;
        };
        if let Some(point) = self.running_pointer_position {
            self.stop_message_dialog_pointer_drag_at(index, point);
        } else if let Some(dialog) = self.message_dialogs.get_mut(index) {
            dialog.state.cancel_pointer_capture();
            self.message_dialog_pointer_capture_index = None;
        }
    }

    fn stop_message_dialog_pointer_drag_at(&mut self, index: usize, point: GuiPoint) {
        if let Some(dialog) = self.message_dialogs.get_mut(index) {
            dialog.state.stop_pointer_drag_at(point);
        }
        self.message_dialog_pointer_capture_index = None;
    }

    fn handle_message_dialog_pointer_move_at(&mut self, index: usize, point: GuiPoint) -> bool {
        let Some(layout) = self.message_dialog_layout_at(index) else {
            return false;
        };
        let sounds = self
            .message_dialogs
            .get_mut(index)
            .map(|dialog| {
                dialog.state.handle_pointer_move(point, &layout);
                dialog.state.take_sound_events()
            })
            .unwrap_or_default();
        self.play_message_dialog_sound_events(sounds);
        true
    }

    pub(crate) fn handle_message_dialog_pointer_move(&mut self, point: GuiPoint) -> bool {
        let Some(top_index) = self.message_dialogs.len().checked_sub(1) else {
            return false;
        };
        let capture_open = self.captured_message_dialog_index().is_some();
        let active_index = self.active_message_dialog_index();
        let target_index = if self.mode != AppMode::Running {
            Some(top_index)
        } else {
            (0..self.message_dialogs.len()).rev().find(|index| {
                self.message_dialog_layout_at(*index)
                    .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout))
                    || (capture_open && active_index == Some(*index))
            })
        };

        if self.primary_pointer_left_down {
            if let Some(target) = target_index {
                let target_is_hit = self
                    .message_dialog_layout_at(target)
                    .is_some_and(|layout| Self::point_in_message_dialog_bounds(point, &layout));
                if target_is_hit {
                    self.message_dialog_active_index = Some(target);
                    let stack_id = self.message_dialogs[target].running_stack_id;
                    self.activate_running_dialog(RunningDialogStackEntry::Message(stack_id));
                }
            }
        }

        for index in 0..self.message_dialogs.len() {
            if Some(index) != target_index {
                self.message_dialog_pointer_left_at(index);
            }
        }
        let Some(target_index) = target_index else {
            return false;
        };
        self.handle_message_dialog_pointer_move_at(target_index, point)
    }

    fn handle_message_dialog_pointer_button_at(
        &mut self,
        index: usize,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let Some(layout) = self.message_dialog_layout_at(index) else {
            return Ok(false);
        };
        if let Some(captured) = self.captured_message_dialog_index() {
            if captured != index {
                self.cancel_message_dialog_pointer_capture_at(captured);
            }
        }
        let pointer_position = self.running_pointer_position;
        let (result, captures_pointer, sounds) = self
            .message_dialogs
            .get_mut(index)
            .map(|dialog| {
                let result = match state {
                    ElementState::Pressed => {
                        if let Some(point) = pointer_position {
                            dialog.state.handle_pointer_down_at(point, &layout);
                        } else {
                            dialog.state.handle_pointer_down(&layout);
                        }
                        None
                    }
                    ElementState::Released => match pointer_position {
                        Some(point) => dialog.state.handle_pointer_up_at(point, &layout),
                        None => dialog.state.handle_pointer_up(&layout),
                    },
                };
                (
                    result,
                    dialog.state.has_pointer_capture(),
                    dialog.state.take_sound_events(),
                )
            })
            .unwrap_or_default();
        match state {
            ElementState::Pressed if captures_pointer => {
                self.message_dialog_pointer_capture_index = Some(index);
            }
            ElementState::Pressed => {
                self.message_dialog_pointer_capture_index = None;
            }
            ElementState::Released => {
                if self.message_dialog_pointer_capture_index == Some(index) {
                    self.message_dialog_pointer_capture_index = None;
                }
            }
        }
        self.play_message_dialog_sound_events(sounds);
        self.persist_message_dialog_checkbox_changes(index);
        if let Some(result) = result {
            self.finish_message_dialog_at(index, result)?;
        }
        Ok(true)
    }

    pub(crate) fn handle_message_dialog_pointer_button(
        &mut self,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if state == ElementState::Released {
            self.stop_message_dialog_pointer_drag_at_current_position();
        }
        let Some(top_index) = self.message_dialogs.len().checked_sub(1) else {
            return Ok(false);
        };
        let hit_index = self
            .running_pointer_position
            .and_then(|point| self.top_message_dialog_hit_index(point));
        let target_index = if self.mode != AppMode::Running {
            Some(top_index)
        } else {
            hit_index
        };
        let Some(target_index) = target_index else {
            if state == ElementState::Released {
                if let Some(captured) = self.captured_message_dialog_index() {
                    self.cancel_message_dialog_pointer_capture_at(captured);
                }
            }
            return Ok(false);
        };
        if state == ElementState::Pressed
            && self.mode == AppMode::Running
            && hit_index == Some(target_index)
        {
            self.message_dialog_active_index = Some(target_index);
            let stack_id = self.message_dialogs[target_index].running_stack_id;
            self.activate_running_dialog(RunningDialogStackEntry::Message(stack_id));
        }
        self.handle_message_dialog_pointer_button_at(target_index, state)
    }

    pub(crate) fn runtime_client_list_keyboard_active(&self) -> bool {
        if self.mode == AppMode::Running {
            self.runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.is_info_only())
                && self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ClientList)
                && self.running_active_dialog == Some(RunningDialogStackEntry::RuntimeClientList)
                && (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.context_menu.is_none()
        } else {
            (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.message_dialogs.is_empty()
                && self.context_menu.is_none()
        }
    }

    pub(crate) fn runtime_client_list_mouse_active(&self) -> bool {
        if self.mode == AppMode::Running {
            (self.game_over_dialog.is_none() || self.runtime_client_list_above_game_over)
                && self.context_menu.is_none()
        } else {
            self.runtime_client_list_keyboard_active()
        }
    }
}
