//! `impl GameApp` — options & configuration methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn apply_classic_game_option_overrides(&mut self) {
        self.scenario_game_options.set_server_signup(
            self.classic_command_line.master_server_signup,
            self.classic_command_line.league_server_signup,
        );
        if let Some(fair_crew) = self.classic_command_line.fair_crew {
            self.startup_view_flags.fair_crew = fair_crew;
            self.scenario_game_options
                .set_lobby_fair_crew(fair_crew, false);
        }
        if let Some(password) = self.classic_command_line.password.as_ref() {
            self.scenario_game_options.set_password(password.clone());
        }
        if let Some(comment) = self.classic_command_line.comment.as_ref() {
            self.scenario_game_options.set_comment(comment.clone());
        }
        if let Some(runtime_join) = self.classic_command_line.runtime_join {
            self.runtime_network_join_allowed = Some(runtime_join);
        }
    }

    pub(crate) fn sync_scenario_game_option_bounds(&mut self) {
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return;
        };
        let surface = self.graphics.surface();
        let bounds = if let Some(lobby) = self.classic_host_lobby.as_ref() {
            lobby
                .controller
                .layout(surface.width() as i32, surface.height() as i32, fonts)
                .game_option_strip
        } else if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.sync_classic_controller();
            lobby
                .controller
                .layout(surface.width() as i32, surface.height() as i32, fonts)
                .game_option_strip
        } else {
            startup_scensel_game_option_bounds(
                surface.width() as i32,
                surface.height() as i32,
                fonts,
            )
        };
        self.scenario_game_options.set_bounds(bounds);
    }

    pub(crate) fn sync_scenario_game_option_constraint(&mut self) {
        let constraint = scenario_fair_crew_constraint(self.menu_state.selected_scenario());
        self.scenario_game_options
            .set_selector_fair_crew_constraint(constraint);
    }

    pub(crate) fn league_player_auth_settings(&self) -> clonk_network::LeagueAuthRequestHead {
        if let Some(auth) = self.league_auth_session.as_ref() {
            return auth.clone();
        }
        match self.network_mode.as_ref() {
            Some(NetworkMode::Client(settings)) => settings.league_auth.clone(),
            Some(NetworkMode::Host(_)) | None => load_league_auth_settings(self.app_paths.as_ref()),
        }
    }

    pub(crate) fn set_league_player_auth_settings(
        &mut self,
        auth: clonk_network::LeagueAuthRequestHead,
    ) {
        if let Some(NetworkMode::Client(settings)) = self.network_mode.as_mut() {
            settings.league_auth = auth.clone();
        }
        if let Some(paths) = self.app_paths.as_ref() {
            if let Err(error) = persist_league_account_preference(paths, &auth.account) {
                tracing::warn!(%error, "failed to persist league account preference");
            }
        }
        self.league_auth_session = Some(auth);
    }

    pub(crate) fn current_options_graphic(&self) -> Option<ImageData> {
        self.active_game_graphics
            .as_ref()
            .and_then(|resources| resources.options.as_deref().cloned())
            .or_else(|| self.assets.dialog_image("Options.png"))
    }

    pub(crate) fn handle_options_tab_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || key != VirtualKeyCode::Tab
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let route = if modifiers.is_empty() {
            Some((false, false))
        } else if modifiers == ModifiersState::SHIFT {
            Some((false, true))
        } else if modifiers == ModifiersState::CONTROL {
            Some((true, false))
        } else if modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT) {
            Some((true, true))
        } else {
            None
        };
        let Some((cycle_sheet, backwards)) = route else {
            // No other exact Alt/Ctrl/Shift mask owns GUIAdvanceFocus or the
            // tabular's Ctrl+Tab bindings. Consume it before the legacy
            // modifier-blind KeyCode mapping can invent a plain Tab.
            return Ok(true);
        };
        let actions = if state == ElementState::Pressed {
            self.startup_options_dialog
                .as_mut()
                .map(|dialog| {
                    if cycle_sheet {
                        dialog.handle_ctrl_tab(backwards)
                    } else {
                        dialog.handle_tab(backwards)
                    }
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.process_options_dialog_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn options_modified_gui_key_is_inert(&self, key: VirtualKeyCode) -> bool {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || key == VirtualKeyCode::Tab
            || map_key_code(key).is_none()
        {
            return false;
        }
        // C4KeyCodeEx matches the exact Alt/Ctrl/Shift mask for the Options
        // dialog bindings. Logo is not part of that mask, so Logo-only input
        // intentionally remains equivalent to the bare key.
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if modifiers == ModifiersState::ALT
            && matches!(key, VirtualKeyCode::ArrowDown | VirtualKeyCode::Space)
            && self
                .startup_options_dialog
                .as_ref()
                .is_some_and(|dialog| {
                    matches!(
                        dialog.focused_program_control(),
                        Some(
                            clonk_frontend::startup_options_dlg::OptionsProgramFocusTarget::LanguageCombo
                                | clonk_frontend::startup_options_dlg::OptionsProgramFocusTarget::FontFaceCombo
                                | clonk_frontend::startup_options_dlg::OptionsProgramFocusTarget::FontSizeCombo
                        )
                    )
                })
        {
            return false;
        }
        !modifiers.is_empty()
    }

    pub(crate) fn handle_game_option_input_dialog_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.game_option_input_dialog.is_none() {
            return Ok(false);
        }
        if self.running_chat_controller().is_some() && !self.running_chat_keyboard_active() {
            return Ok(false);
        }
        if self.context_menu.is_some() {
            return Ok(true);
        }
        let Some(layout) = self.game_option_input_layout() else {
            return Ok(true);
        };
        let Some(fonts) = self.assets.clonk_fonts.clone() else {
            tracing::error!("classic input dialog lost its required GUI fonts");
            return Ok(true);
        };
        let modifiers = InputDialogKeyModifiers {
            shift: self.keyboard_modifiers.shift_key(),
            control: self.keyboard_modifiers.control_key(),
        };
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        let hotkey_modifiers = c4_modifiers == ModifiersState::ALT
            || c4_modifiers == (ModifiersState::ALT | ModifiersState::SHIFT);
        let dialog_hotkey = hotkey_modifiers.then(|| context_menu_hotkey(key)).flatten();
        if hotkey_modifiers && dialog_hotkey.is_none() {
            return Ok(false);
        }
        if let Some(hotkey) = dialog_hotkey {
            if state == ElementState::Released {
                return Ok(false);
            }
            if !self
                .game_option_input_dialog
                .as_ref()
                .is_some_and(|dialog| dialog.controller.has_hotkey(hotkey))
            {
                return Ok(false);
            }
        }
        let clipboard_text = || {
            arboard::Clipboard::new()
                .and_then(|mut clipboard| clipboard.get_text())
                .ok()
        };
        let mut capture_release = false;
        let actions = if state == ElementState::Pressed
            && key == VirtualKeyCode::ContextMenu
            && c4_modifiers.is_empty()
        {
            self.game_option_input_dialog
                .as_mut()
                .map(|dialog| {
                    dialog.controller.request_context_menu_from_key(
                        &layout,
                        clipboard_text_available(),
                        &InputDialogContextLabels::default(),
                    )
                })
                .map(|outcome| {
                    capture_release = outcome.capture_release;
                    outcome.actions
                })
                .unwrap_or_default()
        } else if state == ElementState::Pressed
            && c4_modifiers == ModifiersState::CONTROL
            && matches!(
                key,
                VirtualKeyCode::KeyA
                    | VirtualKeyCode::KeyC
                    | VirtualKeyCode::KeyV
                    | VirtualKeyCode::KeyX
            )
        {
            let shortcut = match key {
                VirtualKeyCode::KeyC => Some(InputDialogClipboardShortcut::Copy),
                VirtualKeyCode::KeyX => Some(InputDialogClipboardShortcut::Cut),
                VirtualKeyCode::KeyV => Some(InputDialogClipboardShortcut::Paste),
                VirtualKeyCode::KeyA => Some(InputDialogClipboardShortcut::SelectAll),
                _ => None,
            };
            let shortcut = shortcut.expect("guarded classic edit shortcut");
            let clipboard = matches!(shortcut, InputDialogClipboardShortcut::Paste)
                .then(clipboard_text)
                .flatten();
            self.game_option_input_dialog
                .as_mut()
                .map(|dialog| {
                    dialog.controller.handle_clipboard_shortcut(
                        shortcut,
                        clipboard.as_deref(),
                        &layout,
                        &fonts.text,
                    )
                })
                .map(|outcome| {
                    capture_release = outcome.capture_release;
                    outcome.actions
                })
                .unwrap_or_default()
        } else if state == ElementState::Pressed {
            if !c4_modifiers.is_empty()
                && matches!(
                    key,
                    VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter | VirtualKeyCode::Escape
                )
            {
                Vec::new()
            } else {
                let edit_key = match key {
                    VirtualKeyCode::Backspace => Some(InputDialogEditKey::Backspace),
                    VirtualKeyCode::Delete => Some(InputDialogEditKey::Delete),
                    VirtualKeyCode::Home => Some(InputDialogEditKey::Home),
                    VirtualKeyCode::End => Some(InputDialogEditKey::End),
                    VirtualKeyCode::ArrowLeft => Some(InputDialogEditKey::Left),
                    VirtualKeyCode::ArrowRight => Some(InputDialogEditKey::Right),
                    _ => None,
                };
                if edit_key.is_some() && c4_modifiers.alt_key() {
                    Vec::new()
                } else if let Some(edit_key) = edit_key {
                    self.game_option_input_dialog
                        .as_mut()
                        .map(|dialog| {
                            dialog.controller.handle_edit_key_down(
                                edit_key,
                                modifiers,
                                &layout,
                                &fonts.text,
                            )
                        })
                        .unwrap_or_default()
                } else if hotkey_modifiers {
                    dialog_hotkey
                        .and_then(|hotkey| {
                            self.game_option_input_dialog
                                .as_mut()
                                .map(|dialog| dialog.controller.handle_hotkey(hotkey))
                        })
                        .unwrap_or_default()
                } else if let Some(gui_key) = map_key_code(key) {
                    self.game_option_input_dialog
                        .as_mut()
                        .map(|dialog| {
                            dialog.controller.route_key_down(
                                gui_key,
                                modifiers.shift,
                                &layout,
                                &fonts.text,
                            )
                        })
                        .map(|outcome| {
                            capture_release = outcome.capture_release;
                            outcome.actions
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
        } else if let Some(gui_key) = map_key_code(key) {
            self.game_option_input_dialog
                .as_mut()
                .map(|dialog| dialog.controller.route_key_up(gui_key))
                .map(|outcome| outcome.actions)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if state == ElementState::Pressed && capture_release {
            self.game_option_input_consumed_keys.insert(key);
        }
        self.finish_game_option_input_dialog_actions(actions)?;
        // C4GUI::Screen routes every key exclusively to the top modal. Some
        // edit/controller bindings intentionally report pass-through, but it
        // is pass-through within the modal dialog, never to ScenarioBrowser.
        Ok(true)
    }

    pub(crate) fn handle_scenario_game_option_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::ScenarioBrowser
            || self.game_option_input_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let release_latched =
            state == ElementState::Released && self.game_option_consumed_keys.remove(&key);
        let hotkey = context_menu_hotkey(key);
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if c4_modifiers.alt_key()
            && !c4_modifiers.control_key()
            && hotkey.is_some_and(|hotkey| {
                self.scenario_game_options
                    .context()
                    .buttons()
                    .iter()
                    .any(|button| button.hotkey() == hotkey)
            })
        {
            if state == ElementState::Pressed {
                let actions = self
                    .scenario_game_options
                    .handle_hotkey(hotkey.expect("checked above"));
                self.finish_game_option_input(actions)?;
                self.game_option_consumed_keys.insert(key);
            }
            return Ok(true);
        }
        let Some(gui_key) = map_key_code(key) else {
            return Ok(release_latched);
        };
        let options_focused = self.scenario_game_options.focused_button().is_some();
        if gui_key == KeyCode::Tab && state == ElementState::Pressed {
            if options_focused {
                self.menu_state
                    .set_dialog_focus(ScenselDialogFocus::Options);
                let outcome = self
                    .scenario_game_options
                    .handle_key_down_with_tab_direction(
                        KeyCode::Tab,
                        self.keyboard_modifiers.shift_key(),
                    );
                self.finish_game_option_input(outcome.actions)?;
                self.game_option_consumed_keys.insert(key);
                return Ok(true);
            }
            self.advance_scensel_dialog_focus(self.keyboard_modifiers.shift_key());
            self.game_option_consumed_keys.insert(key);
            return Ok(true);
        }
        if !options_focused {
            return Ok(release_latched);
        }
        self.menu_state
            .set_dialog_focus(ScenselDialogFocus::Options);
        let outcome = match state {
            ElementState::Pressed => self.scenario_game_options.handle_key_down(gui_key),
            ElementState::Released => self.scenario_game_options.handle_key_up(gui_key),
        };
        if state == ElementState::Pressed && outcome.captured {
            self.game_option_consumed_keys.insert(key);
        }
        self.finish_game_option_input(outcome.actions)?;
        Ok(outcome.captured || release_latched)
    }

    pub(crate) fn runtime_key_config(&self) -> Result<&RuntimeKeyConfig> {
        self.runtime_key_config_cache
            .get_or_init(|| {
                load_runtime_global_key_config(self.app_paths.as_ref())
                    .map_err(|error| format!("{error:#}"))
            })
            .as_ref()
            .map_err(|detail| anyhow!(detail.clone()))
    }

    pub(crate) fn handle_options_control_capture_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        use clonk_frontend::startup_options_controls::ControlDevice;
        let target = self
            .message_dialogs
            .last()
            .and_then(|pending| match pending.continuation {
                MessageDialogContinuation::OptionsControlCapture(target)
                    if target.device == ControlDevice::Keyboard =>
                {
                    Some(target)
                }
                _ => None,
            });
        let Some(target) = target else {
            return Ok(false);
        };
        match state {
            ElementState::Released => {
                self.message_dialog_consumed_keys.remove(&key);
                return Ok(true);
            }
            ElementState::Pressed => {
                self.message_dialog_consumed_keys.insert(key);
            }
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if !c4_modifiers.is_empty() {
            return Ok(true);
        }
        if !KeyboardBindings::is_supported_key(key) {
            tracing::warn!(?key, "ignoring control capture for an unpersistable key");
            return Ok(true);
        }
        let Some(id) = ControlBindingId::ALL.get(target.control).copied() else {
            return Ok(true);
        };
        if !self.bindings.rebind_for_set(target.set, id, key) {
            return Ok(true);
        }
        if let Some(dialog) = self.startup_options_dialog.as_mut() {
            dialog
                .controls_mut()
                .set_label(target, format_key_label(key));
        }
        self.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)?;
        Ok(true)
    }

    pub(crate) fn option_flags(&self, player: i32) -> OptionFlags {
        let player_mouse = self
            .engine
            .player(player)
            .map(|player| player.mouse_control() != 0);
        // C4PlayerList::MouseControlTaken scans raw MouseControl flags on
        // local players. This is deliberately not mouse_owner(): restored
        // data may retain a raw flag after the process-global controller has
        // been cleared.
        let mouse_taken = self.local_controls.assignments().any(|assignment| {
            self.engine
                .player(assignment.owner)
                .is_some_and(|player| player.mouse_control() != 0)
        });
        let mouse = player_mouse.unwrap_or(false);
        OptionFlags {
            sound: self
                .audio
                .as_ref()
                .map(|audio| audio.options.sound_enabled)
                .unwrap_or(false),
            music: self
                .audio
                .as_ref()
                .map(|audio| audio.options.music_enabled)
                .unwrap_or(false),
            mouse_shown: self.mouse_control_allowed
                && player_mouse.is_some()
                && (mouse || !mouse_taken),
            mouse,
        }
    }

    pub(crate) fn handle_game_option_input_dialog_gamepad_event(
        &mut self,
        event: GamepadEvent,
    ) -> Result<(), EngineError> {
        let layout = self.game_option_input_layout();
        let fonts = self.assets.clonk_fonts.clone();
        let actions = self
            .game_option_input_dialog
            .as_mut()
            .map(|dialog| match event {
                GamepadEvent::Direction {
                    button: ControlButton::Left,
                    state: ElementState::Pressed,
                    ..
                } => dialog.controller.handle_gamepad_direction(false),
                GamepadEvent::Direction {
                    button: ControlButton::Right,
                    state: ElementState::Pressed,
                    ..
                } => dialog.controller.handle_gamepad_direction(true),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Pressed,
                    ..
                } => layout
                    .as_ref()
                    .zip(fonts.as_deref())
                    .map(|(layout, fonts)| {
                        dialog
                            .controller
                            .handle_gamepad_low_down(layout, &fonts.text)
                    })
                    .unwrap_or_default(),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::Low,
                    state: ElementState::Released,
                    ..
                } => dialog.controller.handle_gamepad_low_up(),
                GamepadEvent::GuiButton {
                    class: GuiButtonClass::High,
                    state: ElementState::Pressed,
                    ..
                } => dialog.controller.handle_gamepad_high_down(),
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
        self.finish_game_option_input_dialog_actions(actions)
    }

    pub(crate) fn publish_game_over_host_reference_with_config(
        &mut self,
        config: clonk_network::NetworkGameAdvertiserConfig,
    ) {
        if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        let Some(template) = self.advertised_game_reference.clone() else {
            return;
        };
        let parameters = self
            .host_join_snapshot
            .as_ref()
            .map(|snapshot| snapshot.parameters.clone())
            .unwrap_or_else(|| template.parameters().clone());
        let max_players = self
            .engine
            .max_players()
            .unwrap_or_else(|| i32::try_from(self.network_max_players).unwrap_or(i32::MAX));
        let updated = match game_over_host_reference(
            &template,
            parameters,
            &self.control_clients,
            &self.control_player_infos,
            self.engine.teams(),
            max_players,
            &self.snapshot,
        ) {
            Ok(reference) => reference,
            Err(error) => {
                tracing::error!(%error, "failed to rebuild game-over host reference");
                return;
            }
        };

        // The final reference is authoritative even if an optional listener
        // update or one-shot rebind fails.
        self.advertised_game_reference = Some(updated.clone());

        if let Some(advertiser) = self.network_game_advertiser.as_ref() {
            if let Err(error) = advertiser.update_exact(&updated) {
                tracing::error!(%error, "failed to update game-over host reference");
            }
        } else {
            match clonk_network::NetworkGameAdvertiser::start_exact(config, updated.clone()) {
                Ok(advertiser) => self.network_game_advertiser = Some(advertiser),
                Err(error) => {
                    tracing::warn!(%error, "game-over network advertising unavailable");
                }
            }
        }
    }

    fn open_options_language_combo(&mut self) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some((anchor, entries)) = self.startup_options_dialog.as_ref().and_then(|dialog| {
            let anchor = dialog.language_combo_anchor()?;
            let entries = dialog
                .program()
                .language_infos
                .iter()
                .map(|info| {
                    ContextMenuEntry::new(format!("{} - {}", info.code, info.name))
                        .with_icon(ContextMenuIcon::Empty)
                        .with_action(AppContextMenuCommand::OptionsLanguage(info.code.clone()))
                })
                .collect();
            Some((anchor, entries))
        }) else {
            return Ok(false);
        };
        self.open_context_menu_at(entries, anchor)
    }

    fn open_options_font_face_combo(&mut self) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some(anchor) = self
            .startup_options_dialog
            .as_ref()
            .and_then(|dialog| dialog.font_face_combo_anchor())
        else {
            return Ok(false);
        };
        let entries = clonk_frontend::startup_options_dlg::PROGRAM_FONT_FACES
            .into_iter()
            .map(|face| {
                ContextMenuEntry::new(face)
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(AppContextMenuCommand::OptionsFontFace(face.to_string()))
            })
            .collect();
        self.open_context_menu_at(entries, anchor)
    }

    fn open_options_font_size_combo(&mut self) -> Result<bool, EngineError> {
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || !self.message_dialogs.is_empty()
            || self.game_over_dialog.is_some()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some(anchor) = self
            .startup_options_dialog
            .as_ref()
            .and_then(|dialog| dialog.font_size_combo_anchor())
        else {
            return Ok(false);
        };
        let entries = clonk_frontend::startup_options_dlg::PROGRAM_FONT_SIZES
            .into_iter()
            .map(|size| {
                ContextMenuEntry::new(size.to_string())
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(AppContextMenuCommand::OptionsFontSize(size))
            })
            .collect();
        self.open_context_menu_at(entries, anchor)
    }

    fn open_options_display_mode_combo(&mut self) -> Result<bool, EngineError> {
        use clonk_frontend::startup_options_graphics::GraphicsDisplayMode;
        if self.mode != AppMode::Menu
            || self.startup_view != StartupView::Options
            || !self.message_dialogs.is_empty()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some(anchor) = self
            .startup_options_dialog
            .as_ref()
            .and_then(|dialog| dialog.graphics_display_combo_anchor())
        else {
            return Ok(false);
        };
        let entries = GraphicsDisplayMode::ALL
            .into_iter()
            .map(|mode| {
                ContextMenuEntry::new(mode.label())
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(AppContextMenuCommand::OptionsDisplayMode(mode))
            })
            .collect();
        self.open_context_menu_at(entries, anchor)
    }

    pub(crate) fn open_runtime_client_list_option_combo(
        &mut self,
        option: LobbyOptionKind,
        anchor: GuiPoint,
        minimum_width: i32,
    ) -> Result<bool, EngineError> {
        if self.context_menu_pointer_dismissed_lobby_option.take() == Some(option) {
            return Ok(false);
        }
        if self.mode != AppMode::Running || self.context_menu.is_some() {
            return Ok(false);
        }
        let Some(choices) = self.runtime_client_list.as_ref().and_then(|dialog| {
            (!dialog.is_info_only()).then(|| {
                dialog
                    .option_rows()
                    .iter()
                    .find(|row| row.kind == option && row.editable && !row.choices.is_empty())
                    .map(|row| row.choices.clone())
            })?
        }) else {
            return Ok(false);
        };
        let entries = choices
            .into_iter()
            .map(|choice| {
                ContextMenuEntry::new(choice.label)
                    .with_tooltip(choice.tooltip)
                    .with_icon(ContextMenuIcon::Empty)
                    .with_action(AppContextMenuCommand::RuntimeClientOption {
                        option,
                        value: choice.id,
                    })
            })
            .collect();
        let opened =
            self.open_context_menu_at_with_minimum_width(entries, anchor, minimum_width, None)?;
        if opened {
            self.set_context_menu_lobby_option(Some(option));
        }
        Ok(opened)
    }

    pub(crate) fn apply_runtime_client_list_option(
        &mut self,
        option: LobbyOptionKind,
        value: i32,
    ) -> Result<(), EngineError> {
        let valid_choice = self.runtime_client_list.as_ref().is_some_and(|dialog| {
            dialog.option_rows().iter().any(|row| {
                row.kind == option
                    && row.editable
                    && row.choices.iter().any(|choice| choice.id == value)
            })
        });
        if !valid_choice {
            return Ok(());
        }
        match option {
            LobbyOptionKind::ControlMode => {
                if self.engine.is_control_host()
                    && matches!(value, 0..=2)
                    && (!self.network_is_league || value != 2)
                {
                    self.change_running_network_control_mode(value);
                }
            }
            LobbyOptionKind::ControlRate => {
                if self.engine.is_control_host() && (1..=9).contains(&value) {
                    let current = self
                        .network_control_clock
                        .map(NetworkControlClock::control_rate)
                        .unwrap_or_else(|| self.engine.control_rate());
                    if value != current {
                        self.submit_or_execute_running_control_set(0, value - current)?;
                    }
                }
            }
            LobbyOptionKind::RuntimeJoin => {
                if matches!(self.runtime_network_role(), RuntimeNetworkRole::Host) {
                    let allowed = value != 0;
                    let result = self
                        .network
                        .as_ref()
                        .ok_or_else(|| anyhow!("runtime network is unavailable"))
                        .and_then(|network| network.set_join_allowed(allowed));
                    if let Err(error) = result {
                        tracing::error!(%error, allowed, "failed to change runtime join admission");
                        return Ok(());
                    }
                    self.runtime_network_join_allowed = Some(allowed);
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
                    self.publish_running_host_reference();
                    let labels = self.classic_lobby_option_labels();
                    let message = if allowed {
                        labels.runtime_join_free
                    } else {
                        labels.runtime_join_barred
                    };
                    match self
                        .prepare_runtime_flash_message(&message, self.runtime_language_charset)
                    {
                        Ok(message) => self.runtime_flash_message = message,
                        Err(error) => {
                            tracing::warn!(%error, "failed to prepare runtime-join flash message")
                        }
                    }
                }
            }
            LobbyOptionKind::TeamDistribution
            | LobbyOptionKind::TeamColors
            | LobbyOptionKind::RandomTeamCount => {}
        }
        self.refresh_runtime_client_list();
        Ok(())
    }

    fn sync_options_gamepad_device(&mut self) {
        use clonk_frontend::startup_options_controls::ControlDevice;
        use clonk_frontend::startup_options_dlg::OptionsSheet;

        let selected = self.startup_options_dialog.as_ref().and_then(|dialog| {
            (self.mode == AppMode::Menu
                && self.startup_view == StartupView::Options
                && dialog.active_sheet() == OptionsSheet::Gamepad)
                .then(|| dialog.controls().selected_set(ControlDevice::Gamepad))
        });
        self.gamepads
            .set_options_open_slot(selected.and_then(GamepadSlot::from_index));
    }

    pub(crate) fn process_options_dialog_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_options_dlg::OptionsDlgAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_dlg::{
            OptionsDlgAction, OptionsSheet, SoundCheckboxId, SoundSheetAction, SoundVolumeId,
        };
        use clonk_frontend::startup_options_graphics::GraphicsSheetAction;
        use clonk_frontend::startup_options_network::{NetworkCheckboxId, NetworkValidationError};

        for action in actions {
            match action {
                OptionsDlgAction::Back => {
                    let validation = self
                        .startup_options_dialog
                        .as_ref()
                        .map(|dialog| dialog.network().validate_ports())
                        .unwrap_or(Ok(()));
                    match validation {
                        Ok(()) => self.close_options_menu()?,
                        Err(NetworkValidationError::TcpReferenceConflict) => {
                            let message = self.runtime_resource_text(
                                "IDS_NET_ERR_PORT_TCPREF",
                                "TCP port and reference port must be set to different values between 1 and 65535!",
                            );
                            self.show_options_network_validation_error(&message)?;
                        }
                        Err(NetworkValidationError::UdpDiscoveryConflict) => {
                            let message = self.runtime_resource_text(
                                "IDS_NET_ERR_PORT_UDPDISC",
                                "UDP port and discovery port must be set to different values between 1 and 65535!",
                            );
                            self.show_options_network_validation_error(&message)?;
                        }
                    }
                }
                OptionsDlgAction::SheetChanged(sheet) => {
                    self.sync_options_gamepad_device();
                    if sheet == OptionsSheet::Sound && self.audio.is_none() {
                        return Err(classic_parity_engine_error(report_classic_parity_boundary(
                            ClassicParityBoundary::RuntimeAudioSystem {
                                action: "the startup Options Audio sheet",
                            },
                        )));
                    }
                    self.play_ui_sound("Command");
                }
                OptionsDlgAction::ShowLogTimestampsChanged(enabled) => {
                    self.show_log_timestamps = enabled;
                    self.play_ui_sound("ArrowHit");
                }
                OptionsDlgAction::OpenLanguageCombo => {
                    self.open_options_language_combo()?;
                }
                OptionsDlgAction::OpenFontFaceCombo => {
                    self.open_options_font_face_combo()?;
                }
                OptionsDlgAction::OpenFontSizeCombo => {
                    self.open_options_font_size_combo()?;
                }
                OptionsDlgAction::WhiteChatIngameChanged(enabled) => {
                    self.display_flags.white_chat = enabled;
                    self.play_ui_sound("ArrowHit");
                }
                OptionsDlgAction::WhiteChatLobbyChanged(enabled) => {
                    self.white_lobby_chat = enabled;
                    self.play_ui_sound("ArrowHit");
                }
                OptionsDlgAction::PreloadingChanged(_) => {
                    self.play_ui_sound("ArrowHit");
                }
                OptionsDlgAction::ProgramGuiSound(sound) => {
                    self.play_options_sound(sound);
                }
                OptionsDlgAction::FairCrewStrengthChanged(_) => {}
                OptionsDlgAction::ResetConfiguration => {
                    self.play_ui_sound("Command");
                    self.show_options_reset_configuration()?;
                }
                OptionsDlgAction::OpenAdvancedSettings => {
                    self.play_ui_sound("Command");
                    self.show_options_advanced_warning()?;
                }
                OptionsDlgAction::Sound(action) => match action {
                    SoundSheetAction::GuiSound(sound) => self.play_options_sound(sound),
                    SoundSheetAction::TestSound(sound) => self.play_options_test_sound(sound),
                    SoundSheetAction::CheckboxChanged { id, checked } => match id {
                        SoundCheckboxId::FrontendMusic => {
                            self.set_frontend_music_option(checked)?;
                        }
                        SoundCheckboxId::FrontendSoundEffects => {
                            self.set_frontend_sound_option(checked)?;
                        }
                        SoundCheckboxId::GameMusic => {
                            self.set_startup_game_music_option(checked)?;
                        }
                        SoundCheckboxId::GameSoundEffects => {
                            self.set_startup_game_sound_option(checked)?;
                        }
                        SoundCheckboxId::VoiceEnabled => {
                            self.set_startup_voice_enabled(checked)?;
                        }
                        SoundCheckboxId::VoiceActivated => {
                            self.set_startup_voice_activation_mode(checked)?;
                        }
                    },
                    SoundSheetAction::VolumeChanged { id, value } => match id {
                        SoundVolumeId::Music => {
                            self.set_startup_music_volume(i32::from(value))?;
                        }
                        SoundVolumeId::SoundEffects => {
                            self.set_startup_sound_volume(i32::from(value))?;
                        }
                        SoundVolumeId::Voice => {
                            self.set_startup_voice_volume(i32::from(value))?;
                        }
                    },
                },
                OptionsDlgAction::BeginVoicePushToTalkCapture => {
                    self.open_options_voice_capture()?;
                }
                OptionsDlgAction::Graphics(action) => match action {
                    GraphicsSheetAction::OpenDisplayModeCombo => {
                        self.open_options_display_mode_combo()?;
                    }
                    GraphicsSheetAction::DisplayModeChanged(mode) => {
                        self.queue_options_display_request(OptionsDisplayRequest::SetMode(mode));
                    }
                    GraphicsSheetAction::CheckboxChanged { .. }
                    | GraphicsSheetAction::ScaleProposalChanged(_) => {
                        self.play_ui_sound("ArrowHit");
                    }
                    GraphicsSheetAction::SmokeLevelChanged(value) => {
                        self.graphics_smoke_level = value;
                        self.engine.set_smoke_level(value);
                        self.play_ui_sound("ArrowHit");
                    }
                    GraphicsSheetAction::TestScale {
                        old_percent,
                        new_percent,
                    } => {
                        self.begin_options_scale_test(old_percent, new_percent)?;
                    }
                },
                OptionsDlgAction::OpenGraphicsScaleText => {
                    self.open_options_graphics_scale_input()?;
                }
                OptionsDlgAction::BeginControlCapture(target) => {
                    self.open_options_control_capture(target)?;
                }
                OptionsDlgAction::ResetControlBindings(device) => {
                    // `OnResetKeysBtn` is silent (C4StartupOptionsDlg.cpp:416-427);
                    // the button's own `ArrowHit`/`Click` pair is the whole
                    // sequence, and arrives as its own action.
                    self.reset_options_control_bindings(device);
                }
                OptionsDlgAction::GamepadDeviceSelected(set) => {
                    let valid_selection = self.mode == AppMode::Menu
                        && self.startup_view == StartupView::Options
                        && self.startup_options_dialog.as_ref().is_some_and(|dialog| {
                            dialog.active_sheet() == OptionsSheet::Gamepad
                                && dialog.controls().selected_set(
                                    clonk_frontend::startup_options_controls::ControlDevice::Gamepad,
                                ) == set
                                && set < dialog.controls().visible_sets(
                                    clonk_frontend::startup_options_controls::ControlDevice::Gamepad,
                                )
                        });
                    if valid_selection {
                        self.gamepads
                            .set_options_open_slot(GamepadSlot::from_index(set));
                    }
                }
                OptionsDlgAction::GamepadGuiControlChanged(enabled) => {
                    self.gamepad_gui_control = enabled;
                    self.play_ui_sound("ArrowHit");
                    // `RecreateDialog(false)` (C4StartupOptionsDlg.cpp:437)
                    // constructs a whole new dialog through `SwitchDialog`
                    // (:1331), then restores only the active sheet index (:1332)
                    // — every other child, including both `ControlConfigArea`s,
                    // is rebuilt from Config.
                    let return_sheet = self
                        .startup_options_dialog
                        .as_ref()
                        .map(|dialog| dialog.active_sheet());
                    self.startup_tooltip.pointer_left();
                    // `RecreateDialog` forces `fFade = true` (:1328), so the old
                    // dialog is retained to fade out before the new one appears.
                    self.begin_startup_dialog_fade(StartupDialog::Options);
                    self.open_options_menu();
                    if let (Some(dialog), Some(sheet)) =
                        (self.startup_options_dialog.as_mut(), return_sheet)
                    {
                        dialog.restore_sheet(sheet);
                    }
                    self.sync_options_gamepad_device();
                }
                OptionsDlgAction::OpenNetworkText(field) => {
                    self.open_options_network_input(field)?;
                }
                OptionsDlgAction::NetworkPortEnabledChanged { .. } => {
                    self.play_ui_sound("ArrowHit");
                }
                OptionsDlgAction::NetworkCheckboxChanged { id, checked } => {
                    self.play_ui_sound("ArrowHit");
                    if id == NetworkCheckboxId::UseAlternateServer && checked {
                        let hidden = self
                            .startup_options_dialog
                            .as_ref()
                            .is_some_and(|dialog| dialog.network().hide_no_official_league_notice);
                        if !hidden {
                            self.show_options_alternate_server_notice()?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn apply_options_font_selection(
        &mut self,
        selected_face: Option<String>,
        selected_size: Option<i32>,
    ) -> Result<(), EngineError> {
        self.apply_options_font_selection_with_system_fonts(
            selected_face,
            selected_size,
            system_fonts::installed_system_fonts(),
        )
    }

    pub(crate) fn apply_options_font_selection_with_system_fonts(
        &mut self,
        selected_face: Option<String>,
        selected_size: Option<i32>,
        system_fonts: &dyn system_fonts::SystemFontProvider,
    ) -> Result<(), EngineError> {
        let Some((current_face, current_size)) =
            self.startup_options_dialog.as_ref().map(|dialog| {
                (
                    dialog.program().font_face.clone(),
                    dialog
                        .program()
                        .font_size
                        .trim()
                        .parse::<i32>()
                        .unwrap_or(14),
                )
            })
        else {
            return Ok(());
        };
        let face = selected_face.unwrap_or_else(|| current_face.clone());
        let size = selected_size.unwrap_or(current_size);
        let Some(paths) = self.app_paths.as_ref() else {
            return self.show_options_font_error();
        };
        let resources = (|| -> Result<_> {
            let registrations = startup_loader_registrations(paths)?;
            let gui = resolve_classic_font_bundle_for_request_with_system_fonts(
                paths,
                &face,
                size,
                &registrations,
                &registrations,
                system_fonts,
            )?;
            let startup = resolve_classic_startup_font_bundle_for_request_with_system_fonts(
                paths,
                &face,
                size,
                &registrations,
                &registrations,
                system_fonts,
            )?;
            Ok((gui, startup))
        })();
        let (gui, startup) = match resources {
            Ok(resources) => resources,
            Err(error) => {
                tracing::warn!(%error, %face, size, "failed to apply selected options font");
                return self.show_options_font_error();
            }
        };

        if let Some(dialog) = self.startup_options_dialog.as_mut() {
            dialog.program_mut().set_font(face.clone(), size);
        }
        match self.persist_open_options_config() {
            Some(Ok(())) => {}
            Some(Err(error)) => {
                tracing::warn!(%error, "failed to save selected options font");
                if let Some(dialog) = self.startup_options_dialog.as_mut() {
                    dialog.program_mut().set_font(current_face, current_size);
                }
                return self.show_options_font_error();
            }
            None => {
                if let Some(dialog) = self.startup_options_dialog.as_mut() {
                    dialog.program_mut().set_font(current_face, current_size);
                }
                return self.show_options_font_error();
            }
        }

        self.begin_startup_dialog_fade(StartupDialog::Options);
        let ClassicFontBundle {
            fonts,
            tooltip,
            native_source,
        } = gui;
        let native_fonts = self.native_font_cache_for_source(native_source.as_ref());
        let player_selection_fonts = startup.player_selection.clone();
        {
            let assets = Arc::make_mut(&mut self.assets);
            assets.clonk_fonts = Some(fonts.clone());
            assets.startup_clonk_fonts = Some(fonts.clone());
            assets.global_tooltip_font = Some(tooltip.clone());
            assets.startup_global_tooltip_font = Some(tooltip);
            assets.startup_native_font_source = native_source;
            assets.book_fonts = Some(startup.book);
            assets.options_book_fonts = Some(startup.options);
            assets.plrsel_book_fonts = Some(startup.player_selection);
        }
        if let Some(dialog) = self.startup_player_dialog.as_mut() {
            dialog.set_layout_fonts(fonts.as_ref(), player_selection_fonts.as_ref());
        }
        self.graphics.set_clonk_fonts(Some(fonts.clone()));
        self.main_menu_state.menu.set_clonk_fonts(Some(fonts));
        self.native_startup_fonts = native_fonts;
        self.open_options_menu();
        Ok(())
    }

    fn show_options_font_error(&mut self) -> Result<(), EngineError> {
        let message = self.runtime_resource_text("IDS_ERR_INITFONTS", "Error initializing fonts");
        let caption = self.runtime_resource_text("IDS_DLG_ERROR", "Error");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
    }

    fn show_options_reset_configuration(&mut self) -> Result<(), EngineError> {
        let prompt = self.runtime_resource_text(
            "IDS_MSG_PROMPTRESETCONFIG",
            "Are you sure you want to reset all configuration values?",
        );
        let restart = self.runtime_resource_text(
            "IDS_MSG_RESTARTCHANGECFG",
            "For changes to take effect the program has to be restarted.",
        );
        let caption = self.runtime_resource_text("IDS_BTN_RESETCONFIG", "Reset configuration");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                format!("{prompt}|{restart}"),
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::OptionsResetConfiguration,
        )
    }

    fn show_options_advanced_warning(&mut self) -> Result<(), EngineError> {
        let message = self.runtime_resource_text(
            "IDS_MSG_ADVANCED_SETTINGS_WARNING",
            "Some settings only apply after a restart.|Modifications may cause Clonk to stop working correctly. Proceed at your own risk!",
        );
        let caption = self.runtime_resource_text("IDS_DLG_WARNING", "Warning");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogButtons::OK_CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::None,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                true,
            ),
            MessageDialogContinuation::OptionsAdvancedWarning,
        )
    }

    fn show_options_advanced_error(
        &mut self,
        detail: impl std::fmt::Display,
    ) -> Result<(), EngineError> {
        let caption = self.runtime_resource_text("IDS_DLG_ERROR", "Error");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                format!("Unable to access advanced settings: {detail}"),
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
    }

    pub(crate) fn open_options_advanced_dialog(&mut self) -> Result<(), EngineError> {
        let Some(config_path) = self.app_paths.as_ref().map(|paths| paths.config_file()) else {
            self.show_options_advanced_error("application configuration is unavailable")?;
            return Ok(());
        };
        let mut config = match Config::load(&config_path) {
            Ok(config) => config,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
            Err(error) => {
                tracing::warn!(%error, "failed to load advanced configuration");
                self.show_options_advanced_error(error)?;
                return Ok(());
            }
        };
        if self.apply_open_options_config(&mut config).is_none() {
            self.show_options_advanced_error("the Options dialog is unavailable")?;
            return Ok(());
        }
        let return_sheet = self
            .startup_options_dialog
            .as_ref()
            .map(|dialog| dialog.active_sheet())
            .unwrap_or_default();
        let voice_input_devices = match clonk_audio::voice_input_devices() {
            Ok(devices) => devices,
            Err(error) => {
                tracing::warn!(%error, "could not enumerate voice input devices");
                Vec::new()
            }
        };
        let mut controller =
            clonk_frontend::startup_options_advanced::AdvancedConfigController::new(
                advanced_config::sections_with_voice_input_devices(&config, &voice_input_devices),
            );
        controller.set_labels(
            clonk_frontend::startup_options_advanced::AdvancedConfigLabels {
                caption: self
                    .runtime_resource_text("IDS_DLG_ADVANCED_SETTINGS", "Advanced settings"),
                save: self.runtime_resource_text("IDS_BTN_SAVE", "&Save"),
                cancel: self.runtime_resource_text("IDS_BTN_CANCEL", "Cancel"),
            },
        );
        controller.resize(
            self.graphics.surface().width() as i32,
            self.graphics.surface().height() as i32,
        );
        self.startup_options_advanced_dialog = Some(PendingOptionsAdvancedDialog {
            controller,
            return_sheet,
        });
        Ok(())
    }

    pub(crate) fn synchronize_advanced_options_runtime(&mut self) {
        // The advanced editor writes `Network.MasterServerSignUp` straight to
        // the file, so an older netdlg toggle must stop shadowing it.
        self.deferred_config.clear("Network", "MasterServerSignUp");
        self.clear_deferred_display_toggles();
        let paths = self.app_paths.as_ref();
        let is_fullscreen = self.display_flags.is_fullscreen;
        self.display_flags = load_display_flags(paths);
        self.display_flags.is_fullscreen = is_fullscreen;
        self.white_lobby_chat = load_white_lobby_chat(paths);
        self.show_log_timestamps = load_show_log_timestamps(paths);
        self.show_folder_maps = load_show_folder_maps(paths);
        self.ready_check_toasts_enabled = load_ready_check_toasts_enabled(paths);
        let native_config = load_native_config_bytes(paths);
        self.allow_scripting_in_replays = configured_allow_scripting_in_replays(&native_config);
        self.max_refresh_delay_ms = configured_max_refresh_delay_ms(&native_config);
        self.startup_refresh_delay_ms =
            crate::effective_max_refresh_delay_ms(&native_config, self.display_refresh_period_ms);
        let record = load_recording_flag(paths);
        self.startup_view_flags.record = record;
        self.recording_enabled = record && self.recordings_dir.is_some();
        self.startup_view_flags.fair_crew = load_fair_crew_flag(paths);
        self.graphics_smoke_level = load_graphics_smoke_level(paths);
        self.engine.set_smoke_level(self.graphics_smoke_level);
        self.engine
            .set_fire_particles(self.display_flags.fire_particles);
        self.graphics.set_pxs_graphics(self.display_flags.pxs_gfx);
        self.mission_access = paths
            .and_then(|paths| match load_configured_mission_access(paths) {
                Ok(access) => Some(MissionAccessStore::new(access)),
                Err(error) => {
                    tracing::warn!(%error, "failed to reload General.MissionAccess");
                    None
                }
            })
            .unwrap_or_default();
        self.persisted_mission_access = self.mission_access.snapshot();
        self.engine
            .set_mission_access_store(self.mission_access.clone());
        self.bindings = KeyboardBindings::load(paths);
        self.gamepad_bindings = GamepadBindings::load(paths);
        self.gamepads
            .set_axis_calibrations(self.gamepad_bindings.axis_calibrations());
        self.gamepads_enabled = load_gamepads_enabled(paths);
        self.gamepad_gui_control = load_gamepad_gui_control(paths);
        self.engine
            .set_control_key_names(configured_control_key_names(&self.bindings));

        let reloaded_audio = AudioOptions::load(paths);
        if self.audio.is_none() && reloaded_audio.voice_enabled {
            match AudioContext::try_new_with_paths(reloaded_audio.clone(), paths) {
                Ok(audio) => self.audio = Some(audio),
                Err(error) => {
                    tracing::warn!(%error, "voice opt-in could not initialise audio");
                }
            }
        }
        if let Some(audio) = self.audio.as_mut() {
            let music_volume = reloaded_audio.music_volume_percent();
            let sound_volume = reloaded_audio.sound_volume_percent();
            audio.options = reloaded_audio;
            audio.set_music_volume_percent(music_volume);
            audio.set_sound_volume_percent(sound_volume);
        }
        let point_filtering = DisplayOptions::load(paths).point_filtering;
        self.graphics.set_point_filtering(point_filtering);
        let advanced_renderer_config = load_advanced_renderer_config(&native_config);
        self.graphics
            .set_advanced_renderer_config(advanced_renderer_config);
        self.loader_gamma = load_classic_loader_gamma_from_native(&native_config);
        let main_menu_gamma = self.graphics.fragment_gamma_enabled().then(|| {
            Arc::new(
                self.loader_gamma
                    .clone()
                    .unwrap_or_else(clonk_graphics::GammaRamp::standard),
            )
        });
        self.main_menu_state.menu.set_gamma_ramp(main_menu_gamma);
        if let Some(config) = self.loader_render_config {
            self.configure_native_startup_fonts(config.application_scale(), point_filtering);
        }
        self.menu_backdrop_cache = StartupBackdropCache::default();
        self.startup_dialog_fade = None;
    }

    pub(crate) fn process_options_advanced_actions(
        &mut self,
        actions: Vec<clonk_frontend::startup_options_advanced::AdvancedConfigAction>,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_advanced::AdvancedConfigAction;

        let sounds = self
            .startup_options_advanced_dialog
            .as_mut()
            .map(|pending| pending.controller.take_sound_events())
            .unwrap_or_default();
        for sound in sounds {
            use clonk_frontend::startup_options_advanced::AdvancedConfigSound;
            self.play_ui_sound(match sound {
                AdvancedConfigSound::ArrowHit => "ArrowHit",
                AdvancedConfigSound::Click => "Click",
                AdvancedConfigSound::Command => "Command",
            });
        }

        for action in actions {
            match action {
                AdvancedConfigAction::Cancel => {
                    self.startup_tooltip.pointer_left();
                    self.startup_options_advanced_dialog = None;
                }
                AdvancedConfigAction::Save => {
                    let Some((changes, return_sheet)) = self
                        .startup_options_advanced_dialog
                        .as_ref()
                        .map(|pending| (pending.controller.changes(), pending.return_sheet))
                    else {
                        continue;
                    };
                    let saved = self.save_options_advanced_changes(&changes);
                    match saved {
                        Ok(()) => {
                            self.startup_tooltip.pointer_left();
                            self.startup_options_advanced_dialog = None;
                            self.synchronize_advanced_options_runtime();
                            self.open_options_menu();
                            if let Some(dialog) = self.startup_options_dialog.as_mut() {
                                dialog.restore_sheet(return_sheet);
                            }
                            self.sync_options_gamepad_device();
                        }
                        Err(error) => {
                            tracing::warn!(%error, "failed to save advanced configuration");
                            self.show_options_advanced_error(error)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn show_options_network_validation_error(&mut self, message: &str) -> Result<(), EngineError> {
        let caption = self.runtime_resource_text("IDS_ERR_CONFIG", "Configuration error");
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            ),
            MessageDialogContinuation::None,
        )
    }

    /// `ResChangeConfirmDlg::UpdateText` re-renders `IDS_MNU_SWITCHRESOLUTION_TEXT`
    /// with the remaining seconds on every tick (C4StartupOptionsDlg.cpp:115-125).
    fn options_scale_test_message(&self, remaining_seconds: u32) -> String {
        clonk_app_menus::substitute_resource_arguments(
            &self.runtime_resource_text(
                "IDS_MNU_SWITCHRESOLUTION_TEXT",
                "This is your new resolution. Do you like it?|Original resolution will be \
                 restored in %u seconds...",
            ),
            &[&remaining_seconds.to_string()],
        )
    }

    pub(crate) fn begin_options_scale_test(
        &mut self,
        old_percent: i32,
        new_percent: i32,
    ) -> Result<(), EngineError> {
        let dialog = clonk_frontend::message_dialog::MessageDialogState::new(
            self.options_scale_test_message(12),
            self.runtime_resource_text("IDS_MNU_SWITCHRESOLUTION", "Switch resolution"),
            clonk_frontend::message_dialog::MessageDialogButtons::YES_NO,
            clonk_frontend::message_dialog::MessageDialogIcon::CONFIRM,
            clonk_frontend::message_dialog::MessageDialogSize::Regular,
            true,
        );
        self.push_message_dialog(
            dialog,
            MessageDialogContinuation::OptionsScaleTest {
                old_percent,
                new_percent,
                remaining_seconds: 12,
            },
        )?;
        self.queue_options_display_request(OptionsDisplayRequest::SetScale {
            percent: new_percent,
            persist: false,
        });
        Ok(())
    }

    fn open_options_control_capture(
        &mut self,
        target: clonk_frontend::startup_options_controls::ControlCaptureTarget,
    ) -> Result<(), EngineError> {
        // `LoadKeyDescResStr(iKeyID)` — the same localized `IDS_CTL_*` string the
        // key button draws beside its cap (C4StartupOptionsDlg.cpp:162-173,
        // 176-177, 242), not a separate hand-written name.
        let control = self
            .startup_options_dialog
            .as_ref()
            .and_then(|dialog| dialog.labels().control_keys.get(target.control))
            .map(String::as_str)
            .or_else(|| {
                clonk_frontend::startup_options_controls::CONTROL_KEY_LABELS
                    .get(target.control)
                    .copied()
            })
            .unwrap_or_default()
            .to_owned();
        let (message, icon) = match target.device {
            clonk_frontend::startup_options_controls::ControlDevice::Keyboard => (
                clonk_app_menus::substitute_resource_arguments(
                    &self.runtime_resource_text(
                        "IDS_MSG_PRESSKEY",
                        "Press the key for \"%s\" on keyboard block %d.",
                    ),
                    &[&control, &(target.set + 1).to_string()],
                ),
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(24),
            ),
            clonk_frontend::startup_options_controls::ControlDevice::Gamepad => (
                clonk_app_menus::substitute_resource_arguments(
                    &self.runtime_resource_text(
                        "IDS_MSG_PRESSBTN",
                        "Press the button for \"%s\" on gamepad %d.",
                    ),
                    &[&control, &(target.set + 1).to_string()],
                ),
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(25),
            ),
        };
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                message,
                self.runtime_resource_text("IDS_MSG_DEFINEKEY", "Assign key"),
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                icon,
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::OptionsControlCapture(target),
        )
    }

    /// The port-only push-to-talk capture modal (clonk-org/clonk-rs#452). It is
    /// the classic `IDS_MSG_DEFINEKEY` dialog with a port-only prompt, so a
    /// player rebinding voice sees the same flow as rebinding a crew control.
    fn open_options_voice_capture(&mut self) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::new(
                self.runtime_resource_text(
                    "IDS_MSG_PRESSVOICEKEY",
                    "Press the key to hold down while speaking.",
                ),
                self.runtime_resource_text("IDS_MSG_DEFINEKEY", "Assign key"),
                clonk_frontend::message_dialog::MessageDialogButtons::CANCEL,
                clonk_frontend::message_dialog::MessageDialogIcon::Standard(24),
                clonk_frontend::message_dialog::MessageDialogSize::Regular,
                false,
            ),
            MessageDialogContinuation::OptionsVoicePushToTalkCapture,
        )
    }

    /// The key that modal captures. It mirrors
    /// [`Self::handle_options_control_capture_key`]: modified chords are
    /// rejected, the press is consumed either way, and only a key the config
    /// can round-trip is accepted.
    pub(crate) fn handle_options_voice_capture_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        let capturing = self.message_dialogs.last().is_some_and(|pending| {
            matches!(
                pending.continuation,
                MessageDialogContinuation::OptionsVoicePushToTalkCapture
            )
        });
        if !capturing {
            return Ok(false);
        }
        match state {
            ElementState::Released => {
                self.message_dialog_consumed_keys.remove(&key);
                return Ok(true);
            }
            ElementState::Pressed => {
                self.message_dialog_consumed_keys.insert(key);
            }
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if !c4_modifiers.is_empty() {
            return Ok(true);
        }
        if crate::input::encode_virtual_key_code(key).is_none() {
            tracing::warn!(?key, "ignoring voice capture for an unpersistable key");
            return Ok(true);
        }
        if let Some(audio) = self.audio.as_mut() {
            audio.options.voice_push_to_talk = key;
        }
        let label = format_key_label(key);
        if let Some(dialog) = self.startup_options_dialog.as_mut() {
            dialog.set_voice_push_to_talk_label(label);
        }
        self.finish_message_dialog(clonk_frontend::message_dialog::MessageDialogResult::Ok)?;
        Ok(true)
    }

    fn reset_options_control_bindings(
        &mut self,
        device: clonk_frontend::startup_options_controls::ControlDevice,
    ) {
        match device {
            clonk_frontend::startup_options_controls::ControlDevice::Keyboard => {
                self.bindings.reset_all();
            }
            clonk_frontend::startup_options_controls::ControlDevice::Gamepad => {
                self.gamepad_bindings.reset_all();
                self.gamepads
                    .set_axis_calibrations(self.gamepad_bindings.axis_calibrations());
            }
        }
        self.refresh_options_control_labels(device);
    }

    fn refresh_options_control_labels(
        &mut self,
        device: clonk_frontend::startup_options_controls::ControlDevice,
    ) {
        use clonk_frontend::startup_options_controls::{
            ControlCaptureTarget, CONTROL_KEY_COUNT, CONTROL_SET_COUNT,
        };
        let Some(dialog) = self.startup_options_dialog.as_mut() else {
            return;
        };
        for set in 0..CONTROL_SET_COUNT {
            for control in 0..CONTROL_KEY_COUNT {
                let Some(id) = ControlBindingId::ALL.get(control).copied() else {
                    continue;
                };
                let label = match device {
                    clonk_frontend::startup_options_controls::ControlDevice::Keyboard => self
                        .bindings
                        .key_for_set(set, id)
                        .map(format_key_label)
                        .unwrap_or_default(),
                    clonk_frontend::startup_options_controls::ControlDevice::Gamepad => {
                        self.gamepad_bindings.key_label_for_set(set, id)
                    }
                };
                dialog.controls_mut().set_label(
                    ControlCaptureTarget {
                        device,
                        set,
                        control,
                    },
                    label,
                );
            }
        }
    }

    fn show_options_alternate_server_notice(&mut self) -> Result<(), EngineError> {
        self.push_message_dialog(
            clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                "Games using an alternate server are not part of the official league.",
                "Master server",
                clonk_frontend::message_dialog::MessageDialogIcon::NOTIFY,
            )
            .with_us_dont_show_again(false),
            MessageDialogContinuation::OptionsAlternateServerNotice,
        )
    }

    fn open_options_network_input(
        &mut self,
        field: clonk_frontend::startup_options_network::NetworkTextField,
    ) -> Result<(), EngineError> {
        use clonk_frontend::startup_options_network::NetworkTextField;
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Options network input",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        let Some(network) = self
            .startup_options_dialog
            .as_ref()
            .map(|dialog| dialog.network())
        else {
            return Ok(());
        };
        let (message, caption, initial, max_text) = match field {
            NetworkTextField::Port(id) => (
                "Enter network port:",
                "Network port",
                network.port(id).port.to_string(),
                6,
            ),
            NetworkTextField::AlternateServerAddress => (
                "Enter alternate server address:",
                "Master server",
                network.alternate_server_address.clone(),
                256,
            ),
            NetworkTextField::LocalName => (
                "Enter computer name:",
                "Computer name",
                network.local_name.clone(),
                26,
            ),
            NetworkTextField::Nick => ("Enter user name:", "User name", network.nick.clone(), 26),
        };
        let controller = InputDialogController::new(message, caption, InputDialogIcon::None)
            .with_max_text(max_text)
            .with_input_text(&initial);
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::OptionsNetwork(field),
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        Ok(())
    }

    fn open_options_graphics_scale_input(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Options graphics scale input",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        let Some(scale) = self
            .startup_options_dialog
            .as_ref()
            .map(|dialog| dialog.graphics().proposed_scale_percent)
        else {
            return Ok(());
        };
        let controller = InputDialogController::new(
            "Enter display scale (100-300):",
            "Graphics scale",
            InputDialogIcon::None,
        )
        .with_max_text(4)
        .with_input_text(&scale.to_string());
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::OptionsGraphicsScale,
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        Ok(())
    }

    pub(crate) fn open_options_menu(&mut self) {
        self.close_context_menu_silently();
        self.startup_options_advanced_dialog = None;
        // Recreating the dialog destroys its ControlConfigArea before the
        // replacement starts on the Program sheet.
        self.gamepads.set_options_open_slot(None);
        let mut dialog = clonk_frontend::startup_options_dlg::OptionsDlgState::with_all(
            load_options_program_state(
                self.app_paths.as_ref(),
                Some(&self.startup_tooltip_resources),
            ),
            load_options_sound_state(self.audio.as_ref()),
            load_options_graphics_state(self.app_paths.as_ref()),
            load_options_control_state(
                &self.bindings,
                &self.gamepad_bindings,
                self.gamepads.connected_count(),
                self.gamepad_gui_control,
            ),
            load_options_network_state(self.app_paths.as_ref()),
        );
        dialog.set_labels(self.localized_options_labels());
        if let (Some(fonts), Some(book)) = (
            self.assets.clonk_fonts.as_deref(),
            self.assets.options_book_fonts.as_deref(),
        ) {
            dialog.resize(
                self.graphics.surface().width() as i32,
                self.graphics.surface().height() as i32,
                fonts,
                book,
            );
        }
        self.startup_options_dialog = Some(dialog);
        self.replace_startup_dialog(StartupView::Options, StartupDialog::Options);
        self.status_text.clear();
    }

    /// Projects the active language table onto `C4StartupOptionsDlg`'s visible
    /// strings. C++ resolves each key through `LoadResStr` while constructing
    /// the dialog, so the port resolves them at the same moment
    /// (C4StartupOptionsDlg.cpp:609-1033).
    pub(crate) fn localized_options_labels(
        &self,
    ) -> clonk_frontend::startup_options_dlg::OptionsLabels {
        let text = |key: &str, fallback: &str| self.runtime_resource_text(key, fallback);
        clonk_frontend::startup_options_dlg::OptionsLabels {
            // The caption strips its mnemonic marker like every FullscreenDialog title.
            title: text("IDS_DLG_OPTIONS", "&Options").replace('&', ""),
            // The twelve IDS_CTL_* action labels in C++'s own order
            // (C4StartupOptionsDlg.cpp:166-169), each falling back to the
            // shipped US text.
            control_keys: std::array::from_fn(|control| {
                text(
                    clonk_frontend::startup_options_controls::CONTROL_KEY_LABEL_KEYS[control],
                    clonk_frontend::startup_options_controls::CONTROL_KEY_LABELS[control],
                )
            }),
            sheets: [
                text("IDS_DLG_PROGRAM", "Program"),
                text("IDS_DLG_GRAPHICS", "Graphics"),
                // Port-only id. C++ captions this sheet `IDS_DLG_SOUND`, which
                // the ingame menu still uses for its own Sound entry; the port
                // also hosts the voice-chat group here, so the caption reads
                // "Audio" (clonk-org/clonk-rs#452).
                text("IDS_DLG_AUDIO", "Audio"),
                text("IDS_DLG_KEYBOARD", "Keyboard"),
                text("IDS_DLG_GAMEPAD", "Gamepad"),
                text("IDS_DLG_NETWORK", "Network"),
            ],
            back: text("IDS_BTN_BACK", "Back"),
            language: text("IDS_CTL_LANGUAGE", "Language"),
            font: text("IDS_CTL_FONT", "Font"),
            white_chat: text("IDS_MNU_WHITECHAT", "White Chat"),
            white_chat_ingame: text("IDS_CTL_WHITECHAT_INGAME", "Ingame"),
            white_chat_lobby: text("IDS_CTL_WHITECHAT_LOBBY", "Lobby"),
            timestamps: text("IDS_CTL_TIMESTAMPS", "Timestamps"),
            preloading: text("IDS_CTL_PRELOADING", "Preload game data"),
            reset_config: text("IDS_BTN_RESETCONFIG", "Reset configuration"),
            advanced_settings: text("IDS_DLG_ADVANCED_SETTINGS", "Advanced settings"),
            fair_crew_strength: text("IDS_CTL_FAIRCREWSTRENGTH", "Strength of \"Fair Crew\""),
            fair_crew_weak: text("IDS_CTL_FAIRCREWWEAK", "weak"),
            fair_crew_strong: text("IDS_CTL_FAIRCREWSTRONG", "strong"),
            no_language_info: text("IDS_CTL_NOLANGINFO", "Language pack not available."),
            music: text("IDS_CTL_MUSIC", "Music"),
            sound_effects: text("IDS_CTL_SOUNDFX", "Sound effects"),
            // Port-only ids shipped in `planet/System.c4g`; C++ has no
            // voice-chat strings at all (clonk-org/clonk-rs#452).
            voice_chat: text("IDS_CTL_VOICECHAT", "Voice chat"),
            voice_enabled: text("IDS_CTL_VOICEENABLED", "Enable voice chat"),
            voice_activated: text("IDS_CTL_VOICEACTIVATED", "Voice activated"),
            voice_volume: text("IDS_CTL_VOICEVOLUME", "Voice volume"),
            voice_push_to_talk: text("IDS_CTL_VOICEPUSHTOTALK", "Push to talk"),
            display_mode: text("IDS_CTL_DISPLAYMODE", "Display mode"),
            graphics_scale: text("IDS_CTL_GRAPHICSSCALE", "Scale"),
            effects_low: text("IDS_CTL_SMOKELOW", "Low"),
            effects_high: text("IDS_CTL_SMOKEHI", "High"),
            apply_scale: text("IDS_BTN_TESTGRAPHICSSCALE", "Apply"),
            reset_keyboard: text("IDS_BTN_RESETKEYBOARD", "Reset all"),
            gamepad_gui_control: text("IDS_CTL_GAMEPADFORMENU", "Use gamepad for menu control."),
            port_tcp: text("IDS_NET_PORT_TCP", "TCP port"),
            port_udp: text("IDS_NET_PORT_UDP", "UDP port"),
            port_reference: text("IDS_NET_PORT_REFERENCE", "Reference port"),
            port_discovery: text("IDS_NET_PORT_DISCOVERY", "Discovery port"),
            active: text("IDS_CTL_ACTIVE", "Active"),
            use_other_server: text("IDS_CTL_USEOTHERSERVER", "Use alternate internet server"),
            automatic_updates: text("IDS_CTL_AUTOMATICUPDATES", "Enable automatic updates"),
            upnp: text("IDS_CTL_UPNP", "Use UPnP"),
            computer_name: text("IDS_NET_COMPUTERNAME", "Computer name:"),
            chat_name: text("IDS_NET_USERNAME", "Chat name:"),
        }
    }

    pub(crate) fn persist_open_options_config(&self) -> Option<io::Result<()>> {
        let paths = self.app_paths.as_ref()?;
        let dialog = self.startup_options_dialog.as_ref()?;
        Some(persist_startup_options_config(
            paths,
            dialog.program(),
            self.audio.as_ref().map(|audio| &audio.options),
            dialog.graphics(),
            dialog.network(),
            &self.bindings,
            &self.gamepad_bindings,
            self.gamepad_gui_control,
        ))
    }

    pub(crate) fn apply_open_options_config(&self, config: &mut Config) -> Option<()> {
        let dialog = self.startup_options_dialog.as_ref()?;
        apply_startup_options_config(
            config,
            dialog.program(),
            self.audio.as_ref().map(|audio| &audio.options),
            dialog.graphics(),
            dialog.network(),
            &self.bindings,
            &self.gamepad_bindings,
            self.gamepad_gui_control,
        );
        Some(())
    }

    fn close_options_menu(&mut self) -> Result<(), EngineError> {
        let save_result = self.persist_open_options_config();
        // `C4StartupOptionsDlg::SaveConfig` ends with an outright
        // `Config.Save()` — "make sure config is saved, in case the game
        // crashes later on" (C4StartupOptionsDlg.cpp:1188-1189). Leaving the
        // dialog is therefore an explicit save surface, so anything held for
        // the shutdown flush is written here too.
        self.flush_deferred_config();
        self.close_options_menu_with_persist_result(save_result)
    }

    /// Writes `Config.General.MissionAccess` as soon as the shared list the
    /// engine mutates differs from the file.
    ///
    /// `FnGainMissionAccess` only grows that in-memory list
    /// (C4Script.cpp:2466-2471), and C++ leaves the write to `Config.Save()` on
    /// a clean quit (C4Application.cpp:367). A mission the player has already
    /// unlocked is earned progress rather than a runtime toggle, though, so a
    /// round that ends any other way must not relock it — the deliberate
    /// divergence behind clonk-org/clonk-rs#50. Runtime toggles keep C++'s
    /// timing in `DeferredConfig`.
    pub(crate) fn persist_mission_access_if_changed(&mut self) {
        if self.mission_access.matches(&self.persisted_mission_access) {
            return;
        }
        let access = self.mission_access.snapshot();
        // Retrying a failed write every frame would only repeat the warning,
        // so the list counts as persisted either way.
        self.persisted_mission_access = access.clone();
        let Some(paths) = self.app_paths.clone() else {
            return;
        };
        if let Err(error) = persist_mission_access(&paths, &access) {
            tracing::warn!(%error, "could not save General.MissionAccess");
        }
    }

    /// Writes every pending runtime config change now. Used by the explicit
    /// save surfaces; the clean-shutdown path flushes the same store.
    pub(crate) fn flush_deferred_config(&mut self) {
        let Some(paths) = self.app_paths.clone() else {
            return;
        };
        for (section, entries) in self.deferred_config.take_by_section() {
            let updates: Vec<(&str, clonk_app_netplay::NativeConfigValue<'_>)> = entries
                .iter()
                .map(|(key, value)| (key.as_str(), value.as_native()))
                .collect();
            if let Err(error) = persist_native_config_values(&paths, &section, &updates) {
                tracing::warn!(%error, section, "could not save deferred config values");
            }
        }
    }

    pub(crate) fn close_options_menu_with_persist_result(
        &mut self,
        save_result: Option<io::Result<()>>,
    ) -> Result<(), EngineError> {
        let feedback_result = if let Some(Err(error)) = save_result {
            tracing::warn!(error = %error, "failed to save options dialog settings");
            let error = error.to_string();
            let message = format_resource_string(
                self.runtime_resource_text("IDS_ERR_CONFSAVE", "Could not save configuration: %s"),
                &[&error],
            );
            let caption = self.runtime_resource_text("IDS_ERR_CONFIG", "Configuration error");
            let dialog = clonk_frontend::message_dialog::MessageDialogState::regular_ok(
                message,
                caption,
                clonk_frontend::message_dialog::MessageDialogIcon::ERROR,
            );
            self.push_message_dialog(dialog, MessageDialogContinuation::None)
        } else {
            Ok(())
        };
        self.begin_startup_dialog_fade(StartupDialog::MainMenu);
        self.show_main_menu();
        feedback_result
    }

    pub(crate) fn queue_options_display_request(&mut self, request: OptionsDisplayRequest) {
        self.pending_options_display_requests.push_back(request);
    }

    pub(crate) fn tick_options_scale_test_prompt(&mut self) -> bool {
        let Some(prompt_index) = self.message_dialogs.iter().position(|dialog| {
            matches!(
                dialog.continuation,
                MessageDialogContinuation::OptionsScaleTest { .. }
            )
        }) else {
            return false;
        };
        // The template is resolved before the borrow so the countdown refresh
        // uses the same active-language text the dialog opened with.
        let scale_test_template = self.runtime_resource_text(
            "IDS_MNU_SWITCHRESOLUTION_TEXT",
            "This is your new resolution. Do you like it?|Original resolution will be \
             restored in %u seconds...",
        );
        let expires = self
            .message_dialogs
            .get_mut(prompt_index)
            .is_some_and(|dialog| {
                let MessageDialogContinuation::OptionsScaleTest {
                    remaining_seconds, ..
                } = &mut dialog.continuation
                else {
                    return false;
                };
                if *remaining_seconds <= 1 {
                    return true;
                }
                *remaining_seconds -= 1;
                dialog
                    .state
                    .set_message(clonk_app_menus::substitute_resource_arguments(
                        &scale_test_template,
                        &[&remaining_seconds.to_string()],
                    ));
                false
            });
        if expires {
            if prompt_index + 1 == self.message_dialogs.len() {
                if let Err(error) = self.finish_message_dialog(
                    clonk_frontend::message_dialog::MessageDialogResult::Dismissed,
                ) {
                    tracing::error!(%error, "failed to expire graphics scale test");
                }
            } else if let Some((
                PendingMessageDialog {
                    continuation: MessageDialogContinuation::OptionsScaleTest { old_percent, .. },
                    ..
                },
                _,
            )) = self.remove_message_dialog_at(prompt_index)
            {
                if let Some(dialog) = self.startup_options_dialog.as_mut() {
                    dialog.graphics_mut().revert_scale_test();
                }
                self.queue_options_display_request(OptionsDisplayRequest::SetScale {
                    percent: old_percent,
                    persist: false,
                });
            }
        }
        true
    }

    pub(crate) fn finish_game_option_input(
        &mut self,
        actions: Vec<GameOptionAction>,
    ) -> Result<(), EngineError> {
        if self.scenario_game_options.context().is_lobby() {
            for action in actions {
                self.process_lobby_game_option_action(action)?;
            }
            let sounds = self.scenario_game_options.take_sound_events();
            self.play_game_option_sound_events(sounds);
            Ok(())
        } else {
            let sounds = self.scenario_game_options.take_sound_events();
            self.play_game_option_sound_events(sounds);
            self.process_game_option_actions(actions)
        }
    }

    pub(crate) fn persist_game_option_value(&mut self, section: &str, key: &str, value: String) {
        // Saving now makes the file authoritative again, so an older deferred
        // change to the same key must not keep shadowing it.
        self.deferred_config.clear(section, key);
        let Some(paths) = self.app_paths.as_ref() else {
            return;
        };
        if let Err(error) = persist_config_value(paths, section, key, value) {
            tracing::error!(%error, section, key, "failed to persist game option");
            self.status_text = format!("Unable to save game option: {error}");
        }
    }

    /// Saves a complete config while carrying the five in-game Display values
    /// that are still held in the process-local state. C++ mutates its global
    /// `Config` before every complete save, so `Config.Save()` at the
    /// masterserver redirect site (`C4StartupNetDlg.cpp:312-315`) includes
    /// those values even though the Display menu itself waits for shutdown.
    /// Keep the pending entries until the complete write succeeds; a failed
    /// save must remain recoverable by the ordinary shutdown flush.
    pub(crate) fn persist_config_value_with_display(
        &mut self,
        section: &str,
        key: &str,
        value: impl Into<String>,
    ) -> io::Result<()> {
        let Some(paths) = self.app_paths.clone() else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "application paths are unavailable",
            ));
        };
        let path = paths.config_file();
        let mut config = match Config::load(&path) {
            Ok(config) => config,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Config::new(),
            Err(error) => return Err(error),
        };
        self.apply_display_flags_to_config(&mut config);
        config.set_in(Some(section), key, value);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        save_config_preserving_native_general_booleans(&config, &path, None, None)?;
        self.clear_deferred_display_toggles();
        Ok(())
    }

    /// Keep the in-game Display values in the process-local config until the
    /// next C++ save site. `C4MainMenu::MenuCommand` mutates these fields in
    /// memory (`C4MainMenu.cpp:855-882`), while `C4Application::Quit` writes
    /// the complete config only on a clean shutdown (`C4Application.cpp:351-367`).
    /// The other Display rows are intentionally left to their own save-site
    /// audits; changing one row must not eagerly persist or rewrite them.
    pub(crate) fn defer_display_toggle(&mut self, toggle: DisplayToggle) {
        let (section, key, value) = match toggle {
            DisplayToggle::PlayerNames => (
                "Graphics",
                "ShowCrewNames",
                self.display_flags.player_names.to_string(),
            ),
            DisplayToggle::ClonkNames => (
                "Graphics",
                "ShowCrewCNames",
                self.display_flags.clonk_names.to_string(),
            ),
            DisplayToggle::Clock => (
                "Graphics",
                "ShowClock",
                self.display_flags.clock.to_string(),
            ),
            DisplayToggle::Fps => ("General", "FPS", self.display_flags.fps.to_string()),
            DisplayToggle::UpperBoard => (
                "Graphics",
                "UpperBoard",
                match self.display_flags.upper_board {
                    UpperBoardMode::Hide => "Hide",
                    UpperBoardMode::Full => "Full",
                    UpperBoardMode::Small => "Small",
                    UpperBoardMode::Mini => "Mini",
                }
                .to_owned(),
            ),
            DisplayToggle::Portraits
            | DisplayToggle::ShowCommands
            | DisplayToggle::ShowCommandKeys
            | DisplayToggle::WhiteChat => return,
        };
        self.deferred_config.set(section, key, value);
    }

    pub(crate) fn apply_display_flags_to_config(&self, config: &mut Config) {
        config.set_in(
            Some("Graphics"),
            "ShowCrewNames",
            self.display_flags.player_names.to_string(),
        );
        config.set_in(
            Some("Graphics"),
            "ShowCrewCNames",
            self.display_flags.clonk_names.to_string(),
        );
        config.set_in(
            Some("Graphics"),
            "ShowClock",
            self.display_flags.clock.to_string(),
        );
        config.set_in(Some("General"), "FPS", self.display_flags.fps.to_string());
        config.set_in(
            Some("Graphics"),
            "UpperBoard",
            match self.display_flags.upper_board {
                UpperBoardMode::Hide => "Hide",
                UpperBoardMode::Full => "Full",
                UpperBoardMode::Small => "Small",
                UpperBoardMode::Mini => "Mini",
            },
        );
    }

    pub(crate) fn clear_deferred_display_toggles(&mut self) {
        for (section, key) in [
            ("Graphics", "ShowCrewNames"),
            ("Graphics", "ShowCrewCNames"),
            ("Graphics", "ShowClock"),
            ("General", "FPS"),
            ("Graphics", "UpperBoard"),
        ] {
            self.deferred_config.clear(section, key);
        }
    }

    pub(crate) fn process_game_option_actions(
        &mut self,
        actions: Vec<GameOptionAction>,
    ) -> Result<(), EngineError> {
        for action in actions {
            match action {
                GameOptionAction::FocusTraversalRequested { backwards } => {
                    self.advance_scensel_dialog_focus(backwards);
                }
                GameOptionAction::InternetSignupChanged { enabled, .. } => {
                    self.persist_game_option_value(
                        "Network",
                        "MasterServerSignUp",
                        i32::from(enabled).to_string(),
                    );
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
                    remember_for_next_round,
                    ..
                } => {
                    if let Some(password) = remember_for_next_round {
                        self.persist_game_option_value("Network", "LastPassword", password);
                    }
                }
                GameOptionAction::CommentChanged(comment) => {
                    self.persist_game_option_value("Network", "Comment", comment);
                    tracing::info!(
                        "{}",
                        clonk_frontend::game_option_buttons::COMMENT_CHANGED_LOG
                    );
                }
                GameOptionAction::FairCrewPreferenceChanged(enabled) => {
                    self.startup_view_flags.fair_crew = enabled;
                    self.persist_fair_crew_preference(enabled);
                }
                GameOptionAction::RecordPreferenceChanged(enabled) => {
                    self.startup_view_flags.record = enabled;
                    self.recording_enabled = enabled && self.recordings_dir.is_some();
                    self.persist_game_option_value(
                        "General",
                        "Record",
                        i32::from(enabled).to_string(),
                    );
                }
                GameOptionAction::SendLobbyFairCrewControl { .. } => {
                    tracing::error!(
                        "selector game-option controller emitted a lobby-only fair-crew action"
                    );
                }
            }
        }
        Ok(())
    }

    pub(crate) fn open_game_option_input_dialog(
        &mut self,
        request: GameOptionInputDialogRequest,
    ) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        Self::guard_gui_overlay_result(
            "Password/Comment input dialog",
            self.assets.input_dialog_resources().map(|_| ()),
        )?;
        self.close_context_menu_silently();
        self.scenario_game_options.cancel_interaction();
        let icon = match request.kind {
            GameOptionInputKind::Password => InputDialogIcon::LOCKED_FRONTAL,
            GameOptionInputKind::Comment => InputDialogIcon::COMMENT,
        };
        let controller = InputDialogController::new(request.message, request.caption, icon)
            .with_max_text(request.max_text)
            .with_input_text(&request.initial_text);
        self.startup_tooltip.pointer_left();
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::GameOption(request.kind),
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = None;
        self.game_option_input_last_click = None;
        Ok(())
    }

    pub(crate) fn game_option_input_layout(
        &self,
    ) -> Option<clonk_frontend::input_dialog::InputDialogLayout> {
        let dialog = self.game_option_input_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.graphics.surface();
        Some(
            dialog
                .controller
                .layout(surface.width() as i32, surface.height() as i32, &fonts.text),
        )
    }

    pub(crate) fn game_option_input_owns_running_pointer_event(&self) -> bool {
        self.running_chat_controller().is_none()
            || self.running_shared_gui_has_keyboard_focus()
            || self.game_option_input_pointer_capture.is_some()
            || self
                .running_pointer_position
                .zip(self.game_option_input_layout().as_ref())
                .is_some_and(|(point, layout)| Self::point_in_input_dialog_bounds(point, layout))
    }

    pub(crate) fn release_game_option_input_pointer_elements(&mut self) {
        let sounds = self
            .game_option_input_dialog
            .as_mut()
            .map(|dialog| {
                dialog.controller.release_pointer_elements();
                dialog.controller.take_sound_events()
            })
            .unwrap_or_default();
        self.game_option_input_pointer_capture = None;
        self.play_input_dialog_sound_events(sounds);
    }

    pub(crate) fn stop_game_option_input_pointer_drag_at_current_position(&mut self) {
        let point = self.running_pointer_position;
        let layout = self.game_option_input_layout();
        let fonts = self.assets.clonk_fonts.clone();
        if let Some(((point, layout), fonts)) = point.zip(layout).zip(fonts.as_deref()) {
            if let Some(dialog) = self.game_option_input_dialog.as_mut() {
                dialog
                    .controller
                    .stop_pointer_drag_at(point, &layout, &fonts.text);
            }
        }
    }

    pub(crate) fn handle_game_option_input_primary_pointer(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        let point = self.game_option_input_pointer_position;
        let layout = self.game_option_input_layout();
        let fonts = self.assets.clonk_fonts.clone();
        let clicked_edit = point.zip(layout.as_ref()).is_some_and(|(point, layout)| {
            let edit = layout.edit;
            point.x >= edit.x as f32
                && point.x < (edit.x + edit.w) as f32
                && point.y >= edit.y as f32
                && point.y < (edit.y + edit.h) as f32
        });
        let actions = point
            .zip(layout.as_ref())
            .zip(fonts.as_deref())
            .and_then(|((point, layout), fonts)| {
                self.game_option_input_dialog
                    .as_mut()
                    .map(|dialog| match button_state {
                        ElementState::Pressed => {
                            dialog
                                .controller
                                .handle_pointer_down(point, layout, &fonts.text)
                        }
                        ElementState::Released => {
                            dialog
                                .controller
                                .handle_pointer_up(point, layout, &fonts.text)
                        }
                    })
            })
            .unwrap_or_default();
        self.finish_game_option_input_dialog_actions(actions)?;
        if button_state == ElementState::Released
            && clicked_edit
            && self.game_option_input_dialog.is_some()
        {
            let now = Instant::now();
            let is_double = self
                .game_option_input_last_click
                .is_some_and(|last| now.duration_since(last) < Duration::from_millis(500));
            self.game_option_input_last_click = (!is_double).then_some(now);
            if is_double {
                let actions = point
                    .zip(layout.as_ref())
                    .zip(fonts.as_deref())
                    .and_then(|((point, layout), fonts)| {
                        self.game_option_input_dialog.as_mut().map(|dialog| {
                            dialog.controller.handle_pointer_double_click(
                                point,
                                layout,
                                &fonts.text,
                            )
                        })
                    })
                    .unwrap_or_default();
                self.finish_game_option_input_dialog_actions(actions)?;
            }
        }
        Ok(())
    }

    pub(crate) fn finish_game_option_input_dialog_actions(
        &mut self,
        actions: Vec<InputDialogAction>,
    ) -> Result<(), EngineError> {
        let sounds = self
            .game_option_input_dialog
            .as_mut()
            .map(|dialog| dialog.controller.take_sound_events())
            .unwrap_or_default();
        self.play_input_dialog_sound_events(sounds);
        self.process_game_option_input_dialog_actions(actions)
    }

    pub(crate) fn process_game_option_input_dialog_actions(
        &mut self,
        actions: Vec<InputDialogAction>,
    ) -> Result<(), EngineError> {
        for action in actions {
            match action {
                InputDialogAction::FocusChanged(_) | InputDialogAction::TextChanged(_) => {}
                InputDialogAction::SubmittedLine(text) => {
                    if self
                        .game_option_input_dialog
                        .as_ref()
                        .is_some_and(|pending| {
                            pending.purpose == PendingInputDialogPurpose::RunningChat
                        })
                    {
                        if self.running_chat.as_ref().is_some_and(|chat| {
                            matches!(&chat.kind, RunningChatKind::MessageBoardInput(_))
                        }) {
                            self.submit_running_chat_text(text)?;
                            break;
                        }
                        self.process_running_chat_text(&text);
                    } else {
                        tracing::error!(
                            "multiline continuation escaped a compact running-chat dialog"
                        );
                    }
                }
                InputDialogAction::ClipboardWrite(text) => {
                    if let Err(error) =
                        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))
                    {
                        tracing::warn!(%error, "failed to copy classic input-dialog text");
                    }
                }
                InputDialogAction::OpenContextMenu(request) => {
                    let entries = request
                        .items
                        .into_iter()
                        .map(|item| {
                            ContextMenuEntry::new(item.label)
                                .with_tooltip(item.tooltip)
                                .with_icon(ContextMenuIcon::None)
                                .with_action(AppContextMenuCommand::InputDialog(item.command))
                        })
                        .collect();
                    self.open_context_menu_at(entries, request.anchor)?;
                }
                InputDialogAction::Accepted(text) => {
                    let Some(pending) = self.game_option_input_dialog.take() else {
                        continue;
                    };
                    self.startup_tooltip.pointer_left();
                    self.close_context_menu_silently();
                    self.game_option_input_last_click = None;
                    match pending.purpose {
                        PendingInputDialogPurpose::RunningChat => {
                            self.submit_running_chat_text(text)?;
                        }
                        PendingInputDialogPurpose::NetworkJoinPassword => {
                            if text.is_empty() {
                                self.pending_network_join = None;
                                self.status_text.clear();
                                self.resume_startup_music_after_failed_open_game();
                                break;
                            }
                            let Some(password) =
                                clonk_engine::LegacyCString::from_bytes(text.into_bytes())
                            else {
                                self.pending_network_join = None;
                                self.status_text =
                                    "Network password contains an unsupported NUL byte".to_string();
                                self.resume_startup_music_after_failed_open_game();
                                break;
                            };
                            let Some(settings) = self.pending_network_join.as_mut() else {
                                self.status_text =
                                    "Network join settings are unavailable".to_string();
                                break;
                            };
                            settings.password = password;
                            self.launch_pending_network_join()?;
                        }
                        PendingInputDialogPurpose::GameOption(kind) => {
                            let actions = self.scenario_game_options.resolve_input_dialog(
                                kind,
                                GameOptionInputDialogResult::Submitted(text),
                            );
                            self.finish_game_option_input(actions)?;
                        }
                        PendingInputDialogPurpose::OptionsGraphicsScale => {
                            if let Ok(value) = text.trim().parse::<i32>() {
                                let test_action =
                                    self.startup_options_dialog.as_mut().and_then(|dialog| {
                                        let graphics = dialog.graphics_mut();
                                        let _ = graphics.set_scale_spinbox_value(value);
                                        graphics.request_scale_test()
                                    });
                                if let Some(action) = test_action {
                                    self.process_options_dialog_actions(vec![
                                        clonk_frontend::startup_options_dlg::OptionsDlgAction::Graphics(
                                            action,
                                        ),
                                    ])?;
                                }
                            }
                        }
                        PendingInputDialogPurpose::OptionsNetwork(field) => {
                            if let Some(dialog) = self.startup_options_dialog.as_mut() {
                                dialog.network_mut().set_text(field, text);
                            }
                        }
                        PendingInputDialogPurpose::ScenarioMissionAccess => {
                            self.apply_scenario_mission_access(&text)?;
                        }
                        PendingInputDialogPurpose::StartupCrew(action) => {
                            self.complete_startup_crew_input(action, text)?;
                        }
                    }
                    break;
                }
                InputDialogAction::Cancelled => {
                    let Some(pending) = self.game_option_input_dialog.take() else {
                        continue;
                    };
                    self.startup_tooltip.pointer_left();
                    self.close_context_menu_silently();
                    self.game_option_input_last_click = None;
                    match pending.purpose {
                        PendingInputDialogPurpose::RunningChat => {
                            self.close_running_chat()?;
                        }
                        PendingInputDialogPurpose::NetworkJoinPassword => {
                            self.pending_network_join = None;
                            self.status_text.clear();
                            self.resume_startup_music_after_failed_open_game();
                        }
                        PendingInputDialogPurpose::GameOption(kind) => {
                            let actions = self
                                .scenario_game_options
                                .resolve_input_dialog(kind, GameOptionInputDialogResult::Cancelled);
                            self.finish_game_option_input(actions)?;
                        }
                        PendingInputDialogPurpose::OptionsGraphicsScale
                        | PendingInputDialogPurpose::OptionsNetwork(_) => {}
                        PendingInputDialogPurpose::ScenarioMissionAccess => {}
                        PendingInputDialogPurpose::StartupCrew(_) => {}
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn game_option_input_activity(&self) -> (bool, bool) {
        let keyboard_active = self.context_menu.is_none()
            && !self.network_chart_elevated
            && (self.running_chat_active() || self.message_dialogs.is_empty());
        let mouse_active = self.context_menu.is_none()
            && (matches!(self.mode, AppMode::Running) || keyboard_active);
        (keyboard_active, mouse_active)
    }

    pub(crate) fn render_game_option_input_dialog(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if self.game_option_input_dialog.is_none() {
            return Ok(());
        }
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .input_dialog_resources()
            .with_context(|| "classic C4GUI::InputDialog resources are unavailable")?;
        let (keyboard_active, _) = self.game_option_input_activity();
        self.game_option_input_dialog
            .as_ref()
            .expect("checked above")
            .controller
            .render(
                self.graphics.surface_mut(),
                &resources,
                keyboard_active,
                gamma,
            )?;
        let ordered_native = self.graphics.surface().is_clonk_text_capture_active();
        if ordered_native {
            self.next_pending_native_overlay();
        }
        if ordered_native {
            self.render_ordered_context_menu(gamma)?;
            if self.context_menu.is_some() {
                self.next_pending_native_overlay();
            }
        } else {
            self.render_context_menu_panels(gamma)?;
        }
        Ok(())
    }

    pub(crate) fn render_game_option_input_dialog_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        let (_, mouse_active) = self.game_option_input_activity();
        let Some(dialog) = self.game_option_input_dialog.as_ref() else {
            return Ok(false);
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .input_dialog_resources()
            .with_context(|| "classic C4GUI::InputDialog resources are unavailable")?;
        let now = Instant::now();
        let tooltip_visible = dialog
            .controller
            .tooltip_state_at(
                now,
                &dialog.controller.layout(
                    self.graphics.surface().width() as i32,
                    self.graphics.surface().height() as i32,
                    &resources.fonts().text,
                ),
                mouse_active,
            )
            .is_some();
        dialog.controller.render_tooltip_at(
            self.graphics.surface_mut(),
            &resources,
            mouse_active,
            gamma,
            now,
        )?;
        Ok(tooltip_visible)
    }

    pub(crate) fn options_tooltip_target_at(&self, point: GuiPoint) -> Option<StartupTooltip> {
        let dialog = self.startup_options_dialog.as_ref()?;
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let book = self.assets.options_book_fonts.as_deref()?;
        let surface = self.graphics.surface();
        let layout = clonk_frontend::startup_options_dlg::options_dlg_layout(
            surface.width() as i32,
            surface.height() as i32,
            fonts,
            book,
        );
        let default_target = dialog.tooltip_at(point, book);
        if matches!(default_target, Some(StartupTooltip::Resource { .. })) {
            return default_target;
        }
        let in_tabular = point.x >= layout.tabular.x as f32
            && point.x < (layout.tabular.x + layout.tabular.w) as f32
            && point.y >= layout.tabular.y as f32
            && point.y < (layout.tabular.y + layout.tabular.h) as f32;
        if in_tabular {
            return None;
        }
        let title = self.startup_tooltip_resource_no_amp("IDS_DLG_OPTIONS");
        if let Some(tooltip) = clonk_frontend::centered_label_tooltip_at(
            point,
            layout.title_center,
            fonts.title.measure(&title, true),
            StartupTooltip::text(title),
        ) {
            return Some(tooltip);
        }
        None
    }
}
