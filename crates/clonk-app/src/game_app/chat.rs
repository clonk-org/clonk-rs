//! `impl GameApp` — chat, IRC & message board methods.
//!
//! Moved verbatim from the root `impl GameApp` block in `main.rs`
//! (step 6a of the decomposition campaign, see rust/REFACTOR_PLAN.md).
//! Structural only: same crate, same type, same method bodies.

use super::*;

impl GameApp {
    pub(crate) fn handle_external_irc_dialog_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !self.external_irc_dialog_visible
            || (matches!(self.mode, AppMode::Running)
                && !self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc))
        {
            return Ok(false);
        }
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if key == VirtualKeyCode::Escape && modifiers.is_empty() {
            if state == ElementState::Pressed {
                self.hide_external_irc_dialog();
            }
            return Ok(true);
        }
        if self.handle_network_edit_key(key, state)? {
            return Ok(true);
        }
        let actions = if state == ElementState::Pressed
            && key == VirtualKeyCode::F4
            && modifiers == ModifiersState::CONTROL
        {
            self.external_irc_dialog
                .as_mut()
                .map(clonk_frontend::startup_netdlg::NetDlgController::close_active_chat_sheet)
                .unwrap_or_default()
        } else if state == ElementState::Pressed
            && key == VirtualKeyCode::Tab
            && (modifiers == ModifiersState::CONTROL
                || modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT))
        {
            self.external_irc_dialog
                .as_mut()
                .map(|dialog| dialog.cycle_chat_sheet(modifiers.contains(ModifiersState::SHIFT)))
                .unwrap_or_default()
        } else if let Some(key) = map_key_code(key) {
            if key == KeyCode::Tab && !(modifiers.is_empty() || modifiers == ModifiersState::SHIFT)
            {
                Vec::new()
            } else {
                self.external_irc_dialog
                    .as_mut()
                    .map(|dialog| match state {
                        ElementState::Pressed => dialog.handle_key_down_with_tab_direction(
                            key,
                            modifiers == ModifiersState::SHIFT,
                        ),
                        ElementState::Released => dialog.handle_key_up(key),
                    })
                    .unwrap_or_default()
            }
        } else {
            Vec::new()
        };
        self.process_network_dialog_actions(actions)?;
        Ok(true)
    }

    pub(crate) fn handle_runtime_irc_toggle_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }
        let c4_modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if !self.runtime_keyboard_binding_matches(
            "ToggleChat",
            key,
            key == VirtualKeyCode::KeyC && c4_modifiers == ModifiersState::ALT,
        ) {
            return Ok(false);
        }
        if state == ElementState::Pressed {
            self.toggle_external_irc_dialog()?;
            return Ok(true);
        }
        // ToggleChat is a PRIO_Base C4KeyCB with no Up callback. Consume the
        // exact modified release after higher-priority GUI owners without
        // clearing a modifier-blind player control also bound to physical C.
        Ok(true)
    }

    /// Case-sensitive C4MessageInput::ProcessCommand routing. `Ok(true)`
    /// means the name was recognized even when a native gate rejected its
    /// action; only `Ok(false)` reaches IDS_ERR_UNKNOWNCMD.
    pub(crate) fn process_running_chat_command(&mut self, text: &str) -> Result<bool, EngineError> {
        let raw = clonk_script::c4_string_bytes(text);
        let Some(command) = raw.strip_prefix(b"/") else {
            return Ok(false);
        };
        let (name, parameter) = command
            .iter()
            .position(|byte| *byte == b' ')
            .map_or((command, &[][..]), |space| {
                (&command[..space], &command[space + 1..])
            });
        let name = &name[..name.len().min(30)];
        let network_host = matches!(self.runtime_network_role(), RuntimeNetworkRole::Host);
        let game_running = self.mode == AppMode::Running;

        match name {
            b"help" => self.append_running_command_help(),
            b"clear" => {
                if self.control_message_has_lobby() {
                    self.clear_lobby_log();
                } else {
                    self.clear_message_board_log();
                }
            }
            b"fast" => {
                if !game_running {
                    return Ok(false);
                }
                if self.network_is_league {
                    self.append_running_command_resource(
                        "IDS_LOG_COMMANDNOTALLOWEDINLEAGUE",
                        "Command not allowed in league games!",
                    );
                } else {
                    let parameter = native_bytes_as_legacy_text(parameter);
                    let frame_skip = legacy_atoi_i32(&parameter);
                    if frame_skip != 0 {
                        self.frame_skip = frame_skip.clamp(1, 500);
                        self.full_speed = true;
                    }
                }
            }
            b"slow" => {
                if !game_running {
                    return Ok(false);
                }
                self.full_speed = false;
                self.frame_skip = 1;
            }
            b"kick" => {
                if network_host {
                    let target = self
                        .control_clients
                        .snapshot()
                        .into_iter()
                        .find(|client| client.name.as_bytes() == parameter);
                    let Some(target) = target else {
                        let target_name = legacy_presentation_text(parameter);
                        let template = self
                            .runtime_resource_text("IDS_MSG_CMD_NOCLIENT", "Client %s not found!");
                        self.append_control_message_log(
                            format_resource_string(template, &[&target_name]),
                            CONTROL_LOG_COLOR,
                            None,
                        );
                        return Ok(true);
                    };
                    if self.network_is_league && self.runtime_client_has_players(target.client_id) {
                        self.submit_own_league_vote(
                            LeagueVoteSubject {
                                vote_type: clonk_engine::VOTE_TYPE_KICK,
                                data: target.client_id,
                            },
                            true,
                        );
                    } else {
                        let reason = self.runtime_resource_text(
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
                            tracing::error!(%error, "failed to submit running kick command");
                        }
                    }
                }
            }
            b"nodebug" => {
                if !game_running {
                    return Ok(false);
                }
                self.submit_or_execute_running_control_set(1, 0)?;
            }
            b"activate" | b"deactivate" | b"observer" => {
                if !network_host {
                    self.append_running_command_resource("IDS_MSG_CMD_HOSTONLY", "Host only!");
                    return Ok(true);
                }
                let target = self
                    .control_clients
                    .snapshot()
                    .into_iter()
                    .find(|client| client.name.as_bytes() == parameter);
                let Some(target) = target else {
                    let target_name = legacy_presentation_text(parameter);
                    let template =
                        self.runtime_resource_text("IDS_MSG_CMD_NOCLIENT", "Client %s not found!");
                    self.append_control_message_log(
                        format_resource_string(template, &[&target_name]),
                        CONTROL_LOG_COLOR,
                        None,
                    );
                    return Ok(true);
                };
                let update = match name {
                    b"activate" => Some((clonk_engine::CLIENT_UPDATE_ACTIVATE, 1)),
                    b"deactivate" if !self.network_is_league => {
                        Some((clonk_engine::CLIENT_UPDATE_ACTIVATE, 0))
                    }
                    b"observer" if !self.network_is_league => {
                        Some((clonk_engine::CLIENT_UPDATE_SET_OBSERVER, 0))
                    }
                    _ => None,
                };
                if let Some((update_type, data)) = update {
                    if let Some(Err(error)) = self.network.as_ref().map(|network| {
                        network.submit_client_update(clonk_engine::ClientUpdateControlData {
                            update_type,
                            client_id: target.client_id,
                            data,
                            by_client: 0,
                        })
                    }) {
                        tracing::error!(%error, "failed to submit running client update");
                    }
                } else {
                    self.append_running_command_resource(
                        "IDS_LOG_COMMANDNOTALLOWEDINLEAGUE",
                        "Command not allowed in league games!",
                    );
                }
            }
            b"centralctrl" | b"decentralctrl" | b"asyncctrl" => {
                if !network_host {
                    self.append_running_command_resource("IDS_MSG_CMD_HOSTONLY", "Host only!");
                    return Ok(true);
                }
                if self.network_is_league && name == b"asyncctrl" {
                    self.append_running_command_resource(
                        "IDS_LOG_COMMANDNOTALLOWEDINLEAGUE",
                        "Command not allowed in league games!",
                    );
                    return Ok(true);
                }
                self.change_running_network_control_mode(match name {
                    b"centralctrl" => 1,
                    b"decentralctrl" => 0,
                    _ => 2,
                });
            }
            b"set" => {
                if let Some(value) = parameter.strip_prefix(b"maxplayer ") {
                    // In a live network session, isCtrlHost is the initialized
                    // local network client's identity. Never let stale
                    // process-local engine state authorize a real client
                    // (src/C4GameControl.cpp:59-68;
                    // src/C4MessageInput.cpp:472-490).
                    if network_host || (self.network.is_none() && self.engine.is_control_host()) {
                        let maximum = legacy_sscanf_decimal_prefix(value).unwrap_or(0);
                        if maximum == 0 && value != b"0" {
                            self.append_control_message_log(
                                "Syntax: /set maxplayer count".to_string(),
                                CONTROL_LOG_COLOR,
                                None,
                            );
                        } else if !game_running {
                            if let Some(Err(error)) = self.network.as_ref().map(|network| {
                                network.submit_control_set(clonk_network::LegacyControlSet {
                                    value_type: 2,
                                    data: maximum,
                                    by_client: 0,
                                })
                            }) {
                                tracing::error!(
                                    %error,
                                    "failed to submit lobby maximum-player update"
                                );
                            }
                        } else {
                            self.submit_or_execute_running_control_set(2, maximum)?;
                        }
                    }
                } else if parameter == b"comment" || parameter.starts_with(b"comment ") {
                    if network_host {
                        let value = parameter.strip_prefix(b"comment ").unwrap_or_default();
                        self.set_running_network_comment(value);
                    }
                } else if parameter == b"password" || parameter.starts_with(b"password ") {
                    if network_host {
                        let value = parameter.strip_prefix(b"password ").unwrap_or_default();
                        self.set_running_network_password(value);
                    }
                } else if let Some(value) = parameter.strip_prefix(b"faircrew ") {
                    if self.engine.is_control_host() && !self.network_is_league {
                        let strength = if value == b"on" {
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
                        if let Some(strength) = strength {
                            self.submit_or_execute_running_control_set(5, strength)?;
                        }
                    }
                }
            }
            b"script" => {
                if !game_running {
                    return Ok(false);
                }
                if !self.engine.debug_mode() || (self.network.is_some() && !network_host) {
                    return Ok(true);
                }
                let Some(script) = clonk_engine::LegacyCString::from_bytes(parameter.to_vec())
                else {
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
            }
            b"chart" => {
                if !game_running {
                    return Ok(false);
                }
                self.toggle_network_chart();
            }
            // `netgetscen` copies the transferred scenario resource next to the
            // executable, and only for a non-host network client outside the
            // lobby - the lobby has its Resources tab for this
            // (src/C4MessageInput.cpp:527-545). Every other state, and every
            // failure along the way, returns false so the caller emits the
            // ordinary unknown-command error.
            b"netgetscen" => {
                if self.network.is_none() || network_host || self.control_message_has_lobby() {
                    return Ok(false);
                }
                let Some(destination) = self.save_joined_scenario_resource() else {
                    return Ok(false);
                };
                let message = format_resource_string(
                    self.runtime_resource_text(
                        "IDS_MSG_CMD_NETGETSCEN_SAVED",
                        "Got it! Saved to %s",
                    ),
                    &[&destination.to_string_lossy()],
                );
                self.append_control_message_log(message, CONTROL_LOG_COLOR, None);
            }
            _ => {
                if !game_running {
                    return Ok(false);
                }
                let registered = self
                    .engine
                    .message_board_commands()
                    .iter()
                    .any(|command| clonk_script::c4_string_bytes(&command.name) == name);
                if !registered {
                    return Ok(false);
                }
                let Some(command) = clonk_engine::LegacyCString::from_bytes(name.to_vec()) else {
                    return Ok(true);
                };
                let Some(argument) = clonk_engine::LegacyCString::from_bytes(parameter.to_vec())
                else {
                    return Ok(true);
                };
                let player = self
                    .snapshot
                    .hud
                    .local_players
                    .first()
                    .copied()
                    .unwrap_or(-1);
                self.submit_or_execute_running_custom_command(
                    clonk_engine::CustomCommandControlData {
                        command,
                        argument,
                        player,
                        by_client: 0,
                    },
                )?;
            }
        }
        Ok(true)
    }

    pub(crate) fn start_running_chat(&mut self, mode: RunningChatMode) {
        let text = match mode {
            RunningChatMode::All => String::new(),
            RunningChatMode::Allies => "/team ".to_string(),
            RunningChatMode::Say => "\"".to_string(),
        };
        let raw_label = self.runtime_resource_text("IDS_CTL_CHAT", "Cha&t:");
        let label = raw_label.replace('&', "");
        let tooltip = self.runtime_resource_text(
            "IDS_DLGTIP_CHAT",
            "Enter chat messages here and send them with enter.",
        );
        self.suspend_ingame_pointer_for_gui();
        self.network_chart_elevated = false;
        self.message_dialog_active_index = None;
        self.running_chat = Some(RunningChatState {
            history_index: -1,
            active: true,
            kind: RunningChatKind::Ordinary,
        });
        self.show_running_dialog(RunningDialogStackEntry::Chat);
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::RunningChat,
            controller: InputDialogController::new_chat(label, &text).with_chat_tooltip(tooltip),
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = self.running_pointer_position;
        self.game_option_input_last_click = None;
    }

    fn start_message_board_input(&mut self, input: clonk_engine::ActiveMessageBoardInput) {
        let prompt = c4_presentation_text(&input.prompt);
        let tooltip = self.runtime_resource_text(
            "IDS_DLGTIP_CHAT",
            "Enter chat messages here and send them with enter.",
        );
        let compact = !input.prompt.contains('|')
            && self.assets.clonk_fonts.as_deref().is_some_and(|fonts| {
                let screen_width = self.graphics.surface().width() as i32;
                fonts.text.measure(&prompt, true).0 < screen_width / 5
            });
        let controller = if compact {
            InputDialogController::new_chat(prompt, "").with_chat_tooltip(tooltip)
        } else {
            InputDialogController::new(prompt, "", InputDialogIcon::None)
                .with_placement(InputDialogPlacement::BottomThird)
        };
        self.suspend_ingame_pointer_for_gui();
        self.network_chart_elevated = false;
        self.message_dialog_active_index = None;
        self.running_chat = Some(RunningChatState {
            history_index: -1,
            active: true,
            kind: RunningChatKind::MessageBoardInput(input),
        });
        self.show_running_dialog(RunningDialogStackEntry::Chat);
        self.game_option_input_dialog = Some(PendingGameOptionInputDialog {
            purpose: PendingInputDialogPurpose::RunningChat,
            controller,
        });
        self.game_option_input_consumed_keys.clear();
        self.game_option_input_pointer_capture = None;
        self.game_option_input_pointer_position = self.running_pointer_position;
        self.game_option_input_last_click = None;
    }

    pub(crate) fn reconcile_message_board_input_dialog(&mut self) -> Result<(), EngineError> {
        let mut active = self.engine.active_message_board_input().cloned();
        let visible_query = self
            .running_chat
            .as_ref()
            .and_then(|chat| match &chat.kind {
                RunningChatKind::Ordinary => None,
                RunningChatKind::MessageBoardInput(input) => Some(input.clone()),
            });
        if let Some(visible_query) = visible_query {
            if active.as_ref() == Some(&visible_query) {
                return Ok(());
            }
            // The target/player may disappear or AbortMessageBoard may replace
            // the active projection. Never let the stale edit consume a newer
            // query when it next submits.
            self.finalize_running_chat_input()?;
            active = self.engine.active_message_board_input().cloned();
        }
        if self.running_chat.is_some()
            || self.game_option_input_dialog.is_some()
            || self.game_over_dialog.is_some()
            || self.top_message_dialog_is_exclusive()
            || self.external_irc_dialog_visible
            || self.context_menu.is_some()
            || self
                .runtime_client_list
                .as_ref()
                .is_some_and(|dialog| dialog.is_info_only())
        {
            return Ok(());
        }
        if let Some(active) = active {
            self.start_message_board_input(active);
        }
        Ok(())
    }

    pub(crate) fn running_chat_controller(&self) -> Option<&InputDialogController> {
        self.game_option_input_dialog.as_ref().and_then(|dialog| {
            (dialog.purpose == PendingInputDialogPurpose::RunningChat).then_some(&dialog.controller)
        })
    }

    pub(crate) fn running_chat_controller_mut(&mut self) -> Option<&mut InputDialogController> {
        self.game_option_input_dialog.as_mut().and_then(|dialog| {
            (dialog.purpose == PendingInputDialogPurpose::RunningChat)
                .then_some(&mut dialog.controller)
        })
    }

    fn running_chat_contains_point(&self, point: GuiPoint) -> bool {
        self.running_chat_controller().is_some()
            && self
                .game_option_input_layout()
                .as_ref()
                .is_some_and(|layout| Self::point_in_input_dialog_bounds(point, layout))
    }

    fn running_chat_contains_current_pointer(&self) -> bool {
        self.running_pointer_position
            .is_some_and(|point| self.running_chat_contains_point(point))
    }

    pub(crate) fn running_chat_text(&self) -> Option<&str> {
        self.running_chat_controller()
            .map(InputDialogController::text)
    }

    pub(crate) fn running_chat_active(&self) -> bool {
        self.running_chat.as_ref().is_some_and(|chat| chat.active)
            && (self.mode != AppMode::Running
                || self.running_active_dialog == Some(RunningDialogStackEntry::Chat))
    }

    pub(crate) fn running_chat_keyboard_active(&self) -> bool {
        self.running_chat_active() && self.running_shared_gui_has_keyboard_focus()
    }

    pub(crate) fn set_running_chat_active(&mut self, active: bool) {
        if let Some(chat) = self.running_chat.as_mut() {
            chat.active = active;
        }
        if active {
            self.message_dialog_active_index = None;
            self.activate_running_dialog(RunningDialogStackEntry::Chat);
        }
    }

    pub(crate) fn close_running_chat(&mut self) -> Result<(), EngineError> {
        let input = self
            .running_chat
            .as_ref()
            .and_then(|chat| match &chat.kind {
                RunningChatKind::Ordinary => None,
                RunningChatKind::MessageBoardInput(input) => Some(input.clone()),
            });
        let answer_result = match input {
            Some(input) => self.submit_message_board_answer(&input, ""),
            None => Ok(()),
        };
        let finalize_result = self.finalize_running_chat_input();
        answer_result?;
        finalize_result
    }

    fn finalize_running_chat_input(&mut self) -> Result<(), EngineError> {
        let was_active = self.running_chat_active();
        if was_active {
            self.release_message_dialog_pointer_elements();
            self.release_game_option_input_pointer_elements();
        }
        self.running_chat = None;
        self.remove_running_dialog(RunningDialogStackEntry::Chat);
        if self
            .game_option_input_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.purpose == PendingInputDialogPurpose::RunningChat)
        {
            self.game_option_input_dialog = None;
        }
        if self.message_dialogs.is_empty() {
            self.network_chart_elevated = false;
        }
        self.close_context_menu_silently();
        self.game_option_input_last_click = None;
        if was_active {
            self.message_dialog_active_index = if self.network_chart_elevated {
                None
            } else {
                match self.running_active_dialog {
                    Some(RunningDialogStackEntry::Message(stack_id)) => {
                        self.running_message_index(stack_id)
                    }
                    _ => None,
                }
            };
        }
        // Releases swallowed by the modal cannot clear the raw repeat/Tab
        // trackers. Forget them with the synchronized gameplay controls.
        self.pressed_engine_keys.clear();
        self.scoreboard_tab_raw_pressed = false;
        self.clear_local_controls()?;
        Ok(())
    }

    pub(crate) fn browse_running_chat_history(&mut self, older: bool) {
        let Some(chat) = self.running_chat.as_mut() else {
            return;
        };
        chat.history_index += if older { 1 } else { -1 };
        let text = usize::try_from(chat.history_index)
            .ok()
            .and_then(|index| self.message_input_history.get(index))
            .cloned();
        let text = match text {
            Some(text) => text,
            None => {
                chat.history_index = -1;
                String::new()
            }
        };
        let layout = self.game_option_input_layout();
        let fonts = self.assets.clonk_fonts.clone();
        if let Some(controller) = self.running_chat_controller_mut() {
            if text.is_empty() {
                // Selecting and deleting the current line does not invoke the
                // Edit cursor-scroll path, so C++ preserves the old offset.
                controller.set_input_text(&text);
            } else if let (Some(layout), Some(fonts)) = (layout.as_ref(), fonts.as_deref()) {
                controller.replace_edit_text(&text, layout, &fonts.text);
            } else {
                controller.set_input_text(&text);
            }
        }
    }

    fn complete_running_chat_nick(&mut self) {
        let Some(controller) = self.game_option_input_dialog.as_ref().and_then(|dialog| {
            (dialog.purpose == PendingInputDialogPurpose::RunningChat).then_some(&dialog.controller)
        }) else {
            return;
        };
        let text = controller.text().to_string();
        let caret = controller.caret();
        let before_cursor = &text[..caret];
        let start = before_cursor
            .char_indices()
            .rev()
            .find(|(_, character)| {
                character.is_ascii() && !character.is_ascii_alphanumeric() && *character != '_'
            })
            .map_or(0, |(index, character)| index + character.len_utf8());
        let incomplete = clonk_script::c4_string_bytes(&before_cursor[start..]);
        if incomplete.is_empty() {
            return;
        }
        let suffix = self.snapshot.players.iter().find_map(|player| {
            let name = clonk_script::c4_string_bytes(&player.name);
            name.get(..incomplete.len())
                .filter(|prefix| prefix.eq_ignore_ascii_case(&incomplete))
                .map(|_| clonk_script::c4_string_from_bytes(&name[incomplete.len()..]))
        });
        if let Some(suffix) = suffix {
            let Some(layout) = self.game_option_input_layout() else {
                return;
            };
            let Some(fonts) = self.assets.clonk_fonts.clone() else {
                return;
            };
            if let Some(controller) = self.running_chat_controller_mut() {
                controller.handle_text_input(&suffix, &layout, &fonts.text);
            }
        }
    }

    pub(crate) fn process_running_chat_text(&mut self, text: &str) {
        self.process_message_input_text(text, true);
    }

    fn submit_message_board_answer(
        &mut self,
        expected: &clonk_engine::ActiveMessageBoardInput,
        text: &str,
    ) -> Result<(), EngineError> {
        if self.engine.active_message_board_input() != Some(expected) {
            return Ok(());
        }
        let answer =
            LegacyCString::from_bytes(clonk_script::c4_string_bytes(text)).unwrap_or_else(|| {
                tracing::warn!("message-board input contained an embedded NUL; cancelling query");
                LegacyCString::default()
            });
        let by_client = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .or_else(|| {
                self.engine
                    .player(expected.player)
                    .map(|player| player.at_client().get())
            })
            .unwrap_or_else(|| self.offline_local_client_id());
        let Some(control) = self
            .engine
            .prepare_message_board_answer_control(answer, by_client)
        else {
            return Ok(());
        };
        if let Some(network) = self.network.as_ref() {
            let tick = self.local_control_submission_tick();
            if let Err(error) = network.submit_message_board_answer(tick, control) {
                tracing::error!(%error, "failed to submit message-board answer");
            }
        } else {
            self.record_control_batch(std::slice::from_ref(
                &clonk_engine::ControlPacket::MessageBoardAnswer(control.clone()),
            ));
            let _ = self.engine.execute_message_board_answer_control(&control)?;
        }
        Ok(())
    }

    pub(crate) fn submit_running_chat_text(&mut self, text: String) -> Result<(), EngineError> {
        let kind = self
            .running_chat
            .as_ref()
            .map(|chat| chat.kind.clone())
            .unwrap_or(RunningChatKind::Ordinary);
        match kind {
            RunningChatKind::Ordinary => {
                self.finalize_running_chat_input()?;
                self.process_running_chat_text(&text);
                Ok(())
            }
            RunningChatKind::MessageBoardInput(input) => {
                self.store_message_input_history(&text);
                let answer_result = self.submit_message_board_answer(&input, &text);
                let finalize_result = self.finalize_running_chat_input();
                answer_result?;
                finalize_result
            }
        }
    }

    fn submit_running_chat(&mut self) -> Result<(), EngineError> {
        let Some(text) = self.running_chat_text().map(str::to_string) else {
            return Ok(());
        };
        self.submit_running_chat_text(text)
    }

    pub(crate) fn runtime_running_chat_open_mode(
        &self,
        key: VirtualKeyCode,
    ) -> Option<RunningChatMode> {
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        [
            (
                "ChatOpen",
                RunningChatMode::All,
                modifiers.is_empty() && matches!(key, VirtualKeyCode::Enter | VirtualKeyCode::F2),
            ),
            (
                "ChatOpen2Allies",
                RunningChatMode::Allies,
                key == VirtualKeyCode::Enter && modifiers == ModifiersState::SHIFT,
            ),
            (
                "ChatOpen2Say",
                RunningChatMode::Say,
                key == VirtualKeyCode::Enter && modifiers == ModifiersState::ALT,
            ),
        ]
        .into_iter()
        .find_map(|(name, mode, default_matches)| {
            self.runtime_keyboard_binding_matches(name, key, default_matches)
                .then_some(mode)
        })
    }

    pub(crate) fn running_chat_key_has_higher_priority_route(&self, key: VirtualKeyCode) -> bool {
        let Some(controller) = self.running_chat_controller() else {
            return false;
        };
        let modifiers = self.keyboard_modifiers
            & (ModifiersState::ALT | ModifiersState::CONTROL | ModifiersState::SHIFT);
        if modifiers.contains(ModifiersState::ALT) {
            return false;
        }
        if modifiers == ModifiersState::CONTROL {
            return true;
        }
        let edit_focused = controller.focused_control() == InputDialogControl::Edit;
        let edit_key = matches!(
            key,
            VirtualKeyCode::Backspace
                | VirtualKeyCode::Delete
                | VirtualKeyCode::ArrowLeft
                | VirtualKeyCode::ArrowRight
                | VirtualKeyCode::Home
                | VirtualKeyCode::End
        );
        if modifiers == (ModifiersState::CONTROL | ModifiersState::SHIFT) {
            return edit_focused && edit_key;
        }
        if modifiers == ModifiersState::SHIFT {
            return key == VirtualKeyCode::Tab || (edit_focused && edit_key);
        }
        if !modifiers.is_empty() {
            return false;
        }
        if edit_key {
            return edit_focused
                || (key == VirtualKeyCode::Backspace && controller.text().is_empty());
        }
        if key == VirtualKeyCode::Space {
            return !edit_focused;
        }
        if key == VirtualKeyCode::ContextMenu {
            return edit_focused;
        }
        matches!(
            key,
            VirtualKeyCode::Escape
                | VirtualKeyCode::F2
                | VirtualKeyCode::Enter
                | VirtualKeyCode::NumpadEnter
                | VirtualKeyCode::Tab
                | VirtualKeyCode::ArrowUp
                | VirtualKeyCode::ArrowDown
        )
    }

    pub(crate) fn handle_game_over_enter_chat(&mut self, state: ElementState) {
        if state == ElementState::Pressed && !self.running_chat_active() {
            self.start_running_chat(RunningChatMode::All);
        }
    }

    pub(crate) fn handle_running_chat_open_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        if !matches!(self.mode, AppMode::Running) || self.running_chat_active() {
            return false;
        }
        let mode = self.runtime_running_chat_open_mode(key);
        if mode.is_some() && self.local_player_key_binding_in_scope(key) {
            return false;
        }
        if let Some(mode) = mode {
            if state == ElementState::Pressed {
                if self.running_chat.is_some() {
                    if self
                        .running_chat_text()
                        .is_some_and(|text| !text.is_empty())
                    {
                        return true;
                    }
                    if let Err(error) = self.close_running_chat() {
                        tracing::error!(%error, "failed to replace inactive empty running chat");
                        return true;
                    }
                }
                self.start_running_chat(mode);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn handle_running_chat_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }
        if !self.running_chat_active() {
            let chat_open = self.handle_running_chat_open_key(key, state);
            return Ok(chat_open);
        }
        if !self.running_shared_gui_has_keyboard_focus() {
            return Ok(false);
        }
        if self.keyboard_modifiers.alt_key() {
            return Ok(false);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        match key {
            VirtualKeyCode::Escape | VirtualKeyCode::F2 => self.close_running_chat()?,
            VirtualKeyCode::Enter | VirtualKeyCode::NumpadEnter => self.submit_running_chat()?,
            VirtualKeyCode::Tab => self.complete_running_chat_nick(),
            VirtualKeyCode::ArrowUp => self.browse_running_chat_history(true),
            VirtualKeyCode::ArrowDown => self.browse_running_chat_history(false),
            VirtualKeyCode::Backspace => {
                let empty = self.running_chat_text().is_none_or(str::is_empty);
                if empty {
                    self.close_running_chat()?;
                } else if let (Some(layout), Some(fonts)) = (
                    self.game_option_input_layout(),
                    self.assets.clonk_fonts.clone(),
                ) {
                    let modifiers = InputDialogKeyModifiers {
                        shift: self.keyboard_modifiers.shift_key(),
                        control: self.keyboard_modifiers.control_key(),
                    };
                    if let Some(controller) = self.running_chat_controller_mut() {
                        controller.handle_edit_key_down(
                            InputDialogEditKey::Backspace,
                            modifiers,
                            &layout,
                            &fonts.text,
                        );
                    }
                }
            }
            key => {
                let operation = match key {
                    VirtualKeyCode::Delete => Some(InputDialogEditKey::Delete),
                    VirtualKeyCode::ArrowLeft => Some(InputDialogEditKey::Left),
                    VirtualKeyCode::ArrowRight => Some(InputDialogEditKey::Right),
                    VirtualKeyCode::Home => Some(InputDialogEditKey::Home),
                    VirtualKeyCode::End => Some(InputDialogEditKey::End),
                    _ => None,
                };
                if let (Some(operation), Some(layout), Some(fonts)) = (
                    operation,
                    self.game_option_input_layout(),
                    self.assets.clonk_fonts.clone(),
                ) {
                    let modifiers = InputDialogKeyModifiers {
                        shift: self.keyboard_modifiers.shift_key(),
                        control: self.keyboard_modifiers.control_key(),
                    };
                    if let Some(controller) = self.running_chat_controller_mut() {
                        controller.handle_edit_key_down(operation, modifiers, &layout, &fonts.text);
                    }
                }
            }
        }
        Ok(true)
    }

    pub(crate) fn control_message_mentions_local_nick(&self, control: &MessageControlData) -> bool {
        let local_client = self
            .network
            .as_ref()
            .and_then(|network| i32::try_from(network.local_client_id()).ok())
            .unwrap_or(0);
        if local_client == control.by_client {
            return false;
        }
        self.control_clients
            .state(local_client)
            .is_some_and(|client| mentions_nick(control.message.as_bytes(), client.nick.as_bytes()))
    }

    pub(crate) fn external_irc_dialog_contains_point(&self, point: GuiPoint) -> bool {
        let Some(bounds) = self
            .external_irc_dialog
            .as_ref()
            .and_then(|dialog| dialog.chat_bounds_override())
        else {
            return false;
        };
        point.x >= bounds.x as f32
            && point.x < bounds.x.saturating_add(bounds.w) as f32
            && point.y >= bounds.y as f32
            && point.y < bounds.y.saturating_add(bounds.h) as f32
    }

    pub(crate) fn handle_runtime_external_irc_pointer_move(
        &mut self,
        point: GuiPoint,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running)
            || !self.external_irc_dialog_visible
            || (!self.external_irc_dialog_contains_point(point)
                && !self.external_irc_pointer_capture)
        {
            return Ok(false);
        }
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
        Ok(true)
    }

    pub(crate) fn handle_runtime_external_irc_pointer_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) || !self.external_irc_dialog_visible {
            self.external_irc_pointer_capture = false;
            return Ok(false);
        }
        let release_captured = button_state == ElementState::Released
            && std::mem::take(&mut self.external_irc_pointer_capture);
        let Some(point) = self.running_pointer_position else {
            return Ok(release_captured);
        };
        if !release_captured && !self.external_irc_dialog_contains_point(point) {
            return Ok(false);
        }
        if button_state == ElementState::Pressed {
            self.external_irc_pointer_capture = true;
            self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        }
        let Some(fonts) = self.assets.clonk_fonts.clone() else {
            return Ok(true);
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
        Ok(true)
    }

    pub(crate) fn show_external_irc_dialog(&mut self) -> Result<(), EngineError> {
        self.guard_classic_global_gui_bootstrap()?;
        let resource_check = self
            .assets
            .netdlg_assets()
            .context("exact C4ChatDlg graphics are unavailable")
            .and_then(|_| {
                self.assets
                    .clonk_fonts
                    .as_ref()
                    .context("C4ChatDlg fonts are unavailable")
                    .map(|_| ())
            });
        Self::guard_gui_overlay_result("C4ChatDlg", resource_check)?;
        if self.mode == AppMode::Running {
            self.cancel_ingame_mouse_gestures();
            self.menu_title_drag = None;
            self.ingame_menu_close_pointer_capture = None;
            self.script_menu_close_pointer_capture = None;
        }
        self.close_context_menu_silently();
        if self.running_chat_controller().is_some() {
            self.close_running_chat()?;
        }
        if self.external_irc_dialog_visible && self.external_irc_dialog.is_some() {
            self.sync_startup_irc_snapshot();
            return Ok(());
        }
        let (width, height) = {
            let surface = self.graphics.surface();
            (surface.width() as i32, surface.height() as i32)
        };
        let bounds =
            clonk_frontend::startup_netdlg::NetDlgController::standalone_chat_bounds(width, height);
        let active = self.startup_irc_client_active();
        let history = self.message_input_history.iter().cloned().collect();
        let mut dialog = self.new_network_dialog_controller();
        dialog.resize(width, height);
        dialog.set_chat_bounds_override(Some(bounds));
        dialog.set_chat_history(history);
        if !active {
            dialog.show_chat_login();
        }
        self.external_irc_dialog = Some(dialog);
        self.external_irc_dialog_visible = true;
        self.show_or_raise_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        self.irc_dialog_last_click = None;
        self.sync_startup_irc_snapshot();
        if let Some(dialog) = self.external_irc_dialog.as_mut() {
            // The standalone C4ChatControl picks its initial focus after the
            // process-global IRC projection has selected Login versus Chats.
            dialog.force_chat_mode_and_default_focus();
            dialog.pointer_left();
        }
        Ok(())
    }

    pub(crate) fn hide_external_irc_dialog(&mut self) {
        self.close_context_menu_silently();
        self.external_irc_dialog_visible = false;
        self.external_irc_dialog = None;
        self.hide_runtime_default_dialog(RuntimeDefaultDialog::ExternalIrc);
        self.irc_dialog_last_click = None;
        self.external_irc_pointer_capture = false;
    }

    fn toggle_external_irc_dialog(&mut self) -> Result<(), EngineError> {
        if self.external_irc_dialog_visible {
            self.hide_external_irc_dialog();
            Ok(())
        } else {
            self.show_external_irc_dialog()
        }
    }

    pub(crate) fn show_irc_login_on_all_controllers(&mut self) {
        if let Some(dialog) = self.startup_network_dialog.as_mut() {
            dialog.show_chat_login();
        }
        if let Some(dialog) = self.external_irc_dialog.as_mut() {
            dialog.show_chat_login();
        }
    }

    pub(crate) fn localized_irc_chat_strings(
        &self,
    ) -> clonk_frontend::startup_netdlg::NetDlgChatStrings {
        let command_template = |key: &str, fallback: &str| {
            self.runtime_resource_text(key, fallback)
                .replace("%s", "{command}")
        };
        clonk_frontend::startup_netdlg::NetDlgChatStrings {
            chat: self.runtime_resource_text("IDS_DLG_CHAT", "Chat"),
            not_connected: self.runtime_resource_text("IDS_CHAT_NOTCONNECTED", "not connected"),
            server: self.runtime_resource_text("IDS_CHAT_SERVER", "Server"),
            nick: self.runtime_resource_text("IDS_CTL_NICK", "Nickname:"),
            password_optional: self
                .runtime_resource_text("IDS_CTL_PASSWORDOPTIONAL", "Password (optional):"),
            real_name: self.runtime_resource_text("IDS_CTL_REALNAME", "Real name:"),
            channel: self.runtime_resource_text("IDS_CTL_CHANNEL", "Channel:"),
            connect: self.runtime_resource_text("IDS_BTN_CONNECT", "Connect"),
            not_connected_error: self
                .runtime_resource_text("IDS_ERR_NOTCONNECTEDTOSERVER", "Not connected to server."),
            insufficient_parameters: command_template(
                "IDS_ERR_INSUFFICIENTPARAMETERS",
                "/%s: insufficient parameters",
            ),
            invalid_nick: command_template("IDS_ERR_INVALIDNICKNAME2", "/%s: invalid nick name"),
            unknown_command: command_template(
                "IDS_ERR_UNKNOWNCMD",
                "Unknown command: \"%s\" - type /help to get a list of valid commands",
            ),
            not_on_channel: self
                .runtime_resource_text("IDS_ERR_NOTONACHANNEL", "Not on a channel."),
            connecting: self.runtime_resource_text("IDS_NET_CONNECTING", "Connecting to %s at %s"),
        }
    }

    /// Projects the active language table onto the IRC transport's status
    /// lines. C++ resolves each key through `LoadResStr` where
    /// `C4Network2IRCClient` pushes the message (C4Network2IRC.cpp:224-421).
    pub(crate) fn localized_irc_status_templates(&self) -> clonk_network::IrcStatusTemplates {
        let template =
            |key: &str, fallback: &str| self.runtime_resource_text(key, fallback).into_bytes();
        clonk_network::IrcStatusTemplates {
            disconnected_from_server: template(
                "IDS_MSG_DISCONNECTEDFROMSERVER",
                "Disconnected from server (%s).",
            ),
            you_joined_channel: template(
                "IDS_MSG_YOUHAVEJOINEDCHANNEL",
                "You have joined channel %s.",
            ),
            has_joined_channel: template(
                "IDS_MSG_HASJOINEDTHECHANNEL",
                "%s has joined the channel.",
            ),
            you_left_channel: template(
                "IDS_MSG_YOUHAVELEFTCHANNEL",
                "You have left channel %s (%s).",
            ),
            has_left_channel: template("IDS_MSG_HASLEFTTHECHANNEL", "%s has left the channel (%s)"),
            you_were_kicked: template(
                "IDS_MSG_YOUWEREKICKEDFROMCHANNEL",
                "You were kicked from channel %s (%s).",
            ),
            was_kicked: template(
                "IDS_MSG_WASKICKEDFROMTHECHANNEL",
                "%s was kicked from the channel (%s).",
            ),
            has_disconnected: template("IDS_MSG_HASDISCONNECTED", "%s has disconnected (%s)."),
            changes_topic: template("IDS_MSG_CHANGESTHETOPICTO", "%s changes the topic to: %s"),
            sets_mode: template("IDS_MSG_SETSMODE", "%s sets mode %s %s"),
            is_now_known_as: template("IDS_MSG_ISNOWKNOWNAS", "%s is now known as %s"),
            topic_in: template("IDS_MSG_TOPICIN", "Topic in %s: %s"),
        }
    }

    pub(crate) fn render_external_irc_dialog(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if !self.external_irc_dialog_visible {
            return Ok(());
        }
        let assets = self
            .assets
            .netdlg_assets()
            .context("exact C4ChatDlg graphics are unavailable")?;
        let fonts = self
            .assets
            .clonk_fonts
            .clone()
            .context("C4ChatDlg fonts are unavailable")?;
        let controller = self
            .external_irc_dialog
            .as_ref()
            .context("C4ChatDlg controller is unavailable")?;
        let draw_focus = self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc)
            && self.message_dialogs.is_empty()
            && self.context_menu.is_none();
        clonk_frontend::startup_netdlg::NetDlgScreen::render_standalone_chat_dialog(
            self.graphics.surface_mut(),
            &assets,
            &fonts,
            gamma,
            controller,
            draw_focus,
        );
        Ok(())
    }

    pub(crate) fn render_external_irc_dialog_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<bool> {
        if !self.external_irc_dialog_visible
            || !self.runtime_default_dialog_is_top(RuntimeDefaultDialog::ExternalIrc)
            || !self.message_dialogs.is_empty()
            || self.context_menu.is_some()
        {
            return Ok(false);
        }
        let Some(pointer) = self.startup_tooltip.eligible_pointer() else {
            return Ok(false);
        };
        let Some(target) = self
            .external_irc_dialog
            .as_ref()
            .and_then(|dialog| dialog.tooltip_at(pointer))
        else {
            return Ok(false);
        };
        let text = self.resolve_startup_tooltip_text(target);
        if text.is_empty() {
            return Ok(false);
        }
        let font = self
            .assets
            .global_tooltip_font
            .as_deref()
            .context("classic shadowless tooltip font is unavailable")?;
        clonk_frontend::context_menu::draw_classic_tooltip(
            self.graphics.surface_mut(),
            font,
            pointer,
            &text,
            gamma,
        );
        Ok(true)
    }

    pub(crate) fn render_running_chat_layer(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
        ordered_native: bool,
    ) -> Result<()> {
        let Some(controller) = self.game_option_input_dialog.as_ref().and_then(|dialog| {
            (dialog.purpose == PendingInputDialogPurpose::RunningChat).then_some(&dialog.controller)
        }) else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .input_dialog_resources()
            .context("classic C4GUI::InputDialog resources are unavailable")?;
        let keyboard_active = self.context_menu.is_none() && self.running_chat_active();
        let mouse_active = self.context_menu.is_none();
        controller.render_with_activity(
            self.graphics.surface_mut(),
            &resources,
            keyboard_active,
            mouse_active,
            gamma,
        )?;
        if ordered_native {
            self.next_pending_native_overlay();
        }
        Ok(())
    }

    pub(crate) fn render_running_chat_tooltip(
        &mut self,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) -> Result<()> {
        if self.mode == AppMode::Running {
            let Some(pointer) = self.startup_tooltip.eligible_pointer() else {
                return Ok(());
            };
            if self.network_chart_is_elevated_pointer_layer()
                && self.network_chart_contains_point(pointer)
            {
                return Ok(());
            }
            if self.top_scoreboard_message_pointer_target_cached(pointer)
                != Some(RunningDialogStackEntry::Chat)
            {
                return Ok(());
            }
        }
        let Some(controller) = self.game_option_input_dialog.as_ref().and_then(|dialog| {
            (dialog.purpose == PendingInputDialogPurpose::RunningChat).then_some(&dialog.controller)
        }) else {
            return Ok(());
        };
        let assets = Arc::clone(&self.assets);
        let resources = assets
            .input_dialog_resources()
            .context("classic C4GUI::InputDialog resources are unavailable")?;
        let mouse_active = self.context_menu.is_none();
        controller.render_tooltip(self.graphics.surface_mut(), &resources, mouse_active, gamma)
    }

    /// The C4MessageBoard line selected by its live LogBuffer cursor.
    pub(crate) fn message_board_line(&self) -> Option<String> {
        self.message_board.current_line()
    }

    pub(crate) fn advance_message_board_overlay(&mut self) -> MessageBoardOverlay {
        let line_height = self.graphics.message_board_line_height();
        let type_in = self.running_chat_active();
        self.message_board.advance_frame(line_height, type_in)
    }

    pub(crate) fn enqueue_control_message_board_line(&mut self, line: String) {
        let game_time_seconds = self.game_time_seconds();
        self.graphics.set_upper_board_mode(
            frontend_upper_board_mode(self.display_flags.upper_board),
            game_time_seconds,
        );
        for physical_line in self.graphics.prepare_message_board_lines(&line) {
            self.message_board.enqueue(physical_line);
        }
    }

    /// `C4LogSystem::GuiSink::DoLog` (`src/C4Log.cpp:226-240`): every logged
    /// line reaches `C4MessageBoard::AddLog` plus `LogNotify` while the board
    /// is active. `AddLog` stamps the configured timestamp and appends to the
    /// log buffer; an inactive board discards the line
    /// (`src/C4MessageBoard.cpp:327-347,354-366`). The board is constructed by
    /// `C4MessageBoard::Init` for the running game and released with it, so
    /// menu-time script logs stay in the session log and developer console.
    pub(crate) fn drain_game_log_capture(&mut self) {
        let Some(lines) = self
            .game_log_capture
            .as_ref()
            .map(clonk_logging::GameLogCapture::take)
            .filter(|lines| !lines.is_empty())
        else {
            return;
        };
        if !matches!(self.mode, AppMode::Running) {
            return;
        }
        for line in lines {
            let line = self.timestamp_log_line(line);
            self.enqueue_control_message_board_line(line);
        }
    }

    pub(crate) fn scroll_message_board(&mut self, older: bool) {
        self.message_board.scroll(older);
    }

    pub(crate) fn clear_message_board_log(&mut self) {
        self.message_board.clear_log();
    }

    pub(crate) fn set_message_board_line_count(&mut self, line_count: i32) {
        let line_height = self.graphics.message_board_line_height();
        let enabled = self.message_board.set_line_count(line_count, line_height);
        if let Some(paths) = self.app_paths.as_ref() {
            if let Err(error) = persist_config_value(
                paths,
                "Graphics",
                "MsgBoard",
                i32::from(enabled).to_string(),
            ) {
                tracing::warn!(%error, "failed to persist Graphics.MsgBoard");
            }
        }
    }

    pub(crate) fn message_board_overlay(&mut self) -> MessageBoardOverlay {
        let line_height = self.graphics.message_board_line_height();
        let type_in = self.running_chat_active();
        self.message_board.overlay(line_height, type_in)
    }
}
