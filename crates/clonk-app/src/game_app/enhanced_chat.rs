//! Chat presentation. Messages still use the classic control path.

use super::*;
use clonk_frontend::enhanced_chat::{ChatAudience, ChatChannel, ChatMessage, EnhancedChat};
use clonk_frontend::enhanced_chat_view::{self as view, ChatLayout, ChatView};

impl GameApp {
    pub(crate) fn submit_enhanced_chat_text(&mut self, text: String) -> Result<(), EngineError> {
        // Preserve classic control/record order: close and clear pressed controls
        // before dispatching the message (C4MessageInput.cpp:117-157). Keep a local
        // copy solely to reopen the composer when submission fails.
        let saved_running = self.chat.running.clone();
        let mut saved_dialog = self.dialogs.game_option_input.clone();
        if let Some(dialog) = saved_dialog.as_mut() {
            dialog.controller.cancel_interaction();
        }
        self.finalize_running_chat_input()?;
        self.chat.audience_picker = false;
        match self.try_submit_enhanced_chat(&text) {
            Ok(()) => {
                self.chat.enhanced.remember_sent(&text);
                self.chat.enhanced.save_draft("");
                self.chat.enhanced.error.clear();
                self.chat.enhanced.jump_to_latest();
            }
            Err(error) => {
                self.chat.enhanced.save_draft(&text);
                self.chat.enhanced.error = error;
                if self.chat.running.is_none() && self.dialogs.game_option_input.is_none() {
                    self.chat.running = saved_running;
                    self.dialogs.game_option_input = saved_dialog;
                    self.suspend_ingame_pointer_for_gui();
                    self.show_running_dialog(RunningDialogStackEntry::Chat);
                }
            }
        }
        Ok(())
    }

    fn try_submit_enhanced_chat(&mut self, text: &str) -> std::result::Result<(), String> {
        if text.trim().is_empty() {
            return Err("Type a message before sending.".into());
        }
        if self.process_control_message_local_command(text) {
            self.chat.enhanced.show_logs = true;
            return Ok(());
        }
        let player = self
            .snapshot
            .hud
            .local_players
            .first()
            .copied()
            .unwrap_or(-1);
        let parsed = parse_running_message_control(
            text,
            player,
            self.engine.cinematic_film(),
            &self.snapshot,
        );
        let mut control = match parsed {
            Ok(Some(control)) => control,
            Ok(None) => {
                return Err("Message not sent. Check the recipient and message text.".into())
            }
            Err(_) if text.starts_with('/') => {
                return match self
                    .process_running_chat_command(text)
                    .map_err(|error| error.to_string())?
                {
                    true => {
                        self.chat.enhanced.show_logs = true;
                        Ok(())
                    }
                    false => Err("Unknown command. Type /help for available commands.".into()),
                };
            }
            Err(error) => return Err(error.to_string()),
        };
        if control.message_type == MESSAGE_TYPE_NORMAL {
            match self.chat.enhanced.audience {
                ChatAudience::Everyone => {}
                ChatAudience::Allies => control.message_type = MESSAGE_TYPE_TEAM,
                ChatAudience::Private(id) => {
                    if !self
                        .snapshot
                        .players
                        .iter()
                        .any(|candidate| candidate.id == id)
                    {
                        return Err("Recipient left the game. Choose another recipient.".into());
                    }
                    control.message_type = MESSAGE_TYPE_PRIVATE;
                    control.to_player = id;
                }
                ChatAudience::Say => {
                    control = parse_running_message_control(
                        &format!("\"{text}"),
                        player,
                        self.engine.cinematic_film(),
                        &self.snapshot,
                    )
                    .map_err(|error| error.to_string())?
                    .ok_or_else(|| "Type a message before sending.".to_string())?;
                }
            }
        }
        if control.message_type == MESSAGE_TYPE_TEAM && self.engine.team_distribution() == 4 {
            return Err("Cannot send to allies: teams are not known yet.".into());
        }
        if let Some(network) = self.netplay.manager.as_ref() {
            network
                .submit_message(control)
                .map_err(|error| format!("Not sent: {error}"))?;
        } else {
            control.by_client = 0;
            self.record_control_packet(&clonk_engine::ControlPacket::Message(control.clone()));
            self.execute_message_control(control);
        }
        Ok(())
    }

    pub(crate) fn enhanced_chat_audiences(&self) -> Vec<(ChatAudience, String)> {
        let mut choices = vec![
            (ChatAudience::Everyone, "Everyone".into()),
            (ChatAudience::Allies, "Allies".into()),
            (ChatAudience::Say, "Say above crew".into()),
        ];
        let local = &self.snapshot.hud.local_players;
        choices.extend(
            self.snapshot
                .players
                .iter()
                .filter(|player| !local.contains(&player.id))
                .map(|player| {
                    (
                        ChatAudience::Private(player.id),
                        format!("Private: {}", player.name),
                    )
                }),
        );
        choices
    }

    /// The composer's chip: where the typed text will actually go. Explicit
    /// legacy syntax such as `/team` remains usable and the chip follows it.
    fn enhanced_chat_chip(&self) -> (String, [u8; 4]) {
        let text = self.running_chat_text().unwrap_or_default();
        let routed = match parse_running_message_control(text, 0, false, &self.snapshot) {
            Ok(Some(control)) => match control.message_type {
                MESSAGE_TYPE_TEAM => Some(ChatAudience::Allies),
                MESSAGE_TYPE_PRIVATE => Some(ChatAudience::Private(control.to_player)),
                MESSAGE_TYPE_SAY => Some(ChatAudience::Say),
                MESSAGE_TYPE_ME | MESSAGE_TYPE_SOUND | MESSAGE_TYPE_ALERT => {
                    return ("Command".into(), view::COMMAND_COLOR)
                }
                _ => None,
            },
            _ => None,
        };
        if routed.is_none() && text.starts_with('/') {
            return ("Command".into(), view::COMMAND_COLOR);
        }
        let audience = routed.unwrap_or_else(|| self.chat.enhanced.audience.clone());
        let label = match &audience {
            ChatAudience::Everyone => "Everyone".into(),
            ChatAudience::Allies => "Allies".into(),
            ChatAudience::Say => "Say".into(),
            ChatAudience::Private(id) => self
                .enhanced_chat_player_name(*id)
                .map(|name| format!("To {name}"))
                .unwrap_or_else(|| "Recipient left".into()),
        };
        (label, view::audience_color(&audience))
    }

    fn enhanced_chat_player_name(&self, id: i32) -> Option<&str> {
        self.snapshot
            .players
            .iter()
            .find(|player| player.id == id)
            .map(|player| player.name.as_str())
    }

    /// Names who the empty composer's message will reach.
    pub(crate) fn refresh_enhanced_chat_placeholder(&mut self) {
        let placeholder = match &self.chat.enhanced.audience {
            ChatAudience::Everyone => "Message everyone".into(),
            ChatAudience::Allies => "Message your allies".into(),
            ChatAudience::Say => "Say above your crew".into(),
            ChatAudience::Private(id) => self
                .enhanced_chat_player_name(*id)
                .map(|name| format!("Message {name} privately"))
                .unwrap_or_else(|| "Choose a recipient".into()),
        };
        if let Some(controller) = self.running_chat_controller_mut() {
            controller.set_placeholder(placeholder);
        }
    }

    pub(crate) fn replace_enhanced_chat_text(&mut self, text: &str) {
        let layout = self.game_option_input_layout();
        let fonts = self.assets.clonk_fonts.clone();
        if let (Some(layout), Some(fonts), Some(controller)) =
            (layout, fonts, self.running_chat_controller_mut())
        {
            controller.replace_completion(text, text.len(), &layout, &fonts.text);
        }
    }

    pub(crate) fn select_enhanced_chat_audience(&mut self, audience: ChatAudience) {
        let text = self.running_chat_text().unwrap_or_default().to_string();
        let draft = self.chat.enhanced.switch_audience(audience, &text);
        self.replace_enhanced_chat_text(&draft);
        self.refresh_enhanced_chat_placeholder();
        self.chat.enhanced.error.clear();
        self.chat.audience_picker = false;
    }

    fn enhanced_chat_candidates(&self, text: &str) -> Vec<String> {
        if text.starts_with('/') && !text.contains(char::is_whitespace) {
            [
                "/help",
                "/team",
                "/private",
                "/me",
                "/sound",
                "/alert",
                "/clear",
                "/msgboard",
                "/kick",
                "/fast",
                "/slow",
                "/script",
                "/chart",
                "/mute",
                "/unmute",
                "/activate",
                "/deactivate",
                "/observer",
                "/nodebug",
                "/centralctrl",
                "/decentralctrl",
                "/asyncctrl",
                "/set",
                "/netgetscen",
            ]
            .into_iter()
            .map(str::to_string)
            .collect()
        } else {
            self.snapshot
                .players
                .iter()
                .map(|player| player.name.clone())
                .collect()
        }
    }

    fn enhanced_chat_hint(&self) -> String {
        let Some(controller) = self.running_chat_controller() else {
            return String::new();
        };
        let text = controller.text();
        if let Some(command) = text
            .split_whitespace()
            .next()
            .filter(|_| text.contains(' '))
        {
            let hint = match command {
                "/private" => Some("/private <name> <message> · or choose a recipient above"),
                "/team" => Some("/team <message> · visible to allies"),
                "/me" => Some("/me <action> · visible to everyone"),
                "/sound" => Some("/sound <name> · play a chat sound"),
                "/msgboard" => Some("/msgboard <0–20> · classic message board lines"),
                "/kick" => Some("/kick <client name> · host only"),
                "/script" => Some("/script <code> · execute a script command"),
                _ => None,
            };
            if let Some(hint) = hint {
                return hint.into();
            }
        }
        let prefix = EnhancedChat::completion_prefix(text, controller.caret());
        let matches = self
            .enhanced_chat_candidates(text)
            .into_iter()
            .filter(|candidate| {
                !prefix.is_empty() && candidate.to_lowercase().starts_with(&prefix.to_lowercase())
            })
            .take(4)
            .collect::<Vec<_>>();
        if matches.is_empty() {
            "Enter to send · Esc to close".into()
        } else {
            format!("Tab / Shift+Tab: {}", matches.join(" · "))
        }
    }

    pub(crate) fn enhanced_chat_layout(&self, expanded: bool) -> Option<ChatLayout> {
        let fonts = self.assets.clonk_fonts.as_deref()?;
        let surface = self.rendering.graphics.surface();
        Some(ChatLayout::new(
            surface.width() as i32,
            surface.height() as i32,
            self.chat.enhanced_preferences.font(fonts).line_height,
            expanded,
        ))
    }

    pub(crate) fn scroll_enhanced_chat(&mut self, older: bool) {
        if self.chat.audience_picker {
            let count = self.enhanced_chat_audiences().len();
            self.chat.audience_picker_offset = if older {
                self.chat.audience_picker_offset.saturating_sub(1)
            } else {
                (self.chat.audience_picker_offset + 1).min(count.saturating_sub(1))
            };
            return;
        }
        let Some(layout) = self.enhanced_chat_layout(true) else {
            return;
        };
        let Some(fonts) = self.assets.clonk_fonts.as_deref() else {
            return;
        };
        let font = self.chat.enhanced_preferences.font(fonts);
        let counts = self
            .chat
            .enhanced
            .matching_messages()
            .into_iter()
            .map(|message| {
                (
                    message.id,
                    view::message_lines(
                        font,
                        message,
                        layout.feed.w,
                        self.chat.show_log_timestamps,
                        self.rendering.display_flags.white_chat,
                    )
                    .len(),
                )
            })
            .collect::<Vec<_>>();
        self.chat.enhanced.scroll_wrapped(older, &counts);
    }

    pub(crate) fn handle_enhanced_chat_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> bool {
        if !self.enhanced_chat_active() {
            return false;
        }
        let modifiers = self.input_routing.live.modifiers;
        let handled = matches!(
            key,
            VirtualKeyCode::Tab | VirtualKeyCode::PageUp | VirtualKeyCode::PageDown
        ) || key == VirtualKeyCode::End && modifiers.control_key()
            || key == VirtualKeyCode::KeyL && modifiers.control_key()
            || key == VirtualKeyCode::Escape && self.chat.audience_picker
            || key == VirtualKeyCode::Backspace
                && self.running_chat_text().is_none_or(str::is_empty);
        if !handled || modifiers.alt_key() {
            return false;
        }
        if state == ElementState::Released {
            return true;
        }
        match key {
            VirtualKeyCode::Tab if modifiers.control_key() => {
                let choices = self.enhanced_chat_audiences();
                let index = choices
                    .iter()
                    .position(|(audience, _)| *audience == self.chat.enhanced.audience)
                    .unwrap_or(0);
                let next = (index
                    + if modifiers.shift_key() {
                        choices.len() - 1
                    } else {
                        1
                    })
                    % choices.len();
                self.select_enhanced_chat_audience(choices[next].0.clone());
            }
            VirtualKeyCode::Tab => {
                let Some(controller) = self.running_chat_controller() else {
                    return true;
                };
                if controller.composition().is_some() {
                    return true;
                }
                let text = controller.text().to_string();
                let caret = controller.caret();
                let candidates = self.enhanced_chat_candidates(&text);
                if let Some((text, caret)) =
                    self.chat
                        .enhanced
                        .complete(&text, caret, &candidates, modifiers.shift_key())
                {
                    let layout = self.game_option_input_layout();
                    let fonts = self.assets.clonk_fonts.clone();
                    if let (Some(layout), Some(fonts), Some(controller)) =
                        (layout, fonts, self.running_chat_controller_mut())
                    {
                        controller.replace_completion(&text, caret, &layout, &fonts.text);
                    }
                }
            }
            VirtualKeyCode::PageUp | VirtualKeyCode::PageDown => {
                for _ in 0..3 {
                    self.scroll_enhanced_chat(key == VirtualKeyCode::PageUp);
                }
            }
            VirtualKeyCode::End => self.chat.enhanced.jump_to_latest(),
            VirtualKeyCode::KeyL => self.chat.enhanced.toggle_logs(),
            VirtualKeyCode::Escape => self.chat.audience_picker = false,
            _ => {}
        }
        true
    }

    pub(crate) fn handle_enhanced_chat_pointer(
        &mut self,
        state: ElementState,
    ) -> Result<bool, EngineError> {
        if !self.enhanced_chat_active() {
            return Ok(false);
        }
        let Some((point, layout)) = self
            .dialogs
            .game_option_input_pointer_position
            .zip(self.enhanced_chat_layout(true))
        else {
            return Ok(false);
        };
        if view::contains(layout.edit, point) {
            return Ok(false);
        }
        if !view::contains(layout.bounds, point) {
            return Ok(false);
        }
        if state == ElementState::Released {
            return Ok(true);
        }
        if self.chat.audience_picker {
            let audiences = self.enhanced_chat_audiences();
            if view::contains(layout.picker(audiences.len()), point) {
                if let Some((audience, _)) = self
                    .enhanced_chat_picker_row_at_pointer(audiences.len())
                    .and_then(|index| audiences.get(index))
                {
                    self.select_enhanced_chat_audience(audience.clone());
                }
                return Ok(true);
            }
            // A click anywhere else closes the list, as any drop-down does;
            // the chip below toggles it rather than reopening it.
            if !view::contains(layout.audience, point) {
                self.chat.audience_picker = false;
            }
        }
        if view::contains(layout.close, point) {
            self.close_running_chat()?;
            self.chat.dismiss_pointer_release = true;
        } else if view::contains(layout.settings_button, point) {
            self.close_running_chat()?;
            self.chat.dismiss_pointer_release = true;
            self.open_unified_settings(
                clonk_frontend::settings_overlay::SettingsCategory::Interface,
            )?;
        } else if view::contains(layout.overlay_toggle, point) {
            self.chat.enhanced.hidden = !self.chat.enhanced.hidden;
        } else if view::contains(layout.chat_tab, point) || view::contains(layout.all_tab, point) {
            self.chat
                .enhanced
                .show_game_messages(view::contains(layout.all_tab, point));
            self.chat.audience_picker = false;
        } else if view::contains(layout.audience, point) {
            self.chat.audience_picker = !self.chat.audience_picker;
            self.chat.audience_picker_offset = 0;
        } else if view::contains(layout.latest, point) {
            self.chat.enhanced.jump_to_latest();
        }
        Ok(true)
    }

    /// The recipient under the pointer, as an index into all recipients.
    fn enhanced_chat_picker_row_at_pointer(&self, rows: usize) -> Option<usize> {
        let point = self.dialogs.game_option_input_pointer_position?;
        let layout = self.enhanced_chat_layout(true)?;
        (0..layout.picker_capacity(rows))
            .find(|row| view::contains(layout.picker_row(rows, *row), point))
            .map(|row| self.chat.audience_picker_offset + row)
            .filter(|index| *index < rows)
    }

    pub(crate) fn render_enhanced_chat(
        &mut self,
        expanded: bool,
        gamma: Option<&clonk_graphics::GammaRamp>,
    ) {
        let assets = Arc::clone(&self.assets);
        let Some(fonts) = assets.clonk_fonts.as_deref() else {
            return;
        };
        let (audience, audience_color) = self.enhanced_chat_chip();
        let hint = self.enhanced_chat_hint();
        let notice = self
            .running_chat_controller()
            .map(|controller| controller.enhanced_notice().to_string())
            .unwrap_or_default();
        view::render_chat(
            self.rendering.graphics.surface_mut(),
            fonts,
            &self.chat.enhanced,
            &ChatView {
                preferences: &self.chat.enhanced_preferences,
                expanded,
                audience: &audience,
                audience_color,
                hint: &hint,
                notice: &notice,
                timestamps: self.chat.show_log_timestamps,
                white_text: self.rendering.display_flags.white_chat,
                now: Instant::now(),
            },
            gamma,
        );
        if expanded && self.chat.audience_picker {
            let audiences = self.enhanced_chat_audiences();
            let selected = audiences
                .iter()
                .position(|(audience, _)| *audience == self.chat.enhanced.audience);
            let hovered = self.enhanced_chat_picker_row_at_pointer(audiences.len());
            let choices = audiences
                .into_iter()
                .map(|(audience, label)| (label, view::audience_color(&audience)))
                .collect::<Vec<_>>();
            view::render_audience_picker(
                self.rendering.graphics.surface_mut(),
                fonts,
                &self.chat.enhanced_preferences,
                &choices,
                self.chat.audience_picker_offset,
                selected,
                hovered,
                gamma,
            );
        }
    }

    /// Called only after the classic message visibility/authentication checks.
    pub(crate) fn append_control_chat_log(
        &mut self,
        line: String,
        color: u32,
        lobby_sender: Option<i32>,
        control: &MessageControlData,
    ) {
        if self.mode == AppMode::Running && self.chat.enhanced_preferences.enabled {
            let sender = self
                .engine
                .player(control.player)
                .map(|player| c4_presentation_text(player.name()))
                .or_else(|| {
                    self.netplay
                        .control_clients
                        .state(control.by_client)
                        .map(|client| legacy_presentation_text(client.nick.as_bytes()))
                })
                .unwrap_or_else(|| "???".into());
            let channel = match control.message_type {
                MESSAGE_TYPE_TEAM => ChatChannel::Allies,
                MESSAGE_TYPE_PRIVATE => ChatChannel::Private,
                MESSAGE_TYPE_ME => ChatChannel::Action,
                _ => ChatChannel::Everyone,
            };
            let mut message = ChatMessage::conversation(
                &sender,
                channel,
                &legacy_presentation_text(control.message.as_bytes()),
            );
            message.color = self
                .engine
                .player(control.player)
                .and_then(|player| player.color())
                .map(|color| {
                    clonk_frontend::game_lobby::make_color_readable_on_black(
                        (u32::from(color.r) << 16) | (u32::from(color.g) << 8) | u32::from(color.b),
                    )
                })
                .unwrap_or([180, 215, 255, 255]);
            self.chat.pending_chat_message = Some(message);
        }
        self.append_control_message_log(line, color, lobby_sender);
        self.chat.pending_chat_message = None;
    }
}
